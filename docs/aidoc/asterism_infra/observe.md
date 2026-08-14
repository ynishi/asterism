# asterism-infra::observe

Observation — the `tracing` subscriber and the streams it writes.

One channel for everything the application observes about itself
through `tracing`. Call sites anywhere in the workspace write
`tracing::warn!` / `info!` / `error!` and never think about
transport; this module decides where those records go.

## Which stream a record belongs to

Two of the four observation streams are written here: `DiagLog`
(what the application decided or failed at) and `PerfLog` (how long
something took). A call site selects `PerfLog` by naming itself —
`event = "perf.list_index"` — and anything unnamed is a diagnostic.
`ActionLog` and `JobLog` are written by their own subsystems; see
[`asterism_core::domain::observation`] for the four-way split and
why it exists.

## Why a subscriber rather than a handle

`asterism-core` deliberately does not depend on `asterism-infra`, so
a service in the application layer cannot reach a database-backed
sink through a field. Threading a port through `AssetService` /
`PersonaService` / the job handlers for the sake of a swallowed
warning would cost a constructor change per seam. `tracing` installs
one process-global subscriber instead, so a call site needs no
handle and the layering stays intact.

## What is written is written verbatim

Field values reach `attrs` as they were given. Ids, counts,
durations and error strings are fine; a token, a credential or a
user's content is not. Check that before adding a field. How long a
row then survives is the stream's retention policy, not this
module's business.

## Two layers, and where the real decision is made

- a `fmt` layer to stderr, filtered by `RUST_LOG` (default
  `asterism=info`) — the developer-facing view;
- [`ObservationLayer`], which persists to a stream table — the
  durable view, readable long after the terminal is gone.

The filters are deliberately separate. `RUST_LOG` is the dial a
developer reaches for, and it must not change what the application
writes to its own database: the sink keeps a fixed `asterism=info`.

That fixed filter only selects *candidates*. What is actually kept
is
[`StreamPolicy::should_persist`](asterism_core::domain::observation::StreamPolicy::should_persist)
— a filter sees a level and a target, and decisions like "perf
timings, in development only" need the stream.

## Ordering, and what the startup queue does and does not buy

The subscriber must exist before anything can log, but the database
opens later. [`install`] therefore runs first and records accumulate
in a bounded queue until [`DiagSink::attach`] supplies the isle,
after which they flush and later records go straight to the writer.

What that covers: records emitted before the database opened **on a
run that goes on to open it** — argument parsing, migration
warnings, an unusable environment override.

What it does not: a failure *of* the open itself. `attach` is only
reached once the database is up, so a run that dies opening it
flushes nothing. Those records reach stderr and stop there, which is
why the fatal path also prints verbatim rather than relying on this.

## Never in the way

Emitting is a lock plus a channel send — no spawn, no await, no
blocking — so a record can be produced from any thread, including
the one inside a SQLite call. One writer task drains the channel.
When it falls behind, records are dropped and counted rather than
queued without end: the application shares a single SQLite
connection, and diagnostics must never put a user's query behind a
burst of warnings.

## Functions

- `attach` — Points the installed sink at the open database and flushes whatever
- `install` — Installs the process-global subscriber. The sink it writes through

## Types

- `DiagSink` — Handle used to attach the database once it is open.
- `ObservationLayer` — `tracing` layer that turns each event into an [`ObservationRecord`].
- `ObservationRecord` — One row destined for a stream table.
- `ObservationStore` — Handle over the observation streams: reads, and expiry.
- `RetentionSweep` — What one retention pass removed, per stream.

