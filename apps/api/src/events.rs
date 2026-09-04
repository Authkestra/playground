//! The per-visitor flow log.
//!
//! This is the thing a playground can show that documentation cannot: what the
//! engine actually did, in order, for *your* attempt. A challenge was issued, an
//! authenticator answered it, a signature verified, a counter moved from 4 to 5.
//!
//! Written for the visitor, not for us — so it names the step in plain language
//! and says why it mattered. Server-side tracing stays separate and keeps the
//! detail that would only confuse someone learning the flow.
//!
//! Lives in the shared store under the session's TTL, capped, so it costs
//! nothing to leave running and disappears with the session that owns it.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::store::{KeyValue, StoreError};

/// How many events a session keeps. Enough to cover several attempts at a
/// ceremony without letting a bored visitor grow the list without bound.
pub const MAX_EVENTS: usize = 120;

/// How an event reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EventLevel {
    /// Something progressed.
    Info,
    /// A step completed successfully.
    Success,
    /// Rejected for an ordinary reason — a wrong code, a declined consent
    /// screen. Not a fault, and it should not read like one.
    Rejected,
    /// Something went wrong on our side.
    Failed,
}

/// One step of a flow, as the visitor should see it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FlowEvent {
    /// RFC3339.
    pub at: String,
    /// Scenario id, so the UI can group by flow.
    pub scenario: String,
    /// Short label for the step, e.g. "challenge issued".
    pub step: String,
    pub level: EventLevel,
    /// One sentence on what happened and why it matters.
    pub detail: String,
    /// Key/value pairs worth showing verbatim — a counter moving, an algorithm
    /// chosen. Never secrets.
    ///
    /// Always serialised, even when empty. `skip_serializing_if` would have
    /// produced a generated TypeScript type declaring `facts` as always
    /// present while the JSON omitted it, so the frontend would have thrown on
    /// `facts.map(...)`. An empty array costs nothing and keeps the type
    /// honest. (`default` stays, so older stored events still decode.)
    #[serde(default)]
    pub facts: Vec<EventFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EventFact {
    pub name: String,
    pub value: String,
}

impl EventFact {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Append-only flow log, one list per session.
pub struct EventLog {
    kv: Arc<dyn KeyValue>,
    ttl: Duration,
}

impl EventLog {
    pub fn new(kv: Arc<dyn KeyValue>, ttl: Duration) -> Self {
        Self { kv, ttl }
    }

    fn key(session_id: Uuid) -> String {
        format!("events:{session_id}")
    }

    /// Record a step.
    ///
    /// Deliberately infallible from the caller's point of view: a flow must
    /// never fail because its narration could not be written. A failure is
    /// logged server-side and dropped.
    pub async fn record(&self, session_id: Uuid, event: FlowEvent) {
        let encoded = match serde_json::to_string(&event) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, "could not encode a flow event");
                return;
            }
        };
        if let Err(e) = self
            .kv
            .append_capped(&Self::key(session_id), &encoded, MAX_EVENTS, self.ttl)
            .await
        {
            tracing::error!(error = %e, "could not record a flow event");
        }
    }

    /// Every event for a session, oldest first.
    pub async fn read(&self, session_id: Uuid) -> Result<Vec<FlowEvent>, StoreError> {
        let raws = self.kv.list(&Self::key(session_id)).await?;
        Ok(raws
            .iter()
            .filter_map(|raw| serde_json::from_str(raw).ok())
            .collect())
    }

    /// Drop a session's log — used on reset, where the visitor expects a clean
    /// slate.
    pub async fn clear(&self, session_id: Uuid) -> Result<(), StoreError> {
        self.kv.delete(&Self::key(session_id)).await?;
        Ok(())
    }
}

/// Convenience for building events without repeating the timestamp.
pub struct Step {
    scenario: &'static str,
    step: &'static str,
    level: EventLevel,
    detail: String,
    facts: Vec<EventFact>,
}

impl Step {
    pub fn new(scenario: &'static str, step: &'static str, level: EventLevel) -> Self {
        Self {
            scenario,
            step,
            level,
            detail: String::new(),
            facts: Vec::new(),
        }
    }

    pub fn info(scenario: &'static str, step: &'static str) -> Self {
        Self::new(scenario, step, EventLevel::Info)
    }

    pub fn success(scenario: &'static str, step: &'static str) -> Self {
        Self::new(scenario, step, EventLevel::Success)
    }

    pub fn rejected(scenario: &'static str, step: &'static str) -> Self {
        Self::new(scenario, step, EventLevel::Rejected)
    }

