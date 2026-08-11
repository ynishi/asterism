//! Local telemetry — append-only `action_log` access (dogfooding
//! metrics).
//!
//! `action_log` is the ActionLog stream of the observability domain: what
//! the *person* did. Its siblings (`job_log`, `diag_log`, `perf_log`)
//! answer different questions and are written elsewhere; see
//! [`asterism_core::domain::observation`].
//!
//! Operational tier like [`crate::jobs::jobs_snapshot`]: no domain
//! aggregate, no repository port. The struct wraps the writer isle and
//! exposes exactly two operations — `record` (append) and `list`
//! (newest-first read) — consumed by the Tauri commands and the HTTP
//! API. Rows are local-only by design; nothing ever leaves the
//! machine.
//!
//! Contract types are used directly on the boundary (`asterism-infra`
//! already depends on `asterism-contract` for the dispatch runtime) so
//! the two transports share one mapping instead of duplicating it.

use asterism_contract::command::RecordEventCommand;
use asterism_contract::dto::EventDto;
use asterism_contract::query::ListEventsQuery;
use asterism_core::domain::observation::Stream;
use asterism_core::error::DomainError;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// Hard cap on one listing page — keeps an agent-side `limit=999999`
/// from materialising the whole table in memory.
const LIST_LIMIT_CAP: u32 = 5000;

/// Raw `action_log` row as read off the connection thread:
/// `(id, event, occurred_at, persona_id, duration_ms, attrs)`.
type EventRow = (Uuid, String, i64, Option<Uuid>, Option<i64>, Option<String>);

/// Namespaces a caller-supplied kind into an ActionLog event name.
///
/// A kind that already carries the prefix passes through, so a caller
/// that learned the full name is not punished with `action.action.…`.
fn event_name(kind: &str) -> String {
    let stream = Stream::Action.as_str();
    match kind.strip_prefix(stream).and_then(|r| r.strip_prefix('.')) {
        Some(_) => kind.to_string(),
        None => format!("{stream}.{kind}"),
    }
}

/// Inverse of [`event_name`], for the DTO boundary.
fn kind_of(event: &str) -> &str {
    event
        .strip_prefix(Stream::Action.as_str())
        .and_then(|rest| rest.strip_prefix('.'))
        .unwrap_or(event)
}

/// Append/read handle for the `action_log` table.
#[derive(Clone)]
pub struct Telemetry {
    isle: AsyncIsle,
}

