# teams-infra::gc

`gc` — the zero-link sweep (#83 §3 registry-GC shape, the #95
slice): blob bytes that no team links anymore are deleted, after a
reclaim and on demand (`teams-server gc`).

## What the sweep protects

A blob's bytes survive while **any** link row references its digest
— marked-for-purge links included, because a marked link is
restorable during its grace window and restoring a link whose bytes
were swept would be a dangling reference by another name. The
sweep's question is therefore
[`SqliteTeamsRepository::digest_linked_anywhere`], deliberately not
the read surface's visibility predicate.

## The racing same-digest upload, and why the answer is a lock

The #93 adapter's write path makes bytes durable (staging → rename)
**before** the link row commits (#83 §3 ordering). That order is
what makes a dangling link impossible for uploads — and it is
exactly what a concurrent sweep could break: between the upload's
rename and its link commit, the digest has bytes and zero links,
and a sweep deciding in that window would delete bytes whose link
is about to commit — a dangling link, manufactured by the sweeper.

Re-checking links after removing the file cannot close this: the
hazardous interleaving (upload renames → sweep checks links, sees
zero → sweep deletes → upload's link commits) has the re-check land
*before* the link exists, however many times it re-checks. What
closes it is excluding the interleaving: [`GcGuard`] is a
`tokio::sync::RwLock` — every upload holds it **shared** across its
rename→link-commit span (uploads never block each other), and the
sweep holds it **exclusive** across its check-and-delete, so no
upload is ever mid-span while the sweep decides. This leans on the
single-process deployment shape #93 fixed (one server process owns
DB and blob dir); a second process bypasses the guard, which is why
the `gc` CLI documents "stopped server or same process" and why
cross-process coordination stays out of scope (#95 out-of-scope
list).

## Functions

- `sweep_zero_link_blobs` — Deletes every CAS blob that no team links (marked links count as

## Types

- `GcGuard` — The lock that keeps the zero-link sweep and the upload write path

