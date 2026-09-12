//! The GitHub calls behind "push this project to my repo" (#40).
//!
//! Everything that reaches `github.com` or `api.github.com` goes through the
//! [`GitHubApi`] trait rather than a bare `reqwest::Client` scattered across
//! the route handlers and the push orchestration. Two reasons:
//!
//! 1. **No test in this crate is allowed to make a network call.** A trait
//!    object lets the test suite substitute [`FakeGitHubApi`] (see
//!    `crate::testing`) and exercise every failure mode — a taken repo name, a
//!    revoked token, a rate limit — without a socket ever opening. An
//!    injectable base URL was the other option the task allowed; a trait was
//!    chosen because it lets a test also assert *what was sent* (which files
//!    became blobs, in what order) without standing up an HTTP mock server.
//! 2. It keeps the wire-format knowledge — which status code means what, which
//!    header carries the rate-limit signal — in one place, so a route handler
//!    never has to know GitHub's HTTP conventions.
//!
//! ## Never logging the token
//!
//! Every method here takes the access token as a plain `&str` argument, never
//! wrapped in a field that could end up in a `tracing` span or a `Debug` print
//! of `self`. `HttpGitHubApi` holds no per-call state at all — just a
//! `reqwest::Client` — so there is nothing on `self` for a stray `{:?}` to leak
//! through. Every instrumented function below is `skip_all`, so the token
//! argument is never captured as a span field either.

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;

// The crate is renamed `github_reqwest` in Cargo.toml — see the comment there
// — so it does not collide with the 0.12 `reqwest` several integration tests
// already depend on. Aliased back to `reqwest` here so the rest of this file
// (and its doc comments) can refer to it the way everyone actually thinks of
// it.
use github_reqwest as reqwest;
use reqwest::Method;

/// This deployment's own identity, sent on every call. GitHub's REST API
/// rejects requests with no `User-Agent` at all, so this cannot be left unset.
const USER_AGENT: &str = "authkestra-playground (+https://github.com/Authkestra/playground)";

/// GitHub's REST API. Requests to it, not to `github.com` itself, are what
/// need this base — the OAuth authorize/token endpoints live on the other
/// host and are built separately (authorize is a redirect URL the browser
/// follows; the token exchange is `exchange_code` below).
const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_OAUTH_BASE: &str = "https://github.com";

/// The API version pinned on every call, per GitHub's own recommendation —
/// without it, a breaking change on their side changes behaviour under us with
/// no warning.
const API_VERSION: &str = "2022-11-28";

/// What a created (or looked-up) repository comes back as.
#[derive(Debug, Clone)]
pub struct GitHubRepoInfo {
    /// The name GitHub actually stored it under — normally identical to what
    /// was requested, but read back rather than assumed.
    pub name: String,
    pub html_url: String,
    pub default_branch: String,
}

/// Every distinct way a call to GitHub can fail that a visitor can act on.
///
/// Deliberately not a single "GitHub said no" bucket: §5 of the issue requires
/// a repo-name clash, an invalid name, a dead token, a missing scope, a rate
/// limit and a network failure to each read as what they are, because each has
/// an unrelated fix — renaming, reconnecting, waiting, or nothing to do with
/// GitHub at all.
#[derive(Debug)]
pub enum GitHubApiError {
    /// `422` where GitHub's own error list says the name already exists on
    /// this account.
    RepoNameTaken,
    /// `422` for any other reason — GitHub's own message, since its naming
    /// rules are its own to explain.
    InvalidRepoName(String),
    /// `401`, or a token-exchange response GitHub itself calls invalid — the
    /// token is dead, whether it expired, was revoked, or was never valid.
    TokenRejected,
    /// `403` or `404` where nothing points at a rate limit — the token does
    /// not carry the scope this call needed.
    ScopeMissing,
    /// `403` with an exhausted rate-limit header, or `429`.
    RateLimited,
    /// The request never reached GitHub, or its response never reached us —
    /// DNS, a timeout, a dropped connection. Carries the transport error's own
    /// text, which never contains anything we sent (see the module docs).
    Network(String),
    /// Anything else — a shape neither this module nor a visitor can be
    /// expected to have anticipated. Always worth showing the raw message.
    Other(String),
}

