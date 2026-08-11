//! Observation — the `tracing` subscriber and the streams it writes.
//!
//! One channel for everything the application observes about itself
//! through `tracing`. Call sites anywhere in the workspace write
//! `tracing::warn!` / `info!` / `error!` and never think about
//! transport; this module decides where those records go.
//!
//! ## Which stream a record belongs to
//!
//! Two of the four observation streams are written here: `DiagLog`
//! (what the application decided or failed at) and `PerfLog` (how long
//! something took). A call site selects `PerfLog` by naming itself —
//! `event = "perf.list_index"` — and anything unnamed is a diagnostic.
//! `ActionLog` and `JobLog` are written by their own subsystems; see
//! [`asterism_core::domain::observation`] for the four-way split and
//! why it exists.
//!
//! ## Why a subscriber rather than a handle
//!
//! `asterism-core` deliberately does not depend on `asterism-infra`, so
//! a service in the application layer cannot reach a database-backed
//! sink through a field. Threading a port through `AssetService` /
//! `PersonaService` / the job handlers for the sake of a swallowed
//! warning would cost a constructor change per seam. `tracing` installs
//! one process-global subscriber instead, so a call site needs no
//! handle and the layering stays intact.
//!
//! ## What is written is written verbatim
//!
//! Field values reach `attrs` as they were given. Ids, counts,
//! durations and error strings are fine; a token, a credential or a
//! user's content is not. Check that before adding a field. How long a
//! row then survives is the stream's retention policy, not this
//! module's business.
//!
//! ## Two layers, and where the real decision is made
//!
//! - a `fmt` layer to stderr, filtered by `RUST_LOG` (default
//!   `asterism=info`) — the developer-facing view;
//! - [`ObservationLayer`], which persists to a stream table — the
//!   durable view, readable long after the terminal is gone.
//!
//! The filters are deliberately separate. `RUST_LOG` is the dial a
//! developer reaches for, and it must not change what the application
//! writes to its own database: the sink keeps a fixed `asterism=info`.
//!
//! That fixed filter only selects *candidates*. What is actually kept
//! is
//! [`StreamPolicy::should_persist`](asterism_core::domain::observation::StreamPolicy::should_persist)
//! — a filter sees a level and a target, and decisions like "perf
//! timings, in development only" need the stream.
//!
//! ## Ordering, and what the startup queue does and does not buy
//!
//! The subscriber must exist before anything can log, but the database
//! opens later. [`install`] therefore runs first and records accumulate
//! in a bounded queue until [`DiagSink::attach`] supplies the isle,
//! after which they flush and later records go straight to the writer.
//!
//! What that covers: records emitted before the database opened **on a
//! run that goes on to open it** — argument parsing, migration
//! warnings, an unusable environment override.
//!
//! What it does not: a failure *of* the open itself. `attach` is only
//! reached once the database is up, so a run that dies opening it
//! flushes nothing. Those records reach stderr and stop there, which is
//! why the fatal path also prints verbatim rather than relying on this.
//!
//! ## Never in the way
//!
//! Emitting is a lock plus a channel send — no spawn, no await, no
//! blocking — so a record can be produced from any thread, including
//! the one inside a SQLite call. One writer task drains the channel.
//! When it falls behind, records are dropped and counted rather than
//! queued without end: the application shares a single SQLite
//! connection, and diagnostics must never put a user's query behind a
//! burst of warnings.

use std::sync::{Arc, Mutex};

use asterism_contract::query::DiagLevel;
use asterism_core::domain::observation::{Env, Stream};
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use uuid::Uuid;

/// Records held before the sink is attached. Small on purpose — this
/// only has to cover startup, and a run that never opens a database has
/// nothing to flush them to.
const PENDING_CAP: usize = 256;

/// Depth of the queue between emitters and the single writer task.
///
/// A diagnostic must never slow the thing it describes, so a full queue
/// drops rather than blocks: the alternative is user-facing queries
/// waiting behind a burst of warnings on the one SQLite connection the
/// application owns.
const CHANNEL_CAP: usize = 1024;

/// One row destined for a stream table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationRecord {
    /// Namespaced event name (`perf.list_index`), taken from the
    /// event's `event` field. Absent when the call site did not name
    /// itself, which routes the record to `DiagLog`.
    pub event: Option<String>,
    /// `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE`.
    pub level: String,
    /// Emitting module path (`asterism_core::application::…`).
    pub target: String,
    /// Human-readable text — the event's `message` field.
    pub message: String,
    /// Remaining structured fields as a JSON object, `None` when the
    /// event carried nothing but a message.
    pub attrs: Option<String>,
}

impl ObservationRecord {
    /// Which table this record is written to.
    ///
    /// The subscriber owns three streams — `PerfLog`, `JobLog` and
    /// `DiagLog`. `ActionLog` is not one of them: an action is
    /// *commanded* by the UI through [`crate::telemetry::Telemetry`],
    /// which returns a `Result` the caller acts on, whereas everything
    /// here is emitted by the thing describing itself and must never
    /// slow that thing down. A record claiming to be an action, or
    /// naming no stream at all, is a mislabelled call site and lands in
    /// diagnostics — the stream for "something the application did that
    /// nobody classified".
    ///
    /// Note that this is the *destination*, not the claim: policy is
    /// applied to where the row actually lands, so a record cannot
    /// dodge the diagnostics floor by calling itself an action.
    fn stream(&self) -> Stream {
        match self.event.as_deref().and_then(Stream::of_event) {
            Some(Stream::Perf) => Stream::Perf,
            Some(Stream::Job) => Stream::Job,
            _ => Stream::Diag,
        }
    }

    /// Severity as the closed domain type, for the persistence floor.
    fn level(&self) -> Option<DiagLevel> {
        DiagLevel::parse(&self.level).ok()
    }

    /// Reads one attribute — how a stream recovers the values that are
    /// columns for it rather than free attributes.
    fn attr(&self, name: &str) -> Option<serde_json::Value> {
        let parsed: serde_json::Value = serde_json::from_str(self.attrs.as_deref()?).ok()?;
        parsed.get(name).cloned()
    }

    /// [`Self::attr`] as text.
    fn attr_str(&self, name: &str) -> Option<String> {
        self.attr(name)?.as_str().map(str::to_string)
    }

    /// [`Self::attr`] as an integer.
    fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name)?.as_i64()
    }
}