    pub fn failed(scenario: &'static str, step: &'static str) -> Self {
        Self::new(scenario, step, EventLevel::Failed)
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn fact(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.facts.push(EventFact::new(name, value));
        self
    }

    pub fn build(self) -> FlowEvent {
        FlowEvent {
            at: chrono::Utc::now().to_rfc3339(),
            scenario: self.scenario.to_string(),
            step: self.step.to_string(),
            level: self.level,
            detail: self.detail,
            facts: self.facts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemoryKv;

    fn log() -> EventLog {
        EventLog::new(Arc::new(MemoryKv::new()), Duration::from_secs(3600))
    }

    #[tokio::test]
    async fn events_are_returned_oldest_first() {
        let l = log();
        let id = Uuid::new_v4();
        for step in ["first", "second", "third"] {
            l.record(id, Step::info("totp", "x").detail(step).build())
                .await;
        }
        let events = l.read(id).await.unwrap();
        let details: Vec<&str> = events.iter().map(|e| e.detail.as_str()).collect();
        assert_eq!(details, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn a_sessions_log_is_its_own() {
        let l = log();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        l.record(a, Step::info("totp", "x").detail("mine").build())
            .await;

        assert_eq!(l.read(a).await.unwrap().len(), 1);
        assert!(
            l.read(b).await.unwrap().is_empty(),
            "one visitor must not see another's flow"
        );
    }

    #[tokio::test]
    async fn the_log_is_capped() {
        let l = log();
        let id = Uuid::new_v4();
        for i in 0..(MAX_EVENTS + 25) {
            l.record(id, Step::info("totp", "x").detail(i.to_string()).build())
                .await;
        }
        let events = l.read(id).await.unwrap();
        assert_eq!(events.len(), MAX_EVENTS);
        // The newest survive.
        assert_eq!(events.last().unwrap().detail, (MAX_EVENTS + 24).to_string());
    }

    #[tokio::test]
    async fn clearing_leaves_nothing() {
        let l = log();
        let id = Uuid::new_v4();
        l.record(id, Step::info("totp", "x").build()).await;
        l.clear(id).await.unwrap();
        assert!(l.read(id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn facts_survive_the_round_trip() {
        let l = log();
        let id = Uuid::new_v4();
        l.record(
            id,
            Step::success("passkeys", "signature verified")
                .detail("The authenticator proved possession of the private key.")
                .fact("counter", "4 → 5")
                .fact("algorithm", "ES256")
                .build(),
        )
        .await;

        let event = l.read(id).await.unwrap().pop().unwrap();
        assert_eq!(event.level, EventLevel::Success);
        assert_eq!(event.facts.len(), 2);
        assert_eq!(event.facts[0].name, "counter");
        assert_eq!(event.facts[0].value, "4 → 5");
    }

    /// A flow must not fail because its narration could not be written.
    #[tokio::test]
    async fn recording_is_infallible_for_the_caller() {
        struct Broken;
        #[async_trait::async_trait]
        impl KeyValue for Broken {
            async fn get(&self, _: &str) -> Result<Option<String>, StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn set(&self, _: &str, _: &str, _: Duration) -> Result<(), StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn delete(&self, _: &str) -> Result<bool, StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn take(&self, _: &str) -> Result<Option<String>, StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn values_with_prefix(&self, _: &str) -> Result<Vec<String>, StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn delete_with_prefix(&self, _: &str) -> Result<u64, StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn append_capped(
                &self,
                _: &str,
                _: &str,
                _: usize,
                _: Duration,
            ) -> Result<(), StoreError> {
                Err(StoreError::Backend("down".into()))
            }
            async fn list(&self, _: &str) -> Result<Vec<String>, StoreError> {
                Err(StoreError::Backend("down".into()))
            }
        }

        let l = EventLog::new(Arc::new(Broken), Duration::from_secs(60));
        // Must not panic or propagate.
        l.record(Uuid::new_v4(), Step::info("totp", "x").build())
            .await;
    }
}

#[cfg(test)]
mod wire_shape_tests {
    use super::*;

    /// The generated TypeScript declares `facts` as always present, so the JSON
    /// must always carry it. Omitting it when empty would make the frontend
    /// throw on `facts.map(...)`.
    #[test]
    fn facts_is_always_present_even_when_empty() {
        let event = Step::info("totp", "x").detail("y").build();
        assert!(event.facts.is_empty());

        let json = serde_json::to_value(&event).unwrap();
        assert!(
            json.get("facts").is_some(),
            "facts must be serialised even when empty: {json}"
        );
        assert!(json["facts"].is_array());
    }

    #[test]
    fn an_event_without_facts_still_decodes() {
        let event: FlowEvent = serde_json::from_str(
            r#"{"at":"2026-01-01T00:00:00Z","scenario":"totp","step":"x","level":"info","detail":"y"}"#,
        )
        .expect("older stored events must still decode");
        assert!(event.facts.is_empty());
    }
}
