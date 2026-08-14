# asterism-core::domain::job

`Job` — lifecycle model for asynchronous work.

The actual engine lives in `asterism-infra` (apalis + apalis-sql). The
domain layer only owns the kind / state / progress vocabulary;
scheduling, retry policy, and worker orchestration belong to the engine.
Fire-and-forget `tokio::spawn` is intentionally avoided.

## Types

- `Job` — Domain model for a single background job.
- `JobKind` — Kinds of background job Asterism runs.
- `JobState` — Lifecycle state for a job.

