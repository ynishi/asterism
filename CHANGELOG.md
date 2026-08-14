# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **The disclosure vocabulary moved into the core, and `provenance` stopped
  naming two things** (#14, #23) — `provenance` was already the
  derived-from claim graph, whose own documentation says this
  application's lineage is "deliberately **not** a reading of any external
  identity system — xmpMM, C2PA and the rest are channels a claim can
  *arrive* on, never the substrate it is stored in". The AI-disclosure
  feature then took the same word for the thing that stores C2PA.
  `application::provenance_service` is `disclosure_service`,
  `infra::provenance` is `infra::disclosure`, and the types follow
  (`DisclosureService`, the `DisclosureWriter` port, `DisclosureError`).

  The crate split with it. `asterism-provenance` held the vocabulary
  *and* the renderers, so `asterism-core` reached `pngmeta` and a CRC
  through it — the container parser its own manifest records evicting.
  The vocabulary (`DigitalSourceType`, `DisclosureRecord`, `Stamped`) is
  `asterism-core::domain::disclosure` now; the renderers are
  `asterism-disclosure-format`, depending on the core rather than the
  other way round. What forced it rather than leaving it as debt:
  reading a disclosure *back* has to be modelled in the core, because a
  port cannot return a type the core cannot name.

  The job kind goes with it: `disclosure_stamp`, with the handler, the
  dependency field and the operator-facing surface — the events are
  `diag.disclosure*` and the error text a person reads says disclosure,
  which is what the rename was for. A slug is a stored value and
  renaming one is normally a migration; this one has never been in a
  release, an unknown slug is skipped rather than fatal, and the cost of
  a row queued on a development machine before the rename is one
  artefact that stays unmarked until something re-fingerprints it.

  The signed assertion label and its payload tag are
  `io.github.ynishi.asterism.disclosure` and `asterism.disclosure/1`.
  Renaming an identifier inside a tamper-evident document is normally
  the one thing that cannot be undone — but nothing has ever been
  signed, because no build has a certificate to sign with, so there is
  no file to stay compatible with. The version stays `/1`: nothing has
  read shape 1 under the old name.

- **`just check` says when the committed doc artifacts went unchecked**
  (#25) — `aidoc-check` needs a nightly toolchain this workspace does not
  pin, so it sat outside the gate, and a change that deleted a crate left
  `docs/aidoc/` describing it while `just check` went green. The new
  `aidoc-guard` step runs the check when it can and fails on drift as
  before; when the toolchain or `cargo-aidoc` is missing it prints what is
  missing and continues. A gate nobody is told they skipped is not a gate.
  The prerequisites are in the README, and CI installs them — a step that
  warns on every run is a step nobody reads, so on the one machine that
  runs `check` for every change, drift is red rather than a log line.

- **The row records what became of an artefact's disclosure** (#14) —
  stamping wrote a mark into a file and said so in a log line, leaving
  the library unable to answer which artefacts carry one. A mark lives
  in the file's bytes and a downstream conversion strips it, so the row
  is the only place the answer survives, and it is what a re-apply would
  be decided from. The note lands under `extra._trace.disclosure`,
  beside the declared-hash verdict already there — which generalised the
  narrow write those notes need: `note_declared_hash` becomes
  `note_trace_field`, one transaction per key rather than a near-copy of
  the method per key.

  What the note holds is `Stamped`'s own account of itself, so that
  "no certificate was configured" and "the certificate stopped working"
  stay apart in the row as they do in the type. A failed note changes
  nothing — the mark is already in the file or already not.

- **What a dispatch produces is written with its AI disclosure** (#14) —
  the writer landed with nothing calling it; this calls it. Not where
  the work was planned to call it from: stamping immediately after
  `reify` reads an evidence set that does not exist yet, because `reify`
  builds the material from the exporter's string and enqueues the
  hashing that fills `meta_kv` in. That version compiled, passed the
  existing dispatch fixtures, and marked no file at all. So the order is
  a chain — `MaterialHash` enqueues a new `ProvenanceStamp` job once the
  fingerprint lands, which is the first moment there is anything to
  disclose.

  Its own job kind rather than a mode of the hashing one: hashing reads
  bytes and writes a column, stamping rewrites the user's file, and the
  two want different retry policies. A stamp that fails leaves an
  artefact that exists and is unmarked, so the handler returns `Ok` on
  every outcome and reports which halves landed rather than failing a
  completed export over metadata.

  **Only artefacts a dispatch produced are stamped.** Stamping rewrites
  bytes, and doing that to a file somebody imported would be this
  application editing something it was asked to index and not to touch.
  The dispatch trace separates the two, `_dispatch` becomes a named
  constant now that both sides depend on the spelling, and the check is
  a pure function pinned by tests over every shape an imported asset's
  `extra` can take.

  The composition root builds the service unsigned and with the prompt
  withheld — the two documented answers — and an unwired build skips
  rather than fails.

### Changed

- **A disclosure's two halves report their own outcome, so one failing no
  longer cancels the other** (#14) — applying a record writes an IPTC/XMP
  packet and signs a C2PA manifest, and the writer has argued from the
  start that the two fail independently. It did not behave that way.
  Every failure inside the signing block returned early while the packet
  was still in memory, so a signing error threw it away: on the day a
  certificate expires — which the module's own docs call the failure
  every signing deployment eventually meets — exports would have stopped
  carrying the IPTC half, the one that needs no certificate at all. The
  mirror case cost the manifest: a packet too large for a JPEG segment
  even after the reduction failed the whole call.

  The cause was the return type. `Result<Stamped, _>` made the error
  channel total while the operation is composite, and an `Err` has
  nowhere to carry the half that succeeded. `Stamped` now holds a `Half`
  per side — `Written`, `Skipped(reason)` or `Failed(cause)` — and `Err`
  is reserved for the case where nothing could be attempted: the file
  cannot be read, or its container is not one this build writes into.
  Whether a failed half makes a failed export is the caller's judgement,
  and it now has both facts to make it with.

  Three states rather than a boolean, for the reason the digest axes
  already have three: "no packet" was at least four different answers,
  and a video that cannot carry one, a build with no certificate
  configured, and a certificate that stopped working all reported the
  same `false`. `Skipped` names the ones that are not faults.

- **Disclosing the prompt is a decision somebody makes, not a constant**
  (#14) — `DisclosureRecord::with_prompt` says the prompt is "a decision
  the service makes, not a property of the data" and that it "cannot be
  taken back out of a file already published"; the service made no
  decision, filling the field whenever the evidence had one, with nowhere
  to state a different policy. What the field receives is the whole
  AUTOMATIC1111 `parameters` blob — prompt, negative prompt, sampler,
  seed, checkpoint name and hash, and the name and hash of every LoRA —
  so a locally trained model named after a person or a client went into
  every published copy. `record_for` now takes a `PromptDisclosure`
  (`Withhold` / `Embed`) and `DisclosureService` takes one at
  construction. No `Default` and no default chosen: it belongs to the
  composition root, and the asymmetry that should decide it is the one
  the module already applies to terms — withholding can be undone by
  re-applying, publishing cannot be undone at all.

- **A stamp is staged in a temporary nothing can predict, and keeps the
  file's own permissions** (#14) — the rewrite went through a
  deterministic sibling (`shot.png.c2pa-partial`), opened with neither
  `O_EXCL` nor `O_NOFOLLOW`, at whatever the umask gave. An export
  directory is wherever the user pointed the export — possibly shared,
  synced or watched — so anything else able to create a file there could
  place a symlink at that name and have the stamp write the asset
  through it; two concurrent applies to one path shared the temporary
  and interleaved; and the staged copy of the whole asset was
  world-readable for as long as signing took. `tempfile` becomes a real
  dependency and supplies all three of a random name, `O_EXCL` and mode
  0600, with the target's own permissions copied across before the
  rename so that stamping is not also a permission change. Still absent:
  an `fsync` before the rename.

- **Two non-characters XML cannot hold are dropped from the packet**
  (#14) — the filter took the C0 controls and stopped, but XML 1.0's
  `Char` production also excludes U+FFFE and U+FFFF, which cannot be
  written even as numeric references. They are reachable: a PNG text
  chunk is decoded leniently, so a valid encoding of U+FFFF passes
  through into the prompt and into the packet, and nothing noticed —
  the packet is read back as text rather than parsed, so the write
  reported an XMP half that had landed while the file carried an
  unreadable metadata block. The neighbouring non-characters are legal
  and stay.

- **The workspace says what its digests actually rest on, and the signed
  manifest stops claiming a version it does not have** (#14) — the
  `preserve_order` comment in the workspace `Cargo.toml` asserted two
  things that are not true. It said `c2pa` requires the feature: `c2pa`
  does declare it in its own manifest, which is why it is unconditionally
  on, but it does not depend on the semantic — verification re-hashes the
  bytes read out of the file rather than re-serialising a parsed model,
  and the default assertion kind routes through a CBOR map that sorts,
  discarding the author's key order before anything is encoded. And it
  said the digests were safe because they are built from a struct and
  never parsed, which is not a discharge at all: `serde_json::to_value`
  produces a `Map` too, so a value that never met a parser is still an
  `IndexMap` under the feature.

  The comment now names the four stored forms that are hashed or compared
  byte for byte — `material_meta::render`, `series::render`,
  `source_locator::to_storage` and `snapshot_hash` — with what makes each
  one independent of the line, since the reasons differ and only one of
  them is "it sorts". It also states why the line is there at all: what
  this workspace writes back out is somebody else's document, and handing
  it back with the keys re-sorted is an edit nobody asked for.

  `domain::content_hash` gains the rule a digest added beside them owes.
  A digest either **selects** bytes the artefact already carries or
  **re-renders** them, and it has to say which, because the two fail in
  opposite directions: re-rendering too widely reports two different
  artefacts as one and duplicate resolution folds them, while selecting
  too narrowly only misses a match. Re-rendering additionally owes its
  canonical form in full — naming a published scheme is not enough, as
  the rules for numbers and for duplicate keys are what decide the
  answers — and a versioned tag, because a shipped definition cannot be
  edited without changing what every value stored under it meant.

  Neither of the two disclosures claims a build version any more.
  `claim_generator_info` carries a name and nothing else — the
  specification requires only the name, every crate here inherits the
  same `0.0.0`, and a version string identical on every build ever made
  tells a reader nothing while sounding like it tells them something, in
  a document that cannot be corrected after signing. The XMP packet's
  `x:xmptk` drops it too, for that reason and one more: those bytes go
  inside the C2PA hard binding, and the toolkit string was the only
  thing in the packet not read off the record, so a version bump
  re-rendered an unchanged record into different bytes. The module doc
  had already promised the packet is a function of the record and
  nothing else; now it is.

  Both tests were weaker than they read. The manifest one compared the
  emitted field against the same `env!("CARGO_PKG_VERSION")` the code
  used, so it passed at any value; it now asserts the field is absent.
  The packet one compared two renderings inside one build, which cannot
  see a difference that moves both sides together; it now pins the
  toolkit attribute literally.

- **The series key no longer borrows its canonical form from a
  dependency** (#14) — `series::render` hashes a `serde_json::Value`
  parsed out of a container, and was taking its nested key order from
  whichever map type `serde_json`'s feature flags selected. A new
  `series::canonical_value` sorts every object's keys recursively before
  the bytes are rendered, so the digest is a function of the document
  rather than of how it was typed: a JSON object is an unordered
  collection (RFC 8259), and two containers carrying the same fields in a
  different order carry the same document. Arrays keep their order, since
  a JSON array *is* ordered. Byte output is unchanged and no stored key
  moves.

  `serde_json`'s `preserve_order` is now declared in the workspace
  `Cargo.toml` with its reasoning rather than arriving as a side effect of
  `c2pa`. The old test that asserted sorted output and warned in prose
  that this rested on a default has a sibling asserting the property
  itself, plus the negative case; both fail by name if the sort is
  removed, which is the point — the function reads like a no-op and will
  invite deletion.

### Added

- **A generated module inventory, and the end of the hand-written one**
  (#25) — `asterism_core::domain`'s module doc hand-enumerated its
  submodules and had gone stale (27 of 42 covered by the time it was
  caught). The list is replaced by a capability tour plus the doctrines
  code alone misreads (events-not-state, facts vs verdicts,
  freeze-then-refer, attribution's stopping point); the inventory itself
  is generated from each module's opening summary line — rustdoc's own
  index, a one-line grep recorded in the doc, and committed cargo-aidoc
  artifacts under `docs/aidoc/` with `just aidoc` / `just aidoc-check`
  (nightly-only, deliberately outside `check`) turning drift into a
  failing exit code.

- **AI disclosure: the vocabulary, the emitters and the signer**
  (#14) — what an exported file says about how it was made, as values:
  the IPTC digital source type
  (five terms, closed, refusing anything the vocabulary does not define),
  the XMP packet carrying `Iptc4xmpExt:DigitalSourceType` and the four AI
  properties IPTC added in Photo Metadata Standard 2025.1, that packet
  written into a PNG `iTXt` chunk or a JPEG `APP1` segment as a byte
  transform, and the C2PA manifest definition built from the same record
  so the two cannot disagree. `asterism-infra::disclosure` is the adapter
  that puts them into a file and signs the manifest through `c2pa`,
  covering MP4 and MOV as well as stills — signing after the encode,
  which is the only point at which it is possible.

  Two decisions are worth stating. **XMP is written before the manifest is
  signed**: the hard binding covers the packet, so the reverse order
  invalidates the signature, and a test signs a file, edits its packet and
  asserts the binding then fails. **A signing identity is configuration**:
  the IPTC/XMP disclosure is written with or without one, a manifest only
  with, and the C2PA test certificates are refused by name rather than
  used as a fallback — a manifest signed by them validates as untrusted,
  which claims a provenance a reader rejects.

  `domain::disclosure` is the judgement that feeds them, and it is pure:
  which IPTC term is true of an artefact, given the container metadata a
  probe stored and the `derived_from` edges the library recorded. Terms
  are asserted on evidence something wrote, and an artefact nothing
  established gets no term rather than one meaning "unknown".
  `compositeWithTrainedAlgorithmicMedia` turns on whether a recorded
  parent is itself synthetic, which the child's own metadata cannot say.
  `application::disclosure_service` does the reads and owns the port,
  looking at no file metadata at all — which is what lets a file that came
  back from a downstream conversion with its manifest stripped be handed
  to `apply_to` and get the same disclosure again.

  Not yet wired to the export path, and not exposed over HTTP or IPC;
  both are the rest of #14. Unsigned video carries no disclosure at all,
  because the XMP half has no BMFF spelling here, and the writer reports
  that rather than a success it did not have.

- **Material layers, and the chapters an import brings in** (#1) — a
  material now carries layers: an origin (`imported` / `user` / `machine`),
  a role (`structure` / `annotation`), a default flag and an order. Chapters
  declared by a container are read by a `ChapterScan` job (the bundled
  ffmpeg's `ffmetadata` output, one parser for every format instead of one
  per container) into an imported structure layer, which re-probing replaces
  wholesale. A user keeps their own chapter set in a separate layer beside
  the file's and switches between them; editing one never alters the other,
  and the server refuses writes into an imported layer. Existing time-based
  comments become the asset's annotation layer via a total backfill
  (migration V78). The UI's untyped `extra.chapters` reader — dead code
  whose producer never existed — is deleted in favour of a typed chapter
  panel on both the video and audio branches, and an empty imported layer
  ("the file declares no chapters") renders distinctly from no layer at all
  ("never scanned"). MCP gains a read-only `material_layers` tool.

- **CI** (`.github/workflows/check.yml`) — `just check` runs on every pull
  request and on push to `main`, so whether the gates pass is something the
  repository states rather than a claim about whoever last ran the recipe.
  The workflow invokes the recipe instead of restating its six gates, so the
  local gate and CI cannot drift apart. One macOS job for now, which is the
  simple and expensive answer; splitting the portable crates onto Linux is a
  decision left to a measurement. `ui-e2e` (needs a real window) and
  `collation-jsc` (needs macOS's `jsc`) stay out, and the workflow says so
  rather than leaving it to be inferred.

- **MCP transport** (`asterism-server`) — the third adapter over the
  same application services. A curated nine-tool vocabulary
  (`asset_search` / `asset_list` / `asset_get` / `asset_add` /
  `asset_lineage` / `asset_comments` / `asset_comment_add` /
  `catalog_overview` / `dispatch_get`) served over streamable-http at
  `/mcp` on the loopback router (present in both the Tauri-embedded
  server and the standalone binary) and over stdio via
  `asterism-server mcp` (previously a stub). Tool input schemas are
  generated from the `asterism-contract` types that already back HTTP
  and Tauri IPC (new contract feature `json-schema`); domain failures
  surface as tool-level errors carrying the HTTP boundary's
  `{kind, message}` shape.

- **Local data profiles** — `dev` / `dogfood` / `bench` homes under
  `~/.asterism/profiles/`, each with its own default HTTP port, selected
  by build flavour or `$ASTERISM_PROFILE`. A `.asterism-profile` marker
  in the home prevents opening one profile's data under another.
- **Trash and purge** — trashing is reversible and preserves rating,
  comments, group filing and body text; purge is separate, irreversible,
  and only reachable from the trash. A retention sweep purges what has
  aged past `ASTERISM_TRASH_RETENTION_DAYS`.
- **Full-text search** (`asterism-infra/search`) — a BM25 body index on
  Tantivy with Lindera Japanese morphological analysis and an English
  Porter stemmer on one tokenizer chain, persisted outside the SQLite
  transaction and reconstructed by the `index_rebuild` job after a crash.
- **Import adapters** — Claude Code session logs, tapes, persona
  journals, images, video and audio, all behind one CLI whose
  environment resolution happens in the outer command; media inspection
  is shared through `asterism-media-probe`, and video/audio bundling
  uses an LGPL-clean ffmpeg sidecar built by
  `scripts/build-ffmpeg-sidecar.sh`.
- **Export adapters** (`asterism-dispatch-sdk` + `asterism-exporter-*`)
  — outbound dispatch to ComfyUI, the filesystem, and arbitrary HTTP
  endpoints, with per-backend parameter schemas.
- **Two-sided sort contract** — the grid comparator (`Intl.Collator`)
  and its Rust port (`icu_collator`) are checked against shared
  collation fixtures, because Query Groups freeze the backend order into
  `asset_bucket.position` and the two halves must agree.
- **Benchmark corpus generator** (`asterism-benchgen`) — a seeded
  synthetic corpus (ChaCha20) where the seed, not the emitted files, is
  the identity of the corpus.
- **Domain layer** (`asterism-core/domain`) — `Persona` and `Asset`
  aggregates, an open-slug `Modality` and `SourceKind`, a `Visibility`
  model, `ConstellationEdge` with a pure `plan_edges` planner, and every
  repository port.
- **Application layer** (`asterism-core/application`) —
  `PersonaService` and `AssetService` with DTO-in / DTO-out APIs, plus
  the domain ↔ DTO mapping in one place.
- **SQLite backend** (`asterism-infra`) — `rusqlite-isle` on the 0.3
  release line (aligned with `apalis-sql`'s `libsqlite3-sys` cluster);
  append-only migrations gated by `PRAGMA user_version`; schema v1
  covering six `STRICT` tables (`persona`, `asset`, `tag`, `asset_tag`,
  `edge`, `thumb_cache`) with UUID BLOB keys and unix-epoch-ms
  timestamps.
- **Job pipeline** (apalis + `apalis-sql`) — `cover_gen` (modality-
  specific heuristic), `auto_tag` (keywords → channel tags),
  `edge_rebuild` (windowed incremental). Column-level partial updates
  avoid a read-modify-write race; `auto_tag` chain-enqueues
  `edge_rebuild` once the keywords are committed.
- **HTTP API** (`asterism-server`) — axum router bound to loopback, with
  RPC-style routes under `/asterism/*` that mirror the Tauri command
  surface. Clap CLI with `serve` and a placeholder `mcp` subcommand.
- **Contract crate** (`asterism-contract`) — Command / Query / Response
  DTOs derived with `schema-bridge`; TypeScript bindings are regenerated
  from the same source at build time and land in
  `asterism-ui/src/bindings.ts`.
- **Desktop UI** (`asterism-ui`) — Svelte 5 on Tauri v2: persona
  sidebar, modality tabs, dense grid, hover-burst side panel.
- Workspace scaffolding — `Cargo.toml` metadata, README, and this
  changelog.

### Fixed

- **The XMP writer does the two things its module doc promises** (#14) —
  both were promised in prose and neither was done, and in both cases the
  reason nothing caught it is that no fixture could reach the shape.

  **A packet another tool left behind is now removed rather than
  shadowed.** The doc's position is that a file must never leave with two
  packets, because readers disagree about which one wins and the failure
  mode is a stale `digitalSourceType` shadowing a corrected one. Both
  writers recorded only the *first* packet and copied any later one
  through untouched. The walks collect every XMP chunk or segment now,
  replace the first where it stands — which keeps a re-stamped file's
  chunk order stable — and drop the rest. The test that claimed to cover
  this reached its "twice-stamped" input by calling the writer twice, so
  its input had exactly one packet by construction and it re-tested the
  one-packet path under a name that read like it covered both. The new
  fixtures are hand-built rather than produced by the writer, on the
  habit the neighbouring fixtures already state, and they place an
  ordinary chunk ahead of both packets and another between them — with
  the packets adjacent, neither "the bytes between two packets survive"
  nor "the survivor stays where it was" is observable.

  **A JPEG with no scan keeps its packet inside the image.** The
  insertion point is "before the first non-`APPn` marker", and a
  metadata-only file that reaches `EOI` without meeting one fell back to
  the end of the file. That put the `APP1` *after* `EOI`, outside the
  structure, where the module's own reader returns `None` while
  `asterism-infra` records `xmp_written = true`: an export that reported
  success and carried no readable disclosure at all. The walk now brings
  the `EOI` offset back and the packet goes before it, which is the same
  answer the PNG side already gave a file with no `IDAT`. A *truncated*
  JPEG is a different thing and is still refused as malformed — it has
  no `EOI` either.

  Both fixes were checked against the behaviour they replace: reverting
  either one fails its new test by name, and the thirteen `embed` tests
  that predate this change pass under both.

- **Five things the provenance writer said about itself that were not so**
  (#14) — all in `asterism-infra`, all found by reading the code against
  its own comments.

  **A failed read-back is no longer reported as a shortened prompt.**
  `Stamped::prompt_dropped` exists to record that a JPEG segment could
  not hold the packet, so the reduced record was written instead — a fact
  that cannot be recovered from the file afterwards, which is why it is
  reported at all. It was derived by reading the stamped bytes back and
  asking whether the prompt survived, through `.ok().flatten()`, which
  gave the same `true` to three different outcomes: the honest fallback,
  a packet the reader could not find, and a file that would not parse.
  The last two say the writer produced something this crate does not
  recognise, which is a defect rather than a fact about the record, and
  they are errors now — `XmpUnreadable` for the one that has no
  underlying failure to carry. Nothing is known to reach that variant
  today: its one producer was the JPEG writer putting the packet where
  the reader could not see it, fixed above. It is the guard that says
  the two halves have to agree, not a case in the field.

  **A manifest that could not be built no longer blames the
  certificate.** A definition `c2pa` refuses came back as
  `DisclosureError::Identity`, which renders `signing identity: …` — so
  a mapping defect in this crate sent whoever was reading the log to
  their key configuration. It happens strictly before signing, with the
  certificate already loaded, and now has its own variant.

  **The container sniff survives a short read.** `read_head` used one
  `read`, which is allowed to return fewer bytes than the buffer holds
  without being at end of file. Local regular files do not do this;
  NFS, SMB and FUSE do, which is to say every network-mounted library.
  Eleven bytes back instead of twelve made the `ftyp` test fail and a
  perfectly good MP4 report as a container this build does not write
  into.

  **The signed output streams to disk instead of being held whole.** The
  comment on the video path said it streams "so a large video is not read
  into memory whole", and the source did — but the destination was a
  `Cursor<Vec<u8>>` collected with `into_inner()`, so a 2 GB MOV was
  fully resident at the moment of collection, and the still path held two
  buffers at once. `Builder::sign` takes any `Write + Read + Seek`
  destination, so it writes into the sibling temporary the rename will
  move, and `replace` was split into the pieces that path needs. The
  temporary is opened read-write rather than through `File::create`,
  which gives a write-only descriptor: today's signing happens not to
  read its destination, but the `Read` in that bound is there because
  the BMFF handler re-reads box headers to adjust offsets, and relying
  on it staying unexercised would fail on video and nowhere else. What
  this costs is that the temporary is visible for the length of the
  signing rather than for the length of one write.

  **A failed write clears its temporary too.** Splitting `replace`
  surfaced that its own error route returned straight out of a failed
  `fs::write`, which can leave the file created and partly filled —
  exactly the litter beside a watched export directory that the rename
  path already took care to avoid. Both routes clear it now, and a test
  covers the signing path's failure route rather than only its success.

- **When a signing certificate is configured, it is read before it is
  used** (#14) — nothing wires one yet, which is #14's own open item;
  this is what will happen when something does.
  `inspect_certificate` reports what a certificate's extensions say, and
  `SigningIdentity` refuses on the half of it that means the certificate
  cannot sign at all: an extended key usage naming nothing a claim can
  be signed under, or a CA certificate offering to sign one itself.
  Neither reaches `c2pa` as anything better than a failure later, so it
  fails here with a reason.

  The other half is reported and not acted on. A certificate without
  `c2pa-kp-claimSigning` is not one the Conformance Program's issuance
  profile would have produced, and a subject naming no organisation is
  one no validator can display a signer for — both keep a certificate
  off a trust list without stopping it signing for a reader who has
  imported it, which is a use the specification's own guidance describes
  (a private credential store, and self-issued credentials for it). That
  guidance also states the split rather than leaving it to be invented:
  of an extended key usage misconfiguration it says a claim generator
  "should warn its user with an explanation of the problem, but should
  allow the user to choose to proceed with signing". A deployment
  signing for publication would reasonably want the warnings to refuse
  too; `inspect_certificate` is public so that setting has somewhere to
  read from when it is written.

  This started out refusing every certificate without
  `c2pa-kp-claimSigning`, on the belief that the profile requires it in
  addition to `emailProtection` or `documentSigning` — which it does, of
  a certificate a conforming CA *issues*. That is not the set `c2pa`
  will sign with: its accept-list takes any one of six usages, and one
  is enough. The strict version would have refused a
  `documentSigning`-only certificate — a profile IPTC's own publisher
  policy explicitly permits — while telling its operator the certificate
  could not sign, which is false and which they could not have acted on,
  an EKU not being something you can add to an issued certificate. The
  two questions are separate and the code now keeps them separate.

  What the check does *not* look at is worth knowing: everything else
  `c2pa` requires at signing time, which is a good deal more, and in
  particular expiry. A Conformance Program certificate is valid for at
  most 366 days, so every deployment that signs meets that one
  eventually, and it arrives as a signing error rather than as an
  identity one.

  Bytes that do not parse yield no findings rather than a refusal:
  `c2pa` reads the same certificate next with a real validator and says
  something better than a guess from here. What that costs is named
  where it is done — DER rather than PEM, an empty file, and a bundle
  whose every block is something else all pass inspection silently.

- **`parameters` is AUTOMATIC1111's chunk, not ComfyUI's** (#14) — three
  doc comments and a test fixture said otherwise. The rule they were
  making is right and unaffected: a digest must not re-render a value it
  was given, because that puts number formatting and nested key order
  into the digest's definition. The example was wrong. ComfyUI writes
  `prompt` and `workflow`, both JSON; `parameters` is AUTOMATIC1111's and
  holds line-oriented prose. `domain::disclosure` had it right all along,
  which is where the two families are actually told apart.

  It matters because a reader who took the docs literally would reach for
  a JSON decoder on a value that never parses — and the fixture in
  `asterism-importer-image` had already done something adjacent, carrying
  `steps: 30, sampler: euler` under that keyword, which is neither JSON
  nor A1111's grammar. It now carries the real shape: a prompt, a
  `Negative prompt:` line, and one comma-separated settings line.

- **The PNG chunk length is checked, and three comments now describe
  what their code does** (#14) — the length was
  `u32::try_from(payload.len()).unwrap_or(u32::MAX)` under a comment
  claiming the impossible case was made loud. It was the opposite: a
  payload past the bound emitted a chunk whose declared length
  disagreed with the bytes after it, and returned success. The ceiling
  is taken from `pngmeta::MAX_CHUNK_LENGTH`, because that crate reads
  the chunks this one writes and refuses a length above it — a
  hand-written cap would make the two equal by coincidence.
  `PacketTooLarge` covers both containers now, and its message no
  longer names JPEG.

  The three that were prose only: `png::read` stops at the first XMP
  chunk even when its text is unreadable (which matches the writer, and
  now says so); the control-character filter keeps DEL and C1, which
  are legal XML 1.0 and which the comment said were dropped; and
  `IPTC_CV`'s doc claimed a structural guarantee that is actually held
  by a test. Two tests are renamed, because what they assert stopped
  matching what they were called.

- **The documented JPEG packet limit was the segment's, not the
  packet's** (#14) — three docs quoted 65,533 bytes, including the one a
  caller reads when deciding how long a prompt to allow. That is the
  segment's payload; the packet gets 65,504, because the 29-byte
  `http://ns.adobe.com/xap/1.0/` identifier is inside the payload and is
  paid first. A packet between the two figures was refused by a limit the
  documentation did not have, and the caller learned about it only
  through the silent fallback to the reduced record. The docs now point
  at `JPEG_MAX_PACKET`, which is what the writer enforces and what
  `PacketTooLarge` reports, and a test pins the arithmetic.

- **A refused operation says so on screen** (`asterism-ui`) — asking
  Asterism to do something it then refused could leave no trace: the
  failure went to the browser console and the interface carried on,
  including for operations that move or destroy data. The read path had
  no equivalent gap (`Resource` exposes load failures); the write path
  had no owner for them at all. A new `lib/mutate.ts` wraps the write
  calls, puts the refusal and the backend's reason in a sticky toast
  beside the Undo one, and re-throws so that existing rollbacks are
  unaffected. Routed through it: the grid, group and trash paths —
  `trash_asset` (including the duplicate panel's bulk trash),
  `purge_asset`, `restore_asset`, `empty_trash`, `trash_group`,
  `delete_dir`, `delete_asset_comment`, `add_asset_to_group` and
  `remove_asset_from_group`, `unlink_group`. **Not yet routed**, and
  still console-only: tag detach, persona themes, material marks,
  threads, modalities, sessions and setting resets — along with the
  non-destructive half of the write path (metadata edits, reordering,
  the create and rename family). Bulk loops that could partly fail
  now report what actually happened ("moved 3 of 5 to trash — the rest
  was refused") instead of counting a refusal as a success. The path
  is exercised end-to-end: `e2e/refusal.spec.ts` seeds its own dir
  pair over the app's loopback HTTP, provokes a real `delete_dir`
  refusal in the WebView, asserts the toast carries the backend's own
  reason, then deletes the emptied pair with the same gesture,
  asserting that success stays silent.

- **The committed TypeScript bindings are checked against the contract**
  — `asterism-ui/src/bindings.ts` is generated by `src-tauri/build.rs`
  and tracked in git, and nothing compared the two. A contract change
  whose regenerated bindings were never committed would have left a
  stale copy that every gate passed, and passed invisibly: everyone
  builds from a copy regenerated on their own machine, so only a
  consumer reading the file without compiling Rust would have met it.
  `just bindings-check` forces the build script to run, then diffs the
  result against `HEAD`; it runs inside `just check`. The forcing is not
  incidental — `tauri_build` registers `rerun-if-changed` directives,
  which means a warm tree can otherwise skip the script entirely and
  compare the committed file against itself.

- **`rust-test` no longer depends on the caller's colour setting** — the
  recipe counts cargo's `Running` / `Doc-tests` lines against the
  `test result:` lines to prove that every launched binary reported a
  result, and both patterns are anchored at the start of the line.
  Coloured output puts an escape sequence there, so the count came back
  0 launched against 81 reported and the check failed over a suite that
  was 1191 passed / 0 failed. It fixes `CARGO_TERM_COLOR=never` for
  itself now, rather than parsing a shape its caller's terminal can
  change.

- **The e2e suite is now type-checked** (`asterism-ui`) — the specs and
  both WebdriverIO configs sat outside every tsconfig, so `just ui-check`
  reported zero errors over ~4200 lines it never read, and the test
  runner erased their types rather than checking them. A second config
  (`tsconfig.e2e.json`, run as `check:e2e`) covers them without putting
  `describe` / `it` / `browser` in scope for application code. The seven
  diagnostics it surfaced on its first run are fixed: `await $$(…)` now
  goes through `getElements()`, and both configs take `tauri:options`
  and `browser.tauri` from the service's own `TauriCapabilities` instead
  of a local cast.

### Boundaries

- **Data layout**: user data is isolated per local profile rather than
  living at one fixed path. Release builds default to
  `~/.asterism/profiles/dogfood/`, debug builds to
  `~/.asterism/profiles/dev/`, and stress runs select `bench`;
  `$ASTERISM_PROFILE` names a profile and `$ASTERISM_HOME` overrides the
  location outright. A named home carries a `.asterism-profile` marker
  and is refused when opened under a different profile. The UI and the
  standalone server share whichever home is selected.
- **Deletion is two steps**: trash hides an item and keeps everything
  about it; purge is the irreversible half and is reachable only for
  something already trashed. The retention sweep window is
  `ASTERISM_TRASH_RETENTION_DAYS` (default 14); a malformed or
  non-positive value is refused at startup rather than silently
  replaced.

[Unreleased]: https://github.com/ynishi/asterism/commits/main