/// Shared state between the layer (sync, any thread) and the drain task.
#[derive(Default)]
struct SinkState {
    /// Set by [`DiagSink::attach`]. Once present, records go straight
    /// to the writer task instead of accumulating.
    tx: Option<tokio::sync::mpsc::Sender<ObservationRecord>>,
    pending: Vec<ObservationRecord>,
    /// Count of records dropped — before attach because the startup
    /// queue filled, after attach because the writer is behind.
    /// Surfaced in batches rather than per-drop, so a stuck writer does
    /// not turn one problem into a stderr flood.
    dropped: u64,
    /// Value of `dropped` at the last report. Reporting on each
    /// doubling keeps the notice count logarithmic in the loss.
    reported: u64,
}

/// Handle used to attach the database once it is open.
#[derive(Clone)]
pub struct DiagSink {
    state: Arc<Mutex<SinkState>>,
    /// Environment every record is stamped with, and the input to the
    /// per-stream persistence policy. Fixed for the life of the
    /// process: the profile cannot change while it runs.
    env: Env,
}

impl DiagSink {
    /// A sink for the process's own environment.
    fn new(env: Env) -> Self {
        Self {
            state: Arc::new(Mutex::new(SinkState::default())),
            env,
        }
    }

    /// Attaches the writer isle, starts the drain task, and flushes
    /// anything captured during startup.
    ///
    /// Must be called from inside a tokio runtime — the writer task is
    /// spawned here, once, rather than per record. That is what lets an
    /// emitter on any thread (including rusqlite-isle's own OS thread,
    /// which carries no runtime context) still reach the database:
    /// submitting is a channel send, not a spawn.
    ///
    /// Writes are fire-and-forget: a failure to persist a diagnostic is
    /// reported to stderr and otherwise ignored, because the one thing
    /// a diagnostic channel must not do is fail the operation that
    /// produced it.
    pub fn attach(&self, isle: AsyncIsle) {
        let env = self.env;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ObservationRecord>(CHANNEL_CAP);
        let (backlog, dropped) = {
            let mut state = self.state.lock().expect("diag sink poisoned");
            state.tx = Some(tx.clone());
            (std::mem::take(&mut state.pending), state.dropped)
        };
        if dropped > 0 {
            eprintln!(
                "asterism: {dropped} diagnostic record(s) dropped before the database opened"
            );
        }

        tokio::spawn(async move {
            while let Some(record) = rx.recv().await {
                if let Err(err) = insert(&isle, env, record).await {
                    // Cannot report this through `tracing` without
                    // feeding the channel that just failed.
                    eprintln!("asterism: diag_log insert failed: {err}");
                }
            }
        });

        for record in backlog {
            self.submit(record);
        }
    }

    /// Queues (pre-attach) or hands off (post-attach) one record.
    ///
    /// Synchronous and non-blocking on both paths: `on_event` can run
    /// on any thread, including one inside a SQLite call, and must not
    /// wait on anything.
    ///
    /// The per-stream policy is applied here rather than at the insert,
    /// so a record that will not be kept never occupies a queue slot.
    /// Under dogfood every listing produces two perf records that are
    /// destined for nothing; letting them through would mean the
    /// diagnostics they crowd out are exactly the high-value ones the
    /// queue exists to protect.
    fn submit(&self, record: ObservationRecord) {
        if !record
            .stream()
            .policy()
            .should_persist(self.env, record.level())
        {
            return;
        }
        let mut state = self.state.lock().expect("diag sink poisoned");
        let Some(tx) = state.tx.clone() else {
            if state.pending.len() < PENDING_CAP {
                state.pending.push(record);
            } else {
                state.dropped += 1;
            }
            return;
        };
        drop(state);
        if tx.try_send(record).is_err() {
            // Full (writer behind) or closed (shutting down). Either
            // way the fmt layer already put this on stderr.
            let mut state = self.state.lock().expect("diag sink poisoned");
            state.dropped += 1;
            let total = state.dropped;
            let due = total >= state.reported.saturating_mul(2).max(1);
            if due {
                state.reported = total;
            }
            drop(state);
            if due {
                // A gap in a table nobody is told about reads as an
                // absence of events rather than an absence of records —
                // and drops cluster, so the run that loses rows is the
                // busy one whose rows mattered. Reported on a doubling
                // so a wedged writer costs a handful of lines, not one
                // per lost record.
                eprintln!("asterism: {total} observation record(s) dropped (writer behind)");
            }
        }
    }
}

/// Hard cap on one listing page, mirroring `Telemetry`'s — keeps a
/// `limit=999999` from materialising the whole table in memory.
const LIST_LIMIT_CAP: u32 = 5000;

/// Renders `min_level` as the SQL fragment and bindings that select it.
///
/// `min_level` expands to the set of level names at or above it, so the
/// filter becomes `level IN (…)` — an exact predicate the query planner
/// can serve from `idx_diag_log_level_occurred`, rather than something
/// approximated by reading extra rows and sifting them in Rust.
///
/// The set comes from [`DiagLevel::at_least`], so severity ordering
/// still lives in exactly one place; SQL only ever sees literal names.
fn level_clause(min_level: Option<DiagLevel>) -> (String, Vec<String>) {
    let Some(min) = min_level else {
        return (String::new(), Vec::new());
    };
    let levels = min.at_least();
    // Placeholders start at ?5 — the four fixed bindings precede them.
    let placeholders: Vec<String> = (0..levels.len()).map(|i| format!("?{}", i + 5)).collect();
    (
        format!(" AND level IN ({})", placeholders.join(", ")),
        levels.iter().map(|l| l.as_str().to_string()).collect(),
    )
}

/// Handle over the observation streams: reads, and expiry.
///
/// Separate from [`DiagSink`] and deliberately unable to append —
/// records arrive through the subscriber and nowhere else. What this
/// type can do besides read is remove rows past their stream's
/// retention, which is the other half of owning a durable table.
///
/// One handle rather than four because the four tables share an
/// envelope and a caller investigating the application wants all of
/// them; the split matters to the writer, not the reader.
#[derive(Clone)]
pub struct ObservationStore {
    isle: AsyncIsle,
}

/// Row shape scanned for one diagnostic.
type DiagRow = (
    Uuid,
    i64,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);
/// Row shape scanned for one timing.
type PerfRow = (
    Uuid,
    i64,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
);
/// Row shape scanned for one job run.
type JobLogRow = (
    Uuid,
    i64,
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);
/// Row shape scanned for one envelope off the union view.
type ObservationRow = (
    String,
    Uuid,
    i64,
    String,
    String,
    Option<String>,
    Option<String>,
);

impl ObservationStore {
    /// Wraps the writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }

