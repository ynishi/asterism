//! Observability domain — the four streams and their policies.
//!
//! What the application records about itself is split by *writer,
//! volume, value and read pattern*, because those four properties
//! differ across the four kinds and every downstream decision
//! (retention, sampling, where a record may be read) follows from them.
//!
//! This module owns the classification. It does not write anything —
//! `asterism-infra` holds the tables and the sink, and consults
//! [`STREAM_REGISTRY`] for policy rather than repeating it.

use asterism_contract::query::DiagLevel;

/// One of the four observation streams.
///
/// The stream is also the first segment of every event name
/// (`job.cover_gen.failed`), so a record is self-classifying and a
/// misfiled one is visible on sight.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Stream {
    /// What the person did.
    Action,
    /// What a job did.
    Job,
    /// What the application decided or failed at.
    Diag,
    /// How long something took.
    Perf,
}

impl Stream {
    /// Every stream, in a fixed canonical order.
    pub const ALL: [Stream; 4] = [Stream::Action, Stream::Job, Stream::Diag, Stream::Perf];

    /// Stable slug — the event-name prefix and the `stream` column of
    /// the `observation` view.
    pub const fn as_str(self) -> &'static str {
        match self {
            Stream::Action => "action",
            Stream::Job => "job",
            Stream::Diag => "diag",
            Stream::Perf => "perf",
        }
    }

    /// Table this stream is stored in.
    pub const fn table(self) -> &'static str {
        match self {
            Stream::Action => "action_log",
            Stream::Job => "job_log",
            Stream::Diag => "diag_log",
            Stream::Perf => "perf_log",
        }
    }

    /// Side table carrying this stream's tags.
    pub const fn tag_table(self) -> &'static str {
        match self {
            Stream::Action => "action_log_tag",
            Stream::Job => "job_log_tag",
            Stream::Diag => "diag_log_tag",
            Stream::Perf => "perf_log_tag",
        }
    }

    /// Parses a slug. The error lists what is accepted, so a caller
    /// that mistyped learns the closed set instead of silently
    /// selecting nothing.
    pub fn parse(raw: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|s| s.as_str().eq_ignore_ascii_case(raw.trim()))
            .ok_or_else(|| {
                let accepted: Vec<&str> = Self::ALL.iter().map(|s| s.as_str()).collect();
                format!(
                    "unknown stream {raw:?}; expected one of {}",
                    accepted.join(", ")
                )
            })
    }

    /// Which stream an event name belongs to, by its first segment.
    ///
    /// Returns `None` for an unprefixed or unknown name — that is a
    /// misfiled record, not a fifth stream.
    pub fn of_event(event: &str) -> Option<Self> {
        let (head, _) = event.split_once('.')?;
        Self::ALL.into_iter().find(|s| s.as_str() == head)
    }

    /// Policy declared for this stream.
    pub fn policy(self) -> &'static StreamPolicy {
        STREAM_REGISTRY
            .iter()
            .find(|p| p.stream == self)
            .expect("every stream is in STREAM_REGISTRY")
    }
}

/// Which dataset a record was produced against.
///
/// Its absence is what let a development-only performance probe become
/// always-on production persistence: without this column there was no
/// way to ask "was this from the profile I actually use".
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Env {
    /// Disposable development data.
    Dev,
    /// Durable real-world daily-use data.
    Dogfood,
    /// Reproducible large/stress dataset.
    Bench,
    /// Explicit home with no named profile.
    Custom,
    /// The record predates this column, or the profile could not be
    /// resolved when it was written.
    ///
    /// A real state rather than a gap: rows carried over from before
    /// the four-stream split genuinely have no environment, and a
    /// reader that filters on `Env` must be able to name them instead
    /// of having them vanish from every query.
    Unknown,
}

impl Env {
    /// Every environment.
    pub const ALL: [Env; 5] = [
        Env::Dev,
        Env::Dogfood,
        Env::Bench,
        Env::Custom,
        Env::Unknown,
    ];

