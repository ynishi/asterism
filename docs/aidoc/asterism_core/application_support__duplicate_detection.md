# asterism-core::application_support::duplicate_detection

Duplicate detection — what happens the moment a fingerprint lands on
bytes somebody else already holds.

The `material_hash` job writes a digest; this decides what that
digest *means* for the corpus. Three outcomes, chosen by the
[`OnDuplicate`] strategy the registering caller declared: park the
question for a person, fold without asking, or record the coincidence
and move on. All three record the match itself as an
[`identical_to`](EdgeKind::IdenticalTo) edge — a pair ruled apart is
still a pair that hashed the same, and that fact is what stops the
same conflict from being rediscovered as news.

# A function, not a service

Everything else in this module's neighbours is a struct wired at the
composition root, because it holds policy (a retention period) or
collaborators a handler cannot reach. This holds neither: the job
handler already carries every port named here, and the only decision
that is not derived from the row is the fallback in
[`resolve_strategy`]. A struct would add a `JobDeps` field and a
constructor call to hand a handler things it is already holding.

# Detection never fails the hash

Every error out of here is the caller's to log and drop. The digest
is a fact about bytes and it has already been written; a conflict is
a derivation from that fact, and a derivation that fell over must not
take the observation with it. The concrete cost of getting this wrong
is permanent: the backfill walk finds work by asking whether the
fingerprint columns hold an answer
([`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint)),
so a hash rolled back for a failed lookup would be re-read on every
future pass, while a conflict that was not raised is re-raised the
next time the pair is fingerprinted.

## Functions

- `detect_duplicate` — Runs every axis over a freshly written fingerprint, **strongest
- `detect_duplicate_on_axis` — Looks for other holders of a freshly written digest **on one axis**
- `fold_excluded_by` — Which rule, if any, stands between this pair and an automatic fold.
- `resolve_strategy` — Turns an undeclared strategy into the one that will be applied —

## Types

- `Detection` — What one detection did.
- `DetectionOrigin` — Which pass fingerprinted the material.
- `DetectionPorts` — The three ports one detection needs, as one handle.

## Constants

- `LINEAGE_PROBE_BUDGET` — Node budget for one side of the lineage probe — and, reused as the
- `UNDECLARED_STRATEGY` — The strategy an asset that declared none is handled under.

