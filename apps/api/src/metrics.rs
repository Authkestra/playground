//! What visitors try, what they complete, and what they download.
//!
//! Counters only. No per-visitor record, no address, no user agent, no
//! third-party script, and not one byte of added JavaScript — the funnel is
//! inferred entirely from requests the API already serves, so the frontend
//! needed no change to produce any of it.
//!
//! That constraint is not decoration on a product like this one. A playground
//! arguing that authentication should be something you own, instrumented with
//! somebody else's surveillance script, would be arguing against itself in the
//! network tab. The reasoning is written down in
//! `docs/decisions/0009-usage-metrics.md`.
//!
//! ## Shape
//!
//! Every counter lives under `metrics:<iso-week>:` in the shared store, with a
//! TTL, so a week ages out on its own and nothing has to remember to prune it.
//! Weekly because the question is weekly — "what did visitors do this week" —
//! and a bucket no finer than the question cannot be turned into a timeline of
//! one visitor's evening.
//!
//! ## The funnel, without a per-visitor record
//!
//! "Where visitors drop" needs each session counted once per stage, not once
//! per request, or a visitor who toggles eight scenarios looks like eight
//! visitors. The honest way to do that without keeping a list of who did what
//! is a write-once marker keyed by the session, carrying the session's own TTL:
//! [`KeyValue::set_if_absent`] says whether this session has reached this stage
//! before, the counter moves only when the answer is no, and the marker dies
//! with the session that owns it. Nothing survives the session but the number.
//!
//! ## Failure
//!
//! Recording is infallible from the caller's point of view, for the same reason
//! the flow log is: a visitor's request must never fail because our
//! bookkeeping could not be written. A store error is logged and dropped.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::store::KeyValue;

/// How long a week's counters live after the last thing that touched them.
///
/// Twelve weeks is a quarter: long enough to see whether a change to the
/// playground moved anything, short enough that the store is never holding a
/// year of history nobody has looked at. Nothing here is worth keeping longer,
/// because none of it can be asked a new question later — it is already
/// aggregate.
pub const RETENTION_WEEKS: u64 = 12;

const PREFIX: &str = "metrics";

/// The stages a session can reach, counted once each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Changed at least one scenario's configuration.
    Configured,
    /// Attempted at least one ceremony action.
    Acted,
    /// Downloaded a starter kit.
    Downloaded,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Configured => "configured",
            Stage::Acted => "acted",
            Stage::Downloaded => "downloaded",
        }
    }
}

/// Aggregate usage counters over the shared store.
pub struct Metrics {
    kv: Arc<dyn KeyValue>,
    retention: Duration,
    /// The markers live exactly as long as the session they describe.
    session_ttl: Duration,
}

impl Metrics {
    pub fn new(kv: Arc<dyn KeyValue>, session_ttl: Duration) -> Self {
        Self {
            kv,
            retention: Duration::from_secs(RETENTION_WEEKS * 7 * 24 * 60 * 60),
            session_ttl,
        }
    }

    /// The ISO week a moment falls in, as `2026-W38`.
    ///
    /// ISO rather than "week of the year": its weeks always start on Monday and
    /// never split across a year boundary in two directions at once, which is
    /// the failure a home-made week number finds every January.
    fn week_of(now: chrono::DateTime<Utc>) -> String {
        let iso = now.iso_week();
        format!("{}-W{:02}", iso.year(), iso.week())
    }

    fn current_week() -> String {
        Self::week_of(Utc::now())
    }

    fn key(counter: &str) -> String {
        format!("{PREFIX}:{}:{counter}", Self::current_week())
    }

    /// Add one, and say nothing to the caller if the store refused.
    async fn bump(&self, counter: &str) {
        let key = Self::key(counter);
        if let Err(e) = self.kv.increment(&key, self.retention).await {
            tracing::warn!(error = %e, counter, "could not record a usage counter");
        }
    }

    /// Whether this session is reaching this stage for the first time.
    ///
    /// A store failure answers "no", so a blip under-counts rather than
    /// double-counts. Of the two ways to be wrong, a funnel that quietly
    /// inflates is the one that would be believed.
    async fn first_time(&self, session_id: Uuid, stage: Stage) -> bool {
        let key = format!("{PREFIX}:once:{}:{session_id}", stage.as_str());
        match self.kv.set_if_absent(&key, "1", self.session_ttl).await {
            Ok(fresh) => fresh,
            Err(e) => {
                tracing::warn!(error = %e, stage = stage.as_str(), "could not mark a funnel stage");
                false
            }
        }
    }

    async fn reached(&self, session_id: Uuid, stage: Stage) {
        if self.first_time(session_id, stage).await {
            self.bump(&format!("funnel:{}", stage.as_str())).await;
        }
    }