    /// Lists diagnostics newest first. Every filter is optional, so
    /// `limit` alone means "everything recent".
    pub async fn diag(
        &self,
        query: asterism_contract::query::ListDiagQuery,
    ) -> Result<Vec<asterism_contract::dto::DiagDto>, asterism_core::error::DomainError> {
        let Some(limit) = page_limit(query.limit) else {
            return Ok(Vec::new());
        };
        let target = query.target.clone();
        let since = query.since_ms;
        let until = query.until_ms;
        // Parsed at the boundary, so nothing downstream compares raw
        // text: an unusable `min_level` is a caller error, not a filter
        // that quietly matches everything.
        let min_level = query
            .min_level
            .as_deref()
            .map(DiagLevel::parse)
            .transpose()
            .map_err(asterism_core::error::DomainError::Validation)?;
        let (level_sql, level_binds) = level_clause(min_level);
        let rows: Vec<DiagRow> = self
            .isle
            .call(move |conn| {
                // Every filter is in the query, so `LIMIT` cuts a page
                // of matches rather than a page of candidates — a short
                // page means the window is exhausted.
                let sql = format!(
                    "SELECT id, occurred_at, env, event, level, target, message,
                            attrs, correlation_id
                       FROM diag_log
                      WHERE (?1 IS NULL OR target LIKE '%' || ?1 || '%')
                        AND (?2 IS NULL OR occurred_at >= ?2)
                        AND (?3 IS NULL OR occurred_at < ?3){level_sql}
                      ORDER BY occurred_at DESC, id
                      LIMIT ?4"
                );
                let mut stmt = conn.prepare(&sql)?;
                // Fixed bindings first, then one per level name. The
                // names are `DiagLevel` variants rendered by
                // `as_str`, never caller text.
                let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![
                    Box::new(target),
                    Box::new(since),
                    Box::new(until),
                    Box::new(limit),
                ];
                for level in level_binds {
                    binds.push(Box::new(level));
                }
                stmt.query_map(rusqlite::params_from_iter(binds.iter()), |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .map_err(crate::sqlite::map::infra_err)?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    occurred_at_ms,
                    env,
                    event,
                    level,
                    target,
                    message,
                    attrs_json,
                    correlation_id,
                )| asterism_contract::dto::DiagDto {
                    id: id.hyphenated().to_string(),
                    occurred_at_ms,
                    env,
                    event,
                    level,
                    target,
                    message,
                    attrs_json,
                    correlation_id,
                },
            )
            .collect())
    }

    /// Lists timings newest first.
    pub async fn perf(
        &self,
        query: asterism_contract::query::ListPerfQuery,
    ) -> Result<Vec<asterism_contract::dto::PerfDto>, asterism_core::error::DomainError> {
        let Some(limit) = page_limit(query.limit) else {
            return Ok(Vec::new());
        };
        let op = query.op.clone();
        let since = query.since_ms;
        let until = query.until_ms;
        let rows: Vec<PerfRow> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, occurred_at, env, event, op, duration_ms,
                            attrs, correlation_id
                       FROM perf_log
                      WHERE (?1 IS NULL OR op = ?1)
                        AND (?2 IS NULL OR occurred_at >= ?2)
                        AND (?3 IS NULL OR occurred_at < ?3)
                      ORDER BY occurred_at DESC, id
                      LIMIT ?4",
                )?;
                stmt.query_map(params![op, since, until, limit], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .map_err(crate::sqlite::map::infra_err)?;

        Ok(rows
            .into_iter()
            .map(
                |(id, occurred_at_ms, env, event, op, duration_ms, attrs_json, correlation_id)| {
                    asterism_contract::dto::PerfDto {
                        id: id.hyphenated().to_string(),
                        occurred_at_ms,
                        env,
                        event,
                        op,
                        duration_ms,
                        attrs_json,
                        correlation_id,
                    }
                },
            )
            .collect())
    }

