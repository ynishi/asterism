# asterism-core::application

Application layer — use-case services.

- [`persona_service`] — persona lifecycle (register / list / archive /
  delete).
- [`asset_service`]   — asset lifecycle, grid reads, detail views, and
  pipeline job enqueue.
- [`app_setting_service`] — application settings, resolved as
  default → environment variable → stored row (the user's choice
  wins).
- [`mapping`]         — domain ↔ contract DTO conversion (kept in one
  place so the wire boundary stays consistent).
- [`fold_redirect`]   — the one place a surface holding its own id
  set redirects it through the folds that happened since (the
  "paths that name a row keep it" half of the fold read rule).
- [`forge`]           — the verbs of a line of work: the pursuit
  lifecycle and the rounds filed under it. Grouped because they are
  the services whose writes carry intent rather than content
  (doctrine 6).
- `attribution_intake` — the single check on the attribution fields a
  command still carries (a remote caller's assertion arriving through
  the owner's own surface).

Every mutation here takes an
[`AttributionContext`](crate::domain::attribution::AttributionContext)
as a required argument, next to (never inside) the command. The
adapter that received the request chooses it, and it is the only
source a write records from: services do not read the command's own
`author_kind` / `author_subject` / `operator_ai` fields.

Most services here take that argument without persisting it: the
attribution columns exist on `asset`, `dispatch_job`, and the
pursuit family (V79 — forge events are actor-carrying by design,
#29) alone, and adding another table is a design decision, not a
wiring step. Receiving
it is still the point — the argument is what makes a new mutation,
or a new caller of an old one, name the channel it arrived through
before it compiles, so recording operations later is a wiring change
rather than another audit of every write path.

Services take and return contract DTOs; domain types stay confined to
this crate. Tauri command handlers and MCP tool handlers therefore
become thin (DTO in, DTO out, error conversion).

The job engine (apalis) lives in `asterism-infra`; this layer only
enqueues jobs through the `JobQueue` port.

**Everything in here is fronted by a transport adapter** — a Tauri
command, an HTTP route, or both. A use case that only the job
worker, the dispatch runner, or startup drives belongs in the
sibling [`application_support`](crate::application_support) module
instead, where the transport contexts cannot reach it. When a verb
here grows a worker-only counterpart (a sweep, a bulk pass, a
state-machine transition), that counterpart moves across rather
than sitting next to it behind a comment.

