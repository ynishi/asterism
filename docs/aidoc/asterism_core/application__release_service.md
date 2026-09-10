# asterism-core::application::release_service

`ReleaseService` — writing out what a change point carries.

One verb: [`release`](ReleaseService::release) freezes the change
point's folded state, starts a `file` dispatch in `copy` mode over
it, and records that it happened.

Stamping the copies is the other half and is not here. Only the
runner drives it, so it sits in
[`application_support::outbound_stamp`](crate::application_support::outbound_stamp)
where no transport can reach it, under the placement rule
[`application`](crate::application) states.

# The freeze is driven from here, not from inside the forge

[`boundary`](crate::domain::forge::boundary) says which questions the
forge asks downward, and [`Release`] says why the record that names a
snapshot and a dispatch is not one of them. The consequence for this
service is the shape of [`release`](ReleaseService::release): it
reads the line, folds the chain and calls the services that freeze
and dispatch, the way `asterism-teams-client::publish` hands a line's
state to a receiver the forge does not control.

Nothing here writes a forge word onto a core row, and nothing writes
a core id onto a forge one.

# A release with nothing live is refused

A change point whose folded state has no live entry has nothing to
write out. Recording one would produce a release naming a snapshot
that cannot exist — [`Snapshot::new`](crate::domain::snapshot::Snapshot::new)
rejects an empty membership — so the choice is between refusing and
inventing a record of an export that never happened. It is refused,
as [`Blocked`](crate::error::ConflictKind::Blocked): put something on
the line and the same request works.

## Types

- `ReleaseService` — Recording a release.

