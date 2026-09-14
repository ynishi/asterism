# asterism-infra::import_launcher

Starting `asterism-import`, and the two things that is really about.

The [`ImportLauncher`] port's implementation, and the only place an
importer is spawned. Everything above it is arranged so that this
file is the only one on the import path that holds a credential or
knows a process exists — the job handlers spawn `ffmpeg` on their
own account, which is a different path with the same rule.

# Where the binary is

Three rungs: an explicit `$ASTERISM_IMPORT`, then a sibling of the
running executable, then `PATH`. The first three of the ffmpeg
sidecar's four — it also probes fixed install prefixes, which is
right for a thing people install with a package manager and wrong
for one shipped beside the server. They differ at the first rung
too: ffmpeg's override is terminal, and this one falls through, so
an override pointing at nothing is still a run that finds the
binary.

The middle rung is the one worth naming, because it is the rung a
development build lands on: `cargo build` puts `asterism-ui` and
`asterism-import` in one target directory, and neither is on
anybody's `PATH`. Nothing bundles the importer today —
`tauri.bundle.conf.json`'s `externalBin` lists the ffmpeg sidecar
and nothing else — so a shipped app reaches an importer by the
first rung or the third. A failure names every rung it tried,
because "not found" without the list is a message an operator
cannot act on.

# Where the credential is, and where it is not

The definition names an environment variable; this reads it and puts
the value into the **child's environment**, under a fixed name the
importer knows. It is therefore in: this process's environment, the
child's environment. It is not in: the definition, the run record,
the child's argument vector, or any message this file produces.

That last exclusion is why this diverges from the outbound side's
`{{secret}}` template, which renders a credential into the thing it
is building. There, the thing is an HTTP header inside one process.
Here it would be an argument vector, and an argument vector is
readable by every other process on the machine — `ps` is not a
privilege. So the value goes through the environment instead, and
what `ps` shows is the *name* of a header.

Loading a `.env` is the binary's job, done once at startup, for the
reason the outbound side gives: an adapter that went looking for
dotenv files itself would make "which file did this credential come
from" invisible to the definition that named it.

# Where the records go, and how the child is told

`--server`, resolved here from the active profile. It is the same
question the serving process asks — `asterism-ui` binds
`active_profile().default_http_port()` — so two processes reading
one profile meet on one port without either being handed the
other's address.

It is passed only when the definition did not say. A server
started on `--port` is the case a profile cannot answer, and a
definition's own `--server` is what answers it — the same argument
a person running the importer by hand types.

Omitted rather than overridden, because the importer refuses a
repeated `--server` outright ("cannot be used multiple times")
rather than taking the last one. That is clap's answer and not a
choice made here, so passing both would turn every definition that
names its own server into a run that exits 2 before it starts. The
credential's flag goes the other way — appended after the
definition's arguments, where a second occurrence is *also* refused,
which is exactly the point: a definition cannot quietly substitute
its own.

Two earlier shapes failed here, in opposite directions, and both are
worth keeping because either is easy to rebuild.

The first had this file *given* an address, which meant something
had to hand it one after binding a listener — an
`Arc<OnceLock<String>>` filled by whoever served. Nothing filled it.
Every shipped binary called `axum::serve` directly, the cell stayed
empty, and every run in the product would have answered "this server
has not finished starting" forever; the end-to-end test passed
because the *test* filled it.

The second passed nothing and claimed the child worked the address
out for itself from `$ASTERISM_PROFILE`. It does not.
`asterism-import` carries a literal `http://127.0.0.1:8989` default
and does not depend on this crate, so under any profile but dogfood
the child posted at a door with nothing behind it — silently, since
a refused push is a failed run and not a wrong one.

Both are one mistake: an address known in one process and needed in
another, carried by a step somebody has to remember. Asking the
profile at both ends carries nothing.

## Types

- `SubprocessImportLauncher` — Spawns `asterism-import` and waits for it.

## Constants

- `CHILD_SECRET_VAR` — The variable the child reads its credential out of.