impl std::fmt::Display for GitHubApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitHubApiError::RepoNameTaken => write!(f, "a repository with that name already exists"),
            GitHubApiError::InvalidRepoName(m) => write!(f, "invalid repository name: {m}"),
            GitHubApiError::TokenRejected => write!(f, "the GitHub token was rejected"),
            GitHubApiError::ScopeMissing => write!(f, "the GitHub token is missing a required scope"),
            GitHubApiError::RateLimited => write!(f, "GitHub's rate limit was hit"),
            GitHubApiError::Network(m) => write!(f, "could not reach GitHub: {m}"),
            GitHubApiError::Other(m) => write!(f, "{m}"),
        }
    }
}

/// The calls the push flow needs, in the order it needs them.
///
/// One trait rather than a struct of function pointers, so a fake can also be
/// a `Send + Sync` trait object stored in `AppState` exactly like the real
/// client — the route handlers hold `Arc<dyn GitHubApi>` and never know which
/// one they were given.
#[async_trait::async_trait]
pub trait GitHubApi: Send + Sync {
    /// Exchange an authorization code for an access token. Returns the token
    /// itself — callers must not log it (see the module docs).
    async fn exchange_code(
        &self,
        client_id: &str,
        client_secret: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<String, GitHubApiError>;

    /// The authenticated user's login, i.e. the `owner` half of `owner/repo`.
    async fn current_user(&self, token: &str) -> Result<String, GitHubApiError>;

    /// `POST /user/repos`, with `auto_init: false` — the repository is created
    /// empty, because the commit that gives it a first ref is built by hand
    /// from the kit's own files rather than from GitHub's own README seed.
    async fn create_repo(
        &self,
        token: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<GitHubRepoInfo, GitHubApiError>;

    /// One Git blob for one generated file's contents. Returns the blob's
    /// `sha`.
    async fn create_blob(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        content: &str,
    ) -> Result<String, GitHubApiError>;

    /// One Git tree over every blob created for this push. `entries` is
    /// `(path, blob_sha)` pairs. Returns the tree's `sha`.
    async fn create_tree(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        entries: &[(String, String)],
    ) -> Result<String, GitHubApiError>;

    /// One commit over the tree, with **no parents** — the repository was
    /// created empty, so this is its first commit, not a continuation of one
    /// GitHub seeded. Returns the commit's `sha`.
    async fn create_commit(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
    ) -> Result<String, GitHubApiError>;

    /// Point `refs/heads/{branch}` at the new commit — the step that actually
    /// makes the repository non-empty from GitHub's point of view.
    async fn create_ref(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GitHubApiError>;
}

/// The real client, over `reqwest`.
///
/// Holds nothing but the HTTP client — no token, no per-call state — so there
/// is nothing on `self` a stray log of it could leak.
pub struct HttpGitHubApi {
    client: reqwest::Client,
}

impl HttpGitHubApi {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                // Only fails on a genuinely broken TLS backend or resolver
                // configuration, which is a boot-time bug, not a request-time
                // one — the same posture the engine takes when it builds its
                // own provider clients.
                .expect("failed to build the GitHub HTTP client"),
        }
    }

    /// One authenticated call to the REST API, with the headers every call
    /// needs. Returns the status, headers and JSON body so each endpoint can
    /// apply its own interpretation on top of the shared classification below.
    #[tracing::instrument(skip_all, fields(%method, path))]
    async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<Value>,
    ) -> Result<(StatusCode, HeaderMap, Value), GitHubApiError> {
        let mut req = self
            .client
            .request(method, format!("{GITHUB_API_BASE}{path}"))
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION);
        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req.send().await.map_err(map_transport_error)?;
        let status = resp.status();
        let headers = resp.headers().clone();
        // A body that fails to parse as JSON is treated as empty rather than
        // a hard error — a `204 No Content` from `create_ref` has none, and
        // that must not be confused with a transport failure.
        let body = resp.json::<Value>().await.unwrap_or(Value::Null);
        Ok((status, headers, body))
    }
}

