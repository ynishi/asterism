# teams-infra::blob

`blob` — [`LocalFileStorageAdapter`], the v0 backing of the
instance's global CAS (#83 §3, the #93 slice).

## Layout

```text
<root>/sha256/<2ch>/<64hex>   one file per digest — the CAS proper
<root>/staging/               in-flight writes, unique names
```

The path under `sha256/` is the digest's canonical form with the
`sha256:` prefix stripped — this module is the path-mapping edge #83
§3 names as the only place the prefix comes off. Everywhere else
(the port, the link table, the wire) the digest keeps its prefix.

## Write path

Stream into a uniquely named staging file while hashing → verify
the computed digest against the **declared** one (the domain's
[`verify_declared_digest`], so the mismatch arm is the same
rejection everywhere) → `fsync` the file → rename into the final
path → `fsync` the parent directories. This hardens the `.part`
precedent from `asterism-infra`'s preview jobs: same
temp-then-rename shape, plus the fsyncs and the digest gate, because
here the rename is what makes bytes *exist* for the link layer and
a half-written blob must never be reachable under its digest.

A mismatch deletes the staging file and reports the computed digest
(carried by [`DomainError::DigestMismatch`]); nothing lands. An
abandoned write (crash, dropped connection) leaves only a staging
temp, which the startup sweep removes — [`open`] runs it, and it is
the only mechanical cleanup this layer owes (#83 §3 lifecycle).

## Concurrent same-digest writes

Each writer streams into its own staging file and finishes with an
atomic `rename` onto the same final path. `rename` replaces: the
last writer's inode wins, every earlier writer's file is dropped by
the filesystem, and since every renamed file has already been
verified to hash to the digest it is named by, the replacement swaps
bytes for identical bytes — a value-level no-op. Both callers
succeed and one physical copy remains. There is deliberately **no
exists-check shortcut** before or during the write: skipping the
work when the blob is already present would make a duplicate upload
observably cheaper, which is the Harnik-2010 side channel the
upload contract closes by always accepting the full body.

[`open`]: LocalFileStorageAdapter::open
[`verify_declared_digest`]: teams_core::domain::store::verify_declared_digest

## Types

- `LocalFileStorageAdapter` — Local-filesystem CAS adapter — one physical copy per instance,
- `StagingWrite` — One in-flight streaming write: the staging file, the running hash,