    /// Lists job runs newest first.
    pub async fn job_log(
        &self,
        query: asterism_contract::query::ListJobLogQuery,
    ) -> Result<Vec<asterism_contract::dto::JobLogDto>, asterism_core::error::DomainError> {
        let Some(limit) = page_limit(query.limit) else {
            return Ok(Vec::new());
        };
        let job_kind = query.job_kind.clone();
        let outcome = query.outcome.clone();
        let since = query.since_ms;
        let until = query.until_ms;
        let rows: Vec<JobLogRow> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, occurred_at, env, event, task_id, job_kind,
                            outcome, attempt, duration_ms, attrs, correlation_id
                       FROM job_log
                      WHERE (?1 IS NULL OR job_kind = ?1)
                        AND (?2 IS NULL OR outcome = ?2)
                        AND (?3 IS NULL OR occurred_at >= ?3)
                        AND (?4 IS NULL OR occurred_at < ?4)
                      ORDER BY occurred_at DESC, id
                      LIMIT ?5",
                )?;
                stmt.query_map(params![job_kind, outcome, since, until, limit], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .map_err(crate::sqlite::map::infra_err)?;

        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    occurred_at_ms,
                    env,
                    event,
                    task_id,
                    job_kind,
                    outcome,
                    attempt,
                    duration_ms,
                    attrs_json,
                    correlation_id,
                )| asterism_contract::dto::JobLogDto {
                    id: id.hyphenated().to_string(),
                    occurred_at_ms,
                    env,
                    event,
                    task_id,
                    job_kind,
                    outcome,
                    attempt,
                    duration_ms,
                    attrs_json,
                    correlation_id,
                },
            )
            .collect())
    }

    /// Lists every stream on one timeline, newest first.
    ///
    /// Reads the `observation` view — the single-timeline property the
    /// four separate write tables would otherwise cost, recovered on
    /// the read side rather than by conflating the write side.
    pub async fn all(
        &self,
        query: asterism_contract::query::ListObservationsQuery,
    ) -> Result<Vec<asterism_contract::dto::ObservationDto>, asterism_core::error::DomainError>
    {
        let Some(limit) = page_limit(query.limit) else {
            return Ok(Vec::new());
        };
        // Parsed at the boundary for the same reason `min_level` is: a
        // stream name the caller invented would otherwise be a filter
        // that silently matches nothing.
        let stream = query
            .stream
            .as_deref()
            .map(Stream::parse)
            .transpose()
            .map_err(asterism_core::error::DomainError::Validation)?
            .map(|s| s.as_str().to_string());
        let since = query.since_ms;
        let until = query.until_ms;
        let rows: Vec<ObservationRow> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT stream, id, occurred_at, env, event, attrs, correlation_id
                       FROM observation
                      WHERE (?1 IS NULL OR stream = ?1)
                        AND (?2 IS NULL OR occurred_at >= ?2)
                        AND (?3 IS NULL OR occurred_at < ?3)
                      ORDER BY occurred_at DESC, id
                      LIMIT ?4",
                )?;
                stmt.query_map(params![stream, since, until, limit], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .map_err(crate::sqlite::map::infra_err)?;

        Ok(rows
            .into_iter()
            .map(
                |(stream, id, occurred_at_ms, env, event, attrs_json, correlation_id)| {
                    asterism_contract::dto::ObservationDto {
                        stream,
                        id: id.hyphenated().to_string(),
                        occurred_at_ms,
                        env,
                        event,
                        attrs_json,
                        correlation_id,
                    }
                },
            )
            .collect())
    }

    /// Removes observations older than their stream's declared
    /// retention.
    ///
    /// Sits beside the reads rather than on a type of its own: expiry
    /// needs exactly what a read needs — the isle and
    /// [`Stream::table`] — and splitting it out would mean two types
    /// holding the same field for `JobDeps` to carry. Deleting rows
    /// past their window is not appending, so the invariant that
    /// records arrive only through [`DiagSink`] still holds.
    ///
    /// Paged and re-entrant rather than one unbounded `DELETE`: these
    /// tables share the single SQLite connection the whole application
    /// uses, and a sweep that removes a year of perf rows in one
    /// statement would hold that connection for as long as it takes.
    /// The caller re-runs while [`RetentionSweep::should_chain`] holds.
    ///
    /// The windows come from `STREAM_REGISTRY`, so "how long is this
    /// kept" is answered beside the stream it governs, not here. This
    /// is the clock, not the rule.
    pub async fn sweep_retention(
        &self,
        now_ms: i64,
        page: u32,
    ) -> Result<RetentionSweep, asterism_core::error::DomainError> {
        let plan: Vec<(&'static str, &'static str, i64)> = Stream::ALL
            .iter()
            .map(|stream| {
                (
                    stream.as_str(),
                    stream.table(),
                    stream.policy().retention_cutoff_ms(now_ms),
                )
            })
            .collect();
        let page = page as i64;
        self.isle
            .call(move |conn| {
                let mut removed = Vec::with_capacity(plan.len());
                for (stream, table, cutoff) in plan {
                    // `id IN (SELECT … LIMIT)` rather than
                    // `DELETE … LIMIT`, which needs a compile-time
                    // option SQLite is not always built with. Tag rows
                    // follow by cascade, and `changes()` does not count
                    // them — so the page arithmetic stays about
                    // records, not rows touched.
                    let n = conn.execute(
                        &format!(
                            "DELETE FROM {table}
                              WHERE id IN (SELECT id FROM {table}
                                            WHERE occurred_at < ?1
                                            ORDER BY occurred_at
                                            LIMIT ?2)"
                        ),
                        params![cutoff, page],
                    )?;
                    removed.push((stream, n as u64));
                }
                Ok(RetentionSweep { removed })
            })
            .await
            .map_err(crate::sqlite::map::infra_err)
    }
}

/// Clamps a requested page size, or `None` when nothing was asked for.
///
/// `limit = 0` short-circuits before the database is touched: it is a
/// legitimate way to ask "does this endpoint exist" without paying for
/// a scan.
fn page_limit(requested: u32) -> Option<i64> {
    match requested.min(LIST_LIMIT_CAP) {
        0 => None,
        n => Some(n as i64),
    }
}

/// What one retention pass removed, per stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetentionSweep {
    /// `(stream, rows removed)`, in registry order.
    pub removed: Vec<(&'static str, u64)>,
}

impl RetentionSweep {
    /// Total rows removed across all streams.
    pub fn total(&self) -> u64 {
        self.removed.iter().map(|(_, n)| n).sum()
    }

    /// Whether the caller should run another pass: some stream filled
    /// its page, so more rows are past retention than this pass took.
    pub fn should_chain(&self, page: u32) -> bool {
        self.removed.iter().any(|(_, n)| *n >= page as u64)
    }
}

/// Writes one record to the table its stream owns.
///
/// Whether it should be written at all was decided in
/// [`DiagSink::submit`]; by here the record is known to be wanted.
async fn insert(
    isle: &AsyncIsle,
    env: Env,
    record: ObservationRecord,
) -> Result<(), asterism_core::error::DomainError> {
    let stream = record.stream();
    let id = Uuid::new_v4();
    let occurred_at = chrono::Utc::now().timestamp_millis();
    let env = env.as_str();
    let event = record
        .event
        .clone()
        .unwrap_or_else(|| format!("{}.unnamed", Stream::Diag.as_str()));

    match stream {
        Stream::Job => {
            // One row per run, written at completion. `Jobs` (apalis)
            // owns the state machine and keeps only the current row;
            // this is the history of results, which is exactly what
            // that table does not keep.
            let job_kind = record.attr_str("job_kind").unwrap_or_else(|| event.clone());
            let outcome = record
                .attr_str("outcome")
                .unwrap_or_else(|| "unknown".to_string());
            // Defaulted, not left absent: the column is `NOT NULL`, so
            // an unnamed run would fail its insert and vanish — the
            // loudest possible record turning into the quietest.
            let task_id = record
                .attr_str("task_id")
                .unwrap_or_else(|| "unknown".to_string());
            let attempt = record.attr_i64("attempt").unwrap_or(1);
            let duration_ms = record.attr_i64("duration_ms");
            let attrs = record.attrs;
            isle.call(move |conn| {
                conn.execute(
                    "INSERT INTO job_log
                         (id, occurred_at, env, event, attrs,
                          task_id, job_kind, outcome, attempt, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id,
                        occurred_at,
                        env,
                        event,
                        attrs,
                        task_id,
                        job_kind,
                        outcome,
                        attempt,
                        duration_ms
                    ],
                )?;
                Ok(())
            })
            .await
        }
        Stream::Perf => {
            // `op` and `duration_ms` are columns because every perf
            // question groups or sorts by them; the rest of the timing
            // breakdown stays in `attrs`.
            let op = record.attr_str("op").unwrap_or_else(|| event.clone());
            let duration_ms = record.attr_i64("duration_ms").unwrap_or(0);
            let attrs = record.attrs;
            isle.call(move |conn| {
                conn.execute(
                    "INSERT INTO perf_log
                         (id, occurred_at, env, event, attrs, op, duration_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, occurred_at, env, event, attrs, op, duration_ms],
                )?;
                Ok(())
            })
            .await
        }
        // Everything else is a diagnostic. A record that claimed to be
        // an action or a job keeps its claimed name in the `event`
        // column, so the mislabelled call site is one query away rather
        // than being silently normalised out of existence.
        _ => {
            let attrs = record.attrs;
            let level = record.level;
            let target = record.target;
            let message = record.message;
            isle.call(move |conn| {
                conn.execute(
                    "INSERT INTO diag_log
                         (id, occurred_at, env, event, attrs, level, target, message)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, occurred_at, env, event, attrs, level, target, message],
                )?;
                Ok(())
            })
            .await
        }
    }
    .map_err(crate::sqlite::map::infra_err)
}