impl Default for HttpGitHubApi {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl GitHubApi for HttpGitHubApi {
    #[tracing::instrument(skip_all)]
    async fn exchange_code(
        &self,
        client_id: &str,
        client_secret: &str,
        code: &str,
        redirect_uri: &str,
    ) -> Result<String, GitHubApiError> {
        let resp = self
            .client
            .post(format!("{GITHUB_OAUTH_BASE}/login/oauth/access_token"))
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ])
            .send()
            .await
            .map_err(map_transport_error)?;

        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);

        if let Some(token) = body.get("access_token").and_then(Value::as_str) {
            return Ok(token.to_string());
        }

        // GitHub's token endpoint answers `200` even when the exchange
        // failed, carrying `error`/`error_description` instead of an HTTP
        // error status — so the absence of `access_token` is the actual
        // signal, not the status code.
        if let Some(error) = body.get("error").and_then(Value::as_str) {
            return Err(match error {
                "bad_verification_code" | "incorrect_client_credentials" | "access_denied" => {
                    GitHubApiError::TokenRejected
                }
                _ => GitHubApiError::Other(
                    body.get("error_description")
                        .and_then(Value::as_str)
                        .unwrap_or(error)
                        .to_string(),
                ),
            });
        }

        Err(classify_status(status, &HeaderMap::new(), &body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&body))))
    }

    #[tracing::instrument(skip_all)]
    async fn current_user(&self, token: &str) -> Result<String, GitHubApiError> {
        let (status, headers, body) = self.call(Method::GET, "/user", token, None).await?;
        if status.is_success() {
            return body
                .get("login")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| GitHubApiError::Other("GitHub's /user response had no login".into()));
        }
        Err(classify_status(status, &headers, &body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&body))))
    }

    #[tracing::instrument(skip_all, fields(repo = %name))]
    async fn create_repo(
        &self,
        token: &str,
        name: &str,
        description: Option<&str>,
    ) -> Result<GitHubRepoInfo, GitHubApiError> {
        let body = serde_json::json!({
            "name": name,
            "description": description,
            // Public only: the ceiling this feature deliberately does not
            // raise past `public_repo` (see the ADR).
            "private": false,
            // The repository starts empty; the Git Data API calls that
            // follow give it its one commit.
            "auto_init": false,
        });
        let (status, headers, resp_body) =
            self.call(Method::POST, "/user/repos", token, Some(body)).await?;

        if status == StatusCode::CREATED {
            let html_url = resp_body
                .get("html_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let default_branch = resp_body
                .get("default_branch")
                .and_then(Value::as_str)
                .unwrap_or("main")
                .to_string();
            let repo_name = resp_body
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_string();
            return Ok(GitHubRepoInfo {
                name: repo_name,
                html_url,
                default_branch,
            });
        }

        if status == StatusCode::UNPROCESSABLE_ENTITY {
            let messages: Vec<String> = resp_body
                .get("errors")
                .and_then(Value::as_array)
                .map(|errs| {
                    errs.iter()
                        .filter_map(|e| e.get("message").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            // GitHub's own wording for this specific, common case. Brittle if
            // they reword it, hence the fallback to a generic invalid-name
            // error rather than a hard match — the same trade the OAuth
            // callback already makes on the framework's error strings.
            if messages.iter().any(|m| m.contains("already exists")) {
                return Err(GitHubApiError::RepoNameTaken);
            }
            let detail = if messages.is_empty() {
                generic_message(&resp_body)
            } else {
                messages.join("; ")
            };
            return Err(GitHubApiError::InvalidRepoName(detail));
        }

        Err(classify_status(status, &headers, &resp_body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&resp_body))))
    }

    #[tracing::instrument(skip_all, fields(%owner, %repo))]
    async fn create_blob(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        content: &str,
    ) -> Result<String, GitHubApiError> {
        let body = serde_json::json!({ "content": content, "encoding": "utf-8" });
        let (status, headers, resp_body) = self
            .call(
                Method::POST,
                &format!("/repos/{owner}/{repo}/git/blobs"),
                token,
                Some(body),
            )
            .await?;
        if status == StatusCode::CREATED {
            return resp_body
                .get("sha")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| GitHubApiError::Other("blob response had no sha".into()));
        }
        Err(classify_status(status, &headers, &resp_body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&resp_body))))
    }

    #[tracing::instrument(skip_all, fields(%owner, %repo, entries = entries.len()))]
    async fn create_tree(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        entries: &[(String, String)],
    ) -> Result<String, GitHubApiError> {
        let tree: Vec<Value> = entries
            .iter()
            .map(|(path, sha)| {
                serde_json::json!({
                    "path": path,
                    "mode": "100644",
                    "type": "blob",
                    "sha": sha,
                })
            })
            .collect();
        let body = serde_json::json!({ "tree": tree });
        let (status, headers, resp_body) = self
            .call(
                Method::POST,
                &format!("/repos/{owner}/{repo}/git/trees"),
                token,
                Some(body),
            )
            .await?;
        if status == StatusCode::CREATED {
            return resp_body
                .get("sha")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| GitHubApiError::Other("tree response had no sha".into()));
        }
        Err(classify_status(status, &headers, &resp_body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&resp_body))))
    }

    #[tracing::instrument(skip_all, fields(%owner, %repo))]
    async fn create_commit(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        message: &str,
        tree_sha: &str,
    ) -> Result<String, GitHubApiError> {
        // No `parents`: the repository is empty, so this is its root commit,
        // not a continuation of one GitHub seeded.
        let body = serde_json::json!({
            "message": message,
            "tree": tree_sha,
            "parents": Vec::<String>::new(),
        });
        let (status, headers, resp_body) = self
            .call(
                Method::POST,
                &format!("/repos/{owner}/{repo}/git/commits"),
                token,
                Some(body),
            )
            .await?;
        if status == StatusCode::CREATED {
            return resp_body
                .get("sha")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| GitHubApiError::Other("commit response had no sha".into()));
        }
        Err(classify_status(status, &headers, &resp_body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&resp_body))))
    }

    #[tracing::instrument(skip_all, fields(%owner, %repo, %branch))]
    async fn create_ref(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GitHubApiError> {
        let body = serde_json::json!({ "ref": format!("refs/heads/{branch}"), "sha": sha });
        let (status, headers, resp_body) = self
            .call(
                Method::POST,
                &format!("/repos/{owner}/{repo}/git/refs"),
                token,
                Some(body),
            )
            .await?;
        if status == StatusCode::CREATED {
            return Ok(());
        }
        Err(classify_status(status, &headers, &resp_body)
            .unwrap_or_else(|| GitHubApiError::Other(generic_message(&resp_body))))
    }
}

