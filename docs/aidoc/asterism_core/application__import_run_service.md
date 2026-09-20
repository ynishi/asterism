# asterism-core::application::import_run_service

Defining an import, and running one without anybody typing it.

The half of the transport decision #295 left open. Adapters run
themselves and push over HTTP, which settles how an import *talks*
to Asterism and says nothing about what starts it. This starts it.

## What is here and what is deliberately not

Running one on demand, and [`ImportRunService::start_due`] for
whatever is asking on a clock. **The clock is not here** — it is
[`import_scheduler`](super::import_scheduler) — because every hard
question, where the binary is, how a credential reaches it, what
happens when a run is already going, what is left behind when
nothing worked, is answerable without one, and was answered before
there was one (#299, then #302).

The wait a rate limit states is recorded here, as
`ImportRun::retry_after_secs`, and what acts on it is the due
calculation — not the clock, which asks what is due and is told. So
this layer writes that field and hands it on, and never decides
anything by it, which is the same division the rest of the run
record has.

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

**A process-local answer is only sound while one core is open over
a given database, and that is now enforced rather than assumed.**
The Tantivy index is opened for writing unconditionally (#300) and
that lock is exclusive, so a second core over the same index does
not start. The index and the database are resolved together from the
active profile, so for the shipped app this is one core per machine;
a test handing its own tempdir to `init_core_with` gets a pair of
its own and a set of its own, which is the same rule applied and not
an exception to it.

A review round was spent asking which process should host this
supervisor; the answer is that there is one, and nothing here is
conditioned on a mode. That question existed because `CoreMode` did.

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