    /// Stable slug, matching the data-profile slugs used on disk.
    pub const fn as_str(self) -> &'static str {
        match self {
            Env::Dev => "dev",
            Env::Dogfood => "dogfood",
            Env::Bench => "bench",
            Env::Custom => "custom",
            Env::Unknown => "unknown",
        }
    }

    /// Whether this environment is a measurement one — the condition
    /// [`StreamPolicy::dev_only`] tests.
    ///
    /// `dev` and `bench` qualify. `bench` was excluded until 2026-08-05
    /// on the blanket "only dev counts" reading, which left the bench
    /// driver's cold-load measurement with an empty
    /// `GET /asterism/perf` on the very profile that exists to be
    /// measured — a bench home is disposable by definition, so the
    /// failure this gate exists to prevent cannot occur there. It can
    /// on the others: `custom` is an explicit home that may well be the
    /// durable one, and an unresolved profile is not evidence of
    /// anything; treating either as measurement would let the
    /// highest-volume stream persist exactly where the design says it
    /// must not.
    pub const fn is_measurement(self) -> bool {
        matches!(self, Env::Dev | Env::Bench)
    }

    /// Parses a slug, listing the accepted set on failure.
    pub fn parse(raw: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|e| e.as_str().eq_ignore_ascii_case(raw.trim()))
            .ok_or_else(|| {
                let accepted: Vec<&str> = Self::ALL.iter().map(|e| e.as_str()).collect();
                format!(
                    "unknown env {raw:?}; expected one of {}",
                    accepted.join(", ")
                )
            })
    }
}

/// Retention, sampling and persistence floor for one stream.
///
/// Declared in code next to its subject for the same reason
/// `SETTING_REGISTRY` is: a policy that lives beside the thing it
/// governs cannot drift from it.
#[derive(Debug)]
pub struct StreamPolicy {
    /// Stream this policy governs.
    pub stream: Stream,
    /// How long rows are kept before the sweep removes them.
    pub retention_days: u32,
    /// Lowest severity worth persisting, for streams that carry a
    /// severity at all. `None` means every record of this stream is
    /// persisted regardless of level.
    pub persist_floor: Option<DiagLevel>,
    /// When true, records are persisted only in a measurement
    /// environment ([`Env::is_measurement`]: `dev` / `bench`). `Perf`
    /// is the case: its value is in aggregate over a session, not in a
    /// permanent record, and it is the highest volume stream by an
    /// order of magnitude.
    pub dev_only: bool,
    /// One sentence: what question this stream answers.
    pub summary: &'static str,
}

impl StreamPolicy {
    /// Whether a record of this stream, at this level, in this
    /// environment, should be written at all.
    ///
    /// `level` is `None` for streams that carry no severity.
    pub fn should_persist(&self, env: Env, level: Option<DiagLevel>) -> bool {
        if self.dev_only && !env.is_measurement() {
            return false;
        }
        match (self.persist_floor, level) {
            (Some(floor), Some(level)) => level >= floor,
            // A floor with nothing to compare it against would silently
            // drop everything; treat the record as unclassified and
            // keep it rather than lose it.
            (Some(_), None) => true,
            (None, _) => true,
        }
    }

    /// Cut-off timestamp for the retention sweep: rows older than this
    /// are removed.
    pub fn retention_cutoff_ms(&self, now_ms: i64) -> i64 {
        now_ms - (self.retention_days as i64) * 24 * 60 * 60 * 1_000
    }
}