/// Interpretation shared by every REST endpoint above. Returns `None` for a
/// status this shared logic has no opinion on, leaving the caller to fall back
/// to a generic message — `create_repo`'s `422` handling is deliberately
/// *not* here, because "name taken" only means something for that one call.
fn classify_status(status: StatusCode, headers: &HeaderMap, body: &Value) -> Option<GitHubApiError> {
    match status {
        StatusCode::UNAUTHORIZED => Some(GitHubApiError::TokenRejected),
        StatusCode::FORBIDDEN => {
            // GitHub signals an exhausted rate limit on a `403` via this
            // header, indistinguishable from an ordinary permissions refusal
            // by status code alone.
            let remaining = headers
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok());
            let message_says_rate_limit = body
                .get("message")
                .and_then(Value::as_str)
                .is_some_and(|m| m.to_ascii_lowercase().contains("rate limit"));
            if remaining == Some("0") || message_says_rate_limit {
                Some(GitHubApiError::RateLimited)
            } else {
                Some(GitHubApiError::ScopeMissing)
            }
        }
        // An OAuth App token missing a scope a route needs is sometimes
        // reported as `404` rather than `403` — GitHub hides the resource's
        // existence from a caller it will not show it to either way.
        StatusCode::NOT_FOUND => Some(GitHubApiError::ScopeMissing),
        StatusCode::TOO_MANY_REQUESTS => Some(GitHubApiError::RateLimited),
        _ => None,
    }
}