impl Telemetry {
    /// Wraps the writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// Appends one event. `occurred_at` is stamped here (server side)
    /// so client clocks never skew the log; `kind` must be a
    /// non-empty slug and `persona_id`, when present, must parse as a
    /// UUID.
    pub async fn record(&self, command: RecordEventCommand) -> Result<(), DomainError> {
        let kind = command.kind.trim().to_string();
        if kind.is_empty() {
            return Err(DomainError::Validation(
                "RecordEventCommand.kind must not be empty".into(),
            ));
        }
        let persona_id = command
            .persona_id
            .as_deref()
            .map(|raw| {
                Uuid::parse_str(raw)
                    .map_err(|_| DomainError::Validation(format!("invalid persona id: {raw}")))
            })
            .transpose()?;
        let id = Uuid::new_v4();
        let occurred_at = chrono::Utc::now().timestamp_millis();
        let duration_ms = command.duration_ms;
        let payload = command.payload_json;
        let event = event_name(&kind);
        let env = crate::observe::current_env().as_str();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO action_log
                         (id, occurred_at, env, event, attrs, persona_id, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        id,
                        occurred_at,
                        env,
                        event,
                        payload,
                        persona_id,
                        duration_ms
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// Newest-first listing with optional kind / time-window filters.
    /// `limit` is clamped to [`LIST_LIMIT_CAP`]; `limit = 0` returns
    /// an empty page without touching the DB.
    pub async fn list(&self, query: ListEventsQuery) -> Result<Vec<EventDto>, DomainError> {
        let cap = query.limit.min(LIST_LIMIT_CAP);
        if cap == 0 {
            return Ok(Vec::new());
        }
        // Callers speak in kinds (`persona_switch`); the table speaks in
        // namespaced events. Translating on both edges of this one
        // function keeps the naming rule in a single place.
        let kind = query.kind.as_deref().map(event_name);
        let since = query.since_ms;
        let until = query.until_ms;
        let rows: Vec<EventRow> = self
            .isle
            .call(move |conn| {
                // The three optional filters collapse to always-true
                // clauses when absent, so one prepared statement
                // covers every combination.
                let mut stmt = conn.prepare(
                    "SELECT id, event, occurred_at, persona_id, duration_ms, attrs
                       FROM action_log
                      WHERE (?1 IS NULL OR event = ?1)
                        AND (?2 IS NULL OR occurred_at >= ?2)
                        AND (?3 IS NULL OR occurred_at < ?3)
                      ORDER BY occurred_at DESC, id
                      LIMIT ?4",
                )?;
                stmt.query_map(params![kind, since, until, cap as i64], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .map_err(infra_err)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, event, occurred_at_ms, persona_id, duration_ms, payload_json)| EventDto {
                    id: id.hyphenated().to_string(),
                    kind: kind_of(&event).to_string(),
                    occurred_at_ms,
                    persona_id: persona_id.map(|p| p.hyphenated().to_string()),
                    duration_ms,
                    payload_json,
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;

    fn cmd(kind: &str) -> RecordEventCommand {
        RecordEventCommand {
            kind: kind.into(),
            persona_id: None,
            duration_ms: None,
            payload_json: None,
        }
    }

    #[tokio::test]
    async fn record_and_list_roundtrip() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let telemetry = Telemetry::new(isle);
        let persona = Uuid::new_v4();
        telemetry
            .record(RecordEventCommand {
                kind: "persona_switch".into(),
                persona_id: Some(persona.hyphenated().to_string()),
                duration_ms: Some(1234),
                payload_json: Some(r#"{"assets":110000}"#.into()),
            })
            .await
            .unwrap();
        telemetry.record(cmd("app_open")).await.unwrap();

        let all = telemetry.list(ListEventsQuery::default()).await.unwrap();
        assert_eq!(all.len(), 2);

        let switches = telemetry
            .list(ListEventsQuery {
                kind: Some("persona_switch".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(switches.len(), 1);
        assert_eq!(
            switches[0].persona_id.as_deref(),
            Some(persona.hyphenated().to_string().as_str())
        );
        assert_eq!(switches[0].duration_ms, Some(1234));
        assert_eq!(
            switches[0].payload_json.as_deref(),
            Some(r#"{"assets":110000}"#)
        );
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn record_rejects_empty_kind_and_bad_persona_id() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let telemetry = Telemetry::new(isle);
        assert!(telemetry.record(cmd("   ")).await.is_err());
        assert!(
            telemetry
                .record(RecordEventCommand {
                    persona_id: Some("not-a-uuid".into()),
                    ..cmd("app_open")
                })
                .await
                .is_err()
        );
        // Neither attempt may have left a row behind.
        let all = telemetry.list(ListEventsQuery::default()).await.unwrap();
        assert!(all.is_empty());
        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn list_respects_time_window_and_zero_limit() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let telemetry = Telemetry::new(isle);
        telemetry.record(cmd("app_open")).await.unwrap();

        let zero = telemetry
            .list(ListEventsQuery {
                limit: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(zero.is_empty());

        // A window entirely in the past excludes the fresh row; an
        // open-ended window from the past includes it.
        let past_only = telemetry
            .list(ListEventsQuery {
                until_ms: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(past_only.is_empty());
        let open_ended = telemetry
            .list(ListEventsQuery {
                since_ms: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(open_ended.len(), 1);
        driver.shutdown().await.unwrap();
    }
}
