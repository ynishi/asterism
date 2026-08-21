# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The three-state moved out of the digest columns** (#17). The
  `unsupported:<mime>` / `unhashable:no-bytes` marker strings that rode inside
  `material.content_hash`, `content_region_hash` and `meta_hash` became a status
  column beside each digest (`pending` / `computed` / `unsupported` /
  `empty-span` / `too-large` / `not-walked` / `no-bytes` / `failed`), with the
  media type — the genuinely valuable part of the old marker — kept in a reason
  column; the digest columns now hold digests and nothing else, and a migration
  converts every stored row. On the wire, `AssetDto.content_hash` stops carrying
  markers and a `content_hash_status` field says why a digest is absent. The
  second half of the issue rides along: a read that fails now records `failed`
  with the I/O error instead of staying silently pending, so the "still
  fingerprinting" count can actually reach zero — the walk still retries those
  rows every pass, but they surface as the report's new `unreadable_count` (an
  original that is moved, deleted, or on a disconnected disk) instead of holding
  the progress notice open forever.

### Added

- **The forge has a model of its own** (#63). A line is a repository — the top
  of the forge, with one canonical history and everything on it derived by
  folding that history. A history begins at a genesis and grows by one change
  point at a time, each carrying a table of rows over three axes: existence,
  content, name. Order is the parent chain's, never a clock's, so two nodes
  minted in the same millisecond are still ordered and a clock that steps
  backwards changes nothing. Two entries cannot answer to one name, judged after
  the whole table is applied — handing a name from one entry to another in a
  single change point is one gesture rather than a collision. Nothing removes
  anything: taking an entry off the line is a change point that says so, and
  what was dropped stays readable. Domain only for now — no storage, no
  transport, nothing wired.

- **Work can be put on a line, and the two are one act** (#63). The other half
  of the forge's model, and the layer that drives it. Work opens against a line,
  cut from wherever the line is at that moment, and writes passes that never
  read the line — the operation that happens most often is the one that cannot
  contend with anybody. What a pass asks for only means something measured
  against a line, and that happens when the work ends: ending it as satisfied
  produces the close and the change point together, in one value that cannot be
  taken apart, written through one call that keeps all of it or none of it.
  Neither constructor is reachable from outside the model, so a satisfied close
  without its change point is not a thing that can be built. A close that loses
  the head is not reported: the line is read again and the ending decided again,
  against the line as it now is, which is a fresh answer rather than the same
  one retried.

  A collision is an axis this work's request would move that the line moved
  after the work was cut, and that is the whole definition. It is computed from
  the two logs whenever it is asked, never stored, so nothing can go stale and
  there is no flag to clear — and nothing in it mentions what anybody looked at,
  because the model keeps no record of reading. What clears a collision is the
  work asking for something else, which means **an axis stops colliding only
  when the work stops asking for it**: requesting the same value a second time
  changes nothing, since a fold keeps the last value and not the arguments for
  it.

  Resolving is therefore ordinary work, and automatic resolution writes what a
  person would have written by hand, in the same four verbs, into an ordinary
  pass. Five rules ship and a line names one — keep the line's version and carry
  this work's onto a new entry; keep this work's under the contested name and
  move the line's aside; put both aside and take the old entry off; write this
  work's version down and then remove it, so what was tried stays readable; or
  write nothing and leave the collision standing for somebody to answer. Rules
  say what they do, so choosing one is a choice somebody makes rather than a
  default they inherit. What a rule returns is checked rather than trusted: the
  model folds it in and refuses the rule if the collisions it was asked about
  are still there. What a rule writes is recorded as the server rather than as
  the person who asked for it.

  Who did a thing is the forge's own word now: a handle, and whether it was a
  person or the server. What the handle stands for — which authenticated user,
  which instance — is asked through a new face and answered outside, because the
  binding has not happened yet and a node that recorded today's answer would
  have to be rewritten the day the real one arrives. The cost is stated where it
  is paid: a forge node no longer records which agent carried an operation out.
  Time is asked for rather than read, alone among this codebase's services,
  because a timestamp here is evidence in a record that never moves and nothing
  orders anything by it — so a wrong one breaks nothing and misleads for good.

  Work can be discussed as well as done. A thread hangs off a pursuit, one pass,
  one entry as one pass had it, or what landed — the four things worth remarking
  on — and it is the forge's own rather than the annotation surface the layer
  below has, which anchors to snapshots and cards and could not learn these four
  without learning what a pursuit is. The entry anchor names the pass as well as
  the entry, so a remark about one attempt does not follow that entry into every
  other pursuit it is ever carried into. Nothing is overwritten: a correction
  appends a revision and every earlier wording stays readable. Nothing is
  resolved either — whether a remark is dealt with is a word people use about
  their work rather than a shape the record has, so a later message says it.

  Lines can be listed, work can be found by the line it is against, and work
  filed under a larger piece of work can be found by its parent — the last one a
  plain omission, since a pursuit has named its parent since it was written and
  nothing could read it back. Ended work is in those listings, because a listing
  that showed only what is open would hide what was tried and abandoned.

  Still domain and application only: no storage, no transport. The service the
  new one replaces has been renamed `LegacyPursuitService`, since what is
  leaving should carry the qualified name.

- **The digital source type can be asserted by hand** (#23). The last route of
  the provenance policy: an artefact this app generated is known synthetic, an
  imported declaration is trusted, and what is neither is unknown — until the
  signer states what it is, and the statement is signed verbatim, their claim
  under their certificate. `POST /asterism/assets/{id}/source-type` records the
  IPTC term (URI or short name; unknown terms are refused at the door, since
  everything downstream signs this verbatim), an absent `source_type` retracts
  it, and the assertion outranks the container when both speak — with the
  generator's own statements (`AISystemUsed`, the prompt) dropped under a term
  chosen to say no model made this, so the container's contradiction never
  enters the signed claim. A parent carrying an assertion reads as declared,
  never unknown: assert `digitalCapture` on a scanned photograph with no
  metadata at all, and the generated child over it becomes an honest
  `compositeWithTrainedAlgorithmicMedia` where an unasserted parent would have
  left it `trainedAlgorithmicMedia`.

- **The signing key can live in the macOS Keychain** (#23).
  `ASTERISM_DISCLOSURE_KEYCHAIN_KEY` names a private key by its Keychain label,
  in place of the key-file path — one of the two, never both. The key —
  including one held in the Secure Enclave, which the same search finds — never
  enters the process: signing goes through `SecKeyCreateSignature`, so there is
  no heap copy to zeroize and no key file whose mode anyone has to audit, which
  is the custody the C2PA guidance asks of a production signer. The certificate
  chain still travels as a file; it is public material and goes into every
  manifest anyway, and it passes the same acceptance checks as the file-based
  identity. ECDSA only (`es256`, `es384`, `es512`): the Security framework does
  not sign Ed25519, and an RSA arrangement keeps the file form. A label that
  names nothing refuses at startup rather than on the first export, and setting
  the variable on a non-macOS build reports the identity as unavailable instead
  of quietly signing nothing.

- **A selection gesture can carry a sentence** (#65). `trash`, `restore`, and
  `trash_group` accept an optional one-line comment, and the remark lands as an
  `AssetComment` pinned to the gesture — actor, time, and verb on the row — so
  "why this one was thrown" survives the throw and "keep for the pose, not the
  face" survives the salvage. A group's remark fans out to every member asset: a
  comment is per-asset, and the sentence said over a batch is exactly what each
  member's siblings want to surface later. Strictly a footnote mechanism — free
  text, optional, silent when absent — and deliberately not a verdict record,
  which stays #22's territory. Disposal verbs (`empty_trash`, `purge`) take no
  comment: executing a decision already made is not a moment anybody states a
  reason at.

- **The teams plane learns to let go, and to survive** (#95, fifth slice of the
  #83 design — the last area ahead of the share port). Storage reclaim is an
  explicit verb with a conscience: an owner (or the operator, operator-stamped)
  marks a team's blob link for purge, the link vanishes from every read while
  the grace window runs — restorable by unmark the whole time — and only an
  explicit reclaim, refused until a mark has aged past the window, removes the
  ripe links and appends the purge event. The record survives; the bytes go.
  Mark, unmark, and reclaim are each first-class ledger events, and the ledger
  tables stay append-only — the mark state lives on the link row. Blobs nothing
  links anymore are swept by a guarded zero-link sweep (uploads and the sweeper
  share a lock, so a racing same-digest upload can never lose its bytes), inline
  after reclaim and on demand via the `gc` subcommand. And the instance can back
  itself up in one command: quiesce, snapshot the database via `VACUUM INTO` —
  never a live file copy — then database first, blobs after, into a single tar
  whose worst inconsistency is an orphan blob, never a dangling reference.
  Restore is an unpack and a `--db`/`--blobs` flag, proven end-to-end by a test
  that unpacks an archive and reads a blob back through its link.

- **The teams plane holds bytes** (#93, fourth slice of the #83 design). The
  blob port gets its local adapter: a global content-addressed store under
  `blobs/sha256/<2ch>/<64hex>`, written the careful way — stream into staging
  while hashing, verify against the digest the client declared, fsync, rename
  into place, fsync the parent — so no partially written blob is ever visible
  and a crash leaves at most a staging temp the startup sweep clears. Upload
  rides the OCI contract: the declared digest is mandatory, the server hashes
  while writing, and a mismatch rejects the whole operation — no blob, no link,
  no ledger event. Uploads are always accepted in full and deduped server-side
  only; the response never reveals that the instance had the bytes already.
  Bytes land in the CAS first, then the link row and the blob-copy-completed
  event commit in one transaction — an interrupted upload leaves a harmless
  orphan blob, never a dangling reference. Reads stream, never buffer, and a
  digest exists for a caller only through a team they belong to: unknown team,
  non-member, and never-uploaded all return the same 404.

- **The teams plane opens its door** (#91, third slice of the #83 design).
  `teams-server` stops being a stub: instance-local auth v0 — argon2id password
  hashing, opaque session tokens the database stores only as hashes, expiry
  enforced on touch plus a bulk sweep, one rate limiter over every auth endpoint
  from the start — and the `/teams/*` HTTP API, every team-scoped route behind
  the session → user → membership gate. Team create / delete, invite / remove,
  owner grant / revoke follow the authority table; domain refusals (the
  last-owner rule above all) come back as client errors with state and ledger
  untouched, and role changes are readable back through the events route with
  old and new values in the payload. The instance operator is bootstrapped from
  env/CLI with no fixed defaults and refuses to run twice — v0 has exactly one
  operator, an instance capacity outside every roster, operator-stamped in the
  ledger whenever it acts inside a team. `teams-contract` carries the
  request/response types through the schema-bridge flow.

- **The teams plane gets its storage floor** (#89, second slice of the #83
  design). `teams-infra` lands the SQLite layer: a teams-owned database with its
  own fresh migration series, WAL and the sibling's pragma discipline, opened
  through the workspace's one `rusqlite-isle` line. The state tables — team,
  membership, blob links, locators — are the source of truth, and the ledger is
  the record: every public write is one transaction that applies the state
  change and appends its ledger event together or not at all; there is no method
  that writes state without appending and none that appends without state. The
  one documented exception is the locator — private-space operations never land
  in the ledger, by design. The ledger stream is append-only in the API and in
  the schema (no updated_at, no soft delete, abort triggers on UPDATE/DELETE),
  seq is storage-assigned, monotonic and gapless per team, and subjects land in
  an index table so trace queries never parse payload JSON. Domain invariants
  ride inside the transaction: the last-owner rule refuses the write in the same
  tx that would have recorded it. Profile markers publish the same way the
  sibling's do — temp file, sync, hard link — so a crash mid-publish cannot
  wedge a home behind an empty marker.

- **The teams plane gets its crates and its domain floor** (#87, first slice of
  the #83 design). Four crates join the workspace — `teams-core` / `teams-infra`
  / `teams-contract` / `teams-server`, the same layering as their asterism-*
  siblings, `publish = false`, license deliberately undeclared until the plane's
  licensing decision. `teams-core` carries the whole IO-free domain layer:
  identity (User, Membership with owner/member roles as validated TEXT, the
  instance operator as an actor that is never a membership, actor stamps that
  record the display name at write time and never rewrite), the last-owner
  invariant (a team keeps at least one owner; leave, remove, and self-demote are
  refused at the brink), the ledger event envelope (namespaced+versioned kinds,
  typed subject refs, opaque payload) with the v0 kind registry, and the store
  rules — team blob links, locators whose digest is a hint and never a
  verification source, and declared-digest verification whose only outcomes are
  accept-equal or reject-the-whole-op: accept-new-digest is a state the types
  cannot express. The blob / auth / share ports are declared, the share port an
  empty reservation until #63 settles its verbs. `teams-infra` and
  `teams-contract` compile as shells; `teams-server` owns its own binary, fully
  separate from `asterism-server` — the license boundary sits at the bin edge.
  Only direct dependency edges are claimed: `teams-* → asterism-core` and
  nothing else, with no `asterism-* → teams-*` edge anywhere.

- **The project, its mainline, and the merge record — the forge gets a place to
  land** (P1 of #63). The forge could say what a pursuit tried and what a close
  kept, but not what anything landed on. Now the project is the repo of the
  forge's git analogy — per persona, deliberately opened, one `main` line each
  (the `line` table admits named siblings before the code does, so "the
  mainline" is a description, not a type) — and above raw asset ids sits the
  line entry: the identity that stays "the living one" while replacement and
  renaming move beneath it. Four merge verbs (`add` / `replace` / `delete` /
  `rename`) move an entry as an append-only sequence; liveness, current name,
  and current version all derive on read, latest-wins per axis, so history is
  the verb sequence itself. The merge row binds a satisfied close to what it
  landed — approval _is_ the merge event, so an approved entry can never
  silently hold unapproved bytes. This slice is schema and domain only (V83,
  `project.rs`, `line.rs`, derives, tests): no write path yet, every line starts
  empty, and a pursuit that files under no project behaves exactly as before.

- **Filing, and the columns a targeted IN needs** (P2 of #63). A pursuit can now
  say which project it files under, and the ledger can say which line entry an
  `in` is aimed at, which version of that entry the caller saw when it aimed,
  whether the aim reached into another project, and which member an `update`
  revises (V84). Filing is nullable because filing is what mints a pursuit once
  exploration moves below the forge — a pursuit with no project is what the
  always-mint rule left behind, not a mode, and those rows are left as they are
  rather than given a project they never had. The aim rides on the gesture that
  carries it rather than in four loose columns, so a pin with nothing pinned, a
  `remove` claiming it reached out of scope, and anything but an `update`
  superseding a member are all states the domain cannot express; only "just an
  existing `in` may aim" needs a check. Purging a persona now sweeps its project
  and lines with everything else.

- **The filing verb — a project to work under** (P2 of #63). `project_open`
  opens one and mints the `main` line with it in the same transaction,
  `project_get` and `project_list` read them back, and `pursuit_open` takes a
  `project_id` that puts the pursuit's satisfied close on that project's line.
  Two rules the schema cannot hold live in the service: a project name is unique
  among one persona's projects, checked by reading first, so two simultaneous
  opens of one name can both land; and filing never crosses personas, which no
  foreign key can say because `project` carries its own persona and the column
  references only the project. A parent's filing is not inherited — a child says
  where it files or files nowhere. The close still lands nothing: what a filed
  pursuit does at close is P3.

- **A refused submit records what it sent** (#76, carried from #41). The record
  of a call existed only where the call succeeded: the HTTP adapter builds the
  exchange — the request as sent beside the response as received — on its way to
  a handle, and a submit the backend refuses returns an error instead, so
  everything a reader would ask about it left with that error. What stayed on
  the row was one sentence. That is backwards from where the questions are: a
  job that ran produced an artefact carrying its own call note, and a job that
  was refused is the one with nothing to read.

  Exporters now record a call through `DispatchContext.attempt`, and the runner
  writes down whatever landed there after every exporter call — on the arm that
  returned a handle and on the arm that returned an error alike. The record is
  its own column pair (`dispatch_job.attempt_kind` / `attempt_payload`, schema
  V83) beside the handle rather than inside it, because a handle means "a job
  exists over there" and a refused submit has no such job; it reaches a caller
  as `DispatchDto.attempt_json`, the same read a successful dispatch is
  inspected through. The HTTP adapter writes the same `exchange` shape either
  way, now carrying the status it was answered with and, when nothing answered,
  the transport's own words — so a backend that rejected the request reads
  differently from one that was never there. The failure message on the row is
  unchanged, and the credential discipline is the one already in place: the
  secret an `auth` block named is scrubbed out of everything recorded, on the
  way in, including a backend's echo of the request it rejected. Unchanged in
  its limit too — a token a profile interpolates out of its own params into a
  URL or a body was never named as a credential, and this record is one more
  surface it can reach.

  Poll and harvest record on the same channel, for the calls that end a job — a
  routine poll's exchange answers nothing the row does not already say. One
  record per row, replaced by the latest attempt: a re-run is a fresh dispatch,
  so the sequence a reader wants is already the sequence of rows. Rows written
  before the columns read as "nothing recorded"; there is no backfill, because
  the calls they describe are over and were never written down to recover from.

- **The ledger and the cull — selection is recorded** (#22, model on #63).
  Keeping or discarding a generated asset used to move through four unrelated
  routes — trash, a low rating, the inbox label, a fold policy — and none of
  them said that a selection happened, who made it, or out of which set. Now the
  pursuit carries an append-only membership ledger (`pursuit_tx`: an asset
  enters with its origin — generated by a round, imported, or brought in from
  the existing library — and mid-work removal is a reversible gesture, not an
  exit), and a satisfied close records the cull (`cull` / `cull_member`): the
  candidate set is derived from that ledger and frozen — never supplied by the
  caller — and each member's verdict is `keep` or `reject`, with absence meaning
  the act said nothing about it.

  The close's defaults are the model's: a member removed mid-work and not spoken
  for culls as `reject` (materialised as a row, so a reader never re-derives
  it); a `keep` on a removed member is salvage; an `existing`-origin member
  takes `reject` only, because keeping what the library already holds is the
  untouched default, not a statement — salvage excepted. The kept set the close
  event freezes is now exactly the `keep` verdicts, so `ClosePursuitCommand`
  states `verdicts` where it used to state `kept_asset_ids`. The event and the
  cull land in one transaction; an abandoned close applies nothing, as before.

  Reify writes the ledger too: each dispatch output files an `in`/`generated`
  row under its pursuit, and the migration transcribes every already-recorded
  output the same way — one membership per (pursuit, asset), the dispatch's
  clock, NULL attribution (nobody recorded those entries; the migration did).
  Surfaces: `POST /asterism/pursuits/tx` and the `pursuit_tx` MCP tool append
  gestures; `GET /asterism/assets/{id}/culls` and the `asset_culls` tool answer
  the question this issue was opened for — who decided to keep or drop this
  asset, out of what, in which line of work — with the recorded attribution
  (`author_kind` / `operator_ai`) riding every tx and cull read, because "who"
  is the half the four old routes could never answer; the pursuit view now
  carries `txs` and `culls`. History outlives the asset: a ledger member purged
  mid-pursuit can still be rejected (the verdict row carries no foreign key), a
  `keep` of one is refused, and the freezes hold the surviving members. The
  `update` gesture is admitted by the schema's vocabulary but refused by the
  verb — it is the external-edit round-trip's word, and that slice has not
  landed. `pursuit_restamp` now reserves `cull` where it reserved `judgment`,
  the withdrawn spec's name.

- **`just worktree-new <type> <slug>` — a worktree that starts with a target
  directory** (#67). A fresh worktree has no `target/`, so its first gate
  rebuilt the whole dependency graph: `cargo check -p asterism-infra` measured 1
  min 17 s cold against 39 s with a copy of this checkout's `target/` in place.
  The recipe is the three commands the Branches section already prescribed —
  fetch, `git worktree add` from `origin/main`, `branch-check` — with that copy
  added between the second and the third, plus a refusal to run inside a
  worktree, since nothing in git stops `.worktrees/` from nesting.

  A copy, and not the one shared target directory that was the first proposal.
  Cargo treats path dependencies carrying the same name, version and
  workspace-relative path as the same crate even across checkouts
  (rust-lang/cargo#12516, open), which every crate in this workspace satisfies
  against every other worktree; two worktrees pointed at one directory can
  therefore report a gate green against the other branch's binaries, silently,
  whenever the sources are older than that directory's last build. Copies
  collide with nothing, and they do not queue behind cargo's build lock either.

  The copy is made only where it is a copy-on-write clone. On APFS that is 2.9
  GB in under three seconds, no disk consumed until one side writes, and mtimes
  preserved, which is what keeps cargo's fingerprints meaningful; on Linux it is
  `cp --reflink=always` on btrfs, bcachefs or XFS made with `reflink=1` — the
  same operation, with no timing taken for it yet. ext4 clones nothing and is
  what most distributions leave on `/`, so Linux gets the hardlink path below
  more often than the clone. The filesystem is asked before the copy rather than
  judged by the outcome, because neither `cp` answers usefully afterwards:
  `cp -c` does not fail where clonefile is unavailable — it falls back to a real
  byte copy, which would cost more than the build it was meant to save — and GNU
  `cp --reflink=always` does fail, but once per file, and a full `target/` here
  held 74,802 of them. Asking is one clone of one 8 KiB file inside the new
  worktree, removed again before the recipe returns, or named in a NOTE where it
  could not be.

  Where Linux has no clone it hardlinks, which is the one way left to hand over
  a target directory without copying it: seconds and about 10 GB against the 6.3
  minutes and 111 GB the byte copy of the same tree costs at the 301 MiB/s this
  machine writes — and two of those copies do not fit beside a checkout that
  already holds one. A hardlink shares the inode, so a write through one path is
  a write to the other, and only one part of the tree is safe on those terms:
  the artifacts of a megabyte and up under `deps/` (2,307 files, 85.56 GiB),
  which cargo names by a hash of their inputs and swaps by unlinking its own
  copy first. The remaining 33,368 files of 10.44 GiB are copied, because each
  has a writer that opens the file already there — rustc truncating dep-info,
  cargo rewriting its fingerprints and `.rustc_info.json`, a re-run build script
  writing into an `OUT_DIR` nothing cleared, rustdoc overwriting its JSON, and
  `.cargo-lock`, which is the inode two checkouts would queue on. `incremental/`
  is dropped rather than either, since cargo regenerates it and it is 18 GB of
  the 111. A worktree cut this way builds 4 to 16 crates where a cold one builds
  the 753-crate graph.

  The copy is the slow half — 45 seconds with the tree in page cache, six
  minutes reading it cold — so it runs in the background and the recipe returns
  in about two seconds. Nothing reads `target/` until something compiles, and
  the staging happens under `workspace/`, where being gitignored keeps an
  unfinished copy from making the tree dirty and blocking the branch's own
  `-changed` gates. `workspace/target-staging.log` says when it lands; a build
  that starts first gets a cold `target/` of its own and keeps it.

- **`just commit-msg-check` — the commit-message rules are checked rather than
  remembered** (#67). CONTRIBUTING asks for a body wrapped at 72 columns and for
  no CI skip keyword anywhere in a message; both were rules a reader had to
  hold. `scripts/check-commit-msg.py` takes a message file, a revision range, or
  both, and `pre-push` runs it over `origin/main..HEAD` — second, right after
  `branch-check`, since neither assertion's answer changes for having compiled
  the workspace first.

  Python rather than a line of shell, because the count has to be in characters:
  macOS `awk`'s `length()` counts bytes whatever `LC_ALL` says, reads an em dash
  as three columns, and calls a 71-column line 73. That is a wrong answer in the
  direction that invents work.

  Two kinds of commit are announced as unchecked rather than judged: the CI
  bot's, since the aidoc job pushes a regenerated-docs commit whose subject
  names the skip keyword deliberately, onto branches, and rewriting a pushed
  commit is not a move the report could ask for; and merges, whose message
  GitHub generates from a pull request title over an author who typed none of
  it.

  The skip keyword in a _title_ is still a human rule — a title reaches `main`'s
  merge commit and is read the same way, and nothing here can see one. The
  failure it produces is silent: a skipped workflow leaves its checks _pending_
  rather than failing, so nothing turns red and nothing is missing from the
  list. File contents are unaffected, which is why this reads messages and not a
  diff.

- **The pursuit — a minted unit of work over the dispatch loop** (#29, design on
  #21). Content ancestry cannot say "these rounds were one attempt at one
  thing": regeneration shares no derivation edge with the round it supersedes,
  rejected rounds have no descendants, and abandonment has no ancestry
  expression at all. So the unit is minted — three tables (`pursuit`, thin and
  immutable; `pursuit_event`, append-only close/reopen facts with standing
  derived on read; `pursuit_restamp`, the recorded repair verb) and a stamp
  column on `dispatch_job`, RESTRICT everywhere, with the persona purge sequence
  extended to sweep them in dependency order.

  Every dispatch start verb now files its round under a pursuit: supplied ids
  are validated in-persona (continuation is explicit, never inferred from
  content overlap), an absent id mints an anonymous pursuit in the same request,
  and a re-dispatch inherits the prior round's pursuit — an explicit reference
  to a named dispatch, not a heuristic. The migration backfills one single-round
  pursuit per existing dispatch with NULL attribution: nobody opened those
  pursuits, the migration did, and grouping consecutive dispatches by guesswork
  is exactly the inferred correlation the model forbids. Restamping moves a
  round (its returns follow through the dispatch join) or, later, a judgment;
  the move lands atomically with its record, is refused when the caller's
  recorded `from` no longer matches the row — a repair verb that guesses is
  worse than one that refuses — and never crosses personas.

  The lifecycle verbs are a service now (`PursuitService`): open (the explicit
  pre-create — intent named up front, parenthood walled to one persona), close
  (`satisfied` freezes the kept set at that moment into a snapshot the event
  references, sorted ascending server-side so identical kept sets dedupe across
  closes; an empty kept set is the defined "concluded with nothing kept";
  `abandoned` keeps nothing by construction), reopen (legal on an open pursuit —
  a recorded fact, not an error), restamp (the repair verb over a named dispatch
  round), and the standing-deriving reads.

  The membership read completes the domain/application layer: a pursuit opened
  up (`PursuitService::view`) is its row plus its rounds, its **returns**, and
  its events — all derived, nothing stored. Returns are found through two
  virtual generated columns (V80) that surface a `_trace` claim iff it resolved,
  each behind a partial index, so the reverse lookup is an index seek and never
  a library scan; the claim-lane authority order (dispatch join first, direct
  claim only where no hop resolved) is baked into the columns and the probe
  predicates rather than repeated per caller. Listings derive standing from one
  window query instead of one events read per row. The performance question was
  answered with a receipt, not an assumption: `just bench-measure-pursuit` seeds
  a throwaway profile at the documented scale and measures through the real
  adapters — at 100,000 assets / 200 pursuits, single process, warm by
  construction (the process that seeds measures), returns_of p95 is 97µs, the
  composed view p95 199µs, list-with-standings p95 530µs. The last of those is
  the one number that grows with a persona's event total rather than the page
  width (the standing window scans the persona's events; an index for it is a
  recorded follow-up, cheap and not yet earned). That is why there is no
  materialised projection and no job behind this read; the bench stays in the
  tree as the tripwire that says when that decision must be revisited. Transport
  routes (HTTP / Tauri / MCP) are the remaining slice of #29.

  The sidecar claim lane closes the loop outward and back: exporters that write
  a sidecar copy the stamp out as `_asterism.pursuit_id` beside `dispatch_id`,
  and on re-ingest the value is a **claim** — recorded in `_trace` with its own
  resolution marker (it resolves iff the pursuit exists in the ingesting
  persona), never a reason to refuse the file, owned by the claim's
  clear-then-write so a re-declaration that no longer carries one loses it, and
  retried by the same post-reify sweep that repairs dispatch claims (the two
  halves repair independently — an unresolvable derivation does not hold a
  pursuit answer hostage). The rule the membership read will follow, stated now
  so the claim lane is built for it: where the dispatch join and a sidecar copy
  disagree, the dispatch row's own stamp answers — the copy can be stale after a
  restamp — so the disagreement never needs adjudication.

- **A certificate is configuration, and a broken one says so** (#14) —
  `SigningIdentity::from_files` had no caller and no way to reach it, so no
  deployment could turn the manifest half on. Five environment variables now do:
  `ASTERISM_DISCLOSURE_CERT_CHAIN`, `ASTERISM_DISCLOSURE_PRIVATE_KEY`,
  `ASTERISM_DISCLOSURE_SIGNING_ALG`, `ASTERISM_DISCLOSURE_TSA_URL` and
  `ASTERISM_DISCLOSURE_SIGNING_STRICT`, resolved once at startup in the
  composition root.

  Not `SETTING_REGISTRY` keys. That registry holds user preferences and its own
  module doc records that mixing deployment configuration into the namespace is
  what made an earlier resolution order wrong; beyond that, a registry key is
  writable through `PUT /asterism/settings/{key}` without going through any UI,
  and readable back with the value and origin of every layer — so storing a
  _path_ there would hand that route the choice of which file this process
  opens, and publish the location of the private key to the settings screen.
  Signing is the operator's arrangement with an issuer. What it costs is that
  the settings screen shows nothing about signing; the startup log and the
  record beside an export are where it is read.

  `DisclosureWriter` grew a third state to go with it. A build that configured a
  certificate which then failed to load is not a build with no certificate, and
  reporting both as `Skipped(NoSigningIdentity)` tells whoever reads the record
  that the deployment is doing exactly what it was set up to do — at the one
  moment that is untrue. `DisclosureWriter::unavailable` carries a reason into
  the manifest half as `Half::Failed`, which is where this type already said an
  expired certificate belongs. Startup is not refused over it: a
  conformance-profile certificate is valid for at most 366 days, so every
  signing deployment eventually meets an expired one, and exiting there would
  answer an expiry by making the library unopenable.

  That reason names no file. What it is handed reaches the disclosure note
  persisted on every stamped asset's `extra._trace` and leaves through
  `AssetDto::extra_json`, so a failure to read the key would otherwise publish
  the key's path to every client that fetches the asset — permanently, and by a
  wider route than the settings surface this configuration avoided for that same
  reason. The path stays in the startup log, where whoever fixes it is looking.

  `Strictness::Strict` is the opt-in for an installation that publishes: it
  promotes `inspect_certificate`'s warnings to refusals and requires the bundle
  to carry an issuer chain rather than a lone certificate. It cannot be the
  default — it signs nothing at all on a self-issued credential, which the
  specification describes as a legitimate arrangement. It also logs when it
  _accepts_, so that a passing check leaves evidence rather than being inferred
  from the absence of an error.

- **Hosted platforms are a profile of the HTTP exporter, not a second adapter**
  (#35, #30) — a hosted generation API needs three things a self-hosted backend
  does not, and they arrived first as their own crate. That was the wrong axis:
  both speak the same job API — submit, keep a handle, poll, collect — and
  whether the URL is `https`, where the credential comes from, and how long to
  keep waiting are configuration rather than adapter identity. The crate is gone
  and the three are optional blocks on `exporter:http:params`:

  `auth.secret_ref` holds an environment variable _name_; the value is resolved
  per call, rendered into `{{secret}}`, and never enters the params blob — which
  is persisted unedited and readable by anything that can list dispatches. A
  profile without the block binds no secret, and `{{secret}}` in one is refused
  rather than rendered away as an empty string. `fetch` pulls the produced bytes
  into `<asterism_home>/custody/dispatch/<id>/` before the harvest returns, so
  the locator names the file we hold rather than a URL the backend is expected
  to stop serving; without it the backend's own URL stays the locator.
  `deadline_seconds` is per profile with no default, because how long a job may
  take before its result is gone is a property of the backend and the profile is
  where the backend is described; exceeding it fails the job with a message
  starting `deadline exceeded`, so an expiry is distinguishable from a backend
  failure, and a profile that sets none polls until the backend answers.

  The request as sent and the response as received are kept on the dispatch row,
  with the credential redacted on the way in, and the harvest copies that record
  onto every produced asset under `extra.http.call` (#30) — the job id the
  backend gave us, when we asked, what we sent, what came back, and the finished
  job's response whole, because the seed a backend ran with and the prompt as it
  rewrote them sit beside the artefacts array rather than inside it. Both are
  unconditional: a hosted platform hands back a URL and little else, so the
  moment of the call is the only moment that record exists, and the profile flag
  that used to gate it defaulted to discarding it. The note is scrubbed where it
  is built — a backend that echoes the request is how a credential rendered into
  a query string comes back, and this copy lands on an asset. So is the handle,
  on the way into the payload: `handle_from` defaults to the whole submit
  response, so the same echo comes to rest on the dispatch row, and that row is
  now read by callers rather than only by the runner. What no scrub reaches is a
  token a profile interpolated out of its _own params_; `auth` is the way out of
  that, and the crate doc says so where the pattern is documented.

  That record is readable without a database. `DispatchDto.handle_json` carries
  the exporter's handle payload as JSON text, beside the `params_json` it is the
  other half of — for an HTTP dispatch the recorded exchange is under
  `exchange`, and a submit that produced no artefact still answers what it sent
  and what came back, which no asset could. Opaque, because the shape belongs to
  the exporter that issued it; absent while the backend has not accepted the
  job, and for one that failed or was cancelled before it ever did.

  Migration is by alias rather than flag day, because stored params are re-read
  on every re-dispatch: `submit` accepts its old name `dispatch`, the harvest
  map's `source_url` accepts `locator`, and the merged adapter is registered
  under both `http` and `cloud` so existing dispatch rows — and the
  `_dispatch.exporter_slug` on every asset they produced — keep resolving. A
  handle stamped `cloud` by a job in flight across the merge is still accepted.
  The shipped example is now a hosted-shaped profile, streamed by
  `asterism-server schema print exporter:http:params`.

- **The adapter template and JSONPath grammars are shared, behind traits** (#35)
  — `asterism-exporter-common` holds the `{{...}}` substitution and the path
  subset a schema-driven exporter is configured with, and the adapters reach
  them through `TemplateAdapter` / `ResponsePath` rather than calling the
  engine. Not in `asterism-dispatch-sdk`: that crate is the port every backend
  author reads, and machinery there is read by authors who never template
  anything. The traits are what let the HTTP adapter add a `{{secret}}` root —
  one overridden method, with the JSON-leaf and header traversals inherited, so
  the placeholder means the same thing in a header, a body field and a query
  string without three implementations agreeing to.

- **The disclosure vocabulary moved into the core, and `provenance` stopped
  naming two things** (#14, #23) — `provenance` was already the derived-from
  claim graph, whose own documentation says this application's lineage is
  "deliberately **not** a reading of any external identity system — xmpMM, C2PA
  and the rest are channels a claim can _arrive_ on, never the substrate it is
  stored in". The AI-disclosure feature then took the same word for the thing
  that stores C2PA. `application::provenance_service` is `disclosure_service`,
  `infra::provenance` is `infra::disclosure`, and the types follow
  (`DisclosureService`, the `DisclosureWriter` port, `DisclosureError`).

  The crate split with it. `asterism-provenance` held the vocabulary _and_ the
  renderers, so `asterism-core` reached `pngmeta` and a CRC through it — the
  container parser its own manifest records evicting. The vocabulary
  (`DigitalSourceType`, `DisclosureRecord`, `Stamped`) is
  `asterism-core::domain::disclosure` now; the renderers are
  `asterism-disclosure-format`, depending on the core rather than the other way
  round. What forced it rather than leaving it as debt: reading a disclosure
  _back_ has to be modelled in the core, because a port cannot return a type the
  core cannot name.

  The job kind goes with it: `disclosure_stamp`, with the handler, the
  dependency field and the operator-facing surface — the events are
  `diag.disclosure*` and the error text a person reads says disclosure, which is
  what the rename was for. A slug is a stored value and renaming one is normally
  a migration; this one has never been in a release, an unknown slug is skipped
  rather than fatal, and the cost of a row queued on a development machine
  before the rename is one artefact that stays unmarked until something
  re-fingerprints it.

  The signed assertion label and its payload tag are
  `io.github.ynishi.asterism.disclosure` and `asterism.disclosure/1`. Renaming
  an identifier inside a tamper-evident document is normally the one thing that
  cannot be undone — but nothing has ever been signed, because no build has a
  certificate to sign with, so there is no file to stay compatible with. The
  version stays `/1`: nothing has read shape 1 under the old name.

- **`just check` says when the committed doc artifacts went unchecked** (#25) —
  `aidoc-check` needs a nightly toolchain this workspace does not pin, so it sat
  outside the gate, and a change that deleted a crate left `docs/aidoc/`
  describing it while `just check` went green. The new `aidoc-guard` step runs
  the check when it can and fails on drift as before; when the toolchain or
  `cargo-aidoc` is missing it prints what is missing and continues. A gate
  nobody is told they skipped is not a gate. The prerequisites are in the
  README, and CI installs them — a step that warns on every run is a step nobody
  reads, so on the one machine that runs `check` for every change, drift is red
  rather than a log line.

- **The row records what became of an artefact's disclosure** (#14) — stamping
  wrote a mark into a file and said so in a log line, leaving the library unable
  to answer which artefacts carry one. A mark lives in the file's bytes and a
  downstream conversion strips it, so the row is the only place the answer
  survives, and it is what a re-apply would be decided from. The note lands
  under `extra._trace.disclosure`, beside the declared-hash verdict already
  there — which generalised the narrow write those notes need:
  `note_declared_hash` becomes `note_trace_field`, one transaction per key
  rather than a near-copy of the method per key.

  What the note holds is `Stamped`'s own account of itself, so that "no
  certificate was configured" and "the certificate stopped working" stay apart
  in the row as they do in the type. A failed note changes nothing — the mark is
  already in the file or already not.

- **What a dispatch produces is written with its AI disclosure** (#14) — the
  writer landed with nothing calling it; this calls it. Not where the work was
  planned to call it from: stamping immediately after `reify` reads an evidence
  set that does not exist yet, because `reify` builds the material from the
  exporter's string and enqueues the hashing that fills `meta_kv` in. That
  version compiled, passed the existing dispatch fixtures, and marked no file at
  all. So the order is a chain — `MaterialHash` enqueues a new `ProvenanceStamp`
  job once the fingerprint lands, which is the first moment there is anything to
  disclose.

  Its own job kind rather than a mode of the hashing one: hashing reads bytes
  and writes a column, stamping rewrites the user's file, and the two want
  different retry policies. A stamp that fails leaves an artefact that exists
  and is unmarked, so the handler returns `Ok` on every outcome and reports
  which halves landed rather than failing a completed export over metadata.

  **Only artefacts a dispatch produced are stamped.** Stamping rewrites bytes,
  and doing that to a file somebody imported would be this application editing
  something it was asked to index and not to touch. The dispatch trace separates
  the two, `_dispatch` becomes a named constant now that both sides depend on
  the spelling, and the check is a pure function pinned by tests over every
  shape an imported asset's `extra` can take.

  The composition root builds the service unsigned and with the prompt withheld
  — the two documented answers — and an unwired build skips rather than fails.

- **`just rust-test-pkg <crate>…`, and a written answer to when the workspace
  run is worth its cost** (#47) — the contributor docs ruled out a hand-rolled
  `cargo test --workspace` without saying when the sanctioned full run is called
  for, so the narrow run everybody actually wants had no recipe and got
  hand-rolled instead. `rust-test` links every test binary at once — one linker
  process each, gigabytes resident each, as many at a time as `jobs` allows —
  which on a shared or memory-tight machine is enough to push the box into swap
  and take down whatever else is running on it. The new recipe runs the named
  packages with `--no-fail-fast` and returns non-zero if any of them failed; it
  keeps no log and counts no binaries, because those exist to make a workspace
  run auditable and a two-crate run is read in the terminal. `CONTRIBUTING.md`
  now says which of the two to reach for, and that opening a pull request does
  not wait on a full local run — CI runs `just check` on every push, so the full
  result reaches the PR either way. `just check` is unchanged: the gate still
  means the whole suite.

- **Recall by meaning — a picture's words become the body search reads** (#32).
  "What does search see" had one answer: the bytes of the original, when those
  bytes were text. A transcript was findable and a picture was not — even though
  the library already held sentences about it, a title somebody typed, the alt
  text an importer lifted out of the page, the keywords the auto-tag pass wrote,
  the generation prompt sitting in a PNG chunk, a note left in the comment
  thread. None of them reached the index, so the honest description of the
  search surface was "text files only" for a library that is mostly pictures.

  The fix is not a new store: it stops treating "the body" as a synonym for "the
  file's bytes". `domain::derived_text::derive_text` is a pure function that
  composes the projection — file body, declared meta, recovered embedded text,
  comment threads — and it lives in the domain because the rule for what is
  searchable about an asset is a statement about assets, not about a queue. What
  it leaves out is written down with its reasons: the `_trace` bag apart from
  `meta` (bookkeeping is not words about the subject), identifiers (a UUID is
  not a word), and tags — the one exclusion that is a judgement, since a tag is
  the precise instrument full text deliberately is not.

  `domain::embedded_text` recovers the words a container wrote into an artefact,
  and it is not `material_meta`: that module defines a digest and has to stay
  frozen and total, this one defines a document nothing compares for equality,
  so it can be generous where the digest cannot. `zTXt` and `iTXt` are read
  rather than skipped, `tEXt` bytes are tried as UTF-8 and re-read as Latin-1
  when that fails instead of being run through a lossy replacement that shreds
  the accents in `Café`, and a file that never reaches `IEND` keeps the caption
  it already yielded. PNG only, capped at a mebibyte per artefact, walked over
  the buffer `fingerprint::hash_artefact` is already holding — nothing on the
  indexing path opens a file for this.

  Two columns carry it (migration V81). `material.meta_text` holds the recovered
  text, written by the fingerprint pass, where `NULL` means "nobody has looked"
  and `'{}'` is an answer only a walk may give; `asset_body.derived_version`
  stamps which `COMPOSITION_VERSION` composed a cached body, so raising the
  constant re-composes the library without re-reading a single source file.
  Neither column is backfilled by the migration: `JobKind::MaterialText` walks
  the `IS NULL` set for recovery, and `scan_stale_body` re-composes the bodies
  that predate derivation.

  A fold moves text onto the keeper — the headstone's keywords, its labels, the
  comment thread that followed it — so both entry points now re-compose it. The
  `asset_fold` job does that for the pair duplicate detection raised, and also
  deletes the headstone's cached body, since a body left behind is what a
  Tantivy rebuild would read the retired row back in from. A manual merge folds
  inside its own transaction, so the job it enqueues arrives to find the work
  done and takes the refusal branch, which never sees a keeper absorb anything —
  `merge_assets` therefore unindexes the folded rows and enqueues the keeper's
  re-composition itself, once for the ruling rather than once per folded row.
  Every other verb that writes a section of the document (a rename, a declared
  statement, a cover, the auto-tag pass, a hashing pass that recovered metadata)
  re-composes it too, and falls back to clearing the composition stamp when the
  queue will not take the job — the backfill walk only sees rows composed by an
  older reading, so a stamped row it could not queue would otherwise keep a
  stale document until somebody edited it again.

- **Issue labels.** `CONTRIBUTING.md` names four categories — `bug`,
  `enhancement`, `refactor`, `chore` — and asks for at least one on every issue.
  `refactor` and `chore` are new to this repository.

### Deprecated

- **`ProjectService` and `PursuitService`** (#63). They serve the shapes the new
  forge model replaces: a project grouping lines inside the forge, and a
  membership ledger over assets. Nothing new is added to either.

### Changed

- **A round is a core thing again, and the boundary says what the forge claims**
  (#81). An exporter invocation is a call that was made and a record of what
  came back — a lifecycle, the columns a runner resumes from, and no opinion
  about why anyone wanted it. It had been filed under the forge as a passenger
  of the commit that gave that layer its name, whose subject was the pursuit
  sitting flat among forty-six modules and which never argued that a dispatch
  carries intent. `dispatch` is a core module again, byte for byte the same file
  at a different path. What it buys is that the dispatch port names no forge
  type, so the asset service turning a `derived_from: dispatch:<id>` claim into
  the assets that dispatch produced is core work through a core port rather than
  the core reaching up a layer — and resolving a provenance claim was always
  core work. Three doc passages disagreed once the module moved, and they were
  one question: what the forge claims on a round, and where the minting rule
  binds. Both answers are now written in the same words everywhere they appear —
  the claim is the stamp naming the pursuit a round was filed under, and the
  rule binds at the application layer on a forge verb, because the job type is
  complete with that stamp unset. Doctrine 6 also stops saying "the identity
  question and the outbound one", which named no module and could be read as
  either of two; it names all three instead.

- **Dev builds keep line tables and drop variable DWARF.** The default dev
  profile linked every e2e binary at about 0.8 GB, most of it variable and type
  debuginfo nothing here reads: failures are read from test output and panic
  backtraces, which need file:line only. `[profile.dev]` now sets
  `debug = "line-tables-only"`, shrinking both the binaries and the rlibs
  feeding each link — before this, one `rust-test-changed` over a core-touching
  branch wrote 45 GB of `target/` on a shared 31 GB machine and saturated its
  disk. Backtraces still resolve to file:line; a session that does want debugger
  variables overrides locally with `CARGO_PROFILE_DEV_DEBUG=2` without touching
  the tree. (#85)

- **A dispatch carries the pursuit its caller named, and nothing when it named
  none.** Always-mint made every start verb open a `Pursuit` row: supply no
  `pursuit_id` and the server minted one for you, so exporting a selection — a
  catalogue capability, and the one thing the forge is not required for — could
  not happen without a forge object being written. `DispatchService` is a raw
  dispatcher again. It stamps what the caller supplied, leaves the stamp `None`
  where nobody supplied one, and names no forge type at all: the wire field
  stays `pursuit_id` and parses to `CorrelationId`, an id of the raw layer's own
  — which the stamp's own removal, further down, deletes in turn. `redispatch`
  still inherits the prior round's stamp, because naming the prior round is
  itself explicit, and inherits nothing where the prior carried nothing.

  The existence check went with it, which is the part worth stating plainly: a
  supplied id is no longer read at all, so nothing here refuses an id that names
  no live pursuit or one belonging to another persona. That check is about a
  forge object, and answering it from a catalogue verb is what coupled the two
  in the first place; the forge-side export path where it belongs does not exist
  yet, and `restamp` is what corrects a filing until it does. Existing stamped
  rows are untouched — the V79 backfill stands, and a `pursuit` row with no
  project is still residue of the retired rule rather than a mode.

- **The six doctrines are gone, and `dispatch` is a raw-layer module again.**
  `domain::mod`'s doctrine list was a second copy of reasoning that already
  lived next to the types, and it had drifted: read against those types, four of
  the six were contradicted by them, and the boundary the sixth declared was
  broken in eight files. Five modules cited it by number, which made the drift
  read as authority. Nothing is lost by deleting it — `attribution.rs`,
  `snapshot.rs`, `edge.rs` and `forge/pursuit.rs` each state their own rule
  where it is enforced, and the citations now point there. The rewrite of
  doctrine 6 that a later entry here records — naming its three modules instead
  of describing two of them — goes the same way: this deletes what that
  corrected.

  What stands in its place is the one rule a module doc is the right home for:
  the forge uses catalogue types, the catalogue uses a forge id and nothing
  else. It is written with the state of the tree beside it rather than as an
  assertion, because asserting a boundary the dependency graph does not have is
  the defect #81 was opened about. By the end of this entry no file outside the
  forge uses a forge type — and nothing enforces that, which the doc says in as
  many words: the next `use` restores the dependency and no gate notices. The
  crate split is what would make it a rule rather than a fact.

  Applying that rule moved two things. `dispatch` is a catalogue module: an
  exporter running over a frozen set is something that happened to the bytes, it
  runs with no pursuit in sight, and deleting the forge leaves it working while
  deleting it takes the ability to send anything out. And the claim that the
  actor triple is a forge property is withdrawn — `Asset` carries the same
  triple, so `attribution` is a catalogue module the forge uses, and moving it
  across during the crate split would have turned the arrow around. The three
  forge services are no longer re-exported from `application`'s root; callers
  name `application::forge::` and the grouping shows at the use site.

  Two moves follow the same rule further. The forge's ids leave `domain::value`:
  ten of the eleven are named nowhere outside the forge, and the eleventh is the
  dispatch stamp, which `DispatchJob` now carries as an opaque `CorrelationId`
  the forge converts at its own boundary. The field, the column and the sidecar
  key stay `pursuit_id` — three names for one value would buy nothing, and the
  name was never the coupling. And the forge's two persistence ports leave
  `domain::repository` for `domain::forge::repository`, which leaves the raw
  layer's central service holding a `bool`: `CorrelationResolver` answers
  whether a returning artefact's stamp names anything live in its persona, which
  is all ingest ever asked of a pursuit. Both that resolver and the stamp it
  answered for are deleted further down, so neither ships.

  The last crossing was a write, and it moved rather than being translated. The
  dispatch runner used to append one ledger row per output it reified, which put
  a catalogue service on the forge's write port; it now enqueues
  `pursuit_ledger_file` with the dispatch id, and the forge does the writing.
  Everything the filing needs is a column of the row by then, and the write was
  never atomic with the reify anyway — the dispatch was already saved as `Done`
  before the first row. **What the move adds is a window.** A close landing
  before the job runs freezes its candidate set without those outputs, and that
  is permanent; the queue has no retry policy and this kind has no backfill, so
  a filing lost that way stays lost. The window closed a different way: the job
  kind, its handler and the enqueue are deleted further down, and nothing on
  this branch files a ledger at all.

  None of this is enforceable yet, and the doc says so rather than implying
  otherwise. Nothing stops an implementation of that `bool` port from being
  three lines over the forge port it replaced; what a narrow port buys is that
  doing so is a visible choice at a wiring site instead of the default shape of
  the service. Cutting `asterism-forge` into its own crate is what would move
  the refusal into the compiler.

- **The word `catalogue` is gone from the crates.** It came from the six deleted
  design claims, where it named the half that is not the forge; the claims went
  and the word stayed, naming nothing that was ever defined. The layer it
  pointed at is the raw layer, and that is what the prose says now, or the
  concrete module where a sentence was really about `dispatch` or about an asset
  row. Every mention was a doc comment, a code comment or a test fixture — no
  type, column or wire key was ever named after it, and the one string a caller
  reads (the MCP ingest description's example of an outside identifier) is now
  an edition number.

  No mention stays anywhere in the crates. `asterism-importer-sdk` keeps its own
  `catalogue` module, which is an unrelated type — a list of import targets, not
  a name for the store.

- **The forge has a name in the tree, and the boundary it keeps is written
  down.** `pursuit` sat beside `tag` and `group` as one module among forty-six,
  and the flatness cost something specific: the design that introduced it said
  the core/forge split "already exists in the codebase as doctrine 2", when
  doctrine 2 is about edges versus the fold and conflict rulings — all of which
  are core. Nothing in the tree stated the split, so the next step reached for
  the core's own shape to answer a forge question, and proposed recording "this
  one is better" through the fold that means "these are the same thing" (#22).

  `pursuit` now lives under `domain::forge` with its service under
  `application::forge`, and that module's doc states the loop (rounds out and
  in, culling between them, the close that lands the kept set) and the contract:
  intent lives only in the forge, the core is complete without it — importing,
  deduplicating and rating need no pursuit — and what the forge writes onto a
  core row is a correlation id and nothing else. That last clause is stated with
  its exceptions rather than as an absolute, because the forge does write on
  core rows: the `_dispatch` stamp a reified output carries and the `_trace`
  claim a returning artefact brings back are both ids on `asset.extra`, and they
  are how the two layers rejoin after a round trip. What does not go there is a
  verdict. Doctrine 6 says the same in a paragraph.

  Culling is named in the loop and has no record of its own yet; the module doc
  says so rather than leaving the gap for a reader to discover. The record's
  shape — keep or reject, out of which candidate set, written just before the
  close — is drafted on #63, and #22 carries its implementation.

  What stays in the core is as much of the point as what moved. `snapshot` is
  the handle the forge holds the core by; `duplicate_conflict` answers identity,
  which the store asks of itself; `provenance` is what a returning artefact
  declares about where it came from — a claim the exporter writes and ingest
  resolves, with nobody deciding anything, which is why it sits low and runs
  whether or not anybody is pursuing anything.

  No behaviour, no schema, no wire change. Four module paths did move:
  `asterism_core::domain::{pursuit, dispatch}` and
  `asterism_core::application::{pursuit_service, dispatch_service}` are now
  under `forge`, and no compatibility re-export was left behind — two paths to
  one module is the ambiguity this change exists to remove. The service types
  keep their old names through `application`'s re-exports.

- **A pull request runs the tests its own diff calls for; `main` runs them
  all.** CI ran the full workspace suite on every push — every crate's test
  binaries linked, one linker process each — to answer a question about however
  many crates a branch actually touched. Pull request #60 touched two.
  `pre-push` had been taught to scale with the diff; CI had not, which left the
  local gate and the hosted one disagreeing about what a pull request is for.

  A pull request now runs `check-changed`: the same list as `check` with the two
  workspace-wide gates swapped for their `-changed` counterparts. A branch that
  edits one crate tests one crate, and a branch that edits no crate — a
  `Justfile` change, a workflow change — links no test binary and runs no lint.
  Not "no Rust": `bindings-check` still compiles `asterism-ui` and most of the
  workspace behind it. What goes away is the linking, which is where the load
  is. `main` still runs `check` in full on every push.

  A change no single crate owns — the root manifest, the lockfile, the
  toolchain, `fixtures/`, `scripts/` — has no narrow run to make, and both gates
  then run the full recipe when `CI` is set. That branch is the reason this is
  not simply "run less": deferring to CI is not something CI can do, and
  `Cargo.lock` is in that set, so every dependency bump would otherwise have
  tested nothing and reported green.

  What this gives up is a regression in a crate the branch did not edit: a
  dependent broken without being touched. `main`'s own run catches it, one merge
  later than before. That is the trade, and it is stated in the recipe rather
  than left to be inferred — the alternative is every pull request linking every
  test binary in the workspace to find the case that is rare.

  The workflow now checks out full history, because the answer comes from the
  merge base with `origin/main` and the default checkout is a single commit with
  neither. The failure that would otherwise follow is the reason
  `changed-packages` gained a guard: a diff against a ref that does not resolve
  produces no paths, which reads exactly like a branch that changed no crate, so
  CI would have compiled nothing and called itself green. It now refuses and
  names the setting instead — verified by running it where `origin/main` does
  not resolve.

- **A change that only edits prose no longer runs the build.** Pull request #57
  changed eighteen lines across `CONTRIBUTING.md`, `CHANGELOG.md` and
  `.claude/CLAUDE.md`, and bought the workspace test suite, clippy over every
  crate and a rustdoc pass over all of them — 710s of macOS runner, plus a
  second run of the same on `main` behind the merge. The workflow now carries a
  `paths-ignore` list of what a change can touch without changing what
  `just check` would answer: the changelog, the contributing and disclosure
  documents, the readme, the security policy, the two licences, and `.claude/`.

  GitHub skips only when _every_ changed path matches, so a branch that touches
  a crate and the changelog still runs in full — the list never has to be a
  judgement about which mixed changes are safe. Five things are deliberately not
  on it: `.github/` (a change to the workflow has to be answered by the
  workflow), the `Justfile` (it is the definition of green), `docs/aidoc/`
  (generated, but `aidoc-guard` checks it, and a hand-edit there is what that
  gate is for), `fixtures/` (the collation corpus, README included), and
  `.gitignore` (it decides what is tracked, which two steps of the job read).

  This narrows CI rather than bringing it into line with `pre-push`. On a
  prose-only branch `pre-push` still runs the formatting check, the bindings
  check, the three frontend recipes and `aidoc-guard`; CI now runs none of them.
  What the two agree about is the workspace suite.

  Two costs. A skipped workflow produces no run object, so a skipped commit
  cannot be asked for a verdict afterwards — `workflow_dispatch` is added for
  that, and it matters because the `main` run is the verdict on a merged tree
  and not only an answer about the diff that triggered it. And once a check is
  _required_, the requirement is what turns a skip into a pending check that a
  prose-only pull request can never satisfy; the documented remedy is a
  companion workflow of the same name with the inverse filter, which is also
  what the skip keyword will need, so both would have to be settled together.

- **CI answers a pull request in one run instead of two and a manual click.**
  The regeneration step pushes `docs/aidoc/` back to the branch under test, and
  the comment here used to say that a `GITHUB_TOKEN` push starts no workflow
  run. It does. On pull request #50 that push created a second run which GitHub
  parked as `action_required`: a human clicked "Approve and run" five minutes
  later, the approved run spent eleven minutes repeating settled work, and the
  concurrency group cancelled the original nine minutes in — after it had
  pushed, and midway through the `just check` it existed to run. The bot commit
  now carries `[skip ci]`, so the run that regenerates the artifacts is the run
  that judges them, which is what the arrangement always claimed to be.

  Two consequences are written down beside it rather than left to be discovered.
  The head commit carries no check of its own, and a skipped workflow's checks
  stay _pending_ rather than absent, so a future required check would block a
  merge here rather than quietly pass it. And the token can reach `main`: a
  squash merge concatenates the branch's commit messages into the squash commit,
  and a rebase replays the bot's commit as it stands, so either lands
  `[skip ci]` on `main`'s head and skips the run that gives a merged tree its
  verdict. A merge commit takes its message from the pull request title and does
  not carry it, which is why the merge button is restricted to that method.

  Whether a merge commit was _enough_ was open when this was written, because
  GitHub says the token skips a workflow when it appears "in a push" without
  saying which commit of one. It is settled now, and by accident: three of the
  commits merged here quote the literal token in their prose, so under the broad
  reading `main` would have got no run after the merge. It got run 31914528059.
  The scan reads the head commit of a push, not the range behind it.

  Two further costs, and one saving. `aidoc-guard` no longer re-checks, inside
  `just check`, the artifacts the same job regenerated minutes earlier — it
  cannot report drift against a regeneration of the same tree, and it spent a
  second rustdoc pass over the workspace saying so. That skip is only sound
  because `just aidoc` now runs `--strict`, so the doc lints the guard enforces
  are met by the tool that writes the artifacts rather than by a gate downstream
  of it. The cost of that move is where a lint failure now lands: at the first
  step of the job rather than the last member of `check`, so a missing module
  doc now masks the result of clippy, the tests and the UI gates for that run —
  and `just aidoc` is now a recipe that can exit non-zero. Locally it means the
  same lint is reported while the author is still looking at the module. CI
  builds also carry `line-tables-only` debug info: panic messages and backtraces
  still name file and line, and nothing on a runner opens a debugger. The saving
  is the dependency cache moving ahead of the `cargo-aidoc` install — the tool
  ships no release binaries, so it is built from source every run, and from
  there it is cached like any other dependency.

- **CI commits a regenerated doc artifact that no tracked file announces.** The
  regeneration step compared with `git diff` _before_ staging, so a difference
  visible only in an untracked or deleted path — a committed artifact removed by
  hand while the module it describes still exists, which regeneration then
  recreates — left the tree looking unchanged. The step printed "already
  current", pushed nothing, and the artifact never returned to the repository.
  It stages first and compares the index against `HEAD`, which is the question
  it meant to ask. (An added crate or module was never the missed case: those
  also modify a tracked index page.)

- **CONTRIBUTING forbids writing a CI skip keyword in a commit message or a pull
  request title.** GitHub reads those keywords anywhere in a message and does
  not distinguish writing about one from asking for one. A `pull_request` run
  reads the branch's head commit, so a branch whose tip discusses the mechanism
  gets no CI; a pull request title becomes the merge commit's body, so the same
  applies after a merge. Neither failure is visible — a skipped workflow leaves
  its checks at _pending_ rather than failing, so nothing turns red and nothing
  is absent from the list. The rule covers messages and titles only; file
  contents quote the keywords freely, as the workflow comment and CONTRIBUTING
  itself do.

- **The gate before a hand-over costs what the change costs, not what the
  workspace costs.** `just pre-push` was `branch-check` plus `check`, and
  `check` reaches two gates that scale with the repository rather than the diff:
  `rust-test`, which links every test binary in the workspace at once — one
  linker process each, gigabytes resident each — and `rust-clippy`, which
  compiles every target in every crate. On 2026-08-15 both were run on a shared
  machine for a branch whose entire diff was a workflow file and two comments.

  `pre-push` is now `branch-check` plus `check-shared` plus
  `rust-clippy-changed` and `rust-test-changed`. Both narrow gates call a new
  `changed-packages`, which reads the paths the branch changed against
  `origin/main` plus anything uncommitted and maps them to the workspace members
  that own them; it prints `--workspace` instead of a list when the change
  reaches the root manifest, the lockfile or the toolchain, and both callers
  then say there is no narrow run to make rather than quietly starting the run
  they exist to avoid. `check-shared` holds the gates whose cost does not move
  with the workspace — formatting, the bindings check, the three frontend
  recipes — plus `aidoc-guard`, which does read the whole tree but cannot be
  narrowed by package, since the artifacts it checks are one inventory of all of
  it.

  `check` is unchanged and still means the workspace-wide pair: it is CI's entry
  point, and CI is a runner nobody else is sitting on. What the narrow gates
  give up is the crates a change did not edit — they cover what was edited, not
  what depends on it — and CI reports those on the same push. `rust-test` stays
  as the one sanctioned way to run the suite when it is genuinely wanted, now
  says on the way in what it is about to cost, and nothing depends on it any
  more except `check`.

- **A disclosure's two halves report their own outcome, so one failing no longer
  cancels the other** (#14) — applying a record writes an IPTC/XMP packet and
  signs a C2PA manifest, and the writer has argued from the start that the two
  fail independently. It did not behave that way. Every failure inside the
  signing block returned early while the packet was still in memory, so a
  signing error threw it away: on the day a certificate expires — which the
  module's own docs call the failure every signing deployment eventually meets —
  exports would have stopped carrying the IPTC half, the one that needs no
  certificate at all. The mirror case cost the manifest: a packet too large for
  a JPEG segment even after the reduction failed the whole call.

  The cause was the return type. `Result<Stamped, _>` made the error channel
  total while the operation is composite, and an `Err` has nowhere to carry the
  half that succeeded. `Stamped` now holds a `Half` per side — `Written`,
  `Skipped(reason)` or `Failed(cause)` — and `Err` is reserved for the case
  where nothing could be attempted: the file cannot be read, or its container is
  not one this build writes into. Whether a failed half makes a failed export is
  the caller's judgement, and it now has both facts to make it with.

  Three states rather than a boolean, for the reason the digest axes already
  have three: "no packet" was at least four different answers, and a video that
  cannot carry one, a build with no certificate configured, and a certificate
  that stopped working all reported the same `false`. `Skipped` names the ones
  that are not faults.

- **Disclosing the prompt is a decision somebody makes, not a constant** (#14) —
  `DisclosureRecord::with_prompt` says the prompt is "a decision the service
  makes, not a property of the data" and that it "cannot be taken back out of a
  file already published"; the service made no decision, filling the field
  whenever the evidence had one, with nowhere to state a different policy. What
  the field receives is the whole AUTOMATIC1111 `parameters` blob — prompt,
  negative prompt, sampler, seed, checkpoint name and hash, and the name and
  hash of every LoRA — so a locally trained model named after a person or a
  client went into every published copy. `record_for` now takes a
  `PromptDisclosure` (`Withhold` / `Embed`) and `DisclosureService` takes one at
  construction. No `Default` and no default chosen: it belongs to the
  composition root, and the asymmetry that should decide it is the one the
  module already applies to terms — withholding can be undone by re-applying,
  publishing cannot be undone at all.

- **A stamp is staged in a temporary nothing can predict, and keeps the file's
  own permissions** (#14) — the rewrite went through a deterministic sibling
  (`shot.png.c2pa-partial`), opened with neither `O_EXCL` nor `O_NOFOLLOW`, at
  whatever the umask gave. An export directory is wherever the user pointed the
  export — possibly shared, synced or watched — so anything else able to create
  a file there could place a symlink at that name and have the stamp write the
  asset through it; two concurrent applies to one path shared the temporary and
  interleaved; and the staged copy of the whole asset was world-readable for as
  long as signing took. `tempfile` becomes a real dependency and supplies all
  three of a random name, `O_EXCL` and mode 0600, with the target's own
  permissions copied across before the rename so that stamping is not also a
  permission change. Still absent: an `fsync` before the rename.

- **Two non-characters XML cannot hold are dropped from the packet** (#14) — the
  filter took the C0 controls and stopped, but XML 1.0's `Char` production also
  excludes U+FFFE and U+FFFF, which cannot be written even as numeric
  references. They are reachable: a PNG text chunk is decoded leniently, so a
  valid encoding of U+FFFF passes through into the prompt and into the packet,
  and nothing noticed — the packet is read back as text rather than parsed, so
  the write reported an XMP half that had landed while the file carried an
  unreadable metadata block. The neighbouring non-characters are legal and stay.

- **The workspace says what its digests actually rest on, and the signed
  manifest stops claiming a version it does not have** (#14) — the
  `preserve_order` comment in the workspace `Cargo.toml` asserted two things
  that are not true. It said `c2pa` requires the feature: `c2pa` does declare it
  in its own manifest, which is why it is unconditionally on, but it does not
  depend on the semantic — verification re-hashes the bytes read out of the file
  rather than re-serialising a parsed model, and the default assertion kind
  routes through a CBOR map that sorts, discarding the author's key order before
  anything is encoded. And it said the digests were safe because they are built
  from a struct and never parsed, which is not a discharge at all:
  `serde_json::to_value` produces a `Map` too, so a value that never met a
  parser is still an `IndexMap` under the feature.

  The comment now names the four stored forms that are hashed or compared byte
  for byte — `material_meta::render`, `series::render`,
  `source_locator::to_storage` and `snapshot_hash` — with what makes each one
  independent of the line, since the reasons differ and only one of them is "it
  sorts". It also states why the line is there at all: what this workspace
  writes back out is somebody else's document, and handing it back with the keys
  re-sorted is an edit nobody asked for.

  `domain::content_hash` gains the rule a digest added beside them owes. A
  digest either **selects** bytes the artefact already carries or **re-renders**
  them, and it has to say which, because the two fail in opposite directions:
  re-rendering too widely reports two different artefacts as one and duplicate
  resolution folds them, while selecting too narrowly only misses a match.
  Re-rendering additionally owes its canonical form in full — naming a published
  scheme is not enough, as the rules for numbers and for duplicate keys are what
  decide the answers — and a versioned tag, because a shipped definition cannot
  be edited without changing what every value stored under it meant.

  Neither of the two disclosures claims a build version any more.
  `claim_generator_info` carries a name and nothing else — the specification
  requires only the name, every crate here inherits the same `0.0.0`, and a
  version string identical on every build ever made tells a reader nothing while
  sounding like it tells them something, in a document that cannot be corrected
  after signing. The XMP packet's `x:xmptk` drops it too, for that reason and
  one more: those bytes go inside the C2PA hard binding, and the toolkit string
  was the only thing in the packet not read off the record, so a version bump
  re-rendered an unchanged record into different bytes. The module doc had
  already promised the packet is a function of the record and nothing else; now
  it is.

  Both tests were weaker than they read. The manifest one compared the emitted
  field against the same `env!("CARGO_PKG_VERSION")` the code used, so it passed
  at any value; it now asserts the field is absent. The packet one compared two
  renderings inside one build, which cannot see a difference that moves both
  sides together; it now pins the toolkit attribute literally.

- **The series key no longer borrows its canonical form from a dependency**
  (#14) — `series::render` hashes a `serde_json::Value` parsed out of a
  container, and was taking its nested key order from whichever map type
  `serde_json`'s feature flags selected. A new `series::canonical_value` sorts
  every object's keys recursively before the bytes are rendered, so the digest
  is a function of the document rather than of how it was typed: a JSON object
  is an unordered collection (RFC 8259), and two containers carrying the same
  fields in a different order carry the same document. Arrays keep their order,
  since a JSON array _is_ ordered. Byte output is unchanged and no stored key
  moves.

  `serde_json`'s `preserve_order` is now declared in the workspace `Cargo.toml`
  with its reasoning rather than arriving as a side effect of `c2pa`. The old
  test that asserted sorted output and warned in prose that this rested on a
  default has a sibling asserting the property itself, plus the negative case;
  both fail by name if the sort is removed, which is the point — the function
  reads like a no-op and will invite deletion.

- **The dispatch stamp stops being a foreign key** (#81).
  `dispatch_job.pursuit_id` has referenced `pursuit(id)` with
  `ON DELETE RESTRICT` since V79, and it was the one foreign key anywhere in the
  schema pointing from a catalogue table into a forge one — every other
  reference to `pursuit`, `project`, `line` and `cull` is forge-internal. So a
  boundary the rest of this section describes as a fact about the tree was
  contradicted by the database: drop the forge's tables and the catalogue's own
  schema stops standing up.

  V87 rebuilds `dispatch_job` without it. The column survives with its name, its
  type, its rows and its index, because what it records — which pursuit this
  round was filed under — is a fact about the round, and facts about a round are
  what the table is for; what the constraint added on top was an ownership claim
  the forge does not have. Deleting a pursuit that dispatches still name now
  succeeds, and those rows are left alone, each keeping its stamp as a value
  that resolves to nothing. Nothing rewrites them to NULL: "filed under a
  pursuit that has since been deleted" and "never filed" are different
  histories. Nothing refuses a stamp naming no row on the way in either — the
  existence wall the previous entry left one layer down is gone too, which the
  `DispatchService` module doc now says rather than claiming otherwise.

  SQLite cannot drop a constraint, so this is a table rebuild, and the
  twenty-five columns are named on both sides rather than copied with `SELECT *`
  — a column-order surprise over a list that long should fail loudly instead of
  transposing data. All four indexes are recreated, not the two the change is
  about. The persona purge is unchanged: `dispatch_job` still leads, now for its
  snapshot edge alone.

### Removed

- **The base-event pin — the version claim a targeted `in` could make** (#63).
  `TxTarget::base_event_id`, the `pursuit_tx.base_event_id` column, the CHECK
  pairing it to a target and the index over it are gone (V91). A pursuit is cut
  from a line and its `in` already names the entry it works on; a claim about
  which version of that entry the caller was looking at is a second statement,
  and nothing was ever built to make it. No command carried one, the single
  production writer hard-codes the target both columns derive from to `None`,
  and no reader ever asked what the column held — so the `Option` was saying
  "nothing fills this yet" rather than stating a model. `TxTarget` survives as a
  one-field struct, `target_entry_id` and its own CHECK are untouched, and
  `PursuitTxKind::from_columns` takes one fewer argument.

  **Nothing on the wire changes.** No command, DTO, HTTP route, MCP tool or
  TypeScript binding ever carried the pin, so a caller cannot tell the
  difference. **No row loses a value either**: every row this codebase has
  written holds NULL here, and `git log -S` finds no revision of the writer that
  did otherwise. A row written by hand against a real profile is the case no
  migration can answer for, and its value is dropped like any other.

  The name stays in two places, both of which are records of a past shape rather
  than claims about today: V85's DDL, which still adds the column because a
  database walking the chain from scratch has to reach the shape V91 alters, and
  V91's own step and test.

- **The cull — the close's record of what it kept and what it dropped** (#22).
  The concept is gone from every layer at once: `domain::forge::cull` and its
  `Cull`, `CullMember`, `CullVerdict`, `RequestedVerdict` and
  `resolve_verdicts`; `CullId`; the `culls_of` and `culls_for_asset` ports, and
  the cull argument of `append_close`, which is now
  `append_close(&self, event: &PursuitEvent)`; `CullDto`, `CullMemberDto`,
  `AssetCullDto`, `CullVerdictEntry`, and the `verdicts` and `cull_note` fields
  of `ClosePursuitCommand`; the `GET /asterism/assets/{id}/culls` route and the
  `asset_culls` MCP tool; and the sentences the `pursuit_close`, `pursuit_view`
  and `pursuit_tx` tool descriptions spent on verdicts. `PursuitViewDto` no
  longer carries `culls`.

  **What a satisfied close now does, and what it no longer says.** A close
  records that a line of work ended, and nothing else. Its `snapshot_id` is
  always `None`, because the kept set it used to freeze was defined as the
  `keep` verdicts and there are no verdicts to define it — no substitute
  selection was invented to fill the gap, and the ledger was not quietly
  promoted into one. So `satisfied` and `abandoned` now differ in what they say
  about how the work ended rather than in what they write. Read a `None`
  `snapshot_id` as "this close froze nothing", not as "this close concluded with
  nothing kept": the second was a decision the old close could record and the
  new one cannot make. Rows written before this change keep the snapshot they
  recorded, and `PursuitEvent::snapshot_id` is still read on the way out for
  their sake — nothing writes it now. What a pursuit was working on stays where
  it always was, in the ledger the close leaves untouched and `pursuit_view`
  still returns. `PursuitService` correspondingly no longer takes a
  `SnapshotService`.

- **The `cull` and `cull_member` tables, and the restamp subject that named
  them** (V88). V82 created both and is released, so it stands as written; V88
  drops them. Leaving them would not have been neutral — both hold `RESTRICT`
  edges into `pursuit`, `persona`, `pursuit_event` and `snapshot`, so rows
  written before this change would go on refusing a persona purge through tables
  nothing in the code can any longer explain, and the purge path would have had
  to keep naming the concept in order to clear it. **This destroys those rows**;
  the candidate snapshots they pointed at remain, unreferenced, as any other
  unreferenced freeze does. `pursuit_restamp` is rebuilt in the same step to
  narrow its `subject_kind` CHECK back to `('dispatch')`: V82 widened it to
  admit a second subject kind that no verb ever minted, so the copy translates
  nothing.

- **Dispatch, out of the forge** (#29). The forge selects an asset the person
  already manages and stages it into a pursuit. That is the whole of it: no
  export, no round, no returning artefact of its own. What went with the round:
  `PursuitService::restamp_dispatch` and `file_dispatch_outputs`, the service's
  `DispatchRepository` port, the `rounds` a `pursuit_view` used to compose;
  `RestampSubject`, `PursuitRestamp` and the `restamp` port — the subject enum
  had one variant, so the verb and its record left with it;
  `RestampDispatchCommand` and `PursuitViewDto.rounds`; the
  `POST /asterism/pursuits/restamp-dispatch` route, the
  `pursuit_restamp_dispatch` MCP tool and Tauri command; and
  `JobKind::PursuitLedgerFile` with the `pursuit_ledger_file` handler, its
  dispatcher arm, the `pursuit_service` cell on `JobDeps`, and the enqueue the
  dispatch runner made after `reify`.

  **`DispatchService` is not deleted.** It was a raw-layer capability filed
  under the forge, and it moves out intact to
  `asterism_core::application::dispatch_service` — same verbs, same behaviour,
  changed import path. Exporting still works and still stamps the pursuit its
  caller names; what no longer happens is the forge asking for anything back.
  The stamp is now written by the catalogue and read by whoever asks what it
  resolves to, and nothing files an export's outputs into a pursuit's ledger. An
  asset enters a pursuit because somebody recorded that it did, through
  `pursuit_tx`.

- **The `pursuit_restamp` table** (V89). With the verb gone the table is
  unreachable — nothing reads it, nothing writes it, and the persona purge no
  longer sweeps it. **This destroys those rows**, which recorded which pursuit a
  round was re-filed under. `dispatch_job.pursuit_id` and its index outlive this
  step, still written on every export; V90 below is what takes them.

- **The pursuit stamp on a dispatch, and the whole lane that resolved it**
  (V90). A dispatch is a raw-layer export — a frozen input, an exporter, an
  action, and what came back — and which line of work somebody was on when they
  started it is not a fact about the export. So `dispatch_job.pursuit_id` and
  `idx_dispatch_pursuit` are gone, with `DispatchJob.pursuit_id`,
  `DispatchDto.pursuit_id`, and the `pursuit_id` field of
  `CreateDispatchCommand`, `DispatchRunCommand` and `RedispatchCommand`. The
  exporter context no longer carries one, so a sidecar's identity block is
  `dispatch_id` and the source id alone: `SIDECAR_PURSUIT_ID_FIELD` is gone from
  the contract.

  The reads built on the stamp go with it. `DispatchRepository::list_rounds` had
  no production caller left; `PursuitRepository::returns_of` did — the pursuit
  view — and its adapter joined through the column it can no longer name, so
  `PursuitViewDto.returns` is gone and a pursuit view is now the row, its
  events, and its ledger. On the ingest side the sidecar's `pursuit_id` claim
  and everything that answered it are deleted in full: the `CorrelationResolver`
  port and its adapter, `parse_correlation_id`,
  `AssetService::resolve_pursuit_claim`, the `_trace.pursuit_id` and
  `_trace.pursuit_resolved` note fields, and the `trace_pursuit_id` generated
  column with its partial index. The re-resolve sweep is back to the one
  question it started with — did this derivation become answerable.

  **This destroys the filing.** Every dispatch row loses which pursuit it was
  started under, and nothing else records it: the restamp table went one step
  earlier, and the `_trace` bag keeps its `pursuit_id` text only because that
  bag is what an ingest recorded rather than what the schema asserts. Nothing
  re-derives the stamp afterwards. Two rebuilds land here rather than one — V87
  took the foreign key off the column three steps ago and this takes the column
  — because folding them would mean renumbering steps that already exist, and
  V87 answers a question of its own that its test still asks. `dispatch_job` is
  rebuilt with every column named on both sides; `asset` gets a plain
  `DROP COLUMN`, since the column is VIRTUAL generated and rebuilding the
  library's largest table to remove an expression would cost the whole table.

### Fixed

- **An unknown parent no longer makes its child a composite** (#23). The fork
  between `trainedAlgorithmicMedia` and `compositeWithTrainedAlgorithmicMedia`
  read a `bool`, so a parent that declared nothing — no metadata, an unreadable
  blob — counted the same as one that declared a camera, and the manifest
  claimed "a model altered material that did not come from one" about a parent
  nobody knows anything about. Parent evidence is now three-valued
  (`ParentOrigin`: declared synthetic, declared non-synthetic, unknown):
  composite requires a positively declared non-model parent, and an unknown
  parent moves nothing — the term stays at what the child's own container
  states. This follows the provenance policy recorded on #23: the app's own
  generation path is known synthetic, imported declarations are trusted as the
  user's responsibility, and everything else is unknown — signed only as what
  the signer explicitly states, never converted into an assertion by a default.

- **The disclosure rewrite is durable, not merely atomic** (#23). The staged
  rewrite — temporary in the target's directory, then a rename — was atomic in
  the namespace and nothing more: no `fsync` before the rename, so on power loss
  between the write and the writeback the name could point at bytes that never
  reached the disk, leaving a short file where the user's original was. That is
  the opposite of what the module advertises, and it was carried on #23 as a
  trade-off because the cost is real on large videos. Decided now: the data is
  fsynced before the rename and the directory after it — the order the teams
  blob store already writes in — both stamp paths funnel through the one
  `commit`, and an fsync that fails is a failure like any other. The two sit on
  different sides of the rename: a data fsync that fails stops the caller with
  the original untouched, while a directory fsync that fails has already
  replaced it — "written, not yet crash-durable" rather than "not written", and
  a re-apply on that error is a repeat, not a loss, since the disclosure is
  derived from rows. No size threshold and no setting, because "whether power
  loss eats your original" is not a property that should depend on either. On
  Apple platforms the syncs mean more than they read: macOS's plain `fsync`
  stops at the drive's volatile cache, so the standard library issues
  `fcntl(F_FULLFSYNC)` for `sync_all` there — a real flush, with no fallback, so
  a volume that cannot promise it (an SMB share, some enclosures) errors and the
  stamp stops, the same stance as every other fsync failure.

- **The stamp says what it withheld, and a stamp racing the fingerprint is
  refused instead of writing an unmarked file** (#23). The last two correctness
  carries from the disclosure writer review. The writer's fallback ladder made
  its decisions and said nothing: the adapter re-derived the prompt's fate by
  substring-searching the rendered XML for a property name, and once the ladder
  grew a bare-mark tier, a withheld system name was invisible outright — a
  packet reduced to the mark alone and a container that never named its
  generator read identically afterwards. `embed::stamp` now returns what it
  wrote and how far it reduced the record to write it, the adapter records it,
  and the disclosure note carries `system_dropped` beside `prompt_dropped` —
  each true only when its field was asked for and withheld. The read-back keeps
  its one remaining job, refusing a packet the reader cannot parse. And the
  service's `record_for` read `meta_kv` alone, whose `None` merged three states
  the storage keeps apart: fingerprint not yet run, probed and this is what it
  holds, and a format no probe reads. The first is a question nobody has asked
  yet, not an answer — a stamp built on it wrote an unmarked file nothing can
  tell afterwards from one with nothing to say. The service now refuses it as a
  state conflict, keyed on `meta_hash` being `NULL`; a marker there stays an
  answer and proceeds with nothing established.

- **`DisclosureRecord` loses its `Default`, and the packet fallback gains a
  bottom that always fits** (#23). Two more carries from the disclosure writer
  review. The derive constructed the one state the type's own documentation
  forbids — a record with an empty `asset_id`, which reaches a signed assertion
  nobody can correct afterwards; `for_asset` now spells its fields out and the
  derive is gone. And `essential()` was not a bounded fallback: it drops the
  prompt but keeps `ai_system`, an unbounded string read out of someone else's
  file, so a large enough one overflowed the JPEG segment a second time and the
  packet half failed outright. The fallback ladder gets its bottom tier,
  `obligation()` — nothing in the packet but the digital source type, whose
  vocabulary is fixed-size URIs and always fits — and `stamp` steps down to it
  when `essential()` still does not fit. A record with no mark keeps the failure
  instead: with the source type absent there is nothing bounded left to write,
  and reporting an empty write as a success would erase the one signal that
  something was withheld. The same fail-direction now holds one tier up — a
  prompt-only record whose prompt overflows used to come back as "nothing to
  disclose", indistinguishable from a record that had nothing to say.

- **The disclosure writer stops labelling every `ftyp` file MP4, and its JPEG
  walk steps over standalone markers** (#23). Two carries from the disclosure
  writer review. The container sniff treated every `ftyp` brand except `qt  ` as
  MP4, so HEIC, AVIF and M4A — same box, different families — signed under a
  declared `video/mp4` the file contradicts. Membership is now read the way the
  box states it: from the major brand when it is an MP4 dialect, from the
  compatible list behind it when the major brand is a vendor's name (Sony's
  `XAVC`), and the families this build does not write into are refused before
  that list is consulted — an M4A routinely declares `isom` compatible, and
  compatibility with a video dialect does not make audio video. A brand list
  naming nothing recognised is refused too: an unsupported-container report
  rather than a signature under a guess. And the JPEG segment walk assumed every
  marker short of `EOI` carries a length field; `TEM`, the restarts and a stray
  `SOI` carry none, so on a file other decoders accept the walk resynchronised
  at whatever offset the two bytes after one implied. It now records the same
  standalone set the media probe's scanner already steps over, and the packet
  insertion point treats them as the non-application markers they are.

- **`changed-packages` answers for the branch's commits, and a script nothing in
  the build reads no longer selects every crate** (#67). Two defects in one
  recipe, and both reach the gates built on it — `rust-test-changed`,
  `rust-clippy-changed`, and through them `check-changed` and `pre-push`.

  **The working tree counted.** The path list was this branch's diff against the
  merge base _unioned with_ `git status --porcelain`, so staged, unstaged and
  untracked files moved the answer. An untracked file under a sentinel path took
  a branch carrying no commits at all to `--workspace`; the branch adding
  `scripts/check-commit-msg.py` did exactly that to itself. A CI checkout is
  clean and always will be, so local and CI answered differently about the same
  commit — which is the one property a pre-push gate cannot have. The working
  tree is no longer read, and a dirty one now says out loud that it is not being
  attributed rather than quietly widening the answer.

  **`scripts/` was a sentinel wholesale.** It is on that list because
  `asterism-infra`'s chapter-scan tests need what `scripts/gen-test-fixtures.py`
  produces — a reason belonging to that file rather than to the directory above
  it. `scripts/check-commit-msg.py` reads commit messages: no crate compiles it,
  no test invokes it, no fixture comes out of it, and a change to it compiled
  all 21 crates and linked every test binary. Exemptions are now named one file
  at a time and the default stays workspace-wide, because the two mistakes are
  not the same size — a build-feeding script left off the list costs a run that
  was too big, one wrongly on it costs a green report from a suite that never
  ran.

- **The profile guard records who owns a home, and can no longer be switched off
  by leaving a variable out** (#56). The `.asterism-profile` marker is the last
  guard against a mistyped launch pointing one profile's build at another's
  data. It failed at that in two ways, and they are one defect: the marker did
  not reliably record who owns a home.

  **An omission disabled it.** `ASTERISM_HOME` without `ASTERISM_PROFILE`
  resolved silently to `custom`, and `custom` writes no marker — so it took no
  ownership. Open a home as `custom` and then as `dev` and both were admitted,
  deterministically, two processes against one database recording different
  environments. That `custom` writes nothing is not the bug; an unguarded
  scratch home is what it is for. Reaching it by forgetting a variable was.
  `custom` is now selected by name (`ASTERISM_PROFILE=custom`, which requires
  `ASTERISM_HOME`), and an explicit home with no profile named is refused with a
  message naming both variables. Nothing in the repository is affected: every
  producer of `ASTERISM_HOME` already passes `ASTERISM_PROFILE` beside it — six
  recipes in the `Justfile`, two scripts in `crates/asterism-ui/package.json`,
  and the WebDriver config — while `asterism-benchgen` removes the variable
  outright and the bench WebDriver config withholds it on purpose. CI sets
  neither.

  Two things follow. `ASTERISM_PROFILE=custom` is now a valid value where it
  used to be rejected, and the resolution table lives in `select_profile`, which
  takes the two variables as arguments so it can be tested without a
  process-wide `set_var`. And `Env::Custom` is no longer what an observation is
  labelled when the profile cannot be resolved at all — that is `Env::Unknown`,
  whose own documentation claims the case; the two had been collapsed into one
  value no reader could separate again.

  **A marker could be read half-written.** Where one is written it was written
  with `std::fs::write` — a create-and-truncate followed by a write, with the
  file existing and empty in between. A second process reading it there got
  `Ok("")`, which is not the profile name, so the guard refused a legitimate
  open with `marker says ""`. Two application instances starting together reach
  it; on 2026-08-16 a workspace CI run reached it too, and failed one test that
  had nothing to do with profiles — the worst shape it takes, since the red
  points at code that is not at fault.

  The contents are now written to a temporary file, flushed, and published under
  the marker's name, so a reader sees the whole file or no file. The publish is
  a `hard_link` rather than a `rename` — rename replaces, and two processes
  opening one home under _different_ profiles both find no marker, so under
  rename both would succeed and the second would erase the first, passing
  exactly the mistyped-launch case the marker exists to refuse. A link fails
  instead, and the loser re-reads and compares. Where the filesystem has no hard
  links — an external volume is an ordinary place to point `$ASTERISM_HOME` — it
  falls back to `create_new`, which keeps the refusal to replace and gives up
  only the atomicity of the contents.

  Three tests. One walks the whole resolution table, including the row that used
  to yield `custom` silently. The other two are a pair, and the second exists
  because the first is not enough: one releases sixteen openers of a single
  profile at a fresh home and requires all of them to succeed, which a `rename`
  implementation also passes; the other releases eight of each of two profiles
  and requires that the marker agree with whichever won and that every opener of
  the other be rejected, which `rename` fails. Both repeat eight times, because
  on the machine this was checked on — by restoring `std::fs::write` and running
  the pair, an experiment the tree does not carry — a single round caught the
  original only two runs in five.

### Added

- **A generated module inventory, and the end of the hand-written one** (#25) —
  `asterism_core::domain`'s module doc hand-enumerated its submodules and had
  gone stale (27 of 42 covered by the time it was caught). The list is replaced
  by a capability tour plus the doctrines code alone misreads (events-not-state,
  facts vs verdicts, freeze-then-refer, attribution's stopping point); the
  inventory itself is generated from each module's opening summary line —
  rustdoc's own index, a one-line grep recorded in the doc, and committed
  cargo-aidoc artifacts under `docs/aidoc/` with `just aidoc` /
  `just aidoc-check` (nightly-only, deliberately outside `check`) turning drift
  into a failing exit code.

- **AI disclosure: the vocabulary, the emitters and the signer** (#14) — what an
  exported file says about how it was made, as values: the IPTC digital source
  type (five terms, closed, refusing anything the vocabulary does not define),
  the XMP packet carrying `Iptc4xmpExt:DigitalSourceType` and the four AI
  properties IPTC added in Photo Metadata Standard 2025.1, that packet written
  into a PNG `iTXt` chunk or a JPEG `APP1` segment as a byte transform, and the
  C2PA manifest definition built from the same record so the two cannot
  disagree. `asterism-infra::disclosure` is the adapter that puts them into a
  file and signs the manifest through `c2pa`, covering MP4 and MOV as well as
  stills — signing after the encode, which is the only point at which it is
  possible.

  Two decisions are worth stating. **XMP is written before the manifest is
  signed**: the hard binding covers the packet, so the reverse order invalidates
  the signature, and a test signs a file, edits its packet and asserts the
  binding then fails. **A signing identity is configuration**: the IPTC/XMP
  disclosure is written with or without one, a manifest only with, and the C2PA
  test certificates are refused by name rather than used as a fallback — a
  manifest signed by them validates as untrusted, which claims a provenance a
  reader rejects.

  `domain::disclosure` is the judgement that feeds them, and it is pure: which
  IPTC term is true of an artefact, given the container metadata a probe stored
  and the `derived_from` edges the library recorded. Terms are asserted on
  evidence something wrote, and an artefact nothing established gets no term
  rather than one meaning "unknown". `compositeWithTrainedAlgorithmicMedia`
  turns on whether a recorded parent is itself synthetic, which the child's own
  metadata cannot say. `application::disclosure_service` does the reads and owns
  the port, looking at no file metadata at all — which is what lets a file that
  came back from a downstream conversion with its manifest stripped be handed to
  `apply_to` and get the same disclosure again.

  Not yet wired to the export path, and not exposed over HTTP or IPC; both are
  the rest of #14. Unsigned video carries no disclosure at all, because the XMP
  half has no BMFF spelling here, and the writer reports that rather than a
  success it did not have.

- **Material layers, and the chapters an import brings in** (#1) — a material
  now carries layers: an origin (`imported` / `user` / `machine`), a role
  (`structure` / `annotation`), a default flag and an order. Chapters declared
  by a container are read by a `ChapterScan` job (the bundled ffmpeg's
  `ffmetadata` output, one parser for every format instead of one per container)
  into an imported structure layer, which re-probing replaces wholesale. A user
  keeps their own chapter set in a separate layer beside the file's and switches
  between them; editing one never alters the other, and the server refuses
  writes into an imported layer. Existing time-based comments become the asset's
  annotation layer via a total backfill (migration V78). The UI's untyped
  `extra.chapters` reader — dead code whose producer never existed — is deleted
  in favour of a typed chapter panel on both the video and audio branches, and
  an empty imported layer ("the file declares no chapters") renders distinctly
  from no layer at all ("never scanned"). MCP gains a read-only
  `material_layers` tool.

- **CI** (`.github/workflows/check.yml`) — `just check` runs on every pull
  request and on push to `main`, so whether the gates pass is something the
  repository states rather than a claim about whoever last ran the recipe. The
  workflow invokes the recipe instead of restating its six gates, so the local
  gate and CI cannot drift apart. One macOS job for now, which is the simple and
  expensive answer; splitting the portable crates onto Linux is a decision left
  to a measurement. `ui-e2e` (needs a real window) and `collation-jsc` (needs
  macOS's `jsc`) stay out, and the workflow says so rather than leaving it to be
  inferred.

- **MCP transport** (`asterism-server`) — the third adapter over the same
  application services. A curated nine-tool vocabulary (`asset_search` /
  `asset_list` / `asset_get` / `asset_add` / `asset_lineage` / `asset_comments`
  / `asset_comment_add` / `catalog_overview` / `dispatch_get`) served over
  streamable-http at `/mcp` on the loopback router (present in both the
  Tauri-embedded server and the standalone binary) and over stdio via
  `asterism-server mcp` (previously a stub). Tool input schemas are generated
  from the `asterism-contract` types that already back HTTP and Tauri IPC (new
  contract feature `json-schema`); domain failures surface as tool-level errors
  carrying the HTTP boundary's `{kind, message}` shape.

- **Local data profiles** — `dev` / `dogfood` / `bench` homes under
  `~/.asterism/profiles/`, each with its own default HTTP port, selected by
  build flavour or `$ASTERISM_PROFILE`. A `.asterism-profile` marker in the home
  prevents opening one profile's data under another.
- **Trash and purge** — trashing is reversible and preserves rating, comments,
  group filing and body text; purge is separate, irreversible, and only
  reachable from the trash. A retention sweep purges what has aged past
  `ASTERISM_TRASH_RETENTION_DAYS`.
- **Full-text search** (`asterism-infra/search`) — a BM25 body index on Tantivy
  with Lindera Japanese morphological analysis and an English Porter stemmer on
  one tokenizer chain, persisted outside the SQLite transaction and
  reconstructed by the `index_rebuild` job after a crash.
- **Import adapters** — Claude Code session logs, tapes, persona journals,
  images, video and audio, all behind one CLI whose environment resolution
  happens in the outer command; media inspection is shared through
  `asterism-media-probe`, and video/audio bundling uses an LGPL-clean ffmpeg
  sidecar built by `scripts/build-ffmpeg-sidecar.sh`.
- **Export adapters** (`asterism-dispatch-sdk` + `asterism-exporter-*`) —
  outbound dispatch to ComfyUI, the filesystem, and arbitrary HTTP endpoints,
  with per-backend parameter schemas.
- **Two-sided sort contract** — the grid comparator (`Intl.Collator`) and its
  Rust port (`icu_collator`) are checked against shared collation fixtures,
  because Query Groups freeze the backend order into `asset_bucket.position` and
  the two halves must agree.
- **Benchmark corpus generator** (`asterism-benchgen`) — a seeded synthetic
  corpus (ChaCha20) where the seed, not the emitted files, is the identity of
  the corpus.
- **Domain layer** (`asterism-core/domain`) — `Persona` and `Asset` aggregates,
  an open-slug `Modality` and `SourceKind`, a `Visibility` model,
  `ConstellationEdge` with a pure `plan_edges` planner, and every repository
  port.
- **Application layer** (`asterism-core/application`) — `PersonaService` and
  `AssetService` with DTO-in / DTO-out APIs, plus the domain ↔ DTO mapping in
  one place.
- **SQLite backend** (`asterism-infra`) — `rusqlite-isle` on the 0.3 release
  line (aligned with `apalis-sql`'s `libsqlite3-sys` cluster); append-only
  migrations gated by `PRAGMA user_version`; schema v1 covering six `STRICT`
  tables (`persona`, `asset`, `tag`, `asset_tag`, `edge`, `thumb_cache`) with
  UUID BLOB keys and unix-epoch-ms timestamps.
- **Job pipeline** (apalis + `apalis-sql`) — `cover_gen` (modality- specific
  heuristic), `auto_tag` (keywords → channel tags), `edge_rebuild` (windowed
  incremental). Column-level partial updates avoid a read-modify-write race;
  `auto_tag` chain-enqueues `edge_rebuild` once the keywords are committed.
- **HTTP API** (`asterism-server`) — axum router bound to loopback, with
  RPC-style routes under `/asterism/*` that mirror the Tauri command surface.
  Clap CLI with `serve` and a placeholder `mcp` subcommand.
- **Contract crate** (`asterism-contract`) — Command / Query / Response DTOs
  derived with `schema-bridge`; TypeScript bindings are regenerated from the
  same source at build time and land in `asterism-ui/src/bindings.ts`.
- **Desktop UI** (`asterism-ui`) — Svelte 5 on Tauri v2: persona sidebar,
  modality tabs, dense grid, hover-burst side panel.
- Workspace scaffolding — `Cargo.toml` metadata, README, and this changelog.

### Fixed

- **The XMP writer does the two things its module doc promises** (#14) — both
  were promised in prose and neither was done, and in both cases the reason
  nothing caught it is that no fixture could reach the shape.

  **A packet another tool left behind is now removed rather than shadowed.** The
  doc's position is that a file must never leave with two packets, because
  readers disagree about which one wins and the failure mode is a stale
  `digitalSourceType` shadowing a corrected one. Both writers recorded only the
  _first_ packet and copied any later one through untouched. The walks collect
  every XMP chunk or segment now, replace the first where it stands — which
  keeps a re-stamped file's chunk order stable — and drop the rest. The test
  that claimed to cover this reached its "twice-stamped" input by calling the
  writer twice, so its input had exactly one packet by construction and it
  re-tested the one-packet path under a name that read like it covered both. The
  new fixtures are hand-built rather than produced by the writer, on the habit
  the neighbouring fixtures already state, and they place an ordinary chunk
  ahead of both packets and another between them — with the packets adjacent,
  neither "the bytes between two packets survive" nor "the survivor stays where
  it was" is observable.

  **A JPEG with no scan keeps its packet inside the image.** The insertion point
  is "before the first non-`APPn` marker", and a metadata-only file that reaches
  `EOI` without meeting one fell back to the end of the file. That put the
  `APP1` _after_ `EOI`, outside the structure, where the module's own reader
  returns `None` while `asterism-infra` records `xmp_written = true`: an export
  that reported success and carried no readable disclosure at all. The walk now
  brings the `EOI` offset back and the packet goes before it, which is the same
  answer the PNG side already gave a file with no `IDAT`. A _truncated_ JPEG is
  a different thing and is still refused as malformed — it has no `EOI` either.

  Both fixes were checked against the behaviour they replace: reverting either
  one fails its new test by name, and the thirteen `embed` tests that predate
  this change pass under both.

- **Five things the provenance writer said about itself that were not so** (#14)
  — all in `asterism-infra`, all found by reading the code against its own
  comments.

  **A failed read-back is no longer reported as a shortened prompt.**
  `Stamped::prompt_dropped` exists to record that a JPEG segment could not hold
  the packet, so the reduced record was written instead — a fact that cannot be
  recovered from the file afterwards, which is why it is reported at all. It was
  derived by reading the stamped bytes back and asking whether the prompt
  survived, through `.ok().flatten()`, which gave the same `true` to three
  different outcomes: the honest fallback, a packet the reader could not find,
  and a file that would not parse. The last two say the writer produced
  something this crate does not recognise, which is a defect rather than a fact
  about the record, and they are errors now — `XmpUnreadable` for the one that
  has no underlying failure to carry. Nothing is known to reach that variant
  today: its one producer was the JPEG writer putting the packet where the
  reader could not see it, fixed above. It is the guard that says the two halves
  have to agree, not a case in the field.

  **A manifest that could not be built no longer blames the certificate.** A
  definition `c2pa` refuses came back as `DisclosureError::Identity`, which
  renders `signing identity: …` — so a mapping defect in this crate sent whoever
  was reading the log to their key configuration. It happens strictly before
  signing, with the certificate already loaded, and now has its own variant.

  **The container sniff survives a short read.** `read_head` used one `read`,
  which is allowed to return fewer bytes than the buffer holds without being at
  end of file. Local regular files do not do this; NFS, SMB and FUSE do, which
  is to say every network-mounted library. Eleven bytes back instead of twelve
  made the `ftyp` test fail and a perfectly good MP4 report as a container this
  build does not write into.

  **The signed output streams to disk instead of being held whole.** The comment
  on the video path said it streams "so a large video is not read into memory
  whole", and the source did — but the destination was a `Cursor<Vec<u8>>`
  collected with `into_inner()`, so a 2 GB MOV was fully resident at the moment
  of collection, and the still path held two buffers at once. `Builder::sign`
  takes any `Write + Read + Seek` destination, so it writes into the sibling
  temporary the rename will move, and `replace` was split into the pieces that
  path needs. The temporary is opened read-write rather than through
  `File::create`, which gives a write-only descriptor: today's signing happens
  not to read its destination, but the `Read` in that bound is there because the
  BMFF handler re-reads box headers to adjust offsets, and relying on it staying
  unexercised would fail on video and nowhere else. What this costs is that the
  temporary is visible for the length of the signing rather than for the length
  of one write.

  **A failed write clears its temporary too.** Splitting `replace` surfaced that
  its own error route returned straight out of a failed `fs::write`, which can
  leave the file created and partly filled — exactly the litter beside a watched
  export directory that the rename path already took care to avoid. Both routes
  clear it now, and a test covers the signing path's failure route rather than
  only its success.

- **When a signing certificate is configured, it is read before it is used**
  (#14) — nothing wires one yet, which is #14's own open item; this is what will
  happen when something does. `inspect_certificate` reports what a certificate's
  extensions say, and `SigningIdentity` refuses on the half of it that means the
  certificate cannot sign at all: an extended key usage naming nothing a claim
  can be signed under, or a CA certificate offering to sign one itself. Neither
  reaches `c2pa` as anything better than a failure later, so it fails here with
  a reason.

  The other half is reported and not acted on. A certificate without
  `c2pa-kp-claimSigning` is not one the Conformance Program's issuance profile
  would have produced, and a subject naming no organisation is one no validator
  can display a signer for — both keep a certificate off a trust list without
  stopping it signing for a reader who has imported it, which is a use the
  specification's own guidance describes (a private credential store, and
  self-issued credentials for it). That guidance also states the split rather
  than leaving it to be invented: of an extended key usage misconfiguration it
  says a claim generator "should warn its user with an explanation of the
  problem, but should allow the user to choose to proceed with signing". A
  deployment signing for publication would reasonably want the warnings to
  refuse too; `inspect_certificate` is public so that setting has somewhere to
  read from when it is written.

  This started out refusing every certificate without `c2pa-kp-claimSigning`, on
  the belief that the profile requires it in addition to `emailProtection` or
  `documentSigning` — which it does, of a certificate a conforming CA _issues_.
  That is not the set `c2pa` will sign with: its accept-list takes any one of
  six usages, and one is enough. The strict version would have refused a
  `documentSigning`-only certificate — a profile IPTC's own publisher policy
  explicitly permits — while telling its operator the certificate could not
  sign, which is false and which they could not have acted on, an EKU not being
  something you can add to an issued certificate. The two questions are separate
  and the code now keeps them separate.

  What the check does _not_ look at is worth knowing: everything else `c2pa`
  requires at signing time, which is a good deal more, and in particular expiry.
  A Conformance Program certificate is valid for at most 366 days, so every
  deployment that signs meets that one eventually, and it arrives as a signing
  error rather than as an identity one.

  Bytes that do not parse yield no findings rather than a refusal: `c2pa` reads
  the same certificate next with a real validator and says something better than
  a guess from here. What that costs is named where it is done — DER rather than
  PEM, an empty file, and a bundle whose every block is something else all pass
  inspection silently.

- **`parameters` is AUTOMATIC1111's chunk, not ComfyUI's** (#14) — three doc
  comments and a test fixture said otherwise. The rule they were making is right
  and unaffected: a digest must not re-render a value it was given, because that
  puts number formatting and nested key order into the digest's definition. The
  example was wrong. ComfyUI writes `prompt` and `workflow`, both JSON;
  `parameters` is AUTOMATIC1111's and holds line-oriented prose.
  `domain::disclosure` had it right all along, which is where the two families
  are actually told apart.

  It matters because a reader who took the docs literally would reach for a JSON
  decoder on a value that never parses — and the fixture in
  `asterism-importer-image` had already done something adjacent, carrying
  `steps: 30, sampler: euler` under that keyword, which is neither JSON nor
  A1111's grammar. It now carries the real shape: a prompt, a `Negative prompt:`
  line, and one comma-separated settings line.

- **The PNG chunk length is checked, and three comments now describe what their
  code does** (#14) — the length was
  `u32::try_from(payload.len()).unwrap_or(u32::MAX)` under a comment claiming
  the impossible case was made loud. It was the opposite: a payload past the
  bound emitted a chunk whose declared length disagreed with the bytes after it,
  and returned success. The ceiling is taken from `pngmeta::MAX_CHUNK_LENGTH`,
  because that crate reads the chunks this one writes and refuses a length above
  it — a hand-written cap would make the two equal by coincidence.
  `PacketTooLarge` covers both containers now, and its message no longer names
  JPEG.

  The three that were prose only: `png::read` stops at the first XMP chunk even
  when its text is unreadable (which matches the writer, and now says so); the
  control-character filter keeps DEL and C1, which are legal XML 1.0 and which
  the comment said were dropped; and `IPTC_CV`'s doc claimed a structural
  guarantee that is actually held by a test. Two tests are renamed, because what
  they assert stopped matching what they were called.

- **The documented JPEG packet limit was the segment's, not the packet's** (#14)
  — three docs quoted 65,533 bytes, including the one a caller reads when
  deciding how long a prompt to allow. That is the segment's payload; the packet
  gets 65,504, because the 29-byte `http://ns.adobe.com/xap/1.0/` identifier is
  inside the payload and is paid first. A packet between the two figures was
  refused by a limit the documentation did not have, and the caller learned
  about it only through the silent fallback to the reduced record. The docs now
  point at `JPEG_MAX_PACKET`, which is what the writer enforces and what
  `PacketTooLarge` reports, and a test pins the arithmetic.

- **A refused operation says so on screen** (`asterism-ui`) — asking Asterism to
  do something it then refused could leave no trace: the failure went to the
  browser console and the interface carried on, including for operations that
  move or destroy data. The read path had no equivalent gap (`Resource` exposes
  load failures); the write path had no owner for them at all. A new
  `lib/mutate.ts` wraps the write calls, puts the refusal and the backend's
  reason in a sticky toast beside the Undo one, and re-throws so that existing
  rollbacks are unaffected. Routed through it: the grid, group and trash paths —
  `trash_asset` (including the duplicate panel's bulk trash), `purge_asset`,
  `restore_asset`, `empty_trash`, `trash_group`, `delete_dir`,
  `delete_asset_comment`, `add_asset_to_group` and `remove_asset_from_group`,
  `unlink_group`. **Not yet routed**, and still console-only: tag detach,
  persona themes, material marks, threads, modalities, sessions and setting
  resets — along with the non-destructive half of the write path (metadata
  edits, reordering, the create and rename family). Bulk loops that could partly
  fail now report what actually happened ("moved 3 of 5 to trash — the rest was
  refused") instead of counting a refusal as a success. The path is exercised
  end-to-end: `e2e/refusal.spec.ts` seeds its own dir pair over the app's
  loopback HTTP, provokes a real `delete_dir` refusal in the WebView, asserts
  the toast carries the backend's own reason, then deletes the emptied pair with
  the same gesture, asserting that success stays silent.

- **The committed TypeScript bindings are checked against the contract** —
  `asterism-ui/src/bindings.ts` is generated by `src-tauri/build.rs` and tracked
  in git, and nothing compared the two. A contract change whose regenerated
  bindings were never committed would have left a stale copy that every gate
  passed, and passed invisibly: everyone builds from a copy regenerated on their
  own machine, so only a consumer reading the file without compiling Rust would
  have met it. `just bindings-check` forces the build script to run, then diffs
  the result against `HEAD`; it runs inside `just check`. The forcing is not
  incidental — `tauri_build` registers `rerun-if-changed` directives, which
  means a warm tree can otherwise skip the script entirely and compare the
  committed file against itself.

- **`rust-test` no longer depends on the caller's colour setting** — the recipe
  counts cargo's `Running` / `Doc-tests` lines against the `test result:` lines
  to prove that every launched binary reported a result, and both patterns are
  anchored at the start of the line. Coloured output puts an escape sequence
  there, so the count came back 0 launched against 81 reported and the check
  failed over a suite that was 1191 passed / 0 failed. It fixes
  `CARGO_TERM_COLOR=never` for itself now, rather than parsing a shape its
  caller's terminal can change.

- **The e2e suite is now type-checked** (`asterism-ui`) — the specs and both
  WebdriverIO configs sat outside every tsconfig, so `just ui-check` reported
  zero errors over ~4200 lines it never read, and the test runner erased their
  types rather than checking them. A second config (`tsconfig.e2e.json`, run as
  `check:e2e`) covers them without putting `describe` / `it` / `browser` in
  scope for application code. The seven diagnostics it surfaced on its first run
  are fixed: `await $$(…)` now goes through `getElements()`, and both configs
  take `tauri:options` and `browser.tauri` from the service's own
  `TauriCapabilities` instead of a local cast.

### Removed

- **The prompt writer — the field, its argument, and the property it fed** (#39,
  left open by #14) — `Iptc4xmpExt:AIPromptWriterName` was written whenever
  `DisclosureRecord::prompt_writer` held a value, and the only thing that ever
  put one there was the test suite: the sole production builder passed `None`,
  and every other caller of the setter's second argument was under
  `#[cfg(test)]`. A field that can only be `None` in a running application reads
  to a maintainer as a supported capability, and to anyone asking what this
  application discloses as something that might be written.

  Wiring it instead was the alternative, and there is nothing to wire it to.
  IPTC gives the prompt writer a property of its own precisely because that
  person is not thereby the image's creator, so filling it from the asset's
  author or from the operator would assert something nobody stated. The prompt
  reaching a record here is read back out of the container the file arrived in —
  written by somebody else, generated, or rewritten across rounds, for all this
  application knows. A name in a published file cannot be taken back out, which
  is the asymmetry `PromptDisclosure` already turns on, and a person is a
  stronger claim than the text is.

  `with_prompt` therefore takes the prompt alone, the emitter branch is gone,
  the record's module docs state that this application does not disclose a
  prompt writer and why, and a test asserts that no packet names one. If a
  surface for stating it ever exists, it returns under the same withholding
  control the prompt has.

### Boundaries

- **Data layout**: user data is isolated per local profile rather than living at
  one fixed path. Release builds default to `~/.asterism/profiles/dogfood/`,
  debug builds to `~/.asterism/profiles/dev/`, and stress runs select `bench`;
  `$ASTERISM_PROFILE` names a profile and `$ASTERISM_HOME` overrides the
  location outright. A named home carries a `.asterism-profile` marker and is
  refused when opened under a different profile. The UI and the standalone
  server share whichever home is selected.
- **Deletion is two steps**: trash hides an item and keeps everything about it;
  purge is the irreversible half and is reachable only for something already
  trashed. The retention sweep window is `ASTERISM_TRASH_RETENTION_DAYS`
  (default 14); a malformed or non-positive value is refused at startup rather
  than silently replaced.

[Unreleased]: https://github.com/ynishi/asterism/commits/main
