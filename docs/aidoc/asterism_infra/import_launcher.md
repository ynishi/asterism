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

The middle rung is the one worth naming — the comment beside
`thumb_ffmpeg`'s ladder records what skipping it cost there, "it
reported `ffmpeg is required` on exactly the machines the sidecar
exists for", and a bundled importer beside a bundled server is the
identical shape. A failure names every rung it tried, because "not
found" without the list is a message an operator cannot act on.

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

# Where the records go, and why nothing here says

Nothing passes `--server`. The child resolves it the way the
operator's shell does — the active profile's port — and it inherits
`$ASTERISM_PROFILE` from this process, so it resolves the same
profile this one is serving. A server on a port of its own puts
`--server` in the definition's arguments, which is what a person
running the importer by hand does already.

The first shape told the child instead, which meant this file had to
be given an address, which meant something had to hand it one after
binding a listener — an `Arc<OnceLock<String>>` filled by whoever
served. Nothing filled it. Every shipped binary called `axum::serve`
directly, the cell stayed empty, and every run in the product would
have answered "this server has not finished starting" forever. The
end-to-end test passed because the *test* filled it. A wiring step
that can be forgotten is one that will be, and the way to not forget
it turned out to be not having one.

## Types

- `SubprocessImportLauncher` — Spawns `asterism-import` and waits for it.

## Constants

- `CHILD_SECRET_VAR` — The variable the child reads its credential out of.

