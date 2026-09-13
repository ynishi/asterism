# asterism-infra::import_launcher

Starting `asterism-import`, and the two things that is really about.

The [`ImportLauncher`] port's implementation: the one place in this
workspace that spawns an importer. Everything above it is arranged so
that this file is the only one holding a credential and the only one
that knows a process exists.

# Where the binary is

Three rungs, and the same three the ffmpeg sidecar uses, in the same
order: an explicit `$ASTERISM_IMPORT`, then a sibling of the running
executable, then `PATH`. The middle rung is the one worth naming —
the comment beside `thumb_ffmpeg`'s ladder records what skipping it
cost there, "it reported `ffmpeg is required` on exactly the machines
the sidecar exists for", and a bundled importer beside a bundled
server is the identical shape. A failure names every rung it tried,
because "not found" without the list is a message an operator cannot
act on.

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

## Types

- `SubprocessImportLauncher` — Spawns `asterism-import` and waits for it.

## Constants

- `CHILD_SECRET_VAR` — The variable the child reads its credential out of.

