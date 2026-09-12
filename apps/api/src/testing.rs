//! Test fixtures, shared by the integration suites.
//!
//! Compiled into the library rather than gated behind `#[cfg(test)]` because
//! integration tests link this crate as an external dependency and so cannot
//! see its test-only items. It pulls in no extra dependencies.
//!
//! Everything here uses [`MemoryKv`], so the suite runs with no Redis. The
//! Redis backend is covered separately by the tests in [`crate::store`], which
//! skip unless `REDIS_URL` is set.

use std::sync::Arc;
use std::time::Duration;

use std::sync::Mutex;

use crate::credentials::KvCredentialStore;
use crate::engine::{EngineFactory, ProviderCredentials};
use crate::github_api::{GitHubApi, GitHubApiError, GitHubRepoInfo};
use crate::github_push::GithubPushState;
use crate::github_token_store::GithubTokenStore;
use crate::killswitch::{KillSwitch, KillSwitchState};
use crate::routes::AppState;
use crate::scenario::captcha::CaptchaKeys;
use crate::scenario::ScenarioRegistry;
use crate::session::{DemoSessionStore, DEFAULT_TTL_HOURS};
use crate::settings::{CookieSameSite, GithubKitCredentials, RelyingParty, Settings, XffPosition};
use crate::store::{KeyValue, MemoryKv};

/// A `GitHubApi` that never makes a network call.
///
/// Not `#[cfg(test)]`-gated, for the same reason the rest of this file is
/// not: integration tests link this crate as an external dependency and
/// cannot see anything hidden behind that attribute. Every call succeeds with
/// a small, deterministic default unless a test overrides the relevant
/// `Mutex`-wrapped field — which also doubles as a recorder of what was sent,
/// so a test can assert the kit-to-blob mapping without a mock HTTP server.
#[derive(Default)]
pub struct FakeGitHubApi {
    pub exchange_result: Mutex<Option<Result<String, GitHubApiError>>>,
    pub current_user_result: Mutex<Option<Result<String, GitHubApiError>>>,
    pub create_repo_result: Mutex<Option<Result<GitHubRepoInfo, GitHubApiError>>>,
    pub create_blob_error: Mutex<Option<GitHubApiError>>,
    pub create_tree_error: Mutex<Option<GitHubApiError>>,
    pub create_commit_error: Mutex<Option<GitHubApiError>>,
    pub create_ref_error: Mutex<Option<GitHubApiError>>,

    /// The content of every blob created so far, in call order.
    pub blobs_created: Mutex<Vec<String>>,
    /// The `(path, blob_sha)` pairs of the most recently built tree, if any.
    pub tree_entries: Mutex<Option<Vec<(String, String)>>>,
    /// How many commits have been created.
    pub commit_calls: Mutex<u32>,
    /// Whether the most recent commit carried an empty `parents` list.
    pub last_commit_had_no_parents: Mutex<Option<bool>>,
    /// How many refs have been created.
    pub ref_calls: Mutex<u32>,
}

#[async_trait::async_trait]
impl GitHubApi for FakeGitHubApi {
    async fn exchange_code(
        &self,
        _client_id: &str,
        _client_secret: &str,
        _code: &str,
        _redirect_uri: &str,
    ) -> Result<String, GitHubApiError> {
        self.exchange_result
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok("fake-github-token".to_string()))
    }

    async fn current_user(&self, _token: &str) -> Result<String, GitHubApiError> {
        self.current_user_result
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok("fake-user".to_string()))
    }

    async fn create_repo(
        &self,
        _token: &str,
        name: &str,
        _description: Option<&str>,
    ) -> Result<GitHubRepoInfo, GitHubApiError> {
        self.create_repo_result
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| {
                Ok(GitHubRepoInfo {
                    name: name.to_string(),
                    html_url: format!("https://github.com/fake-user/{name}"),
                    default_branch: "main".to_string(),
                })
            })
    }

    async fn create_blob(
        &self,
        _token: &str,
        _owner: &str,
        _repo: &str,
        content: &str,
    ) -> Result<String, GitHubApiError> {
        if let Some(e) = self.create_blob_error.lock().unwrap().take() {
            return Err(e);
        }
        let mut blobs = self.blobs_created.lock().unwrap();
        blobs.push(content.to_string());
        Ok(format!("fake-blob-sha-{}", blobs.len()))
    }

    async fn create_tree(
        &self,
        _token: &str,
        _owner: &str,
        _repo: &str,
        entries: &[(String, String)],
    ) -> Result<String, GitHubApiError> {
        if let Some(e) = self.create_tree_error.lock().unwrap().take() {
            return Err(e);
        }
        *self.tree_entries.lock().unwrap() = Some(entries.to_vec());
        Ok("fake-tree-sha".to_string())
    }

    async fn create_commit(
        &self,
        _token: &str,
        _owner: &str,
        _repo: &str,
        _message: &str,
        _tree_sha: &str,
    ) -> Result<String, GitHubApiError> {
        if let Some(e) = self.create_commit_error.lock().unwrap().take() {
            return Err(e);
        }
        *self.commit_calls.lock().unwrap() += 1;
        // The real client always sends an empty `parents` array (the
        // repository is created empty), so the fake records that same fact
        // rather than the array's contents, which it never receives.
        *self.last_commit_had_no_parents.lock().unwrap() = Some(true);
        Ok("fake-commit-sha".to_string())
    }

    async fn create_ref(
        &self,
        _token: &str,
        _owner: &str,
        _repo: &str,
        _branch: &str,
        _sha: &str,
    ) -> Result<(), GitHubApiError> {
        if let Some(e) = self.create_ref_error.lock().unwrap().take() {
            return Err(e);
        }
        *self.ref_calls.lock().unwrap() += 1;
        Ok(())
    }
}

