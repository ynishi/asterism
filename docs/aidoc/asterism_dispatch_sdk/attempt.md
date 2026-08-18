# asterism-dispatch-sdk::attempt

`AttemptRecord` — what an exporter has to say about a call it made,
whether or not the call produced a [`Handle`](crate::Handle).

A [`Handle`](crate::Handle) is the record of a job the backend
accepted, and it is the only thing [`Exporter::dispatch`] can hand
back. That leaves the refused submit — the case a reader has the most
questions about — with nowhere to write: which endpoint, with which
body, and what the backend actually said all go out with the error,
and what survives on the row is one message string.

This is the second channel. The core hands the exporter an
[`AttemptRecorder`] on [`DispatchContext.attempt`], the exporter
records what it sent and what came back as it makes the call, and the
core persists whatever landed there — on the arm that returned a
handle and on the arm that returned an error alike.

Kept separate from the return type on purpose. Widening
[`ExporterError`](crate::ExporterError) would make every error site
decide what to attach, and returning a `Handle` for a job that does
not exist would put a reference to nothing where the poll loop
expects a backend job. Both change what the trait's own words mean;
a recorder the exporter writes to leaves `dispatch` saying exactly
what it said before.

[`Exporter::dispatch`]: crate::Exporter::dispatch
[`DispatchContext.attempt`]: crate::DispatchContext::attempt

## Types

- `AttemptRecord` — One call as the exporter wants it remembered.
- `DiscardAttempts` — An [`AttemptRecorder`] that keeps nothing.

## Traits

- `AttemptRecorder` — Where an exporter puts an [`AttemptRecord`].

## Constants

- `DISCARD_ATTEMPTS` — Shared [`DiscardAttempts`], so a context built without a row to write