/// The environment stamped on every record this process writes.
///
/// Resolved once: the profile cannot change while the process runs, and
/// a diagnostic must not pay for an environment lookup. A profile that
/// fails to resolve is `custom` — refusing to record anything because
/// the classification is uncertain would lose exactly the diagnostics
/// that explain why.
pub(crate) fn current_env() -> Env {
    use std::sync::OnceLock;
    static ENV: OnceLock<Env> = OnceLock::new();
    *ENV.get_or_init(|| {
        crate::paths::active_profile()
            .map(Into::into)
            .unwrap_or(Env::Custom)
    })
}

/// `tracing` layer that turns each event into an [`ObservationRecord`].
pub struct ObservationLayer {
    sink: DiagSink,
}

impl<S: tracing::Subscriber> Layer<S> for ObservationLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RecordVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.take().unwrap_or_default();
        let name = visitor.event.take();
        self.sink.submit(ObservationRecord {
            event: name,
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message,
            attrs: visitor.into_attrs_json(),
        });
    }
}

/// Splits an event's fields into `message`, `event`, and everything else.
///
/// `message` and `event` are lifted out because they are not attributes
/// of the record — they identify it. Everything remaining is opaque
/// detail whose shape is a function of `event`, which is exactly the
/// `attrs` column.
#[derive(Default)]
struct RecordVisitor {
    message: Option<String>,
    event: Option<String>,
    attrs: serde_json::Map<String, serde_json::Value>,
}

impl RecordVisitor {
    fn into_attrs_json(self) -> Option<String> {
        if self.attrs.is_empty() {
            return None;
        }
        serde_json::to_string(&serde_json::Value::Object(self.attrs)).ok()
    }

    fn insert(&mut self, field: &Field, value: serde_json::Value) {
        self.insert_named(field.name(), value);
    }

    /// Name-based half of [`Self::insert`], split out so the
    /// identity/attribute partition can be tested without constructing
    /// a `tracing::Field` (which only a real callsite can mint).
    fn insert_named(&mut self, name: &str, value: serde_json::Value) {
        if let serde_json::Value::String(text) = &value {
            match name {
                "message" => {
                    self.message = Some(text.clone());
                    return;
                }
                "event" => {
                    self.event = Some(text.clone());
                    return;
                }
                _ => {}
            }
        }
        self.attrs.insert(name.to_string(), value);
    }
}

impl Visit for RecordVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert(field, serde_json::Value::String(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, serde_json::Value::String(value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, serde_json::Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, serde_json::Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, serde_json::Value::Bool(value));
    }
}

/// The sink the installed subscriber writes through. Global so that
/// whoever opens the database can hand it over without the handle being
/// threaded from `main` through every init seam.
static SINK: std::sync::OnceLock<DiagSink> = std::sync::OnceLock::new();

/// Installs the process-global subscriber. The sink it writes through
/// is a private global, handed the database later by [`attach`].
///
/// Call once, as early in `main` as possible — before anything that can
/// log. A second call is a no-op: `tracing` refuses to replace an
/// installed global, and swallowing that keeps a test which installs
/// its own subscriber from aborting the process.
pub fn install() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let sink = SINK.get_or_init(|| DiagSink::new(current_env())).clone();
    // stderr verbosity is the developer's dial.
    let stderr_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("asterism=info"));
    // The database's is not. Turning `RUST_LOG=debug` on to read one
    // stack trace must not start persisting every sqlx statement apalis
    // issues while polling — so the sink keeps its own fixed filter:
    // this workspace's targets only, info and above.
    //
    // This filter selects *candidates*; what is actually kept is the
    // per-stream policy applied at insert. The two are separate because
    // a filter can only see a level and a target, and the decision
    // ("perf timings, in development only") needs the stream.
    let db_filter = tracing_subscriber::EnvFilter::new("asterism=info");
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_filter(stderr_filter),
        )
        .with(ObservationLayer { sink }.with_filter(db_filter))
        .try_init();
}