/// Settings suitable for tests: plain HTTP, localhost relying party.
pub fn test_settings(admin_token: Option<&str>) -> Settings {
    Settings {
        port: 0,
        cookie_secure: false,
        session_ttl_hours: DEFAULT_TTL_HOURS,
        admin_token: admin_token.map(|t| t.to_string()),
        allowed_origins: vec!["http://localhost:3000".to_string()],
        // Tests reach the router directly with no proxy in front, so they
        // identify callers by X-Forwarded-For rather than a trusted header.
        trusted_client_ip_header: None,
        relying_party: RelyingParty {
            id: "localhost".to_string(),
            origin: "http://localhost:3000".to_string(),
            name: "test".to_string(),
            extra_origins: Vec::new(),
        },
        xff_position: XffPosition::Rightmost,
        cookie_same_site: CookieSameSite::Lax,
        public_base_url: "http://localhost:8000".to_string(),
        github_kit: GithubKitCredentials::default(),
    }
}

/// Application state backed by an in-process store.
pub fn test_state(kill_switch: KillSwitch, admin_token: Option<&str>) -> AppState {
    test_state_with_settings(kill_switch, test_settings(admin_token))
}

/// As [`test_state`], but with OAuth providers configured — so the OAuth
/// control offers options and the auth routes will serve them.
pub fn test_state_with_providers(
    kill_switch: KillSwitch,
    providers: &[(&str, &str, &str)],
) -> AppState {
    let mut creds = ProviderCredentials::default();
    for (id, client_id, secret) in providers {
        creds.insert_for_test(id, client_id, secret);
    }
    build_state(
        kill_switch,
        test_settings(None),
        creds,
        CaptchaKeys::default(),
    )
}

/// As [`test_state_with_providers`], plus captcha site keys.
///
/// Both provider-select controls offer nothing without credentials, so a suite
/// that walks every scenario needs both halves supplied or the captcha scenario
/// passes vacuously.
///
/// The keys are fictional. Nothing here reaches a provider: `verify` is the
/// only path that would, and no test calls it against a live endpoint.
pub fn test_state_with_all_credentials(
    kill_switch: KillSwitch,
    providers: &[(&str, &str, &str)],
    captcha: &[(&str, &str, &str)],
) -> AppState {
    let mut creds = ProviderCredentials::default();
    for (id, client_id, secret) in providers {
        creds.insert_for_test(id, client_id, secret);
    }
    let mut keys = CaptchaKeys::default();
    for (id, site_key, secret) in captcha {
        keys.insert_for_test(id, site_key, secret);
    }
    build_state(kill_switch, test_settings(None), creds, keys)
}

/// As [`test_state`], with settings supplied by the caller.
pub fn test_state_with_settings(kill_switch: KillSwitch, settings: Settings) -> AppState {
    build_state(
        kill_switch,
        settings,
        ProviderCredentials::default(),
        CaptchaKeys::default(),
    )
}

/// As [`test_state`], but with the separate "push to GitHub" credentials
/// configured and a caller-supplied `GitHubApi` — a [`FakeGitHubApi`] in every
/// test in this suite, since no test may reach the real GitHub.
pub fn test_state_with_github(
    kill_switch: KillSwitch,
    client_id: &str,
    client_secret: &str,
    api: Arc<dyn GitHubApi>,
) -> AppState {
    let mut settings = test_settings(None);
    settings.github_kit = GithubKitCredentials::for_test(client_id, client_secret);
    build_state_with_github(
        kill_switch,
        settings,
        ProviderCredentials::default(),
        CaptchaKeys::default(),
        api,
    )
}

