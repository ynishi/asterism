# teams-infra::backup

`backup` — the all-in-one instance backup (#83 §4, the #95 slice):
quiesce → `VACUUM INTO` snapshot → DB-first archive.

## The three steps, and why in this order

1. **Quiesce + snapshot.** The whole `VACUUM INTO` runs inside one
   `AsyncIsle` call — the isle *is* the single writer of the
   deployment shape (#83 §4), so holding its one connection for the
   duration is the app-level consistency point: no repository write
   can interleave with the snapshot. The snapshot is never a copy
   of the live file — copying a live SQLite file is a documented
   corruption path (WAL content is not in the main file), while
   `VACUUM INTO` writes a complete, transactionally consistent
   database.
2. **DB first, blobs after.** The archive receives the snapshot
   before a single blob is read. The asymmetry is #83 §3's, applied
   to backup (§4): every link in the snapshot points at bytes that
   were durable *before* that link committed and are read *after*
   the snapshot — so the worst inconsistency a backup can hold is
   an orphan blob (uploaded after the snapshot; harmless, the
   restored instance's sweep collects it), never a dangling DB
   reference. The one caveat is a **reclaim + sweep landing between
   the two steps** — it can remove bytes the snapshot still links —
   which is why backup wants a quiet instance (the CLI holds the
   writer for step 1, and the single-process shape means a stopped
   or idle server for the rest).
3. **One archive.** A plain uncompressed tar (the workspace
   manifest's `tar` line says why no compression): `db/teams.db`
   first, then `blobs/sha256/<shard>/<hex>` for every blob. Tar
   preserves entry order, so "DB first" is not only what the code
   does — it is readable in the artefact, and the tests assert it
   there. `staging/` is never archived: its contents are garbage by
   definition (#93's startup sweep deletes them).

## Where the snapshot lands

In a fresh local temp directory, never at the destination: the
destination may be a mounted/rclone network target, and a live
SQLite file must never sit on network storage (#83 §4 hard rule —
network storage is a backup *destination* only). Only the finished
archive is written to the destination path.

## Restore

Documentation, not a command (#95): unpack the archive, place
`db/teams.db` where the server's `--db` points and `blobs/` where
`--blobs` points, start the server. The restore e2e in
`teams-server` proves the unpacked pair serves an existing link
end-to-end. The full text ships on `teams-server backup --help`.

## Functions

- `create_backup` — Runs the whole backup: quiesce + `VACUUM INTO` through `isle`, then

## Types

- `BackupReport` — What a completed backup wrote — enough for the CLI to report and

## Constants

- `ARCHIVE_BLOBS_PREFIX` — The blob tree's prefix inside the archive; entries continue with
- `ARCHIVE_DB_ENTRY` — The snapshot's entry name inside the archive — first entry, always.

