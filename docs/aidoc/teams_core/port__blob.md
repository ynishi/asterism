# teams-core::port::blob

`port::blob` — backing storage for the instance's global CAS.

Hand-rolled trait, house style (#83 §3): `object_store` / OpenDAL
stay adapter details if S3 arrives — they are not the port. The
five verbs are the day-1 set because GC, orphan audit and backup
need `delete` + `list` from the start, not only `put` / `get`.

The v0 adapter (`teams-infra`) is a local-filesystem layout with a
staging dir and a stream-hash → verify → fsync → rename write path;
none of that shows here, deliberately.

## Traits

- `BlobStore` — Content-addressed blob storage — one physical copy per instance,

