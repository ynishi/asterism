//! Lifecycle state of a dispatch as seen by the exporter's
//! [`crate::Exporter::poll`] response.
//!
//! The core drives the state machine — the exporter only reports "what
//! I see on the backend right now". Terminal states (`Done` / `Failed`
//! / `Cancelled`) stop the poll loop; non-terminal states keep it
//! running with a re-enqueue delay derived from the progress hint (if
//! any).

use serde::{Deserialize, Serialize};

/// Optional soft progress signal returned alongside a
/// [`DispatchState::Running`] response.
///
/// Not a strict contract — some backends only expose "we're still
/// working", others hand back a percent + a message. The exporter
/// fills in whichever fields it can cheaply lift; unknown fields are
/// left as `None`. The core surfaces this to the UI progress channel
/// but never uses it as a control signal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProgressHint {
    /// Discrete step count (units are backend-defined; ComfyUI hands
    /// back sampler step index, LoRA train hands back training step).
    pub current: Option<u64>,
    /// Expected total steps (`None` when the backend does not
    /// enumerate).
    pub total: Option<u64>,
    /// Human-readable status message ("sampling", "vae decode",
    /// "checkpointing", …).
    pub message: Option<String>,
}

/// State of one dispatched backend job.
///
/// Serialised into `DispatchJob.state_slug` at rest via
/// [`DispatchState::slug`] for cheap SQL filtering; the terminal
/// variants also carry payloads (`error` message, `Cancelled` reason)
/// for the detail view. The persisted column stores the slug only —
/// the payload lives in adjacent columns (`error_message`,
/// `cancelled_reason`) so the enum stays open to future variants
/// without a stored-format migration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DispatchState {
    /// Not yet handed to the backend (queued locally).
    Pending,
    /// Backend accepted the job and is working on it.
    Running(ProgressHint),
    /// Backend reports the job finished successfully; ready for
    /// [`crate::Exporter::harvest`]. Idempotent — polling a `Done`
    /// handle again should keep returning `Done`.
    Done,
    /// Backend reports an unrecoverable failure. The core stops the
    /// poll loop and records the reason on the job row.
    Failed {
        /// Short human-readable reason (backend error message,
        /// exporter-side translation of a status code, …).
        message: String,
    },
    /// The core cancelled the job (user action or shutdown). Exporters
    /// return this if they can confirm the backend accepted the
    /// cancel; otherwise they leave the state at whatever the backend
    /// last reported and let the core close out with its own reason.
    Cancelled {
        /// Optional reason (`"user"`, `"shutdown"`, `"timeout"`, …).
        reason: Option<String>,
    },
}

impl DispatchState {
    /// Slug used as the persisted `state_slug` column value.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running(_) => "running",
            Self::Done => "done",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
        }
    }

    /// True when the state signals the poll loop should stop and the
    /// core should either harvest (Done) or close out (Failed /
    /// Cancelled).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_covers_every_variant_stably() {
        // Guard against silent slug drift — these strings flow into
        // the DB `state_slug` column and are consumed by the poll loop
        // + list queries.
        assert_eq!(DispatchState::Pending.slug(), "pending");
        assert_eq!(
            DispatchState::Running(ProgressHint::default()).slug(),
            "running"
        );
        assert_eq!(DispatchState::Done.slug(), "done");
        assert_eq!(
            DispatchState::Failed {
                message: "boom".into()
            }
            .slug(),
            "failed"
        );
        assert_eq!(
            DispatchState::Cancelled { reason: None }.slug(),
            "cancelled"
        );
    }

    #[test]
    fn only_terminal_states_stop_the_poll_loop() {
        assert!(!DispatchState::Pending.is_terminal());
        assert!(!DispatchState::Running(ProgressHint::default()).is_terminal());
        assert!(DispatchState::Done.is_terminal());
        assert!(
            DispatchState::Failed {
                message: "boom".into()
            }
            .is_terminal()
        );
        assert!(DispatchState::Cancelled { reason: None }.is_terminal());
    }
}