fn build_state(
    kill_switch: KillSwitch,
    settings: Settings,
    credentials: ProviderCredentials,
    captcha: CaptchaKeys,
) -> AppState {
    build_state_with_github(
        kill_switch,
        settings,
        credentials,
        captcha,
        Arc::new(FakeGitHubApi::default()),
    )
}

fn build_state_with_github(
    kill_switch: KillSwitch,
    settings: Settings,
    credentials: ProviderCredentials,
    captcha: CaptchaKeys,
    github_api: Arc<dyn GitHubApi>,
) -> AppState {
    let kv: Arc<dyn KeyValue> = Arc::new(MemoryKv::new());
    let ttl = Duration::from_secs((settings.session_ttl_hours.max(1) as u64) * 3600);
    let creds = KvCredentialStore::new(kv.clone(), ttl);
    let settings = Arc::new(settings);
    let configured = credentials.configured();
    let signing = Arc::new(crate::signing::SigningKeys::for_test(
        &settings.public_base_url,
    ));
    // The same key the state publishes, so the resource scenario validates
    // against what `/.well-known/jwks.json` actually serves.
    let registry = ScenarioRegistry::for_tests_from(crate::scenario::RegistryConfig {
        oauth_providers: configured,
        captcha,
        signing: signing.clone(),
    });

    AppState {
        sessions: Arc::new(DemoSessionStore::new(
            kv.clone(),
            registry,
            settings.session_ttl_hours,
            creds.clone(),
        )),
        kill_switch: Arc::new(kill_switch),
        engines: Arc::new(EngineFactory::new(credentials, false)),
        settings,
        credentials: Arc::new(creds),
        ceremonies: Arc::new(crate::ceremony::CeremonyStore::new(kv.clone())),
        events: Arc::new(crate::events::EventLog::new(kv.clone(), ttl)),
        signing,
        github_push: Arc::new(GithubPushState::new(
            github_api,
            GithubTokenStore::new(kv, crate::github_token_store::TOKEN_TTL),
        )),
    }
}

/// A router over freshly built test state.
pub fn test_app(kill_switch: KillSwitch, admin_token: Option<&str>) -> axum::Router {
    crate::build_router(test_state(kill_switch, admin_token))
}

/// Create a shared store for durability tests.
/// Multiple AppStates can be built over this same store to verify that state
/// persists across restarts.
pub fn shared_store() -> Arc<dyn KeyValue> {
    Arc::new(MemoryKv::new())
}

/// Build test state with a shared store and a custom kill switch state.
/// This is used to test that kill switch state survives across fresh AppState
/// constructions — simulating a cold start.
pub fn test_state_with_shared_store(
    store: Arc<dyn KeyValue>,
    init_state: KillSwitchState,
) -> AppState {
    test_state_with_shared_store_and_admin(store, init_state, None)
}

/// As [`test_state_with_shared_store`], but with an admin token, so the
/// `/admin/kill-switch` route is actually mounted.
///
/// Needed to exercise the path an operator really takes — flipping the switch
/// over HTTP rather than calling `set_state` directly — across a simulated cold
/// start. A handler that mutated a local copy and forgot to persist would pass
/// the direct test and fail this one.
pub fn test_state_with_shared_store_and_admin(
    store: Arc<dyn KeyValue>,
    init_state: KillSwitchState,
    admin_token: Option<&str>,
) -> AppState {
    let kill_switch = KillSwitch::new(Some(store.clone()), init_state);
    let settings = Arc::new(test_settings(admin_token));
    let ttl = Duration::from_secs((settings.session_ttl_hours.max(1) as u64) * 3600);
    let creds = KvCredentialStore::new(store.clone(), ttl);
    let signing = Arc::new(crate::signing::SigningKeys::for_test(
        &settings.public_base_url,
    ));
    let registry = ScenarioRegistry::for_tests_from(crate::scenario::RegistryConfig {
        signing: signing.clone(),
        ..Default::default()
    });

    AppState {
        sessions: Arc::new(DemoSessionStore::new(
            store.clone(),
            registry,
            settings.session_ttl_hours,
            creds.clone(),
        )),
        kill_switch: Arc::new(kill_switch),
        engines: Arc::new(EngineFactory::new(ProviderCredentials::default(), false)),
        settings,
        credentials: Arc::new(creds),
        ceremonies: Arc::new(crate::ceremony::CeremonyStore::new(store.clone())),
        events: Arc::new(crate::events::EventLog::new(store.clone(), ttl)),
        signing,
        github_push: Arc::new(GithubPushState::new(
            Arc::new(FakeGitHubApi::default()),
            GithubTokenStore::new(store, crate::github_token_store::TOKEN_TTL),
        )),
    }
}
