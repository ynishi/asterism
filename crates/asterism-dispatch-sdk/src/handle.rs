//! `Handle` — the exporter's backend-side reference to an in-flight job.
//!
//! The exporter returns one of these from [`crate::Exporter::dispatch`]
//! and receives it back on every subsequent [`crate::Exporter::poll`]
//! and [`crate::Exporter::harvest`] call. The core persists the raw
//! bytes verbatim (opaque JSON payload); the exporter is free to
//! interpret them however it needs to reach the same backend job on
//! process restart.
//!
//! The `kind` slug lets the exporter fast-fail with a clear error
//! ("this handle was issued by ComfyExporter, not GeminiExporter")
//! when the caller feeds the wrong handle to the wrong adapter — that
//! only happens on programmer error, but the failure mode is worth
//! naming.

use serde::{Deserialize, Serialize};

/// Opaque reference to one in-flight backend job.
///
/// Serialised into the persisted `DispatchJob.handle_json` column so
/// polling / harvesting survives a server restart. Exporters embed
/// whatever they need to reach the backend again (ComfyUI prompt id,
/// Gemini operation name, VDSL run task id, filesystem watch cookie,
/// …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handle {
    /// Exporter slug that issued this handle (`"comfy"`, `"gemini"`,
    /// `"vdsl"`, `"alc-sd-bake"`, …). Used only for cross-exporter
    /// mismatch guard, not for routing (routing goes through the
    /// `DispatchJob.exporter_slug` field the core owns).
    pub kind: String,
    /// Exporter-specific reference blob. Exporters typically build
    /// this with `serde_json::json!(...)`.
    pub payload: serde_json::Value,
}

impl Handle {
    /// Wraps an exporter-specific payload with the exporter's own
    /// slug. Convenience for exporters that carry the slug as a
    /// `const` in their impl.
    pub fn new(kind: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
        }
    }
}