/// Closed registry of stream policies.
///
/// The retention figures follow the value-per-row column of the design
/// document: what is individually valuable is kept long, what is
/// valuable only in aggregate is kept just long enough to aggregate.
pub const STREAM_REGISTRY: &[StreamPolicy] = &[
    StreamPolicy {
        stream: Stream::Action,
        retention_days: 365,
        persist_floor: None,
        dev_only: false,
        summary: "What the person did.",
    },
    StreamPolicy {
        stream: Stream::Job,
        retention_days: 90,
        persist_floor: None,
        dev_only: false,
        summary: "What a job did, one row per run at completion.",
    },
    StreamPolicy {
        stream: Stream::Diag,
        retention_days: 365,
        // The floor sits at `info`, not `warn`. What made info-level
        // diagnostics voluminous was the per-listing perf probe, and
        // that is its own stream now; what remains at info is startup
        // narration — once per run, and the only durable record of
        // things like which port the bundled app is serving on.
        // Below info is `debug`, which is a developer's dial and does
        // not belong in a durable table.
        persist_floor: Some(DiagLevel::Info),
        dev_only: false,
        summary: "What the application decided or failed at.",
    },
    StreamPolicy {
        stream: Stream::Perf,
        retention_days: 7,
        persist_floor: None,
        dev_only: true,
        summary: "How long something took.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stream_has_exactly_one_policy() {
        assert_eq!(STREAM_REGISTRY.len(), Stream::ALL.len());
        for stream in Stream::ALL {
            // Panics if absent or ambiguous.
            assert_eq!(stream.policy().stream, stream);
        }
    }

    #[test]
    fn an_event_name_classifies_itself() {
        assert_eq!(Stream::of_event("job.cover_gen.failed"), Some(Stream::Job));
        assert_eq!(Stream::of_event("perf.list_index"), Some(Stream::Perf));
        // A name without a stream prefix is misfiled, not a new stream.
        assert_eq!(Stream::of_event("cover_gen.failed"), None);
        assert_eq!(Stream::of_event("audit.something"), None);
    }

    #[test]
    fn an_unknown_slug_is_rejected_with_the_accepted_set() {
        let err = Stream::parse("actions").unwrap_err();
        assert!(err.contains("action"), "{err}");
        assert!(Env::parse("prod").unwrap_err().contains("dogfood"));
        // Case and surrounding space are caller noise, not a mismatch.
        assert_eq!(Stream::parse(" Diag ").unwrap(), Stream::Diag);
        assert_eq!(Env::parse("DOGFOOD").unwrap(), Env::Dogfood);
    }

    #[test]
    fn perf_is_persisted_in_measurement_envs_and_not_in_dogfood() {
        let perf = Stream::Perf.policy();
        assert!(perf.should_persist(Env::Dev, None));
        // The bench profile exists to be measured; an empty perf read
        // on it is what the cold-load run hit (2026-08-05).
        assert!(perf.should_persist(Env::Bench, None));
        assert!(!perf.should_persist(Env::Dogfood, None));
    }

    #[test]
    fn the_diag_floor_admits_startup_narration_but_not_a_developer_dial() {
        let diag = Stream::Diag.policy();
        assert!(diag.should_persist(Env::Dogfood, Some(DiagLevel::Error)));
        assert!(diag.should_persist(Env::Dogfood, Some(DiagLevel::Warn)));
        // Once-per-run narration is often the only durable record of
        // how a process was configured.
        assert!(diag.should_persist(Env::Dogfood, Some(DiagLevel::Info)));
        assert!(!diag.should_persist(Env::Dogfood, Some(DiagLevel::Debug)));
        assert!(!diag.should_persist(Env::Dogfood, Some(DiagLevel::Trace)));
    }

    #[test]
    fn only_measurement_envs_count_for_the_perf_stream() {
        // An explicit home may well be the durable one, and a profile
        // that would not resolve is not evidence of anything — treating
        // either as measurement would persist the highest-volume stream
        // exactly where the design says it must not go. Bench is on the
        // other side of that line: disposable by definition.
        let perf = Stream::Perf.policy();
        for env in [Env::Dev, Env::Bench] {
            assert!(perf.should_persist(env, None), "{env:?}");
        }
        for env in [Env::Dogfood, Env::Custom, Env::Unknown] {
            assert!(!perf.should_persist(env, None), "{env:?}");
        }
    }

    #[test]
    fn the_env_carried_by_pre_split_rows_is_a_member_of_the_closed_set() {
        // V36 stamps migrated rows `unknown`. A reader mapping that
        // column to `Env` must be able to name it, or every row written
        // before the split disappears from every filtered query.
        assert_eq!(Env::parse("unknown").unwrap(), Env::Unknown);
    }

    #[test]
    fn retention_cutoff_is_the_declared_window_back_from_now() {
        let now = 1_800_000_000_000;
        let day = 24 * 60 * 60 * 1_000;
        assert_eq!(
            Stream::Perf.policy().retention_cutoff_ms(now),
            now - 7 * day
        );
        assert_eq!(
            Stream::Action.policy().retention_cutoff_ms(now),
            now - 365 * day
        );
    }
}
