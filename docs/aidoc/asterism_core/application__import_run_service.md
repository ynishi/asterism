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
without a clock. The timer is also where the wait a rate limit
states — carried here as `ImportRun::retry_after_secs` — finally
gets a consumer; this slice records it and reads it nowhere.

## Starting is not waiting

[`ImportRunService::run`] answers as soon as the importer is on its
way, with the run it just opened, and the outcome arrives on that
row. The first shape held the answer until the child exited, which
read well — the answer *is* the outcome — and was wrong twice over:
an import of any size outlives an HTTP client's patience, and the
caller this exists for is a scheduler, which wants to start things
and not to sit with them.

## The lock is this process's, not the table's

Two runs of one definition would each take up from a position the
other is about to move, so one has to be refused. The question is
"is a child of **mine** still going", and that is about one process:
it is true only while this process lives, and a process that died
took its children with it.

The first shape asked the table instead — a run row said `running`
and a second run read it — which made the row a lock as well as a
record. A crash then left a row nothing would ever close and a
definition nothing could ever start, recoverable only by editing
SQLite. It also raced: two callers could both read "nothing running"
before either wrote.

So the answer lives in a set held here, and
[`ImportDefinitionRepository::abandon_running`] closes at startup
whatever a previous process left open.

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