/// Points the installed sink at the open database and flushes whatever
/// was captured before it existed.
///
/// A no-op when [`install`] was never called — a library test that
/// opens a database has no subscriber and wants none.
pub fn attach(isle: AsyncIsle) {
    if let Some(sink) = SINK.get() {
        sink.attach(isle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A diagnostic with the given level, named or not.
    fn diag(level: &str, message: &str) -> ObservationRecord {
        ObservationRecord {
            event: None,
            level: level.into(),
            target: "asterism_core::test".into(),
            message: message.into(),
            attrs: None,
        }
    }

    #[test]
    fn visitor_splits_identity_from_attributes() {
        // `message` and `event` identify the record; the rest is detail
        // whose shape depends on the event, and stays queryable.
        let mut v = RecordVisitor::default();
        v.insert_named("message", serde_json::Value::String("boom".into()));
        v.insert_named("event", serde_json::Value::String("diag.boom".into()));
        v.insert_named("asset_id", serde_json::Value::from(7));
        assert_eq!(v.message.as_deref(), Some("boom"));
        assert_eq!(v.event.as_deref(), Some("diag.boom"));
        let json = v.into_attrs_json().unwrap();
        assert_eq!(json, r#"{"asset_id":7}"#);
    }

    #[test]
    fn message_only_event_carries_no_attrs_blob() {
        let mut v = RecordVisitor::default();
        v.insert_named("message", serde_json::Value::String("text only".into()));
        assert_eq!(v.message.as_deref(), Some("text only"));
        assert_eq!(v.into_attrs_json(), None);
    }

    #[tokio::test]
    async fn records_queue_until_a_database_is_attached_then_flush() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let sink = DiagSink::new(Env::Dogfood);
        sink.submit(diag("WARN", "before attach"));
        // Still queued — nothing to write to yet.
        assert_eq!(count_rows(&isle).await, 0);

        sink.attach(isle.clone());
        // `attach` spawns the inserts; give them a turn to land.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(count_rows(&isle).await, 1);

        sink.submit(ObservationRecord {
            attrs: Some(r#"{"n":1}"#.into()),
            ..diag("ERROR", "after attach")
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(count_rows(&isle).await, 2);
    }

    #[tokio::test]
    async fn a_record_emitted_off_the_runtime_still_reaches_the_database() {
        // The regression that matters: `list_index`'s perf event fires
        // inside an `isle.call` closure, which rusqlite-isle runs on a
        // bare OS thread with no tokio context. Submitting must not
        // depend on being inside the runtime.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let sink = DiagSink::new(Env::Dev);
        sink.attach(isle.clone());

        let emitter = sink.clone();
        std::thread::spawn(move || {
            emitter.submit(ObservationRecord {
                event: Some("perf.list_index".into()),
                level: "INFO".into(),
                target: "asterism_infra::test".into(),
                message: "from a bare thread".into(),
                attrs: Some(r#"{"op":"list_index","duration_ms":3,"query_ms":3}"#.into()),
            });
        })
        .join()
        .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let (op, duration): (String, i64) = isle
            .call(|conn| {
                conn.query_row("SELECT op, duration_ms FROM perf_log", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
            })
            .await
            .unwrap();
        assert_eq!(op, "list_index");
        assert_eq!(duration, 3);
    }

    #[tokio::test]
    async fn a_record_lands_in_the_table_its_event_name_selects() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let sink = DiagSink::new(Env::Dev);
        sink.attach(isle.clone());

        sink.submit(ObservationRecord {
            event: Some("perf.list_index".into()),
            attrs: Some(r#"{"op":"list_index","duration_ms":9}"#.into()),
            ..diag("INFO", "timing")
        });
        // Unnamed, so it is a diagnostic — not silently dropped for
        // failing to declare a stream.
        sink.submit(diag("WARN", "unnamed"));
        // Claiming a stream the subscriber does not write must not let
        // a record dodge the diagnostics floor: it lands in `diag_log`
        // under the policy that actually governs it.
        sink.submit(ObservationRecord {
            event: Some("action.persona.switched".into()),
            ..diag("DEBUG", "mislabelled")
        });
        sink.submit(ObservationRecord {
            event: Some("job.cover_gen.failed".into()),
            attrs: Some(
                r#"{"task_id":"t-1","job_kind":"cover_gen","outcome":"failed","attempt":2,"duration_ms":140}"#
                    .into(),
            ),
            ..diag("INFO", "job run finished")
        });
        // A run that named no task still has to land: the column is
        // `NOT NULL`, so without a default this row would fail its
        // insert and disappear instead of being visibly incomplete.
        sink.submit(ObservationRecord {
            event: Some("job.cover_gen.panicked".into()),
            attrs: Some(r#"{"job_kind":"cover_gen","outcome":"panicked"}"#.into()),
            ..diag("INFO", "job run finished")
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(count_rows(&isle).await, 1);
        assert_eq!(count_table(&isle, "perf_log").await, 1);
        assert_eq!(count_table(&isle, "action_log").await, 0);

        // The five values a job question groups or filters by are
        // columns, recovered from the attributes the call site named.
        let rows: Vec<(String, String, String, i64, Option<i64>)> = isle
            .call(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT task_id, job_kind, outcome, attempt, duration_ms
                       FROM job_log ORDER BY outcome",
                )?;
                stmt.query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })?
                .collect::<Result<_, _>>()
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "t-1".into(),
                    "cover_gen".into(),
                    "failed".into(),
                    2,
                    Some(140)
                ),
                // Defaulted rather than dropped, and visibly so.
                (
                    "unknown".into(),
                    "cover_gen".into(),
                    "panicked".into(),
                    1,
                    None
                ),
            ]
        );
    }

    #[tokio::test]
    async fn per_stream_policy_decides_what_is_kept() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        // Dogfood: perf is development-only, and `debug` is below the
        // diagnostics floor.
        let sink = DiagSink::new(Env::Dogfood);
        sink.attach(isle.clone());

        sink.submit(ObservationRecord {
            event: Some("perf.list_index".into()),
            attrs: Some(r#"{"op":"list_index","duration_ms":9}"#.into()),
            ..diag("INFO", "timing")
        });
        sink.submit(diag("DEBUG", "developer detail"));
        // Startup narration is once per run and is often the only
        // durable record of how this process was configured, so the
        // floor sits below it.
        sink.submit(diag("INFO", "serving on port 8989"));
        sink.submit(diag("WARN", "worth keeping"));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_eq!(count_table(&isle, "perf_log").await, 0);
        assert_eq!(count_rows(&isle).await, 2);
    }

    #[tokio::test]
    async fn a_dropped_record_never_occupies_a_queue_slot() {
        // The queue protects high-value diagnostics from being crowded
        // out. A perf record under dogfood is destined for nothing, so
        // it must not take a slot on its way to being discarded.
        let sink = DiagSink::new(Env::Dogfood);
        for _ in 0..(PENDING_CAP * 2) {
            sink.submit(ObservationRecord {
                event: Some("perf.list_index".into()),
                attrs: Some(r#"{"op":"list_index","duration_ms":1}"#.into()),
                ..diag("INFO", "timing")
            });
        }
        let state = sink.state.lock().unwrap();
        assert_eq!(state.pending.len(), 0);
        assert_eq!(state.dropped, 0);
    }

    #[tokio::test]
    async fn the_queue_is_bounded_so_a_databaseless_run_cannot_grow_without_end() {
        let sink = DiagSink::new(Env::Dogfood);
        for i in 0..(PENDING_CAP + 10) {
            sink.submit(diag("WARN", &format!("{i}")));
        }
        let state = sink.state.lock().unwrap();
        assert_eq!(state.pending.len(), PENDING_CAP);
        assert_eq!(state.dropped, 10);
    }

    /// Counts rows in one stream table.
    async fn count_table(isle: &AsyncIsle, table: &'static str) -> i64 {
        isle.call(move |conn| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        })
        .await
        .unwrap()
    }

    /// Seeds one row directly, so reader tests do not depend on the
    /// subscriber or on task scheduling.
    async fn seed(isle: &AsyncIsle, occurred_at: i64, level: &str, target: &str) {
        let (level, target) = (level.to_string(), target.to_string());
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO diag_log
                     (id, occurred_at, env, event, level, target, message)
                 VALUES (?1, ?2, 'dev', 'diag.seeded', ?3, ?4, 'seeded')",
                params![Uuid::new_v4(), occurred_at, level, target],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_returns_newest_first() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed(&isle, 100, "INFO", "asterism_core::a").await;
        seed(&isle, 300, "WARN", "asterism_core::b").await;
        seed(&isle, 200, "ERROR", "asterism_core::c").await;

        let rows = ObservationStore::new(isle)
            .diag(asterism_contract::query::ListDiagQuery::default())
            .await
            .unwrap();
        let order: Vec<i64> = rows.iter().map(|r| r.occurred_at_ms).collect();
        assert_eq!(order, vec![300, 200, 100]);
    }

    #[tokio::test]
    async fn min_level_is_a_floor_not_an_exact_match() {
        // Asking for `warn` must not hide the errors above it — that
        // is the whole point of the filter.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed(&isle, 100, "INFO", "asterism_core::a").await;
        seed(&isle, 200, "WARN", "asterism_core::b").await;
        seed(&isle, 300, "ERROR", "asterism_core::c").await;

        let rows = ObservationStore::new(isle)
            .diag(asterism_contract::query::ListDiagQuery {
                min_level: Some("warn".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let levels: Vec<&str> = rows.iter().map(|r| r.level.as_str()).collect();
        assert_eq!(levels, vec!["ERROR", "WARN"]);
    }

    #[tokio::test]
    async fn target_and_time_window_narrow_the_listing() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed(
            &isle,
            100,
            "WARN",
            "asterism_core::application::app_setting",
        )
        .await;
        seed(&isle, 200, "WARN", "asterism_infra::jobs").await;
        seed(
            &isle,
            300,
            "WARN",
            "asterism_core::application::app_setting",
        )
        .await;

        let reader = ObservationStore::new(isle);
        let by_target = reader
            .diag(asterism_contract::query::ListDiagQuery {
                target: Some("app_setting".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(by_target.len(), 2);

        let windowed = reader
            .diag(asterism_contract::query::ListDiagQuery {
                since_ms: Some(200),
                until_ms: Some(300),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(windowed.len(), 1);
        assert_eq!(windowed[0].occurred_at_ms, 200);
    }

    #[tokio::test]
    async fn limit_is_honoured_and_zero_reads_nothing() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        for i in 0..5 {
            seed(&isle, i, "WARN", "asterism_core::a").await;
        }
        let reader = ObservationStore::new(isle);
        let capped = reader
            .diag(asterism_contract::query::ListDiagQuery {
                limit: 2,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(capped.len(), 2);

        let none = reader
            .diag(asterism_contract::query::ListDiagQuery {
                limit: 0,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_min_level_is_rejected_rather_than_ignored() {
        // A filter that silently matches everything is worse than no
        // filter: the caller believes they narrowed the listing.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed(&isle, 100, "INFO", "asterism_core::a").await;

        let err = ObservationStore::new(isle)
            .diag(asterism_contract::query::ListDiagQuery {
                min_level: Some("warning".into()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            asterism_core::error::DomainError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn a_stored_level_outside_the_closed_set_never_matches_a_floor() {
        // `level IN (…)` names the five known levels, so a row written
        // with anything else simply is not in the set — it cannot leak
        // into "show me the bad ones" through a rank fallback.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed(&isle, 100, "FATAL", "asterism_core::a").await;
        seed(&isle, 200, "WARN", "asterism_core::a").await;

        let rows = ObservationStore::new(isle)
            .diag(asterism_contract::query::ListDiagQuery {
                min_level: Some("trace".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let levels: Vec<&str> = rows.iter().map(|r| r.level.as_str()).collect();
        assert_eq!(levels, vec!["WARN"]);
    }

    #[test]
    fn at_least_selects_the_level_and_everything_above_it() {
        assert_eq!(
            DiagLevel::Warn.at_least(),
            vec![DiagLevel::Error, DiagLevel::Warn]
        );
        assert_eq!(DiagLevel::Error.at_least(), vec![DiagLevel::Error]);
        assert_eq!(DiagLevel::Trace.at_least().len(), DiagLevel::ALL.len());
    }

    #[test]
    fn parse_is_case_insensitive_and_names_the_accepted_set_on_failure() {
        assert_eq!(DiagLevel::parse("WaRn").unwrap(), DiagLevel::Warn);
        assert_eq!(DiagLevel::parse(" error ").unwrap(), DiagLevel::Error);
        let err = DiagLevel::parse("warning").unwrap_err();
        // The message has to be enough to fix the call without looking
        // anything up.
        assert!(err.contains("WARN"), "{err}");
        assert!(err.contains("ERROR"), "{err}");
    }

    #[test]
    fn every_level_the_subscriber_can_write_is_parseable() {
        // The writer stores `tracing::Level::to_string()`; if the two
        // sets ever diverge, a persisted row becomes unfilterable.
        for level in [
            tracing::Level::TRACE,
            tracing::Level::DEBUG,
            tracing::Level::INFO,
            tracing::Level::WARN,
            tracing::Level::ERROR,
        ] {
            let stored = level.to_string();
            DiagLevel::parse(&stored)
                .unwrap_or_else(|e| panic!("{stored} is not a known DiagLevel: {e}"));
        }
    }

    #[tokio::test]
    async fn a_filtered_page_is_exact_however_the_rows_are_arranged() {
        // The filter is a SQL predicate, so where the matches sit
        // relative to the noise does not matter. Both arrangements —
        // matches newest, matches oldest — must return the full page.
        for warnings_first in [true, false] {
            let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
            let (warn_range, info_range) = if warnings_first {
                (0..5, 5..45)
            } else {
                (40..45, 0..40)
            };
            for i in warn_range {
                seed(&isle, i, "WARN", "asterism_core::a").await;
            }
            for i in info_range {
                seed(&isle, i, "INFO", "asterism_core::a").await;
            }

            let rows = ObservationStore::new(isle)
                .diag(asterism_contract::query::ListDiagQuery {
                    min_level: Some("warn".into()),
                    limit: 5,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(
                rows.len(),
                5,
                "warnings_first={warnings_first}: a page must not come up short"
            );
            assert!(rows.iter().all(|r| r.level == "WARN"));
        }
    }

    #[tokio::test]
    async fn a_short_page_means_exhausted_not_truncated() {
        // The contract `limit` now promises: fewer rows than asked for
        // means there were no more matches, full stop.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        for i in 0..100 {
            seed(&isle, i, "INFO", "asterism_core::a").await;
        }
        seed(&isle, 100, "ERROR", "asterism_core::a").await;

        let rows = ObservationStore::new(isle)
            .diag(asterism_contract::query::ListDiagQuery {
                min_level: Some("error".into()),
                limit: 50,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    async fn count_rows(isle: &AsyncIsle) -> i64 {
        isle.call(move |conn| conn.query_row("SELECT COUNT(*) FROM diag_log", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    /// Seeds one row into each stream at the given timestamp.
    async fn seed_every_stream(isle: &AsyncIsle, occurred_at: i64) {
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO action_log (id, occurred_at, env, event)
                 VALUES (?1, ?2, 'dev', 'action.persona.switched')",
                params![Uuid::new_v4(), occurred_at],
            )?;
            conn.execute(
                "INSERT INTO job_log
                     (id, occurred_at, env, event, task_id, job_kind, outcome,
                      attempt, duration_ms)
                 VALUES (?1, ?2, 'dev', 'job.cover_gen.failed', 't-1',
                         'cover_gen', 'failed', 1, 12)",
                params![Uuid::new_v4(), occurred_at],
            )?;
            conn.execute(
                "INSERT INTO diag_log
                     (id, occurred_at, env, event, level, target, message)
                 VALUES (?1, ?2, 'dev', 'diag.search.commit_failed', 'WARN',
                         'asterism_core', 'text')",
                params![Uuid::new_v4(), occurred_at],
            )?;
            conn.execute(
                "INSERT INTO perf_log
                     (id, occurred_at, env, event, op, duration_ms)
                 VALUES (?1, ?2, 'dev', 'perf.list_index', 'list_index', 7)",
                params![Uuid::new_v4(), occurred_at],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_union_view_puts_every_stream_on_one_timeline() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed_every_stream(&isle, 100).await;
        let store = ObservationStore::new(isle);

        let all = store
            .all(asterism_contract::query::ListObservationsQuery::default())
            .await
            .unwrap();
        let mut streams: Vec<&str> = all.iter().map(|o| o.stream.as_str()).collect();
        streams.sort_unstable();
        assert_eq!(streams, vec!["action", "diag", "job", "perf"]);

        // Narrowing to one stream is the same read, filtered.
        let jobs = store
            .all(asterism_contract::query::ListObservationsQuery {
                stream: Some("job".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].event, "job.cover_gen.failed");
    }

    #[tokio::test]
    async fn an_unknown_stream_is_rejected_rather_than_matching_nothing() {
        // Same failure mode `min_level` had: a filter the caller
        // believes narrowed the listing, silently returning an empty
        // page instead of saying the name was wrong.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let err = ObservationStore::new(isle)
            .all(asterism_contract::query::ListObservationsQuery {
                stream: Some("jobs".into()),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            asterism_core::error::DomainError::Validation(_)
        ));
    }

    #[tokio::test]
    async fn each_stream_reader_returns_its_own_columns() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        seed_every_stream(&isle, 100).await;
        let store = ObservationStore::new(isle);

        let perf = store
            .perf(asterism_contract::query::ListPerfQuery::default())
            .await
            .unwrap();
        assert_eq!(perf.len(), 1);
        assert_eq!(
            (perf[0].op.as_str(), perf[0].duration_ms),
            ("list_index", 7)
        );

        let jobs = store
            .job_log(asterism_contract::query::ListJobLogQuery {
                outcome: Some("failed".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_kind, "cover_gen");
        assert_eq!(jobs[0].attempt, 1);
        assert_eq!(jobs[0].task_id, "t-1");

        // An outcome nobody wrote is an empty page, not an error.
        // `outcome` looks closed — the writer's enum has four variants
        // — but the column is not: this sink writes `"unknown"` when a
        // record arrives without the attribute, so values outside the
        // enum legitimately exist. A stream name is different, and is
        // rejected: the view's `stream` column is a literal, so a
        // fifth value cannot be in the table at all.
        let none = store
            .job_log(asterism_contract::query::ListJobLogQuery {
                outcome: Some("completed".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn retention_expires_each_stream_on_its_own_window() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let now = 1_800_000_000_000_i64;
        let day = 24 * 60 * 60 * 1_000;
        // 30 days back: past `perf`'s week, inside everything else's.
        seed_every_stream(&isle, now - 30 * day).await;
        seed_every_stream(&isle, now).await;
        let store = ObservationStore::new(isle.clone());

        let sweep = store.sweep_retention(now, 100).await.unwrap();
        assert_eq!(sweep.total(), 1);
        assert_eq!(
            sweep.removed,
            vec![("action", 0), ("job", 0), ("diag", 0), ("perf", 1)]
        );
        assert_eq!(count_table(&isle, "perf_log").await, 1);
        assert_eq!(count_table(&isle, "action_log").await, 2);
    }

    #[tokio::test]
    async fn a_full_retention_page_asks_to_be_run_again() {
        // The chain is what keeps a year of backlog from leaving in one
        // statement on the connection the whole application shares.
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let now = 1_800_000_000_000_i64;
        let day = 24 * 60 * 60 * 1_000;
        for i in 0..3 {
            seed_every_stream(&isle, now - (30 + i) * day).await;
        }
        let store = ObservationStore::new(isle.clone());

        let first = store.sweep_retention(now, 2).await.unwrap();
        assert_eq!(first.total(), 2);
        assert!(first.should_chain(2));

        let second = store.sweep_retention(now, 2).await.unwrap();
        assert_eq!(second.total(), 1);
        assert!(!second.should_chain(2));
        assert_eq!(count_table(&isle, "perf_log").await, 0);
    }

    #[tokio::test]
    async fn retention_takes_the_tags_with_the_record() {
        let (isle, _driver) = crate::sqlite::open_and_migrate_in_memory().await.unwrap();
        let now = 1_800_000_000_000_i64;
        let old = now - 30 * 24 * 60 * 60 * 1_000;
        let id = Uuid::new_v4();
        isle.call(move |conn| {
            conn.execute("PRAGMA foreign_keys = ON", [])?;
            conn.execute(
                "INSERT INTO perf_log (id, occurred_at, env, event, op, duration_ms)
                 VALUES (?1, ?2, 'dev', 'perf.list_index', 'list_index', 7)",
                params![id, old],
            )?;
            conn.execute(
                "INSERT INTO perf_log_tag (record_id, tag) VALUES (?1, 'slow')",
                params![id],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        ObservationStore::new(isle.clone())
            .sweep_retention(now, 100)
            .await
            .unwrap();
        assert_eq!(count_table(&isle, "perf_log_tag").await, 0);
    }
}