    /// A visitor arrived and was given a session.
    pub async fn session_created(&self) {
        self.bump("sessions").await;
    }

    /// A scenario's configuration changed. `active` is whether the new value
    /// leaves it switched on, since switching one off is not trying it.
    pub async fn configured(&self, session_id: Uuid, scenario: &str, active: bool) {
        self.reached(session_id, Stage::Configured).await;
        if active {
            self.bump(&format!("scenario:{scenario}:enabled")).await;
        }
    }

    /// A ceremony step was attempted.
    pub async fn action_attempted(&self, session_id: Uuid, scenario: &str) {
        self.reached(session_id, Stage::Acted).await;
        self.bump(&format!("scenario:{scenario}:attempted")).await;
    }

    /// A ceremony step succeeded.
    pub async fn action_completed(&self, scenario: &str) {
        self.bump(&format!("scenario:{scenario}:completed")).await;
    }

    /// A starter kit was downloaded, counted against the configuration it was
    /// generated from.
    pub async fn downloaded(&self, session_id: Uuid, configuration: &str) {
        self.reached(session_id, Stage::Downloaded).await;
        self.bump(&format!("download:{configuration}")).await;
    }

    /// The weekly view, newest week first.
    ///
    /// Reads at most `weeks` buckets by name rather than scanning everything
    /// under `metrics:`, so the cost of this is bounded by the window asked for
    /// and not by how long the deployment has been up.
    pub async fn snapshot(&self, weeks: u64) -> Vec<WeekView> {
        let now = Utc::now();
        let mut out = Vec::new();
        for back in 0..weeks.max(1) {
            let at = now - chrono::Duration::weeks(back as i64);
            let week = Self::week_of(at);
            let prefix = format!("{PREFIX}:{week}:");
            match self.kv.entries_with_prefix(&prefix).await {
                Ok(entries) => out.push(WeekView::from_entries(week, &prefix, entries)),
                Err(e) => {
                    tracing::warn!(error = %e, week, "could not read a week of counters");
                }
            }
        }
        out
    }
}

/// One week's counters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekView {
    /// ISO week, as `2026-W38`.
    pub week: String,
    /// Sessions handed out.
    pub sessions: u64,
    /// How far sessions got.
    pub funnel: Funnel,
    /// Per scenario: tried, attempted, completed.
    pub scenarios: BTreeMap<String, ScenarioCounts>,
    /// Downloads, keyed by the configuration that produced them.
    pub downloads: BTreeMap<String, u64>,
}

/// Sessions that reached each stage. Every one of these is counted once per
/// session, so `downloaded / sessions` is a conversion rate rather than a
/// ratio of two different kinds of thing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Funnel {
    pub configured: u64,
    pub acted: u64,
    pub downloaded: u64,
}

/// What happened to one scenario. `attempted` and `completed` count requests,
/// not sessions: a visitor who fails a TOTP code twice and then succeeds is
/// three attempts and one completion, which is the interesting shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioCounts {
    pub enabled: u64,
    pub attempted: u64,
    pub completed: u64,
}

impl WeekView {
    fn from_entries(week: String, prefix: &str, entries: Vec<(String, String)>) -> Self {
        let mut view = WeekView {
            week,
            ..Default::default()
        };

        for (key, value) in entries {
            let Some(counter) = key.strip_prefix(prefix) else {
                continue;
            };
            let Ok(count) = value.parse::<u64>() else {
                // A counter that is not a number is a bug elsewhere, not a
                // reason to refuse the whole week.
                tracing::warn!(counter, value, "ignoring a non-numeric counter");
                continue;
            };

            let parts: Vec<&str> = counter.split(':').collect();
            match parts.as_slice() {
                ["sessions"] => view.sessions = count,
                ["funnel", "configured"] => view.funnel.configured = count,
                ["funnel", "acted"] => view.funnel.acted = count,
                ["funnel", "downloaded"] => view.funnel.downloaded = count,
                ["scenario", id, metric] => {
                    let entry = view.scenarios.entry((*id).to_string()).or_default();
                    match *metric {
                        "enabled" => entry.enabled = count,
                        "attempted" => entry.attempted = count,
                        "completed" => entry.completed = count,
                        _ => {}
                    }
                }
                // A configuration can contain a `:`-free id list, but splitting
                // on `:` would still break a future id that carried one, so
                // take everything after the first segment verbatim.
                ["download", ..] => {
                    if let Some(configuration) = counter.strip_prefix("download:") {
                        view.downloads.insert(configuration.to_string(), count);
                    }
                }
                _ => {}
            }
        }

        view
    }
}