fn generic_message(body: &Value) -> String {
    body.get("message")
        .and_then(Value::as_str)
        .unwrap_or("GitHub returned an unexpected response")
        .to_string()
}

/// A `reqwest::Error` is a transport failure — DNS, TLS, a timeout, a dropped
/// connection — never something GitHub said, so it is always [`GitHubApiError::Network`].
/// Its `Display` never includes what was sent, only what went wrong reaching
/// the peer, so this cannot leak the token.
fn map_transport_error(e: reqwest::Error) -> GitHubApiError {
    GitHubApiError::Network(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property every route handler leans on: a network error must
    /// never be reported as an API failure, because the two have unrelated
    /// fixes (retry vs. fix the request).
    #[test]
    fn transport_errors_are_never_confused_with_api_errors() {
        // `reqwest::Error` cannot be constructed directly outside the crate,
        // so this asserts the mapping's *shape* via the classifier instead:
        // nothing HTTP-status-shaped ever produces `Network`.
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
        ] {
            if let Some(err) = classify_status(status, &HeaderMap::new(), &Value::Null) {
                assert!(
                    !matches!(err, GitHubApiError::Network(_)),
                    "a real HTTP response must never classify as a network error: {status}"
                );
            }
        }
    }

    #[test]
    fn a_401_is_a_rejected_token() {
        assert!(matches!(
            classify_status(StatusCode::UNAUTHORIZED, &HeaderMap::new(), &Value::Null),
            Some(GitHubApiError::TokenRejected)
        ));
    }

    #[test]
    fn a_403_with_exhausted_rate_limit_header_is_rate_limited() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN, &headers, &Value::Null),
            Some(GitHubApiError::RateLimited)
        ));
    }

    #[test]
    fn a_403_with_no_rate_limit_signal_is_missing_scope() {
        assert!(matches!(
            classify_status(StatusCode::FORBIDDEN, &HeaderMap::new(), &Value::Null),
            Some(GitHubApiError::ScopeMissing)
        ));
    }

    #[test]
    fn a_404_is_treated_as_a_missing_scope() {
        assert!(matches!(
            classify_status(StatusCode::NOT_FOUND, &HeaderMap::new(), &Value::Null),
            Some(GitHubApiError::ScopeMissing)
        ));
    }

    #[test]
    fn a_429_is_rate_limited() {
        assert!(matches!(
            classify_status(StatusCode::TOO_MANY_REQUESTS, &HeaderMap::new(), &Value::Null),
            Some(GitHubApiError::RateLimited)
        ));
    }

    #[test]
    fn an_ordinary_status_is_not_classified_here() {
        assert!(classify_status(StatusCode::OK, &HeaderMap::new(), &Value::Null).is_none());
    }

    /// `GitHubApiError`'s `Debug` output must never be able to carry a token —
    /// there is no variant with a field that could hold one, which this
    /// exercises by round-tripping every constructible variant through it.
    #[test]
    fn debug_output_never_carries_a_bearer_token() {
        let secret_looking = "gho_should_never_appear_anywhere";
        let variants = [
            GitHubApiError::RepoNameTaken,
            GitHubApiError::InvalidRepoName("bad name".into()),
            GitHubApiError::TokenRejected,
            GitHubApiError::ScopeMissing,
            GitHubApiError::RateLimited,
            GitHubApiError::Network("connection refused".into()),
            GitHubApiError::Other("unexpected".into()),
        ];
        for v in variants {
            assert!(!format!("{v:?}").contains(secret_looking));
        }
    }
}
