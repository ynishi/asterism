# asterism-core::application::import_run_service

Defining an import, and running one without anybody typing it.

The half of the transport decision #295 left open. Adapters run
themselves and push over HTTP, which settles how an import *talks*
to Asterism and says nothing about what starts it. This starts it.

## What is here and what is deliberately not

Running one on demand. **No schedule**: a timer belongs on top of
this and calls it, and every hard question — where the binary is,
how a credential reaches it, what happens when a run is already
going, what is left behind when nothing worked — is answerable
without a clock. The timer is also where
[`Disposition::FailedRetryable`](crate::domain::import_definition)'s
stated wait finally gets a consumer; this slice only records it.

## Spawning is not this layer's

[`ImportLauncher`] is the port, and the reason it exists rather than
a `Command` a few lines down is the reason every port here exists:
this crate does not touch the machine. It also keeps the credential
out of this layer entirely — the launcher is handed a variable
*name* and resolves it while building the child's environment, so
the value never enters a type anything here holds, logs, or could
accidentally persist.

## Types

- `ImportRunService` — Stored imports, and running them.
- `LaunchOutcome` — What starting one produced.
- `LaunchSpec` — What the launcher is asked to start.

## Traits

- `ImportLauncher` — Starts an importer and waits for it.

