//! `AttemptRecord` — what an exporter has to say about a call it made,
//! whether or not the call produced a [`Handle`](crate::Handle).
//!
//! A [`Handle`](crate::Handle) is the record of a job the backend
//! accepted, and it is the only thing [`Exporter::dispatch`] can hand
//! back. That leaves the refused submit — the case a reader has the most
//! questions about — with nowhere to write: which endpoint, with which
//! body, and what the backend actually said all go out with the error,
//! and what survives on the row is one message string.
//!
//! This is the second channel. The core hands the exporter an
//! [`AttemptRecorder`] on [`DispatchContext.attempt`], the exporter
//! records what it sent and what came back as it makes the call, and the
//! core persists whatever landed there — on the arm that returned a
//! handle and on the arm that returned an error alike.
//!
//! Kept separate from the return type on purpose. Widening
//! [`ExporterError`](crate::ExporterError) would make every error site
//! decide what to attach, and returning a `Handle` for a job that does
//! not exist would put a reference to nothing where the poll loop
//! expects a backend job. Both change what the trait's own words mean;
//! a recorder the exporter writes to leaves `dispatch` saying exactly
//! what it said before.
//!
//! [`Exporter::dispatch`]: crate::Exporter::dispatch
//! [`DispatchContext.attempt`]: crate::DispatchContext::attempt

use serde_json::Value;

/// One call as the exporter wants it remembered.
///
/// The shape of `payload` belongs to the exporter, exactly as
/// [`Handle::payload`](crate::Handle::payload) does: the core writes it
/// down and hands it back on reads without looking inside. `kind` names
/// whose shape it is, so a reader that does want to look knows which
/// grammar to read it with.
///
/// Nothing here is a promise that the call failed. An exporter that
/// records every submit gives a reader one place to look for what was
/// sent, instead of one place for accepted jobs and another for refused
/// ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    /// Exporter slug whose grammar `payload` is written in — the same
    /// slug [`Handle::kind`](crate::Handle::kind) carries.
    pub kind: String,
    /// Exporter-specific record of the call.
    pub payload: Value,
}

impl AttemptRecord {
    /// Wraps an exporter-specific payload with the exporter's own slug.
    pub fn new(kind: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
        }
    }
}

/// Where an exporter puts an [`AttemptRecord`].
///
/// Takes `&self` rather than `&mut self` because the exporter holds it
/// through a shared borrow on a `Copy` context, and calls it from the
/// middle of an `async fn` — the implementation owns whatever
/// synchronisation it needs.
///
/// A record replaces any earlier one from the same dispatch. One row
/// carries the record of its latest attempt: a re-run after a refusal is
/// a fresh dispatch row of its own
/// (`DispatchService::redispatch`), so accumulating here would collect
/// retries of a single tick rather than the history a reader is after.
pub trait AttemptRecorder: Send + Sync {
    /// Records this call, replacing whatever the exporter recorded
    /// earlier in the same dispatch.
    fn record(&self, record: AttemptRecord);
}

/// An [`AttemptRecorder`] that keeps nothing.
///
/// What a call site with no row to write onto passes: exporter tests, and
/// any caller driving an exporter outside the dispatch state machine. The
/// exporter still records; the record goes nowhere.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscardAttempts;

impl AttemptRecorder for DiscardAttempts {
    fn record(&self, _record: AttemptRecord) {}
}

/// Shared [`DiscardAttempts`], so a context built without a row to write
/// onto can borrow one instead of keeping a local alive.
pub static DISCARD_ATTEMPTS: DiscardAttempts = DiscardAttempts;
