# asterism-core::application_support::outbound_stamp

Stamping what a dispatch wrote, before the run reports done.

[`OutboundStamping`] is what the runner calls once the files exist
and before [`reify`](super::DispatchRunnerService::reify) parks the
row in `Done`. An implementation answers whether the run was a
release and, when it was, writes the disclosure and the history that
chose the set into every copy; [`ReleaseStamping`] is that answer.

# Why this is here and not beside `ReleaseService`

Only the runner drives it. [`application`](crate::application) says a
verb that grows a worker-only counterpart moves across rather than
sitting next to its transport-fronted sibling, and this is that
counterpart. Recording a release is the other half and stays where a
caller can ask for it
([`ReleaseService`](crate::application::ReleaseService)).

Two things in the process hold one and they are the same object: the
runner's `DispatchRunEnv`, which calls it, and
[`SupportServices`](super::SupportServices), which is how a test
reaches the one the composition root built. Neither `ServerCtx` nor
`AppState` carries either — the support bundle is the field the
transport wrappers skip, so no route and no command has an object to
call this on.

# What a release records when a stamp does not land

A row per copy, once a pass runs at all. The alternative was to stop
at the first failure, and it made "this release has no file rows"
mean a third thing — beside "the run has not finished" and "this
build does not stamp" — that nothing could tell from the other two.
So a file whose stamp could not be attempted is recorded as a failure
with the reason in it, the pass continues, and the first error is
returned afterwards so the runner still says so out loud.

The two states that write nothing are the two that leave: a dispatch
that is not a release, and a build with no disclosure service to
stamp with. Neither reaches a file.

That is the same reading [`Stamped`] already asks for on its two
halves: an outcome that says nothing happened, and does not say why,
is the value the type was reshaped to remove.

## Types

- `OutboundFile` — One file a run wrote, and the library row it is a copy of.
- `ReleaseStamping` — Stamps a release's copies, and answers for nothing else.

## Traits

- `OutboundStamping` — What a dispatch calls when it has written its files and has not yet

