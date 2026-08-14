# asterism-benchgen 0.0.0

`asterism-benchgen` — the seeded corpus behind the grid/import benches.

Bench data cannot be a folder of whatever images happened to be on the
machine: the numbers stop being comparable between runs, and a corpus
that came out of one generator run carries that run's accidents (one
persona, one aspect ratio, no group of interesting size). So the corpus
is defined by a seed and regenerated from it — [`model::SpecStream`] is
the definition, everything else here is materialisation.

Two tiers, because one corpus cannot serve both benches: 110,000 assets
at ~1.3 MB each is 165 GB.

- **T-file** (`s` = 5,000 / `m` = 12,000): real PNGs on disk, so the
  import path (hash + `thumb_gen` jobs) does real decode work.
- **T-meta** (`l` = 110,000): specs only. Rows are seeded straight into
  the repository by `seed-meta`; `corpus --preset l` writes just
  the manifest, so both tiers agree on what corpus `(seed, l)` means.

Presets are prefixes of one stream: S ⊂ M ⊂ L for a given seed.

Five subcommands — three that build a corpus, two that measure one:

- `corpus` — materialise the corpus directory (PNGs + manifest).
- `seed-meta` — T-meta: rows straight into the bench profile's
  database, thumbnails included ([`seed_meta`]).
- `load-file` — T-file: the corpus pushed through the running bench
  server's HTTP API so the import jobs do real work ([`load_file`]).
- `measure-import` — `load-file` plus the wait for the jobs it
  enqueued, written up as a result file ([`measure`]).
- `measure-cold` — first-listing cost against a just-restarted
  server, warm repeat alongside it ([`measure`]).

Every write path is fenced to the bench profile: `seed-meta` refuses
a database outside `profiles/bench` and everything that speaks HTTP
refuses the Dogfood port. Neither fence is optional in the direction
that matters — there is no flag that points any command at the real
library.

