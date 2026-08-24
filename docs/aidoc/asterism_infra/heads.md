# asterism-infra::heads

Trained-head storage (#132 phase 2): the artifact a training run
writes, and the one pointer promotion moves.

Layout under [`crate::paths::heads_dir`]:

```text
heads/
  head-v1-9f80e2c1/head.json   one training run's artifact, eval included
  head-v2-1a2b3c4d/head.json
  current                      the promoted label, or absent — zero-shot
```

An artifact is immutable once written: a retrain is a new label,
never an overwrite, so `current` can move backwards as well as
forwards and every eval stays inspectable. The artifact stores
**only the trained rows** — tags below the training floor keep
their zero-shot behaviour implicitly — which is why a head is
kilobytes, not megabytes, at realistic ruling counts.

Promotion is the caller's verdict, not this module's: writing an
artifact records what training produced (wins and losses alike, a
loss being exactly the evidence that zero-shot should stand);
[`promote`] only moves the pointer. The training caller promotes
on a strict held-out win; a pull ([`install_pulled`]) promotes on
the person's explicit act — the two verdicts that may move the one
pointer.

## Functions

- `artifact_for` — Builds an artifact from a run's outputs, stamping the schema and
- `bind_current` — Resolves the promoted head against the bound encoder — the scoring
- `current_label` — The promoted label, if any. Absent (or dangling) means zero-shot.
- `discriminated_label` — Appends the content discriminator to an ordinal stem: eight hex
- `install_pulled` — Installs a pulled head artifact and promotes it — the member half
- `load_artifact` — Reads one label's artifact, verifying the schema tag and that the
- `next_head_label` — The next ordinal label stem under `heads_root`, counting up from
- `promote` — Points `current` at a label — the promotion. The pointer is the
- `verify_artifact` — The checks a head must pass before it may score against `identity`
- `write_artifact` — Writes one run's artifact under its label. Refuses an existing

## Types

- `TagHeadArtifact` — One training run, persisted whole: identity, rows, and the eval

## Constants

- `CURRENT_FILE` — File name of the promotion pointer inside the heads root.
- `HEAD_ARTIFACT_SCHEMA` — The artifact schema this module writes and reads.
- `HEAD_FILE` — File name of the artifact inside a head's directory.

