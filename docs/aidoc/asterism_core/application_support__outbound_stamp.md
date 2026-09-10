# asterism-core::application_support::outbound_stamp

Stamping what a dispatch wrote, before the run reports done.

[`OutboundStamping`] is what the runner calls once the files exist
and before [`reify`](super::DispatchRunnerService::reify) parks the
row in `Done`. [`ReleaseStamping`] is the one implementation: it asks
whether the run was a release and, when it was, writes the disclosure
and the history that chose the set into every copy.

# Why this is here and not beside `ReleaseService`

Only the runner drives it. [`application`](crate::application) says a
verb that grows a worker-only counterpart moves across rather than
sitting next to its transport-fronted sibling, and this is that
counterpart: no route and no command reaches it, and the environment
the runner is handed is the only place in the process holding one.
Recording a release is the other half and stays where a caller can
ask for it ([`ReleaseService`](crate::application::ReleaseService)).

# What a release records when a stamp does not land

A row per copy, always. The alternative was to stop at the first
failure, and it made "this release has no file rows" mean a third
thing — beside "the run has not finished" and "this build does not
stamp" — that nothing could tell from the other two. So a file whose
stamp could not be attempted is recorded as a failure with the reason
in it, the pass continues, and the first error is returned afterwards
so the runner still says so out loud.

That is the same reading [`Stamped`] already asks for on its two
halves: an outcome that says nothing happened, and does not say why,
is the value the type was reshaped to remove.

## Types

- `OutboundFile` — One file a run wrote, and the library row it is a copy of.
- `ReleaseStamping` — Stamps a release's copies, and answers for nothing else.

## Traits

- `OutboundStamping` — What a dispatch calls when it has written its files and has not yet

