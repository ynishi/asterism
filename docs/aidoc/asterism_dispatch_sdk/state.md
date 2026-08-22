# asterism-dispatch-sdk::state

Lifecycle state of a dispatch as seen by the exporter's
[`crate::Exporter::poll`] response.

The core drives the state machine — the exporter only reports "what
I see on the backend right now". Terminal states (`Done` / `Failed`
/ `Cancelled`) stop the poll loop; non-terminal states keep it
running with a re-enqueue delay derived from the progress hint (if
any).

## Types

- `DispatchState` — State of one dispatched backend job.
- `ProgressHint` — Optional soft progress signal returned alongside a