/// A stable name for the configuration a download was generated from.
///
/// Sorted and joined, so the same set of scenarios always produces the same
/// name however the visitor arrived at it — otherwise "passkeys then TOTP" and
/// "TOTP then passkeys" would be two rows describing one choice. A kit with
/// nothing switched on is `none`, which is a real and interesting row: it means
/// somebody downloaded the bare engine.
pub fn configuration_name(active: &[&str]) -> String {
    if active.is_empty() {
        return "none".to_string();
    }
    let mut ids: Vec<&str> = active.to_vec();
    ids.sort_unstable();
    ids.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryKv;

    fn metrics() -> Metrics {
        Metrics::new(Arc::new(MemoryKv::new()), Duration::from_secs(3600))
    }

    #[tokio::test]
    async fn a_week_starts_empty_rather_than_absent() {
        let m = metrics();
        let weeks = m.snapshot(1).await;
        assert_eq!(weeks.len(), 1);
        assert_eq!(weeks[0].sessions, 0);
        assert!(weeks[0].downloads.is_empty());
    }

    #[tokio::test]
    async fn what_visitors_try_and_complete_are_counted_separately() {
        let m = metrics();
        let session = Uuid::new_v4();

        m.configured(session, "totp", true).await;
        m.action_attempted(session, "totp").await;
        m.action_attempted(session, "totp").await;
        m.action_completed("totp").await;

        let week = m.snapshot(1).await.remove(0);
        let totp = &week.scenarios["totp"];
        assert_eq!(totp.enabled, 1);
        assert_eq!(totp.attempted, 2, "a failed attempt is still an attempt");
        assert_eq!(totp.completed, 1);
    }

    #[tokio::test]
    async fn switching_a_scenario_off_is_not_trying_it() {
        let m = metrics();
        let session = Uuid::new_v4();

        m.configured(session, "passkeys", true).await;
        m.configured(session, "passkeys", false).await;

        let week = m.snapshot(1).await.remove(0);
        assert_eq!(week.scenarios["passkeys"].enabled, 1);
    }

    #[tokio::test]
    async fn the_funnel_counts_each_session_once_however_busy_it_is() {
        let m = metrics();
        let session = Uuid::new_v4();

        for _ in 0..8 {
            m.configured(session, "totp", true).await;
        }
        for _ in 0..5 {
            m.action_attempted(session, "totp").await;
        }

        let week = m.snapshot(1).await.remove(0);
        assert_eq!(
            week.funnel.configured, 1,
            "one visitor toggling eight times is one visitor"
        );
        assert_eq!(week.funnel.acted, 1);
        assert_eq!(week.funnel.downloaded, 0);
    }

    #[tokio::test]
    async fn two_visitors_are_two_visitors() {
        let m = metrics();
        m.configured(Uuid::new_v4(), "totp", true).await;
        m.configured(Uuid::new_v4(), "totp", true).await;

        let week = m.snapshot(1).await.remove(0);
        assert_eq!(week.funnel.configured, 2);
    }

    #[tokio::test]
    async fn downloads_are_counted_against_the_configuration_that_made_them() {
        let m = metrics();
        m.downloaded(Uuid::new_v4(), "passkeys+totp").await;
        m.downloaded(Uuid::new_v4(), "passkeys+totp").await;
        m.downloaded(Uuid::new_v4(), "none").await;

        let week = m.snapshot(1).await.remove(0);
        assert_eq!(week.downloads["passkeys+totp"], 2);
        assert_eq!(week.downloads["none"], 1);
        assert_eq!(week.funnel.downloaded, 3);
    }

    #[test]
    fn one_configuration_has_one_name_however_it_was_reached() {
        assert_eq!(
            configuration_name(&["totp", "passkeys"]),
            configuration_name(&["passkeys", "totp"]),
        );
        assert_eq!(configuration_name(&["passkeys", "totp"]), "passkeys+totp");
        assert_eq!(configuration_name(&[]), "none");
    }

    #[test]
    fn a_week_is_named_by_its_iso_week() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-09-14T00:33:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(Metrics::week_of(at), "2026-W38");
    }

    /// A visitor's request must never fail because a counter could not be
    /// written, and a store that is down must not inflate the funnel either.
    #[tokio::test]
    async fn a_store_that_is_down_is_survivable_and_does_not_double_count() {
        let m = Metrics::new(
            Arc::new(crate::testing::BrokenKv),
            Duration::from_secs(3600),
        );
        let session = Uuid::new_v4();

        // None of these may panic or propagate.
        m.session_created().await;
        m.configured(session, "totp", true).await;
        m.action_attempted(session, "totp").await;
        m.action_completed("totp").await;
        m.downloaded(session, "totp").await;

        assert!(
            m.snapshot(2).await.is_empty(),
            "a week that could not be read is absent rather than reported as zero"
        );
    }
}
