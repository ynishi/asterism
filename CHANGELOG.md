# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Hand an asset to a team, from the pane that is showing it** (#200). The act
  #66 exists for. #148 decision 5 gives content one way onto a team's line — a
  verb scoped to open work, so a team never holds an asset that is not attached
  to work — and #152 built the whole of it as a client function that no screen
  offered as an act of its own. Publishing a line has called it per content
  since #199; handing over one asset, deliberately, is what the detail pane does
  now.

  **What travels is said before it goes.** The file itself and the marks a
  person wrote on it; not the thumbnails, not what was indexed from the file,
  not the marks an import or a machine made — the receiving side can make those
  again (decision 4). It reads above the control rather than beside the result,
  because it is a thing to know before pressing.

  **It goes onto whichever work the shared-lines drawer has open.** The team,
  the line and the pursuit are the shared catalog's already, and it holds them
  whether or not the drawer is showing — so work is opened once and promoted to
  from as many assets as you like. With none open the pane says where to go and
  offers to open the drawer, rather than showing a control that cannot act.

  **A promotion is not a landing.** What it pushes is a round, and a round is a
  request: the entry reaches the line when the work is closed as satisfied.
  Pressing twice is not safe either — decision 7 mints a team asset per
  promotion — and what stops a second one is a relation row this machine wrote,
  so the answer says "this machine had already promoted it" rather than claiming
  the team holds it once.

  Three answers come back that a screen has to keep apart: the entry and the
  team's copy of the bytes, the digest read at promote time, and whether the
  team already held those bytes — where "nobody asked" is a third state and not
  a no. A Collection and a multi-material asset are refused, each with a message
  saying which: what a team holds for one is a conversion whose composition
  decision 3 leaves open.

  `just ui-e2e-teams` drives it against a real `teams-server`: an asset is
  seeded into the profile over the app's own loopback HTTP surface, promoted
  from its pane onto work opened in the drawer, and the round it pushed is read
  back off that work rather than off the answer the write returned.

- **Work a team's line from the app: open a pursuit, push a round, close it**
  (#198). #148 decision 10 says working on a shared line needs no copy, and the
  four verbs that take one from opening to closing now reach a screen. Nothing
  was added to `asterism-contract`: the forge is mirrored path for path
  (decision 19), so a shared line answers with the same `ForgePursuitDto` the
  local one does and the commands map nothing.

  **A line replaces the list rather than opening beside it.** `ForgePanel` puts
  a list and a line's tabs side by side and says in its own header that it is
  wider than this drawer for exactly that; this one is `min(30rem, 92vw)`.
  Pressing a line puts its frame where the list was, with the way back in its
  header, and the three tabs under it — contents, work, history — are the
  forge's three answers about one line. What does not come across is the
  conversation the forge mounts under whichever of its tabs is showing: the
  server mirrors the thread routes and the member's client does not carry them.

  **What a round can carry here, and what it cannot.** Rename, remove, and the
  add that puts a removal back — the operations that name entries the line
  already holds. Putting something new on a team's line is content entering the
  team, which is the promotion and #198's sibling; until that lands, this
  surface moves what is there and brings nothing to it. It says so where
  somebody would otherwise look for the control.

  **One fold, both planes.** What a close would leave is folded from the line
  and the work together, and that fold is now `lib/forge-projection.ts` rather
  than a second copy written against the second catalog. The two stores stay
  apart, which is what decision 16 asks for — the fold reads nothing and holds
  nothing, it takes two shapes and returns rows.

  **Two answers this surface does not have.** `ForgeWork` shows how many
  landings have arrived since the work was cut and what it collides with; the
  team server mirrors both routes and the desktop has no command for either, so
  a close here can be refused by something the screen never showed. That is a
  gap rather than a decision. A refusal also arrives as its message and nothing
  more: the forge's conflicts answer with a reason token and the desktop keeps
  the sentence and drops the token, so there is nothing here to turn into
  advice.

  `just ui-e2e-teams` drives it end to end against a real `teams-server` — the
  fixture seeds a second team with a line holding one entry, because the first
  team's emptiness is what the connect spec is about, and the app itself cannot
  seed one.

- **The team's roster on screen, and a team you can make from the app** (#196).
  The frame #194 built gains its second tab, and the connection gains the one
  act that is about no team in particular.

  **Ids rather than names, and the tab says why.** A membership row carries a
  user id and a role and nothing else; the name on a ledger event is a snapshot
  the act took, and there is no equivalent to read here. A reader comparing the
  two tabs would otherwise be left to work that out. The viewer's own row is
  marked, which is what turns a list of strangers into a place the reader is in.

  **Founding a team sits beside the field, not on a tab.** Every tab is an
  answer about the team named above them, and this is the act that is about
  none. It shows wherever there is a connection rather than only where no team
  is named: the field is `required`, so naming a team is a one-way trip on this
  surface, and an offer that appeared only before the first trip could be taken
  once per window. Founding also lands the reader on what they made, rather than
  printing an id to copy into the field directly above.

  **What #171's roster bullet asks for is not one thing.** It reads "Create a
  team, see the members, invite and join under `RegistrationPolicy`, leave", and
  those five verbs sit at four different depths: creating and reading are wired
  end to end, the membership writes have no client half, `leave` has no route,
  and **`join` does not exist anywhere**. `RegistrationPolicy` is consulted by
  exactly one of them — its only consumer is `may_create_team` — so inviting and
  leaving answer to the authority table and the last-owner rule instead. This
  lands the two that were ready; the rest are siblings, and `join` is a finding
  about the issue rather than a specification.

- **The team's ledger on screen, and the frame its tabs land in** (#194). Who
  did what, in what capacity, is the team's own record, and #171 asks that a
  member be able to read it from the app. This is the first of that umbrella's
  surfaces, so it builds the frame as well: #190's design said the tabs were a
  design and not a component and that the first surface to land would build
  them, and this is that surface. The connection and the team sit above the
  tabs, because they are what the tabs are answers about; publishing sits inside
  the lines tab, because it seeds a line.

  **The read's shape decides more than taste does.** A ledger has no final page
  and the wire says so: a page that fills its limit always carries a cursor, and
  a short page carries none — meaning nothing lay past there _when the page was
  taken_. So the foot never announces an end. It offers to read more where there
  is a cursor and to ask again where there is not, and asking again resumes from
  the last seq the walk saw rather than from the beginning, which would append a
  second copy of everything. The actor's name is shown as stored rather than
  resolved, because it is a snapshot taken at write time and a later rename must
  not change a past record under its reader. The capacity is shown beside it,
  which is the distinction #83 §1 exists to keep.

  **Kinds and payloads are rendered as themselves.** A screen that mapped each
  kind to a sentence would be a second place every new kind has to be learned,
  going stale where nobody is looking — the trap #148 decision 14 names for the
  projection. `forge.*` is still growing them. So a kind this screen has never
  seen arrives intact, and what an event carries is one press away as the JSON
  it is.

  An empty ledger is reported as a fault rather than as a quiet state: founding
  a team appends its own event, so a team that answers with nothing has answered
  wrongly.

  **The shapes cross a boundary, and the crossing has a place.** A fact reaching
  this screen passes two — the wire a member's client and a team server speak,
  and the contract this app speaks to its own frontend — and `bindings.ts` is a
  projection of the contract alone, which is what gives every screen one set of
  types to know. So `asterism-contract::teams` carries the ledger's shapes and
  the command maps to them, rather than handing a screen a type belonging to a
  server it never talks to. A test holds that boundary now
  (`src-tauri/tests/boundary.rs`), because the first draft of this change
  crossed it and nothing said so: what refused the crossing could only report
  that three names were not the contract's, not that a layer had been skipped.

- **The team plane can be driven end to end, against a `teams-server` of its
  own** (#192). Every read on that plane is a request to a second binary, and
  `just ui-e2e` builds and launches one — so the surfaces #171 is about had no
  way to be tested at all. `just ui-e2e-teams` is the run that can: it builds
  the server, and `wdio.teams.conf.ts` starts it against a database nothing has
  touched, provisions an account and a team, and stops it afterwards.

  **Why it is a separate run rather than more specs.** A run may hold one
  stateful fixture or two, and #188 is what two looks like — a spec that fails
  in a full run and passes alone, with the signature of a fixture left in one of
  two states. The separation costs nothing here: this database is made empty per
  run and thrown away, which the app's own profile cannot be, since the e2e
  suite provokes verbs against seeded content and puts back what it takes.

  **What the smoke spec reaches.** Four commands that no test had ever called —
  the session read, connect, the team's lines, and disconnect — over the real
  wire, and the three phases in the order a person meets them: nobody to ask,
  somebody to ask with no team named, and a team whose lines are read. The
  middle one is the assertion that matters, because "This team hosts no lines."
  used to appear there too (#190 fixed the panel; nothing until now could
  confirm it from the outside).

  **A fixture that does not come up stops the run.** Readiness is the server's
  own line rather than the port answering, so another process holding the port
  is not mistaken for this server starting, and the rejection carries what the
  server last said rather than guessing at a cause. No spec runs after it: the
  launcher rethrows exactly one class of hook failure, and that is the one
  `onPrepare` raises — while the teardown still runs, so the server is stopped
  and the database removed on that path too.

- **The team catalog, and the design the team plane's surfaces share** (#190).
  #171 lists what the team plane has no surface for; what it did not say is what
  any of those surfaces looks like, and the drawer they attach to holds a
  connection form, a typed team id, a list of lines and a publish form in one
  column — every one of them load-bearing for whatever lands beside it. This is
  #177 done for the other plane, and it lands in the same place: the catalog's
  own doc, `lib/stores/shared.svelte.ts`, which already carried the reasoning
  for what exists there.

  Three things keep it from being the forge's frame with a base URL. **A team is
  not only its lines** — the memberships that say who is in it, the lines it
  hosts and the ledger of what was done in what capacity all sit under one team,
  where the local plane has only lines. They are three answers about one team,
  so they are tabs on one frame, and selecting a line inside the first brings
  the forge's own frame with it, because decision 19 mirrors the local surface
  path for path and a shared line is the same subject a local one is. A subset
  of that frame: the member's client does not carry the conversation verbs, so
  contents, work and history cross and threads do not. Which of the three tabs
  leads is taste, and the doc says so.

  **There is a connection, and before it there is nothing.** Every read here is
  a request to a server, which gives "nothing to show" two meanings a screen
  must not merge: nobody has been asked yet, and there is nobody to ask. An
  empty list under a dropped connection would say the team hosts nothing, which
  is a claim about a team nobody is talking to. `phase` tells them apart, as the
  frame's own state rather than a resource's — a `Resource` knows whether it is
  loading and whether it failed, and neither answers whether there is a server
  behind it. Its middle state, connected with no team chosen, is where a window
  already begins: the field starts empty. A picker for "the teams I am in"
  populates that state rather than introducing it.

  **The panel reads it, so the two are told apart on screen and not only in the
  doc.** It was showing "This team hosts no lines." with no team named, and
  offering to publish a line to whichever team the empty field meant. The list
  now belongs to the state where a team is chosen, the connection form to the
  state where there is none, and a person with no team named is asked to name
  one.

  **A promotion does not start from a team.** #152's client converts a local
  Asset, so the subject is the asset and the team is where it goes; a verb on
  this frame would ask somebody to name their asset from a screen that is not
  showing it. It belongs to the asset detail pane, and the model adds that it
  cannot start there either without work already open — decision 5 gives content
  exactly one entry point, scoped to an open pursuit, so the team never holds an
  Asset that is not attached to work. It is also a write that is not safe to
  press twice: decision 7 mints a TeamAsset per promotion rather than finding
  the first, which is the asymmetry `publish` carries and `clone` does not.

  Two things the design corrects rather than inherits. The roster's verbs answer
  to four rules and not one: #171's body hangs create, invite, join and leave on
  `RegistrationPolicy`, where the code puts create there, invite behind an
  owner, leave behind the last-owner rule — and **offers no join verb at all**,
  so a tab built from that sentence would have grown a button with nothing
  behind it. And #130's fetch-for-me, which #171 places under this umbrella, is
  a verb the model panel gains rather than a tab here: the subject is the
  encoder somebody is training, which is the promotion's direction read
  backwards.

  Where a stored credential lives is deferred rather than answered, with its
  alternatives named — the OS keychain, the profile directory, or the window
  #167 chose. The answer interacts with a provider path this plane does not have
  yet, and #163 is what brings it.

- **A line can be worked from the screen: a pursuit from open to close** (#170).
  #180 left one button disabled, and it was the one that made the rest of the
  forge reachable. What it opens is a third tab beside contents and history,
  because work is a third answer about the same line rather than a different
  subject — what the line says, what somebody is asking it to say, and how it
  got here. It sits between the two, since working a line is the common path and
  reading the chain is the occasional one.

  **Nothing a round asks for is on the line.** `push` does not read the line at
  all, which is what lets two people work against one line without contending,
  and the only moment anything lands is a close with `satisfied`. So the rounds
  are drawn as a log and the contents tab stays the answer to what the line
  holds — a distinction the e2e spec asserts from both sides, reading the
  contents once before the close and once after.

  **The log is the editor.** There is no staging area between picking images and
  writing a round: pressing add writes one, and a correction is another round,
  which is what the model stores anyway. A grid selection is where content comes
  from — the same set dispatch and snapshot read, so the gesture is one somebody
  already knows — and because the forge is an overlay, the picking happens
  before it opens. Names default to the basename under the card, which is now
  shared rather than copied, so the name on a new entry is the one the person
  saw. The other three verbs name something that already exists, which a
  selection cannot express, so rename, replace and remove sit on the row
  instead.

  **The rows they sit on are the fold of both logs** — the line with the work
  applied over it, which is what a satisfied close would leave and what neither
  log says alone. It applies the model's rules rather than approximating them,
  and it takes the model's two steps to do it: the work folds to one row per
  entry first — per axis the last operation wins, and then the winning existence
  decides what the row says, so an entry on its way off keeps the name the line
  already had rather than one this work gave it in passing — and only then does
  the line come in, where a removal of something it is not holding leaves the
  line's own row exactly as it was. Each row states both steps, because drawing
  an entry as gone and offering to change it are questions about different ones.
  Putting a removal back adds that same entry, by its own id, because an entry
  returning under a new one is a new arrival and the record would say so.

  It is not the model's answer and cannot be — a landing arriving meanwhile
  changes what it is folded onto, and most landings touch nothing the work asks
  for. Reading it early is what lets somebody fix a name before the close tells
  them to: two live entries under one name is a refused landing, and the case
  that will actually happen is a name defaulted from a filename meeting one the
  line already holds. Counting only what the work asked for would be silent on
  exactly that, so the warning is read off the same fold.

  **A tile is a thumbnail where there is one, and what the thing is where there
  is not.** A line refers to assets rather than pictures — the first card this
  repository's own e2e picks up is a recording — and an entry carrying one was
  an empty grey box, indistinguishable from one whose thumbnail had not arrived,
  because a thumb miss and a thumb that is never coming are the same transparent
  pixel. The tile reads the card's own `media`, which is the field the grid
  reads.

  **With nothing picked, the add control gets out of the way rather than going
  dead.** The drawer is an overlay, so a button telling somebody to select in
  the grid is the thing stopping them; it steps the drawer aside instead,
  keeping the line and the work, and the sidebar brings it back to them.

  **A discard says what it needs first.** The forge refuses to drop a line while
  any work is open against it, because dropping takes the history that work was
  cut from — so the confirmation counts the open pursuits and names where to
  close them, rather than promising they go with it.

  **A refusal to close carries a reason, and the reason is not the action.**
  `mutate` puts the message on screen and reads no further. `blocked` arrives
  both for a line that moved and for a line that is archived, and only its
  message separates them, so both actions are offered; `settled` has no action
  at all.

  **Conversations get their own place rather than a fifth anchor kind on the
  thread drawer**, which is the question #170's fourth surface opened with. The
  two are separate aggregates down to the service and their messages answer to
  different fields — a forge message carries what it said first and every
  revision of it, an app-level one carries role and refs — so one surface
  holding both would be a component with two halves that never run together. One
  is opened from a piece of work, a round, an entry as that round had it, or a
  change point on the line, and all four are shown in one place under the tabs:
  a conversation is about something rather than beside it, and opening one from
  a round should not move the reader away from the round. Every correction is on
  screen, because a correction the reader does not see is a sentence still
  attributed to somebody who withdrew it.

  `e2e/forge-pursuit.spec.ts` drives it through the real backend, which is the
  only place a command's name, arguments and answer are checked against the app
  that has to answer them. It also makes the assertion `forge-line.spec.ts` said
  it could not: its line held nothing, so its discard reported zero. This one
  puts an asset on a line and reads the release count against real content.

- **The forge opens from the app, and a line has its whole lifecycle** (#180).
  #177 placed the catalog and left the panel unmounted, because where the forge
  sits was one of two questions its design deliberately did not answer. It is a
  place you go, not a way you filter: the grid's facets are properties an asset
  _has_ — persona, modality, tag — and a line is not one. A line refers to
  assets and names them its own way, so one asset sits on two lines under two
  names, and narrowing the grid by a line would replace what it lists rather
  than filter it. So: a drawer, opened from the sidebar under a `Forge` heading
  of its own, with `shared lines` moved beside it out of **Trash**, which fitted
  it no better.

  Creating a line is in none of #170's four children — the first is read-only
  and the third is rename, re-point, standing and discard — and its absence is
  not academic: a machine that has never had a line has none, and nothing in the
  app could make one, so a read-only panel over them showed an empty list
  forever. The whole lifecycle is here. The rule a new line takes is chosen
  rather than defaulted, because it decides how that line settles a collision.
  Discard sits behind the confirm modal and only on an archived line, which is
  the only standing it can be reached from, and it reports how many assets it
  released — the answer no later read can reconstruct.

  Closing the drawer ends the question rather than pausing it: the selection and
  everything it produced go with it, because a fold of a chain that moved in
  between is worse than no answer, and #170's second child will move chains.
  What a line let go stays reachable in its own collapsed section, dimmed and
  dashed — findable, and not mistakable for contents.

  Two suites, because they answer different questions. `forge.test.ts` pins what
  each write invalidates and what closing drops, against mocked seams.
  `e2e/forge-line.spec.ts` drives the lifecycle through the real backend in the
  WebView, which is the only place the seven commands' names, arguments and
  answers are checked at all — a unit test asserts the shape its own author
  wrote down twice.

- **The forge's catalog, and the design its four screens share** (#177). #170
  lists four surfaces and says what each one lands; what it did not say is what
  any of them looks like, and the first one cannot be opened without deciding
  that — a lines panel is the frame the other three attach to, so its layout
  fixes where a pursuit is opened from, where the line verbs sit, and whether a
  thread has somewhere to hang. Deciding that from inside whichever piece is
  built first is how a frame ends up shaped by that piece.

  The design is the catalog's own doc rather than a document beside it. A
  separate design file is a second place the truth lives, updated only when
  whoever implements something remembers it exists; CONTRIBUTING says the same
  as a rule. So `lib/stores/forge.svelte.ts` carries the reasoning and makes the
  three reads, and `ForgePanel.svelte` is the frame — contents first with the
  history behind a tab, because working a line is the common path and reading
  the chain answers how it arrived, which is asked after the fact. A layout
  leading with the chain makes the common verb something reached through a log.
  That ordering is an assumption about intended use rather than a measurement,
  and the doc says so where somebody would look for it.

  Two shapes follow from the model rather than from taste. The chain does not
  fork — `History::record` refuses a change point whose parent is not the head —
  so there is no branch graph, which is the pattern most version-history UIs are
  built around. And nothing is ever removed: `states` answers with entries that
  are off the line as well as on it, so what a line let go is part of the
  record. Those sit in their own section, collapsed, drawn so they cannot be
  read as contents — findable, which is what a record is for, and distinct,
  which is what stops one being reused as though it were still held.

  Nothing here writes, and the panel is not mounted: where the forge sits in the
  app is one of the two questions the design leaves open, and mounting it would
  answer that by implementation rather than by decision.

- **The TypeScript bindings are a projection of the contract, and a check says
  so** (#175). `bindings.ts` used to hold whatever a screen had asked for:
  `build.rs` said a type entered its list "in the change that consumes it",
  because "exporting a shape before the screen that shapes it is how a binding
  drifts from what the screen actually needs". That ground does not hold —
  `schema-bridge` generates the file, no line of it is hand-written, and a
  projection has nothing to drift from. What the rule produced was a binding
  layer waiting on the UI: a verb reachable over HTTP and over IPC could not be
  named in TypeScript until somebody edited a build script.

  The rule is withdrawn and forty-five types now reach TypeScript — the forge
  whole (pursuits, rounds, threads, collisions, every command), tag
  administration, series strategies, snapshots and bulk group membership, the
  observation stream, and the sort vocabulary. `tests/export_parity.rs` reads
  the contract for `SchemaBridge` derives and `build.rs` for its export list,
  and fails on a type in the first and not the second unless it is on an
  allow-list carrying a reason. Seven entries are there. Six are a diagnostics,
  job-log or perf read that no Tauri command serves, so no TypeScript caller has
  a path to one. The seventh is `DiagLevel`, on a different ground: the write
  side does not take it either, because `RecordDiagCommand::level` is a `String`
  on purpose — "so the bindings stay flat; an unknown value is a validation
  error, not a guessed level".

  The count `build.rs` used to state is gone from it — it said "three deliberate
  omissions" while fifty-two types stayed out, twenty-one of them named nowhere
  at all, which is what a number describing a list does in a file that does not
  hold the list. Nothing had ever compared the two; the same paragraph said so.

- **A model section in Settings: the encoder stated, the head managed** (#130).
  The encoder is app infrastructure — bundled, nothing to choose — so the
  section states it and no more: id, dimensions, preprocessing revision, or the
  one honest "no model bound". What a person manages is the head, and this is
  the first screen to reach the verbs #132 built: which head scores now, the
  held-out eval that promoted it, how many rulings exist and how many tags clear
  the training floor, a Train now button, and a paste box for the head a team
  published — v0 of the pull is the artifact itself, fetched with the person's
  own session.

  A new read answers it (`GET /asterism/heads/status`, `head_status` over IPC)
  rather than a widened `VisualModelStatusDto`, which says which encoder the
  process bound and goes on saying only that. The restart badge is derived on
  the server — what would bind at the next launch against what is bound now — so
  a pointer this encoder would refuse asks for the relaunch that drops to
  zero-shot, exactly as a cleared pointer does. Both verbs enqueue a job, and
  the panel surfaces that job's own sentence verbatim: the promotion verdict, or
  the refusal naming the encoder a pulled head was trained against.

- **The source type is asserted from the asset detail panel** (#108). A "Source
  type" row with three states: the person's assertion (term, who, when, Edit and
  Retract), the term the container's evidence establishes (labelled as the
  container's, behind an Override…), and unknown — with a container not yet
  fingerprinted saying "not yet read" rather than "declares nothing". The select
  is closed over the five IPTC terms, so the panel cannot send what the backend
  would refuse. Behind it, a new read (`GET /asterism/assets/{id}/source-type`)
  carries evidence and assertion each on its own; the derivation is the one
  `record_for` already composes, extracted rather than re-derived.

- **The teams plane's license is declared: AGPL-3.0-or-later** (#162). The four
  `teams-*` manifests replace the deliberately-undeclared comment with the
  field, the canonical AGPL-3.0 text lands at the root as `LICENSE-AGPL`, and
  README's Licence section states both regimes and the boundary — the guarded
  direction, an `asterism-*` crate depending on a `teams-*` crate, stays empty.
  The call #83 §4 deferred to publish time, made at v0.1.0; `publish = false`
  stands untouched.

- **A release workflow: the macOS download opens like an app** (#165). A pushed
  `v*` tag builds the production-shaped bundle through `just dogfood-build`,
  signs it with the Developer ID identity under the hardened runtime, notarizes
  the .app and the DMG each with their own stapled ticket, asserts what
  Gatekeeper would ask (`spctl --assess`, `stapler validate`), and attaches the
  DMG to a _draft_ release — publishing stays a human decision. A
  `workflow_dispatch` run rehearses the same pipeline into a workflow artifact
  without touching a release. The six Apple secrets enter by name only; the
  workflow header is the checklist of what each one is.

- **A member can take a copy of what a team holds** (#153, #148 decision 10). A
  clone is a detached copy, which makes it an import rather than a forge
  concept: it mints its own `AssetId`, it writes no relation row — a row there
  means "I put this there", and a copy did not — and it says where it came from
  the way every other import does, through a new `SourceKind::TEAM_LINE` slug
  and a locator naming the team, the line, the entry and the team asset it was
  taken from. Cloning something already here is answered from the existing
  duplicate machinery before a byte moves, because that locator is built from
  those four ids and carries nothing the caller chose.

  The locator is a path rather than a name, and the reason is that everything
  downstream which wants bytes asks whether there are any: a copy filed under a
  logical name would arrive with no hash, no thumbnail, no cover text and no
  promoting it onward, each failing silently. The one part of the path that is
  not an id is the file extension, taken from what the line calls the entry
  because an extension is the only thing that classifies a material — so
  renaming an entry across extensions on the team's line makes a re-clone a
  second copy, which is stated where the path is built rather than left to be
  found.

- **A private line can seed a team's line** (#153, #148 decision 11). Publishing
  transfers the current state: the team gets a genesis and one change point
  holding what the line holds now. Replaying the chain is an option at init and
  nothing more — a line seeded with its current state cannot be given its
  history afterwards, because the history would have to arrive underneath change
  points that already exist.

  Replaying is a **re-enactment**, and the word is in the type, in what the verb
  answers, and in what the panel says. The acts are restamped to whoever
  published, because the original actors are not necessarily members of the team
  and inventing a handle for them would be the team's record claiming what it
  does not know. So the team's line does not record who did the work upstream;
  at this boundary the question is who brought this here, and the restamped act
  answers exactly that. What it costs is stated rather than discovered: a
  re-enactment sends every content the line ever named, including everything an
  entry was replaced with and everything taken back off. Work logs and
  conversations do not cross at all — nothing reads the private line's pursuits,
  so the abandoned rounds #66 decision 2 protects have no path across.

  The cheaper seeding is also the narrower one. It cannot take a line whose live
  entries share content, because a promotion's repeat check is keyed on the
  asset and the line, so the second entry would be answered from the first and
  the team would receive one where the private line has two. That is refused
  outright rather than narrowed in silence, and a re-enactment takes it, because
  a chain names each entry in its own right. Both refusals are answered before
  the team's line is opened: a refusal met half-way through leaves a line on
  somebody else's team, and unlike a local one that is not the publisher's to
  tidy away.

- **Shared lines list in their own panel** (#153, #148 decision 16). A shared
  line is served through rather than mirrored, so the panel holds no copy: it
  reads when it opens, when a line is selected, and when a write it made changed
  the answer, and it empties when the connection goes rather than showing the
  last thing the server said. Kept apart from the local lines rather than mixed
  into them, which is what having two sources honestly looks like. It shows how
  many change points a line has, which is the visible difference between one
  published as it stands and one whose chain was re-enacted, and it carries the
  clone and publish verbs with what re-enacting costs written beside the choice.

  The desktop's write surface grew by one, and the mutation-surface guard
  records it. Only the clone counts: it writes to this machine, so it names the
  owner's own operation surface. The other seven commands reach a team's server,
  where the author is the authenticated member and the team stamps it, so a
  context stated locally would be a second answer to a settled question.

- **A member's machine can promote an Asset onto a team's line** (#152, #148
  decisions 3–9 and 12–15). The member's half of #148: a client that talks to a
  team server, the one composite act that hands an Asset over, and the relation
  that records it at home. A promotion gathers what cannot be re-derived — the
  material's bytes and the marks whose layer origin is `User` — brings the
  content in against open work, pushes a round that names the entry, and writes
  one row on the promoting machine. Thumbnails, indexed bodies and
  `Imported`/`Machine` marks stay home, because the receiving side can make them
  again (#148 decision 4).

  Two members promoting identical bytes get a `TeamAsset` each over one stored
  copy (#148 decision 7), which is the only arrangement where "who brought what"
  survives the second contributor. Ids do not cross: a team's ids arrive as a
  `TeamScopedId`, which has no conversion to or from a local `AssetId` in either
  direction, so #148 decision 6's forbidden read does not compile. Subjects and
  digests do cross, which is what that decision says may.

  A shared line is served through rather than mirrored (#148 decision 16), so
  every read is a request and there is no staleness to reason about. Wanting a
  line locally is what a clone is for, and a clone is #153's.

- **`asterism-teams-wire` — the leaf both planes may link** (#152, #148 decision
  15). #83 §4 forbids `asterism-* → teams-*` in any form, and prescribes its own
  second choice for vocabulary both sides need and neither owns: a leaf that
  depends on neither. This is it, MIT/Apache-2.0 and named on §4's
  `asterism-<thing>` rule, carrying the session, team, roster, ledger-page,
  content-verb and projection shapes. The existing `teams-contract` could not be
  that leaf — its licence is deliberately undeclared pending §4 and it declares
  a `teams-core` dependency — so those shapes moved rather than being copied,
  and `teams-contract` keeps what a member's client does not speak: the roster
  verbs, the blob upload, the purge two-step and the head registry.

- **Descriptive metadata travels as a captured projection** (#152, #148
  decisions 12–14). Keyed `(line, entry)` on the teams plane and outside the
  forge, so it can be lost without any line lying about the present. The body is
  opaque end to end — no column, no validation, no index, and no DTO naming a
  field inside it — and the one thing that opens one is a single mapper on the
  member's side, which branches on the version the body carries. What may travel
  is declared at that mapper, so an input nobody declared does not leave: a
  column added to the local model later starts out staying home (#148 decision
  13). The write rides on the round push rather than getting a verb of its own,
  which is what keeps a second editing surface from growing beside the verbs.

- **`AssetLink` — the correspondence, held only on the member's machine** (#152,
  #148 decisions 8 and 9). Keyed `(team_id, line_id, entry_id)`, all three fixed
  by the client rather than learned back from the server. The server holds no
  reference to a local Asset in either direction. The relation is advisory and
  attended: either end may vanish and neither may break the other, so the table
  carries no foreign key — `RESTRICT` would let a team's row forbid a local
  delete and `CASCADE` would destroy the evidence a check is meant to find — and
  a verify and a reap go looking instead, on GitLab's loose-foreign-key and
  `git annex fsck` precedent. A reap removes link rows and touches nothing else.

- **Schema V104 (app) — `team_asset_link`** (#152). The relation above, with the
  absent foreign key argued at the batch, and one index for the read a local
  plane makes while looking at an Asset.

- **Schema V9 (teams) — `asset_projection`** (#152). One row per entry, replaced
  rather than versioned, scoped by `team_id` so a read cannot cross to another
  team's description. `version` is a fact about the envelope rather than a field
  of the body, which is what lets the plane keep and sweep bodies it has never
  opened.

- **The team's forge is served over HTTP** (#151, #148 decisions 5 and 19). The
  local forge surface is mirrored under `/teams/{team_id}/forge/*` — same paths
  below the prefix, same DTOs from `asterism-contract::forge`, same handler
  form, and refusals on `asterism-server`'s status table down to the `reason`
  token a conflict carries. Every route sits behind the existing `auth_gate` +
  `team_gate` pair.

  Three things differ from the local surface, and each is what hosting is. The
  services are built per request, over a `TeamForge` carrying the team and the
  capacity the gate established, because a context-held service could carry
  neither. The author is the authenticated member: `author_kind` and
  `author_subject` are **refused** rather than overwritten, since the gate has
  already answered who is asking and a write claiming to be somebody else is the
  one shape #83 §1 forbids outright — only `operator_ai` is still the caller's
  to state. And the ledger event records the capacity while the forge node
  records who (#148 revision 6), which is two records and two fields.

- **Membership is what decides a forge verb, and seniority is not** (#148
  revision 5). `TeamVerb` gains `ForgeWork` and `ForgeDiscard`. Opening a line,
  renaming it, re-pointing its rule, moving its standing, opening work, pushing
  a round, resolving, closing, everything said in a thread and bringing content
  in are a member's acts — they all leave a record, and anyone who can read the
  line can recover from any of them. Discarding a line is the exception, because
  it is the verb that takes the log with it, and it asks for an owner. An admin
  standing outside the roster answers neither: #83 §1 grants an admin the
  destructive pair on the team's own substrate and nothing implicit inside a
  team they are not in.

  Revision 5 also says "the restrictive setting stays available and is the
  default", which pulls against the paragraph it closes. This ships the argued
  reading and leaves that knob unbuilt rather than inventing a default for it;
  the divergence is recorded on `TeamVerb::ForgeWork` and settling it belongs to
  #148.

- **Content enters a team against open work** (#148 decision 5). A new verb,
  `PUT /teams/{team_id}/forge/pursuits/{id}/content?digest=…`, is the one entry
  point: the team never holds an asset unattached to work, and the content is
  there before the round that names it. The byte path is the blob upload's,
  unchanged — streamed, always hashed whole, durable before any row commits —
  and what it adds after that is the `team_asset` a round can name, its
  `team_blob_link`, and one `forge.content.entered/1` event, all in a single
  transaction. Work that has ended is a `409` carrying `settled`; work of
  another team reads as absent; a digest marked for purge is refused with
  `blocked` rather than re-linked, because minting an asset over bytes a reclaim
  is coming for would hand a line content scheduled to disappear.

  Identical bytes promoted twice mint two assets over one stored copy (#148
  decision 7), which is the only arrangement where "who brought what" survives
  the second contributor.

- **Two bulk reads over what a team holds** (#148 decision 19).
  `POST …/forge/content/resolve` answers which of a list of team asset ids the
  team holds and what each was converted from; ids it did not mint come back as
  unknown rather than as a refusal. `POST …/forge/content/have` answers which of
  a list of digests it already has, and exists to avoid re-sending bytes and for
  nothing else — it answers inside one team, to that team's members, about
  digests the asker is holding anyway, so what it reveals is what uploading
  would reveal one round trip later. Asked across teams it would be the
  deduplication side channel #83 §3 closes by making the link row the visibility
  boundary. A digest marked for purge answers as not held. Both are bounded at
  500 per request and refuse rather than truncate, because a truncated answer to
  either is a wrong one.

- **Schema V8 — what a `team_asset` was converted from** (#151). `digest` and
  `entered_for`, both nullable, both unindexed. V7 left the composition open in
  as many words and the content verb is what settles it: one blob is the whole
  of the v0 conversion, and a conversion composed some other way leaves the
  column empty rather than carrying a digest standing for one part of itself.
  Not `UNIQUE`, per decision 7. `entered_for` records decision 5's attachment
  rather than enforcing it — the invariant is checked at the door, in the
  transaction that writes the row — and carries no foreign key, because
  `Lines::discard` deletes the work against a line and a `RESTRICT` key would
  refuse the one verb that releases this content.

- **A thirteenth `forge.*` kind** (#151). `forge.content.entered/1` joins
  `FORGE_KINDS`, named for the act like its siblings. Its payload carries the
  asset, the digest and the work; its subjects are the digest and the pursuit,
  so a trace query reaches a promotion from either end without parsing a
  payload. One event per promotion even when the store already held the bytes,
  for decision 7's reason.

- **`GET /teams/{team_id}/events/subject`** (#151). The subject-filtered ledger
  read has been answerable in the repository since #83 §2 and had no way to be
  asked; what made it worth exposing is what it can now be asked about, since
  the ledger carries forge subjects. Same page contract as the stream read —
  keyset over `seq`, a short page ends the walk — over a paged sibling of
  `events_for_subject`, because a subject filter bounds by what rather than by
  how much and the forge's busiest subjects gain a row per push.

- **The team plane hosts a forge** (#150, #148 decision 20). `teams-infra` gains
  `TeamForge`, a set of adapters behind the forge ports `asterism-core` already
  declares — `Lines`, `Pursuits`, `Closings`, `Threads`, `Actors` and `Store` —
  over the team's own database. `asterism-core` does not change by a line of
  code; the one new dependency edge is `teams-infra → asterism-core`, which #83
  §4 permits outright.

  §4's never-list has since been narrowed rather than left as it was (#148
  revision 10): it forbade `teams-* → asterism-contract` entirely, and the
  reason it states — "those are the local app's plumbing; teams-infra owns its
  own" — does not hold for `asterism-contract::forge`, which declares no
  Asterism dependency, is MIT/Apache and `publish = false`, and already rode in
  `teams-core`'s graph transitively. So that one module may be named from the
  teams plane, and `command`, `query` and `dto` stay forbidden, where the stated
  reason is the true one. `asterism-infra` and `asterism-server` are untouched
  by the narrowing. Nothing in this entry depends on it — the transport is what
  takes it up (#151, below).

  The point of embedding rather than fronting is decision 17: every write-port
  method is one transaction holding the forge write **and** its ledger append,
  which an event spanning two processes and two databases could not be. It goes
  through the same allocation of `seq`, the same registry check and the same
  subject-index rows the repository's own gestures use, so a write that is
  refused leaves neither a forge row nor a ledger entry. Team scope lives in the
  adapter and in no port signature — the seat `Lines::list` reserves for whoever
  knows what a person is — and is enforced on reads as tightly as on writes, so
  an id belonging to another team reads back as absent.

  Nothing was served when this landed. The HTTP surface arrived with #151,
  below, and the member's client with #152, above.

- **Schema V7 — the forge's tables on the teams database** (#150). The local
  plane's `line`, `change_point`, `change_row`, `pursuit`, `pursuit_node`,
  `pursuit_op`, `forge_actor` and the three thread tables, replicated under the
  same names in a separate database file, plus `team_asset` — the TeamAsset
  surrogate (#148 decisions 3 and 7), carrying identity and nothing else until
  the content verb lands. The deliberate differences are `team_id` on every
  table, `UNIQUE (team_id, name)` on `line` — the name-uniqueness question the
  forge's `Name` leaves to whoever owns the namespace, answered here by the team
  — the two content keys pointing at `team_asset`, and `forge_actor` keyed
  within a team with the write-time `display_name` snapshot from #149.

  `team_id` deliberately carries no foreign key to `team`. Every key inside the
  forge is `RESTRICT`, so a cascade into `line` is refused by the change points
  on it: the key would either break a team deletion that works today or quietly
  destroy a line's whole history as a side effect of a membership gesture.
  Deleting a team therefore leaves its forge rows behind for now, and wiring a
  deletion to `Lines::discard` — which is what actually releases a line's
  contents — belongs with the transport and the client.

- **Twelve `forge.*` event kinds, and three forge subject refs** (#150).
  `FORGE_KINDS` registers `forge.line.opened/1` through
  `forge.thread.renamed/1`, named after the verb rather than the table. They are
  a slice of their own beside `V0_KINDS`, and `is_registered_kind` is the union
  a writer asks; the envelope does not change, which is what the namespace
  reservation in `ledger.rs` was for. `SubjectRef` gains `forge_line`,
  `forge_pursuit` and `forge_thread`, so "which events touched this line" is an
  index walk rather than payload parsing. Rename-shaped kinds carry the old
  value and the new, read inside the transaction that replaces it; no payload
  carries a message body or content, because the ledger is append-only and a
  copy there is one nothing can act on later.

- **A check that the two transports still owe each other every verb** (#173).
  `asterism-server`'s `tests/transport_parity.rs` reads the router and the
  desktop's command module, pairs them by name, and fails when either direction
  goes short: a route with no command, a command with no route, or an allow-list
  entry that no longer matches the tree. Each exception carries the reason it is
  one, because a difference with a reason is a decision and one without is the
  defect.

  The rule itself was already written, in `http`'s module doc. What it lacked
  was anything that re-measured it, and the count stated there went stale three
  times — twice while #136's sixteen-verb debt was being paid, and once when
  #169 added a verb to both surfaces and left the total behind. That is what
  CONTRIBUTING's third documentation rule says happens to a number describing a
  list kept somewhere else, and the rule's other branch is what this took: no
  count is stated anywhere now, and the prose points at the lists. The check
  covers the direction the drift has actually come from, which #136 recorded:
  every one of its sixteen verbs landed in a change whose scope said routes.
  `changed-packages` selects one crate per changed path, so a branch touching
  only the desktop's commands is caught by `main`'s run rather than its own, and
  the test's doc says so. `commands.rs` gains the section that was missing
  beside it — why there are two transports over one service graph at all, rather
  than the desktop calling the loopback surface its own process serves.

### Changed

- **A `CLAUDE*.md` is ignored wherever it sits, and so is a nested `.claude/`.**
  The ignore list named the root `CLAUDE.md`, which is not where the risk lives.
  Claude Code reads a `CLAUDE.md` or a `CLAUDE.local.md` from the directory it
  starts in and every directory above it, and a session starts wherever somebody
  is working — so a personal file can sit beside any crate, and the tree would
  have taken it. `CLAUDE*.md` matches by basename at every depth. The same
  correction applies to the directory: `**/.claude/*` reaches a `.claude/`
  beside a crate, which the root-anchored `.claude/*` did not, and neither
  tracked nor ignored is not a state that directory should be in.

  `.claude/CLAUDE.md` stays tracked through the negation, which has to come last
  — git takes the last matching pattern, so `CLAUDE*.md` below it would ignore
  the symlink instead. The negation is the declaration: a tracked file the
  ignore list refuses reads exactly like somebody's `git add -f` past it, and
  nothing in the result says which it was. It is also what makes the path
  addable at all — git consults the index while the file is in it, so the file
  survives either way, but the first add is refused without the negation, and so
  is a re-add after `git rm --cached` or after a Windows checkout writes the
  link out as a regular file.

  `AGENTS.md` is deliberately not treated the same way. A subdirectory can carry
  one, and it is repository content rather than a machine's local state.

- **The instructions are `AGENTS.md`, and `.claude/` is one symlink.** What a
  repository tells an agent belongs somewhere an agent other than Claude Code
  can find, so it is `AGENTS.md` at the root now. Claude Code reads `CLAUDE.md`
  rather than that name, so `.claude/CLAUDE.md` is committed as a symlink back
  to it: one file to edit, no wiring for anyone who clones, and nothing said
  twice. `.gitignore` takes the rest of the directory, which is Claude Code's
  working directory on a machine — local settings, local memory, plugin caches,
  session state — and never was this repository's to carry. The pattern is
  `.claude/*` and not `.claude/`, because git cannot re-include a file whose
  parent directory is excluded.

  `.claude/settings.json` went with it. It denied `git push` and `gh pr create`
  to agents, and it read as enforcement while being neither: a deny list is
  client-side and per-person, advisory where it exists and absent everywhere
  else. What can hold an agent to anything is the remote's own settings, which
  nothing local can switch off, and this file is not where their contents are
  recorded. Keep a personal deny list for the reminder; committing one and
  calling it a guard is what stops being true here. What a deny list did cover
  and a branch rule does not — `gh release`, `gh repo edit`, `gh repo delete` —
  is worth keeping in yours.

  CI's ignore list moved with the file. `AGENTS.md` is prose no step reads, and
  leaving it off both copies meant a one-line edit to it bought the workspace
  suite, a clippy pass over every crate and a rustdoc pass over all of them —
  the cost that list exists to refuse. `.claude/**` stays beside it, matching
  the symlink and any `CLAUDE.md` a checkout writes there itself.

  A clone on Windows without Developer Mode writes the symlink out as a text
  file holding its target path. `CONTRIBUTING.md` says to leave that file alone,
  since editing it dirties a tracked path, and to put a `CLAUDE.md` holding
  `@AGENTS.md` at the root instead, where the ignore list already expects one.

- **The three reviews are one plugin, and a checkout no longer carries agents.**
  `pub-checker` and `reviewer` sat in `.claude/agents/`, so cloning is how they
  reached a machine, while `doc-reviewer` was already installed from this
  repository's marketplace. They run at one moment and nobody wants two of them
  and not the third, so they ship together: `plugins/review`, installed with
  `/plugin install review@asterism`. `doc-review` is gone as a name, and anyone
  who had it installed replaces it.

  An agent stopped being something a checkout carries, so the prose that sent
  one to `.claude/agents/` went with them, in the pointer memory, in
  `CONTRIBUTING.md` and in `README.md`: a machine now has all three reviews or
  none, and when it has none the answer is to say the change was not reviewed
  rather than to review it in their place.

  The pointer memory is shorter for a second reason. Three of its bullets had
  grown into second copies of the documents it points at — which gates to run
  locally, the hand-over ordering, and the review loop before a pull request —
  and CONTRIBUTING.md carries all three. What is left is the pointers and the
  facts a reader cannot get from them.

  `pub-checker` gained the one question about a diff that is not about its
  contents. `.claude/` is Claude Code's own directory on a machine, so anything
  committed out of it arrives carrying that risk, and this repository tracks one
  path there behind a `.gitignore` that refuses the rest. Two edits can widen
  that — the ignore rules themselves, and the tracked file's own target or type
  — and `AGENTS.md`, which that file resolves to and which was itself ignored
  until this change, is the third door and is reported on any edit at all. When
  one of them is in a diff the report opens with
  `HUMAN REVIEW REQUIRED — <path>`, quotes the before and after, says what could
  now be committed that could not be before, and asks a human to confirm in
  their own words that they asked for it. Being small, obviously correct, or a
  revert does not exempt it, and neither an earlier turn nor the handed-over
  task counts as the answer — those are what a wrong edit there would come from.

  The last of it is where the reviews stop. `reviewer` said there is no round 3
  and then offered, in the next sentence, to split a branch that needed one into
  another issue — and that is the exit an agent takes, four rounds deep, for
  defects it could have fixed that hour. Both agents now stop the same way
  instead: a finding landing where an earlier round already edited says the
  design is what is wrong, so the report opens with `DESIGN REVIEW REQUIRED`,
  names the one thing to settle, and leaves it to a human. `doc-reviewer` had no
  rounds at all — it owns the prose, so the churn came out of it while the only
  stopping rule sat in the agent that had recused itself — and it now reads
  `git log -p origin/main..HEAD -- <file>` to see whether the passage it is
  quoting is one this branch has already rewritten.

- **`GET /teams/{team_id}/events` answers with a page, not the whole stream**
  (#149). The response is now an object — `{ "events": [...], "next_after": N }`
  — and takes `?after=<seq>&limit=<n>`, defaulting to 100 and clamping at 500. A
  call with no parameters returns the first page rather than everything, which
  is a breaking change for anything reading the array this used to return.

  It is worth one because the previous shape had no bound at all: a ledger only
  grows, every team-scoped mutation appends to it, and the response size was
  therefore a function of how long the team had existed. The cursor is a keyset
  over `seq`, which `(team_id, seq)` already orders, so a page costs the same
  wherever in the stream it falls and never shifts under a reader while appends
  land above it. `next_after` is `null` only when a page came back shorter than
  the limit it asked for; a page that filled its limit carries a cursor even
  when it happened to end at the last event there is, because whether anything
  follows is only answerable by asking. Paging this before `forge.*` events
  start landing is cheaper than paging it afterwards.

- **The instance capacity is an admin, not an operator** (#148 revisions 7 and
  8). `InstanceOperator` is `InstanceAdmin`, `TeamAuthority::Operator` and
  `LedgerActor::Operator` rename with it, and `user_account.is_operator` becomes
  `is_admin` by column rename. "Operator" was carrying several meanings at once
  — this capacity, the agent that carried a write out, and the human who runs
  the deployment — and this is the one with a single writer and no wire format
  pinning it.

  Two of those surfaces are visible to a caller. `SessionDto.operator` is now
  `admin`, and `actor_kind` on the wire reads `"admin"` where it read
  `"operator"`. Events written before the rename keep the old tag in storage
  forever — `ledger_event` is append-only, guarded by triggers — so the domain
  carries `#[serde(alias = "operator")]` permanently on the read side. Writes
  emit `admin`; no later migration may assume the old tag has gone.

- **An instance may hold more than one admin** (#148 revision 8). The bootstrap
  path refused a second admin on the ground that the capacity had exactly one
  holder. A single holder is a person who can be unavailable, and an instance
  whose only admin is unreachable has no path back to its own destructive verbs.
  `bootstrap-admin` is still how the _first_ admin arrives on an instance with
  no account to authenticate as; it is no longer a limit on how many there may
  be. The duplicate-login refusal is untouched.

- **`SubjectRef::ForgeIdentity` carries the forge's vocabulary rather than an
  opaque string** (#148 revision 4). It is now the pair #102 fixed — what the
  handle stands for (`owner`, `subject`, `unrecorded`, `server`) and, for the
  one kind that names somebody, whom. The canonical string the index and the
  wire carry is the bare word, or `subject:<token>`, and a value outside that
  vocabulary is refused at the boundary instead of stored and puzzled over
  later. Nothing on a production path had ever written one, so no rows migrate.

### Added

- **The last two verbs of #136's debt gain commands, and the parity debt hits
  zero**: `train_tag_head` (enqueue a `HeadTrain` run over the rulings under the
  bound encoder — no input, the corpus is every ruling) and `pull_tag_head`
  (enqueue a `HeadPull` install of a head artifact the caller fetched;
  verification is the job's, promotion applies on the next launch). Commands
  only — #130's model panel is the screen that will invoke them, and the
  artifact stays opaque JSON on this side, so the local plane still carries no
  dependency on the teams crates and no registry credential. The `http` module
  doc's count is re-measured both ways: 20 routed handlers without a same-name
  command, every one a sanctioned difference; 167 of 178 commands with one.

- **The maintenance verbs reach the desktop, with a place to press them**
  (#136). Four Tauri commands — `rebuild_index`, `rescan_duplicates`,
  `remeasure_dims` (the route's two shapes: asset ids overwrite, a scope fills
  blanks unless it is `all`), and `organize_by_location`, the only one whose
  service takes an attribution argument — and a Maintenance section in the
  settings panel that invokes them. The three job verbs report into the existing
  jobs ticker with no new wiring; the organize backfill runs synchronously and
  reports its summary in place, with the multi-minute cost on a large library
  stated in the hint. `OrganizeByLocationCommand` / `OrganizeByLocationResult`
  enter `bindings.ts` now that a screen consumes them.

  This answers #136's open screen question for these four — a settings section,
  not a screen of their own — and shrinks the parity debt in `asterism-server`'s
  `http` module doc to two: `train_tag_head` / `pull_tag_head`, whose screen is
  #130's model panel. Both directions of the count are re-measured: 22 routed
  handlers without a same-name command, 165 of 176 commands with one.

- **A forge handle remembers the name it was minted under** (#148 revision 9).
  `forge_actor` gains a nullable `display_name`, written when the row is minted
  and not afterwards — the same captured-not-referenced discipline as the teams
  plane's actor stamp, and held by the same `ON CONFLICT DO NOTHING` that makes
  minting idempotent. A caller with no name to state mints a row without one.
  The attribution triple has no display name to give today, so this is the seat
  rather than the feature: a caller that has one writes it at mint and needs no
  migration to start.

- **What erasing a person costs is written down** (#148 decision 21). The
  `ledger` module doc now names the three records that have to answer together —
  the actor stamp on every event, the subject index, and `forge_actor` on the
  local plane — the mechanisms that could answer, and the order constraint: this
  is settled before any tamper-evidence chain exists, because after one, erasure
  by rewriting a row is gone permanently. The ledger's retention is stated in
  the same spirit: keep-all for v0, named as a position with a cost, which is
  that `VACUUM INTO` rewrites the whole file on every backup.

### Removed

- **`port::share`** (#148 revision 3). An empty marker trait with no implementor
  and no reference anywhere in the workspace, reserved for a vocabulary #63 has
  not fixed. A seam that holds nothing is not holding a place.

### Added

- **Ten more of the socket's verbs reach the desktop** (#136): series-strategy
  CRUD (`list_series_strategies`, `create_series_strategy`,
  `update_series_strategy`, `delete_series_strategy`), tag administration
  (`rename_tag`, `delete_tag`, `merge_tags`), the observation timeline
  (`list_observations`, `list_streams`), and `asset_declare_source_type` — each
  a thin twin of its route, the writes attributed from
  `AttributionContext::owner_surface()`, with `SeriesStrategyService` and the
  observation store newly wired into `AppState`.

  The count in `asterism-server`'s `http` module doc is re-measured in both
  directions: 26 routed handlers have no command of the same name — down from 34
  by nine implemented here and `fetch_visual_model` retired with the model-fetch
  prototype, back up by `train_tag_head` / `pull_tag_head`, which landed with
  #132 after the last count. Of the 26, six remain debt — the four maintenance
  verbs #136 deferred plus those two head verbs; `declare_asset_source_type`
  moved to the alias twins (`asset_declare_source_type`). `get_setting` comes
  off the list by decision rather than by implementation: `list_settings`
  returns every registry key fully resolved and the write verbs return the
  resolved row, so a single-key IPC read would be a second way to ask an
  answered question.

- **The forge reaches the desktop.** Twenty-five Tauri commands covering the
  forge's twenty-eight routes, with the three forge services reaching `AppState`
  — twenty-five rather than twenty-eight because the four `about` reads collapse
  into one, for the reason below. The forge's routes shipped over three pull
  requests without a single command, because each of those issues listed "the
  desktop" as out of scope and nothing added them up.

  This closes the forge's share of a wider gap rather than the gap. Counted the
  direction that goes short — routed handlers with no command of the same name —
  the tree still has 34, of which nine are the same job under another name and
  nine are things a person never invokes (process controls, byte-serving routes,
  diagnostics). The other sixteen are verbs a person would reach for and cannot:
  series-strategy CRUD, `rename_tag` / `delete_tag` / `merge_tags`,
  `rebuild_index`, `rescan_duplicates`, `organize_by_location`,
  `remeasure_dims`, `list_observations`, `list_streams`,
  `declare_asset_source_type`, `fetch_visual_model` and `get_setting`.

  Two differences from the routes are by design. Attribution comes from
  `AttributionContext::owner_surface()` rather than from command fields, because
  the desktop's IPC is the owner's surface rather than a caller making a claim.
  And the id is an argument rather than a path segment, so the command struct's
  own id field goes unread.

  The four `about` reads are one command rather than four. Those exist as four
  routes because a _path_ cannot express a wrong id combination — a property of
  routes, not of the question — so over IPC the anchor arrives as a kind plus
  optional ids and a wrong combination is refused at runtime rather than being
  unwritable.

  The rule itself is now in RustDoc, stated once in `asterism-server`'s `http`
  module with the two other transports pointing at it — including the count
  above, so the debt is visible where the rule is, and what MCP owes, which is
  nothing: it is a curated vocabulary rather than a projection of the routes.

- **The conversation's verbs are on HTTP** (#122). Opening a conversation about
  something in the forge, saying something in it, correcting what was said,
  naming it, and reading it back from any of the four things it can hang off —
  nine routes, on the conventions the line's and the pursuit's verbs
  established.

  `about` is four routes rather than one taking a discriminator, because an
  anchor has four variants of three different arities: one query-string form
  would need a different set of required parameters per value, and no router
  refuses a wrong combination. Four paths, each carrying exactly the ids its
  anchor needs, leave the wrong combination nowhere to be written.

  `get` answers with every message and every correction, not with what each
  message says now — a correction the reader does not see leaves a withdrawn
  sentence attributed to whoever withdrew it. `amend` answers with the
  correction it appended rather than the message as it now reads, which is the
  same distinction from the other side. And an anchor is resolved rather than
  accepted: the service reads the pursuit or the line and the model builds the
  anchor, so work nobody opened is a `404` while an entry the round never
  touched is a `400`.

### Fixed

- **A landing says which work it came out of, and now the schema checks it**
  (#119). `change_point` names four things and only `line_id` was a key:
  `from_work`, `by_node` and `parent_id` were bare `BLOB`s. Migration V102
  rebuilds the table with `from_work` keyed to `pursuit` — composite, on
  `(line_id, from_work)`, so the work has to be on _this_ line rather than
  merely exist somewhere — and `by_node` keyed to `pursuit_node`.

  These two were the references nothing checked at any level. A parent the line
  never had is refused inside the write and again by the read; `from_work` and
  `by_node` were handed straight through, so a row naming work that does not
  exist came back looking like a landing out of nowhere.

  `parent_id` stays bare, which is this issue's question answered rather than
  carried again. A parent is either the genesis or a change point, and the
  genesis is columns on `line` rather than a row — giving it a row to point at
  means one whose `from_work` and `by_node` are both NULL, costing those two
  columns their `NOT NULL` for every real row, to duplicate a check two layers
  already make. `pursuit.base_id` and `pursuit_node.parent_id` are the same
  shape and stay bare for the same reason.

  A database whose rows the new keys would not hold does not migrate: the step
  asks `foreign_key_check` before it lands and takes the rebuild back with it.

- **Nine refusals answered with the wrong status.** Three of them —
  `this line is archived`, `archive it first`, and work still open against a
  line being dropped — each name a state change after which the identical
  request goes through, which is a `Blocked` conflict. They answered `400` and
  now answer `409` with `reason: "blocked"`, on the two routes that reach them —
  `close` for the archived line, `discard` for the other two, and each says in
  its message what to change. Six more were the caller addressing something that
  is not there — a parent dir when a dir is created, a target parent dir when
  one is moved, a target dir when a group is filed, either group named in a
  link, a change point on a line, and a round in a pursuit — and answer `404`
  rather than `400`.

  Four of those six needed their query split as well as their status changed. A
  single verdict covered both "it belongs to another persona" and "it is not
  there", so one caller had to fix a request while the other had to look
  somewhere else, and both were handed the same sentence. Presence is asked
  first at all four now, and the persona mismatch keeps `400` with a message
  that says only that.

  The three `Blocked` ones were left alone twice before this. First on the
  argument that none of them is _waiting_ — true, and not what `Blocked` asks;
  then because moving them changes the status two routes answer with, which is a
  cost rather than a reason.

- **`POST /asterism/forge/threads` refuses an id the anchor kind has no use
  for.** Naming `"round"` while passing an entry id is now a `400`, where it
  opened a conversation about the round — an answer to a question the caller did
  not ask. The four `about` reads get this from their paths, which have nowhere
  to put the extra id; this route takes the anchor in a body, and so does the
  desktop's one `about` command, so the check is made where the anchor arrives.

- **A conflict's retry token is decided in one place.** `DomainError::reason()`
  answers which refusals carry retry advice and what it is; the HTTP, MCP and
  desktop surfaces each used to answer it separately, which is three chances to
  disagree about something a client acts on. `ConflictKind::worth_retrying` is
  gone with it — it had no caller and could not get one, since every client of
  this is TypeScript or an agent reading JSON.

- **A pursuit the store could not have been given is now refused over a port,
  not only in a unit test.** Reading work replays `Pursuit::push` and
  `Pursuit::end` the way reading a line replays `History::record`; the line half
  was pinned over a port and the work half was not, leaving "the way" a claim.
  Dropping the marking makes a corrupt row answer `Conflict { Settled }` —
  telling a caller that a row which could never have been written is already
  decided.

- **A repository no longer decides what a refusal means to a caller** (#122,
  carried from #121). `DomainError` has four shared variants and nothing said
  which one a refusal belongs to, so the choice was made at each of the 58 sites
  that raised a `Conflict` — 39 of them inside `asterism-infra`, where a SQLite
  repository was answering an API question. #121 did not cause this; giving
  `Conflict` a kind turned a vague `409` into advice a client acts on, and made
  the wrong answers reachable.

  The four definitions are now written into `DomainError`'s module doc, and
  `asterism-infra` has its own vocabulary for what storage did — `Absent`,
  `CorruptRow`, `UniqueViolation`, `PreconditionUnmet`, `StaleWrite`,
  `AlreadyDecided`, `Impossible` — with one hand-written conversion whose
  RustDoc is the table a new refusal is settled against. A test refuses any
  repository that names a `Conflict` variant directly, so the conversion stays
  the only door.

  Twenty-one answers change, and each is a contract change:

  - A **reply** naming a message of another conversation: `409` → `400`.
  - **Correcting** a message of another conversation: `409` → `400`. The same
    reading, and a separate verb — a sentence about replies does not cover it.
  - A directory **moved into itself**: `409` → `400`.
  - A group **containing itself**: `409` → `400`.
  - **Every stored value that will not decode**: `400` → `500`. Fourteen sites —
    a group kind, a decoder token, a tag-evidence disposition, an asset role
    (three sites), a fold policy, four measurement statuses, a duplicate axis, a
    fold exclusion and an edge kind. The same `parse` serves a caller's argument
    and a column, and only the caller's argument is the caller's to fix; the
    column side had been inheriting the request side's answer. One of these sat
    three lines below a corrupt-blob check that already answered `500`.
  - A **query group's rule naming the group itself**: `409` → `400`. It had been
    taking the cycle branch, which told the caller to break the cycle at one of
    its other references — there are none.
  - A **query reference cycle** reached through `set_query_json`: `400` → `409`.
    The same cycle reached through `link` already answered `409`, which is the
    one-situation-two-answers this change exists to remove.
  - **Ruling on a tag suggestion that does not exist**: `409` → `404`. One
    refusal had covered both "absent" and "already ruled" because the update
    could not tell them apart; it asks now, and "already ruled" stays `409`.

  The first four and the query-group one are requests nothing contends with: the
  caller addressed one thing and described another, and no state change makes
  that hold. The decode failures are the opposite — the request was fine and the
  row was not.

- **The pursuit's nine verbs are on HTTP** (#121). Opening work against a line,
  writing a round, letting the line's rule answer what it collides with, ending
  it, and the four reads a screen needs beside those — all under
  `/asterism/forge/`, on the conventions the line's verbs established: an act is
  a path segment, the id comes from the path, and a write answers by reading its
  subject back, so no caller is left holding a value it knows is stale.

  Three things the issue left open are decided here. `resolve` answers 200
  whether or not the rule wrote a round, because a rule that leaves a collision
  to a person is an outcome rather than a failure; the body says which happened
  and carries what is still colliding either way. A pursuit is one read rather
  than the line's two, because it is an opening, a few rounds and at most one
  close. And an operation names its entry even when it adds one, so that a round
  which forks an entry and then fills the fork can name it twice.

  No forge type reaches `bindings.ts` yet: no screen imports one, and a binding
  written before the screen that shapes it is a guess.

### Fixed

- **A conflict now says which kind it is, so a caller knows whether asking again
  is worth anything** (#121). `DomainError::Conflict(String)` was every conflict
  at once. Work that had already ended and a close that lost a race to a landing
  reached the caller as the same thing with different prose — so a client could
  retry all of them and loop forever on the ended one, or retry none and give up
  on the race it would have won.

  It carries a `ConflictKind` now, and the kind is what the caller does next:
  `Raced` (something landed between the read and the write; the same request may
  win next time), `Blocked` (the same request works once something else changes,
  and the message says what), `Settled` (already decided; retrying is always
  wrong) and `Clashes` (conflicts with something already there; a different
  request works). Every conflict site in the codebase was sorted by the state it
  describes, and a few messages were rewritten to match the kind they carry.

  All three surfaces carry it as a `reason` token beside `kind` — HTTP, MCP, and
  the desktop, whose `UiError` keeps the `{ kind, message }` shape it always had
  and gains one field on conflicts. Every conflict is still a `409`: each really
  is a clash with the current state, and the status was never the thing that
  could separate them.

- **A record the store should not have been holding no longer reads as the
  caller's fault** (#121). Reading a line or a piece of work replays the rules
  that writing enforced, so a row that could not have been written does not come
  back — which is right, and which was arriving as though the caller had done
  something. Giving conflicts a kind made that dangerous rather than merely
  untidy: a forked chain would have told a client the read was a race and worth
  trying again, about a row that reads back the same way forever.

  Those refusals are now `ForgeError::Unwritable`, and they answer as
  infrastructure. The refusal underneath travels with them, so the message still
  names the invariant the row broke. Nine places in the SQLite adapter that
  described a stored row and answered `Validation` — every one of their messages
  begins "a stored" — answer the same way now, which is what that crate's own
  convention already said they should.

  Nothing a caller sees over SQLite changes: that adapter was already flattening
  these into infrastructure errors, discarding the model's answer rather than
  reading it. What changes is that the model says it, so the in-memory store and
  any store written later say it too.

- **The guard on the forge's boundary reads syntax instead of lines, and was
  blind to the half of its subject somebody would actually write** (#121).
  `forge_boundary.rs` keeps the forge liftable into a crate of its own: it
  collects what forge code names outside the forge and refuses anything not on a
  small allow-list, and it separately refuses the domain half reaching up into
  the application layer. It matched text — a line beginning `use crate::`, minus
  any line containing the word "forge" — and every application-side forge path
  contains that word, so `use crate::application::forge::…` was thrown away
  before the comparison. The filter was meant to skip the forge naming itself,
  and could not tell that from the forge's model naming the forge's own service,
  which is the coupling that gets written by accident.

  Three more ordinary shapes went through it, all demonstrated against the tree:
  `pub use`, a `use` that rustfmt had wrapped across lines, and a renamed
  import, which was recorded with the rename attached and so would have reported
  an allow-listed word as coupling. A whole file of tests was measured as
  production code, because the `#[cfg(test)]` that brings such a file in sits in
  the parent.

  It now parses with `syn`, expands the `use` tree through its groups, renames
  and globs, follows the parent's declaration to know which files are tests, and
  additionally catches a `crate::…` path written out where it is used rather
  than imported — which the text scan never looked for at all.

- **A round written by a rule is asked whether its content exists, the same as a
  round written by a person** (#121). `PursuitService::push` asked the boundary
  about every operation before writing; `resolve` handed what the rule produced
  straight to the pursuit log, so a rule's operations went in with the boundary
  never asked.

  Nothing had gone wrong, and only by accident of who writes the rules: the five
  that ship reuse content the divergence already named. The trait is open,
  though, and a deployment carrying a rule that minted a reference would have
  written it unverified — refused by the SQLite foreign key as an infra error
  rather than as something a caller can act on, and refused by nothing at all in
  memory.

  `resolve` can now answer with a refusal where it previously wrote. It is the
  refusal `push` already gave, worded the same way, and it leaves the round
  unwritten with the collision still standing. The two verbs share the check
  rather than the write: a rule's round is recorded as the server's, and routing
  one verb through the other would have stamped it with the caller instead.

- **The forge asks whether content exists, not whose it is — so one person's
  work can name another person's asset** (#120). `boundary::Store` asked
  `owns(persona, asset)` and `PursuitService::push` took a `PersonaId` to answer
  it with. Two things were wrong with that, and building a second surface over
  the forge is what made both visible.

  A line carries no owner — `Lines::list` says so, grouping and access are
  outside the forge — so "real but belonging to somebody else" was not a reason
  a reference was unusable, and refusing it made the case a shared line exists
  for impossible to express: private work rising into something shared brings
  the content of whoever had it.

  And the check could not refuse a caller who wanted to pass. The caller chose
  both halves of the pair, a persona is a column on the asset row, and nothing
  here knew whether the caller was that persona — so naming the asset's own
  persona always succeeded. What it caught was a client that paired the two
  wrongly. It read as a guard on whose asset this is, and it was a consistency
  check on two values one caller supplied.

  `Store::exists(asset)` is the question the forge actually has, `push` lost the
  argument, and `PersonaId` left the forge's shared vocabulary with it — the
  list in `forge_boundary.rs` is one entry shorter because the forge needs less,
  which is the only reason it ever should be.

  Nothing is deferred by this. "Who" is a question the forge already asks, once,
  through `boundary::Actors`: a write carries an `Actor`, the handle is resolved
  by the side that knows what a user is, and it is a handle precisely so that it
  exists before authentication binds it and keeps pointing at the same actor
  afterwards. A persona was never the forge's word for who. Access is per line
  and outside the forge, so what governs putting content on one is who may write
  to that line — and if an owner ever had to be recorded rather than an author,
  it would be an `Actor` on the entry, resolved through the same contract.

### Removed

- **The model-fetch prototype** (#132 phase 4, closing what #126 opened). The
  app no longer downloads encoders: `POST /asterism/models/fetch`, the
  `ModelFetch` job, the registry-entry types and staged installer in
  `asterism-vision`, the `models-staging/` path, and `model-lab`'s `registry`
  verb are gone, and `reqwest` leaves `asterism-infra` with them. The redesign
  made the flow pointless twice over — the encoder becomes app infrastructure
  (bundled, phase 0) and the only thing that travels is the trained head,
  kilobytes on the instance's existing store. `model-lab` keeps
  `prepare/verify/qualify`: the provider still produces and qualifies the
  bundled encoder. The instance's registry route (#127) stays, awaiting its
  phase-3 repurposing as the head pointer.

### Added

- **A trained head travels to the team** (#132 phase 3, the last phase). The
  instance's registry — whose model-entry schema lost its only consumer when the
  fetch flow retired — now carries the head artifact itself:
  `PUT`/`GET /teams/heads/registry`, operator-published, opaque as ever, the
  envelope alone validated (the `asterism-tag-head-v1` tag, a label, the encoder
  identity a pull must be able to refuse on). The artifact is kilobytes of JSON,
  so it rides the registry row whole — no blob store involved. Existing
  model-entry rows are cleared by the migration: nothing could consume them any
  more, and the new envelope's read would have refused them anyway.

  On the member side, `POST /asterism/heads/pull` hands the fetched artifact to
  a `HeadPull` job (inline, credential-free — the caller holds the instance
  session, the app never does). The job verifies it with exactly the checks the
  startup bind runs — encoder identity, row widths, key shapes; a head that
  could not bind must not install — then installs it into the local `heads/`
  store and promotes it. Precedence needed no machinery: a pulled head and a
  winning local retrain move the same one pointer, last promotion wins, and
  rollback is re-promoting an older label.

  Labels grew a content discriminator (`head-v3-1a2b3c4d`) so a published head
  never collides with another member's local ordinals; the identical artifact
  re-pulled is a re-promote, and a different head under a taken label is refused
  rather than renamed — the label a team talks about stays attached to the bytes
  that score.

- **The app carries its encoder** (#132 phase 0). The bundled encoder is
  `siglip2-base-patch16-256-q4v` — the current model with a q4f16 vision tower
  over an int8 text tower, ~372 MB against the fp32 pair's ~1.5 GB — chosen by
  measurement, not preference. Three candidates ran the same fixture
  qualification (seed 42, 24 bases, EN+JA vocabulary): SigLIP v1 at 224/int8
  (~205 MB) failed outright — Japanese matching zero, English recall 0.11 at any
  usable floor, its 32k vocabulary against this family's 256k — and the all-int8
  pair (~412 MB) held Japanese but lost recall (0.59) to the q4f16-vision mix
  (0.79) at the same floor. The winner's floor is 0.10, one step below fp32's
  0.12, because quantization compresses the cosine distribution: the tag floor
  is now measured per model id, while the visual-edge floor measured the same on
  both builds and stays one constant.

  Binding resolves profile first, bundle second: the profile-local `models/`
  stays the override — including its ambiguous two-package refusal, which the
  bundle deliberately does not paper over — and only an empty `models/` falls
  back to the shipped encoder, found through `ASTERISM_BUNDLED_MODELS` or
  `bundled-models/` beside the executable. Placing the prepared package into the
  desktop bundle (and pointing the variable at the resource directory) is the
  packaging side's step; the recipe that produces the package is in `model-lab`,
  revision-pinned by commit so what `prepare` fetches cannot drift under the
  digests it records.

- **The promoted head scores** (#132, the scoring side phase 2 promised). At
  startup, beside the encoder, the process reads the `heads/current` pointer and
  binds the promoted head — after verifying it: the artifact must carry the
  bound encoder's exact identity, every row must be the encoder's width, and
  every key must be a tag id. A pointer that cannot be honoured is a warning and
  zero-shot scores; startup never fails over a file a person can delete.

  In the suggestion pass, a tag the head holds a trained row for follows the
  row's verdict — its acceptance probability, floored at even odds, no text
  vector needed at all — while every other tag stays zero-shot. The phase-1
  machinery pays off unchanged: the walk stamp is per-head, so binding a new
  head re-offers the whole encoded library through the ordinary batch walk (now
  also seeded at startup, beside the encode walk's seed), re-scored from cached
  vectors and never re-encoded, with rulings untouched and the superseded head's
  unaffirmed suggestions retired.

  One scale wart, carried knowingly: a trained row's probability and a zero-shot
  cosine land in the same queue. Within one tag the scale is consistent, and the
  queue sorts per asset; a screen that renders both side by side is where this
  gets revisited.

- **The tag head trains on the person's own rulings** (#132 phase 2, the tune).
  Every accepted or rejected suggestion is a labeled example, and the new
  `HeadTrain` job (`POST /asterism/heads/train`) turns them into a head: per-tag
  logistic rows over the assets' **cached** vectors — CPU seconds, never a
  re-encode. No ML crate came with it: one row is a small convex problem, and
  the trainer is a page of plain, tested arithmetic in the domain.

  Nothing is promoted on faith. A tag trains only with enough rulings on both
  sides; each trainable tag holds out part of its rulings (deterministically —
  the split is a function of stable ruling order, so reruns reproduce it); the
  candidate and the zero-shot baseline are scored on the same held-out set; and
  the run promotes only on a strict win — a tie is churn with nothing bought. A
  losing run still writes its artifact and eval, because "zero-shot is still
  better" is a result, not a failure.

  Artifacts are immutable under ordinal labels (`heads/head-v1/…`), kilobytes
  each since only trained rows are stored, and promotion is one pointer file —
  which also makes rollback a promotion of an older label. The scoring side that
  reads the pointer (once, at startup — the encoder's bind-once rule) is the
  follow-up branch: until it lands, promotion records the verdict and the
  zero-shot pass still scores everything.

- **Suggestions know which head proposed them** (#132 phase 1, the identity
  split). The encoder's identity keys the vectors; the head keys what was made
  of them. `tag_evidence` rows and the walk stamp now carry a head ref — today
  always the zero-shot head, which is exactly what every existing row was scored
  by, so the migration backfills the truth rather than a guess.

  The plumbing is the point: a trained head (#132 phase 2) will arrive as a new
  ref, and three structural properties are already in place for it. The walk's
  page selects on "not stamped under the _current_ head", so a head swap
  re-offers the whole encoded library through the ordinary batch walk —
  re-scored from cached vectors, never re-encoded. The evidence upsert lets a
  new head replace an **unruled** suggestion while a person's ruling stays out
  of every head's reach, structurally — and within one head the first score
  still stands, the pre-#132 guarantee, now per head. And after each pass,
  suggestions a superseded head made that the current head no longer affirms
  leave the queue, so a retrain cannot strand stale proposals behind.

  Behaviour today is unchanged: one head exists, the scoring is the same cosine
  over the same vocabulary matrix, at the same floor.

- **The app installs a model from a registry entry** (#126, the fetch half of
  the first serving step). What the instance now serves, the app could not yet
  consume: a package still reached `models/` by hand.
  `POST /asterism/models/fetch` takes the entry by reference (`{"url": …}`) or
  by value (`{"entry": …}` — the inline form exists because the instance's
  registry route sits behind its session gate, and neither the queue row nor
  this server should grow a credential) and enqueues a `ModelFetch` job that
  downloads each file, verifies it against the entry's digests, and lands the
  package with one rename.

  The entry stops being ad-hoc JSON: `asterism-vision` gains the typed
  `RegistryEntry` beside `ModelPackage`, `model-lab registry` authors through
  it, and the app parses with it — one type, so the two sides cannot drift. All
  of the install's filesystem half (staging, digest checks, resume, retirement,
  the final verify through the same `ModelPackage::open` the binder uses) lives
  there too, unit-tested with made-up bytes; the job handler owns only the
  network.

  The pipeline has no retry policy, so the install resumes instead: a re-run
  keeps every staged file whose digest verifies and downloads only the rest.
  Staging sits **beside** `models/`, never inside it — the binder counts every
  `models/` subdirectory holding a manifest, and a crashed install staged there
  would read as a second package and turn the feature off. Landing is a
  replacement (other packages retired, the binder's one-package rule; re-running
  an install also heals an ambiguous directory), and binding stays a restart —
  swapping models remains `clear_derived` plus restart, not a side effect of
  fetching.

- **The instance carries the model registry entry** (#126, the first serving
  step). A model package reaches a machine today by a person placing it, which
  on a shared instance leaves "everyone runs the same qualified model" an
  operational hope — while `ModelIdentity` keys every stored vector and tag
  suggestion, so two members on different models produce derived rows their
  shared mainline cannot compare. The operator now publishes the
  provider-authored entry (`asterism-model-registry-entry-v1`, out of
  `asterism-model-lab registry`) with `PUT /teams/models/registry`, and any
  authenticated account reads it back with `GET` — the first instance-scoped
  routes, outside the team gate because there is no team to gate on.

  The bytes come back verbatim. The entry is the member app's trust anchor — its
  fetch flow verifies every downloaded byte against the entry's digests — and
  the instance is transport, not an authority: the domain validates only the
  envelope (one JSON object, the `-v1` schema tag, a non-empty `model_id`),
  because typing the body here would grow the hosted plane a reading of the
  model contract that the #83 dependency rule keeps out.

  Publishing supersedes the live entry in the same transaction — one active
  model per instance is the distribution invariant, and the schema holds it with
  a unique index over a constant expression, since a unique index on the
  nullable column would not (SQLite treats NULLs as distinct). Superseded rows
  are kept, stamped: the rollback question #126 leaves open stays answerable. No
  ledger append — the ledger's streams are per-team, and instance-scope audit is
  a deliberate deferral, recorded in the migration rather than drifted into.

- **A line of work is reachable over HTTP** (#120). The forge's verbs were
  callable from inside the process and nowhere else. Under
  `/asterism/forge/lines` a caller can now open a line, list them, read one
  whole or folded, rename it, point it at a different rule, archive it, reopen
  it and drop it; `/asterism/forge/strategies` beside it says which rules this
  deployment carries.

  The verbs are acts and are spelled as acts — `POST …/{id}/archive` rather than
  a resource with a method — which is the form `/asterism/personas/archive` and
  `/asterism/assets/{id}/source-type` already use. The prefix exists because
  `/asterism/threads` belongs to the annotation surface on the raw layer, the
  same collision `CoreCtx` has between `thread_service` and
  `forge_thread_service`.

  **Every write answers with the line.** The four that move a line's description
  return nothing from the service, and a caller told only `{"renamed": true}`
  has to ask again for the name, the standing and the stamp that moved — the
  second request a screen forgets. `discard` is the exception that proves it: it
  answers with the asset ids the drop released, and after that write there is no
  record left to derive them from.

  The wire shapes are a module of their own (`asterism_contract::forge`) and the
  conversions are in `asterism-core`'s `application::mapping`, where every other
  conversion is. They stay out of `bindings.ts` until a screen imports one: that
  list is what the UI consumes, and the forge has no screen yet.

### Fixed

- **A conversation written by a clock that stepped backwards can be read again**
  (#102). A thread was read in stamp order and handed to `Thread::say` one at a
  time, so a reply kept with an earlier stamp than the message it answers was
  refused — and the thread became permanently unreadable, which is the inverse
  of what the read half is for. The three places saying a wrong time breaks
  nothing were wrong about this one. A conversation is still read in the order
  it was said, and still reads as a transcript rather than a tree: the one
  message that moves is a reply the stamps put before its parent, and it moves
  to just after it. Replies answering each other in a circle are still refused,
  because none of them can be put after its parent.

- **A drop asks the standing inside the write, as it already asked the work**
  (#102). A line is dropped from the archive, and the port held that condition
  against a `Line` the service read before the write — so a line taken back out
  of the archive in between was dropped anyway, which is the same race
  `covering` exists to refuse, one field over. Both stores read the standing
  where it cannot go stale and refuse with `Conflict`.

  The refusal for a work list that does not match is now two refusals, because
  it was telling two stories with one message. Work the drop did not name is
  work opened since the caller read the list, and that is the race: `Conflict`.
  A name that is not against this line cannot arrive by a race at all — nothing
  removes a pursuit but a drop of its line, and that line is the one being
  dropped — so it is the caller naming somebody else's work, and it answers
  `Validation`, as the model's `NotThisLine` does one layer up.

- **A pursuit's nodes are ordered by its chain, like everything else** (#102).
  `pursuit_node` carried a `seq` column, and the two places explaining it
  disagreed: the migration argued it was how the log is ordered rather than a
  copy of something else, while the row types conceded that a parent chain would
  say the same thing. The change-point side had already settled it by keeping no
  sequence and walking the links. The column and its index are gone, and both
  logs are read by the same walk — which is stricter than the sort was: two
  nodes on one parent is a log that forked, and a node the walk cannot reach is
  a log with a hole in it, where a sort would have quietly read the reachable
  part.

- **The store no longer guesses at a value it cannot read, or accepts a node the
  log never had** (#102). Two defects with one root, both found by review after
  the adapter shipped.

  The row readers coerced every enum with a wildcard arm, so an `outcome` the
  model has no name for came back as `satisfied` — work that gave up reading as
  work that landed. A `CHECK` keeps such a row out of an ordinary write, which
  is why it looked harmless; the read half is what answers for a database
  somebody repaired by hand, and answering by guessing is the one thing it must
  not do. Every arm is exhaustive now and an unknown value is a read that fails.

  And a close whose change point named a node the line never had was written and
  never readable again. The unique indexes refuse a parent used twice, not one
  that was never there, and the port takes the line id and the closing
  separately — so a closing decided against one line and committed against
  another goes in, and `restore::chain` refuses the whole history from then on.
  Both writes now check the parent against the log they are landing on, inside
  the transaction where the answer cannot go stale. Not a foreign key, because a
  parent is either the genesis or a change point, the genesis is a column rather
  than a row, and SQLite has no key pointing at two tables — giving it a row
  would mean one whose `from_work` and `by_node` are both NULL, which is the
  shape the model refuses to have as a type and no better as a table.

### Added

- **`Threads` has an implementation: what was said about work is kept** (#102).
  The model held 489 lines of conversation — four anchors, messages, corrections
  that append rather than overwrite — and the port in front of it had no
  implementation at all. No service, no adapter, nothing that could keep a
  remark. `ThreadService` opens a conversation, says something in one, corrects
  what was said and renames the thread; `V98` gives it three tables; both stores
  satisfy the port.

  `restore` gained its third door. A stored thread could not be rebuilt before
  this — `Message::new` mints an id and `Thread::open` mints one, so nothing
  could hold the ids a store kept. Messages go back through `Thread::say` one at
  a time, which is what makes a stored reply meet the refusal a fresh one meets,
  including the case a store can produce and a caller cannot: a reply kept with
  an earlier stamp than the message it answers.

  An anchor is resolved rather than accepted. `Anchor` is built from the thing
  itself so that a thread hanging off something nobody wrote is not a value
  anybody can make — and a caller has ids, not things. So `Anchored` names what
  to look for in ids, and the service reads the pursuit or the line before a
  thread exists. An entry a round did not touch is refused by the model, reached
  through the service because the service is what has the round to ask it of.

  The tables are `forge_thread`, `forge_thread_message` and
  `forge_thread_revision`, and the prefix is the point: `thread` is taken by the
  annotation surface on the raw layer, which anchors to snapshots, cards and
  query groups. Neither could carry the other's anchors without learning what
  the other layer is made of. The same collision appears in `CoreCtx`, where the
  new field is `forge_thread_service` beside the existing `thread_service`.

  Dropping a line now takes what was said about its work with it. Every anchor a
  thread can have — a pursuit, a round, an entry as a round had it, a change
  point — is something a drop deletes, so a thread left behind would be a remark
  about nothing. Over SQLite two of the four anchors are foreign keys, so a drop
  that ignored them was refused rather than wrong; the in-memory store had
  nothing to refuse it and kept the dangling thread. Both are fixed, and the
  test that says so was written failing over both.

- **A line can be archived, reopened and dropped through a service** (#102). The
  three verbs existed in the model and nowhere else, so nothing could release a
  held asset: the forge refused to let an asset it names be deleted, and there
  was no way to stop naming it. `Lines` gains `set_standing` and `discard`,
  `LineService` gains `archive`, `reopen` and `discard`, and the refusal now has
  a way out — archive the line, end the work against it, drop it, and what it
  was holding comes back as the answer.

  `discard` returns what the drop released rather than dropping and leaving the
  caller to work it out, because after the write there is no record left to work
  it out from. It is the union of both logs, which is `discard::releases`
  finally having a caller: a line holds what its chain named, its work holds
  what its operations named, and a caller adding those up itself is a caller who
  can forget the second one.

  The port takes the pursuits the drop covers, and refuses when the work against
  the line is not that set. What a caller was told a drop frees was computed
  from a list, and a pursuit opened since that list was read is content the
  answer left out — silently, and in the direction that leaves bytes held by
  nothing. Same shape as ending work: the store does not re-derive the decision,
  it refuses to write when the decision no longer describes what is there.

  Over SQLite the drop defers its foreign keys to the commit. Every key inside
  the forge is `RESTRICT` and `pursuit.parent_id` points at `pursuit`, so work
  filed under work is a chain that no single ordering of deletes answers.
  Deferring keeps the one check that matters: a reference into the line from
  outside it still fails, and fails the whole drop.

### Changed

- **A close that loses its parent is decided again inside the write, once**
  (#102). `PursuitService::close` used to loop five times: read the line,
  decide, write, and on a conflict read again — with `ATTEMPTS = 5` as the
  number of times a caller was willing to lose before the whole thing came back
  as a conflict. Every attempt decided outside the write, so every attempt could
  lose the same way, and the bound was the only thing standing between a busy
  line and a caller waiting forever.

  The decision still happens outside the write, where it belongs. What changed
  is what happens when the write refuses: `Closings::commit` now takes a
  `Deciding` alongside the closing, and asks it for a second answer against the
  two logs as the write finds them — under the transaction that already holds
  the write lock, where nothing can arrive between the read and the write. That
  attempt is final. There is no loop and no number: either the parent is free,
  or one re-decision settles it.

  Two refusals reach the second decision and are named as such beside the port:
  a change point already where this one would go, and a node already where this
  ending would go — somebody landed on the line, or a round arrived on the work.
  Everything else is final, including work that has already ended, which is the
  one refusal that asking again cannot change. Both stores implement that
  division rather than each holding half of it.

  The port lost its `on` parameter with the loop. It named the head a caller
  decided against, and nothing compared it to anything —
  `UNIQUE (line_id, parent_id)` refuses a taken parent as part of the insert,
  and the closing already carries the node it names. In SQLite an attempt is a
  savepoint, so an ending written before the change point that refused comes
  back out before the second decision is made; the in-memory store gets the same
  property from holding its lock across both.

- **`WorkLog` is gone; a pursuit is its own chain** (#102). It held `open`,
  `rounds` and `close`, nothing outside a `Pursuit` ever held one, and all six
  of `Pursuit`'s verbs were straight delegations to it — an indirection that
  named a thing rather than being one. The fields sit on `Pursuit` now, `push`
  and `end` keep the invariants they always did, and the accessor for the
  opening node is `opening` because `open` is the verb that makes a pursuit and
  one name cannot be both.

  The name is the other half. Everything in the forge is a log: a line has a
  chain of change points, a pursuit has a chain of rounds. Calling one of them
  "Log" says only "this is a record", which inside the forge distinguishes
  nothing — and "Work" said nothing `Pursuit` was not already saying. The prose
  went with the type: forty-three places said "work log" where they meant a
  pursuit.

  The forge's tables carry the model's words for the same reason: `pursuit`,
  `pursuit_node` and `pursuit_op`, where V96 first wrote `work`. That name
  existed to avoid colliding with the first model's `pursuit` table, which V95
  drops two steps earlier.

### Added

- **The application builds the forge it uses** (#102). `init_core` constructs
  `SqliteForge`, `SqliteStore` and `SqliteActors` over the connection everything
  else shares, and hands `LineService` and `PursuitService` to `CoreCtx`.
  Neither has a transport, so a test is their only caller — which is the point:
  a service wired to the wrong store compiles and passes every test that builds
  its own world, and the only thing that catches it is landing work on a line
  the same process then reads back.

- **A purge the forge is blocking says so** (#102). Deleting a persona cascades
  to its assets, and an asset a line names cannot go — so the purge already
  refused, with a foreign-key error that names a column and tells nobody what to
  do. It now refuses with how many assets are held and which lines hold them,
  and with what releases them: dropping the line, since taking an entry off does
  not. Asked inside the same transaction as the delete, like the live check
  beside it, so the answer cannot go stale between saying it and acting on it. A
  persona the forge is not holding purges exactly as before.

- **The forge's two questions have real answers** (#102). `SqliteStore` answers
  whether a persona holds an asset — trashed included, because trashing is
  reversible and the row is still theirs, and reading that stamp here would make
  an operation legal or not depending on what somebody had tidied away that
  morning. `SqliteActors` answers what a handle stands for, over a `forge_actor`
  table (`V97`) minted on first sight. Four kinds: the two an `Author` has, one
  for a write that named nobody, and one for the instance itself, which is what
  a line's rule writes as. Keyed on the author and nothing else — an agent
  acting for somebody does not make a second somebody. Three of the four carry
  no subject, and SQLite counts NULLs as distinct in a unique index, so the
  index is over `COALESCE(subject, '')`; a plain one admits a second owner.

- **The forge has a store, and the scenario runs over both** (#102). `V96` adds
  the six tables the in-memory store was written in — `line`, `change_point`,
  `change_row`, `work`, `work_node`, `work_op` — and `SqliteForge` satisfies
  `Lines`, `Pursuits` and `Closings` over them. One adapter rather than one per
  port, because a close writes a change point, its rows and an ending together
  and two adapters sharing a transaction only reads as sharing when they are the
  same object. Taking a domain value apart and putting one back moved to
  `asterism_infra::forge::rows`, which both stores use, so the SQLite tables can
  be read as owing what that module already says.

  **The concurrency control is the write's own constraint.** Two nodes on one
  parent is a fork, which both logs refuse in the model, so
  `UNIQUE (line_id, parent_id)` and `UNIQUE (work_id, parent_id)` refuse it as
  part of the insert — nothing reads a head and compares. A second ending needs
  its own partial index, because it sits on the first, which is a parent nobody
  has used. Telling one violation from another is done on the exact column list
  SQLite reports, because `work_node.work_id` is the second ending and is a
  _prefix_ of `work_node.work_id, work_node.parent_id`, which is a fork — so a
  substring test asked about the ending matches the fork, and reports "this work
  has already ended" when somebody merely pushed a round first. The two are kept
  apart at the one place the difference is available: a fork is answered by
  reading again, and an ending already there is not answered by anything.

  **`content` restricts `asset`, on both logs.** An asset a line or a work log
  holds cannot be deleted, and purging the persona that owns it is refused at
  the same edge. Taking the entry off the line does not release it — undoing a
  removal is adding that entry back, and that needs the content to be there.
  What releases it is dropping the line, which the model answers with
  `discard::releases` and no port has a verb for yet.

  `forge_over_ports_e2e` now runs its scenario over both stores from one body.
  Two disagreements surfaced doing that and both were the in-memory store being
  wrong: it refused an abandoned close because the line had moved, though an
  abandoned close puts nothing on the line and the model refuses one for nothing
  but the wrong line or a second ending; and it compared a head where the index
  compares a parent. Both now state the rule the schema states.

- **A line has a standing, and says what it holds** (#102). The model had no
  answer to "this line is finished with" and no answer to "what would be lost if
  it went", which left the layer below with no way to protect the bytes a line
  points at. `Standing` is `Open` or `Archived` — beside the name and the
  strategy rather than in the chain, for the reason a rename is not a change
  point — and an archived line refuses `record`, so nothing lands on one and no
  satisfied close reaches one. Giving up still works: work against a finished
  line can close abandoned, because that puts nothing on it.

  `Line::holds` is every content any change point on the line has ever named,
  and `Pursuit::holds` is the same for a work log. **This is what the layer
  holding the bytes may not let go of.** A line says what is on it _now_ —
  alive, under this name, at this content — so a line pointing at bytes somebody
  deleted is a line lying about the present. That is different from a log of
  past events, which stays true whatever happens to what it names, and it is why
  the ledger this model replaced could name an asset without holding it.

  Taking an entry off the line releases nothing: the change point that put it
  there is still in the chain and still names it — and it has to, because
  undoing a removal is adding that entry back, which needs the content to still
  be there. So the set only grows, and the one thing that shrinks it is dropping
  the line. `Line::may_drop` holds that rule — dropping is reachable only
  through the archive, as purging is reachable only through the trash everywhere
  else here, and a line with work still open against it refuses with the count.

  What a drop releases is asked as one question, `discard::releases`, rather
  than added up by the caller. Dropping a line takes the work against it, so the
  releasable set is the union of both logs' `holds` — and the half a caller
  would forget is the second one, because work that gave up put nothing on the
  line and what it named is in its log and nowhere else. Forgetting it looks
  exactly like success, which is why the union is the shape of the answer.

  **Rewriting is deliberately not a verb.** No filter, no rebase, no editing a
  change point after the fact. A filtered change point could not name the work
  it came out of, because that work asked for something else; what a filter is
  for is reachable already — open a new line and put on it what should have been
  there — and the old line is then archived and dropped.

- **The forge's model can be built back from stored values** (#102).
  `model::restore` is the one door an id comes in by. Every other constructor in
  the model mints — a line mints its id and its genesis, work mints its id and
  its opening node — which is why the read half of the ports had no
  implementation but a fake for as long as it did: nothing could hold an id
  somebody else chose. The door is one module rather than a `from_persisted` per
  type, because spread across the types there would be a piece of it on each and
  nothing that reads as the whole. What keeps it honest is that it assembles
  nothing itself: the nodes go back one at a time through `History::record`,
  `WorkLog::push` and `WorkLog::end`, so a stored chain meets the refusals a
  fresh write meets. A chain whose parents do not line up, a table that would
  leave two live entries under one name, a round after the ending — each is a
  read that fails rather than a value the model would not have written. The
  chain needs no sequence column either: a change point carries its parent, so
  the points arrive in any order and the links are walked.

- **An in-memory forge store, and the scenario run over it** (#102).
  `asterism_infra::memory::forge` satisfies `Lines`, `Pursuits` and `Closings`
  over rows under a `Mutex` — decomposing a domain value on the way in and
  rebuilding it through `restore` on the way out. That is the whole reason it
  exists: a fake that kept the domain objects answers every call correctly by
  construction and never asks whether a line can be rebuilt from what was
  written down, which is the question the read half is for. The row types are
  named for the tables the SQLite adapter will create, so it can be read as a
  specification of what that adapter owes.

  `forge_over_ports_e2e` runs `asterism-core`'s `forge_scenario` again through
  the services and that store, and adds two things the model alone cannot be
  asked: that a close which loses the race re-reads and lands on the second
  attempt, and that a stored state the model would have refused does not come
  back through the read half.

### Removed

- **The forge's first model, tables and all** (#102). The pursuit whose standing
  derived from a stream of lifecycle events, the ledger of membership gestures
  beside it, and the line whose entries moved through four verbs written one
  event at a time are gone — services, ports, adapters, transport and schema.
  The model settled in #63 keeps a line's history as a chain of change points
  carrying a table, and work as a log of rounds; nothing in the old shape can be
  read as either, so it goes rather than being carried across.

  **What disappears from the outside.** Eight HTTP routes under
  `/asterism/pursuits`, eight MCP tools (`pursuit_open` / `pursuit_view` /
  `pursuit_close` / `pursuit_reopen` / `pursuit_tx` and the three `project_*`
  reads), seven Tauri commands, and the commands and DTOs they were spelled in —
  `OpenPursuitCommand`, `ClosePursuitCommand`, `RecordPursuitTxCommand`,
  `ReopenPursuitCommand`, `OpenProjectCommand`, `PursuitDto`, `PursuitEventDto`,
  `PursuitViewDto`, `PursuitTxDto`, `ProjectDto`. The replacement has no
  transport yet: its verbs are reachable from inside the process and nowhere
  else, which is the honest state to be in until the adapter under them exists.
  No screen loses anything — the UI never had one for this, and the generated
  bindings shrink by nine types nothing imported.

  **What disappears from the database.** `project`, `line`, `line_entry`,
  `line_merge`, `line_event`, `pursuit`, `pursuit_event` and `pursuit_tx` are
  dropped (V95), children before parents because every edge in the family is
  `ON DELETE RESTRICT`. Only `project` and `line` ever had a production writer,
  and what it wrote was a project row with an empty `main` beside it — the entry
  and event tables were reached by tests alone, so no instance holds a line with
  anything on it. The persona purge loses six of its eleven ordered deletes and
  keeps the one it is named for.

  **`tests/forge_boundary.rs` now runs with no exemptions.** Its list of files
  serving the replaced model was the measure of what was left; it is empty, so
  the guard reads every line of forge code and the forge names nothing outside
  itself but the five words of shared vocabulary.

### Changed

- **PUBLIC_DEVELOPMENT.md separates classification from permission to act.** A
  new paragraph states that an ALLOW outcome only says information may appear in
  a public artifact — it grants no push, publish, or release, and coding agents
  stay barred from pushing, publishing, and opening pull requests regardless of
  how a diff classifies. An agent had read the policy's "public by default"
  stance as licence to treat remote pushes as outside the rules; the document
  now closes that reading.

- **docs/aidoc/ documents the whole workspace again.** The nineteen-crate
  `exclude` list left the workspace manifest, and CI's cargo-aidoc floor rose to
  `^0.4.0` — the release that removed the 512 KiB `llms-full.txt` lint the list
  existed to stay under (no spec publishes a size limit for that file, and its
  consumers chunk it rather than reading it whole). The teams plane, both
  plug-in planes with their adapters, `asterism-media-probe` and
  `asterism-benchgen` get generated docs back; the full inventory renders at
  638,823 bytes across 276 chunks with nothing dropped.

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

- **A model earns its place before the app trusts it** (#112). The provider-side
  half of the model split arrives as `asterism-model-lab`, a separate binary in
  the `asterism-import` category — the actor is the provider, and nothing in the
  app's dependency graph reaches back to it. Four verbs: `prepare` downloads a
  known model's towers from their official source (a compiled-in recipe table,
  because a recipe is a provider decision that belongs in code review) and
  writes the digest-pinned manifest; `verify` opens, loads and smokes the
  package exactly the way the app will, because it links the same use-side
  library; `qualify` measures it against the fixture set; `registry` authors the
  entry a future in-app fetch flow consumes. The measurement itself moved out of
  the test harness into a shared `fixtures::eval` module so the floor the tool
  suggests and the ordering CI asserts cannot drift — and its two floor
  heuristics, codified from the hand-made calls, reproduce them against the real
  model: suggested edge floor 0.9205 against the adopted 0.92, suggested tag
  floor 0.12 exactly. `convert` and `train` stay charter, stated and
  unimplemented.

- **Pixels can propose tags, and a person rules** (#112). The tag phase of the
  visual layer: after an image is encoded, a `visual_tag_suggest` job scores its
  vector against every channel Tag name's cached text embedding (filled lazily;
  a rename re-encodes because the name is the encoder's input) and writes scored
  `suggested` evidence above a measured floor — 0.12, the knee of the fixture
  precision/recall sweep, recorded with its curve. A suggestion and a person's
  tag are different kinds of row by construction: the job inserts only where no
  `(asset, tag, model)` evidence exists, so an accepted or rejected ruling is
  structurally out of a rerun's reach; acceptance is what links the tag in
  `asset_tag`, which stays the sole source of truth, and a rejection is scoped
  to the model that earned it. The detail pane shows the open suggestions as
  dashed chips with their scores — accept links, reject dismisses for good — the
  same verbs served over HTTP (`/asterism/assets/{id}/tag-suggestions` +
  accept/reject), and `/asterism/models/status` says which model, if any, the
  process bound.

- **Pixels can propose neighbours** (#112). The encoder phase lands end to end
  behind opt-ins. A `visual_similarity` edge kind joins the synthetic population
  with its own owner: the visual rebuild recomputes it from stored feature
  vectors over the whole persona history — deliberately not the ±48h candidate
  window — and each rebuild's delete is scoped to its own subset, so neither the
  windowed rebuild, the visual one, nor anything a person asserted can be
  destroyed by the other. Vectors live in a `visual_feature` projection keyed by
  the model's full derivation identity (model id, feature kind, preprocessing
  revision beside them), with failure records in the same rows so absence is the
  pending state and the extraction walk offers each image exactly once per
  model; replacing a model deletes exactly its own output. Ingest fans out a
  `visual_feature` job for image assets at fingerprint priority; a completed
  encode chains the visual edge rebuild, which materialises only a bounded top
  set above a score floor (provisional until the fixture measurements pin it).
  `RetrievalIntent::Similar` — declined since the port was cut — is answered by
  a brute-force cosine scan over the persona's vectors. The encoder itself is
  `asterism-vision`'s ONNX Runtime path behind an `onnx`/`vision` feature pair
  (a default build never downloads or links onnxruntime): it loads a
  digest-verified model _package_ — the data contract with provider-side
  preparation: two towers, a tokenizer, a manifest with per-file SHA-256,
  license, and source — owns the SigLIP preprocessing recipe as an explicit
  revision, refuses revisions it does not implement, and asserts the declared
  dimension against what the tower actually produces. A `vision`-featured server
  binds the profile's `models/` package at startup (exactly one, or none —
  ambiguity is reported, not guessed), seeds the backfill, and a profile with no
  package behaves exactly as before the feature existed: visual jobs skip,
  `Similar` declines, nothing else changes.

- **The visual pipeline gains its evaluation fixtures, inside the system**
  (#112). Grading pHash and the coming encoder needs images whose relationships
  are known because they were generated, and `PUBLIC_DEVELOPMENT.md` rules
  personal images out — but that material is consumed by nothing outside the
  system, so it is deliberately not a corpus: no directory layout, no manifest
  file, no CLI. A new `asterism-vision` crate — the model-_use_ side of the
  visual features, which the app will import; model _preparation_ stays a
  provider-side tool outside the app's dependency graph — starts with a
  `fixtures` module (behind a `fixtures` feature) that tests and benches call
  in-process: deterministic scenes whose spec is the ground truth (derived EN/JA
  tags and captions, white-rimmed shapes on one grid cell each so nothing
  occludes what a tag asserts), a seeded relation stream (look-alike, semantic
  sibling, hard negative), transform helpers for the near-duplicate variants,
  and unrelated noise and queries for the honest-failure case. An earlier shape
  of this change materialised the same scenes as an external corpus behind a
  benchgen subcommand; the design review on the issue removed that separation,
  and this entry is what remains.

- **Generator parameters are extracted from stored metadata behind their own
  port** (#19). The model and seed a run recorded sit inside free-text metadata
  values — a ComfyUI graph, an A1111 parameter line — and reading them out is a
  parser, not a mapping. Three layers now do it: an outcome vocabulary and a
  `ParamExtractor` port in the core, whose outcome is six states rather than an
  `Option` (`extracted` / `not applicable` / `absent` / `indirect` / `ambiguous`
  / `not yet`) so a value behind a graph link stays findable and a disagreement
  is refused rather than guessed; a pure A1111 line tokeniser in
  `asterism-media-probe`, with no opinion about which keys matter; and the
  judgement — which input key names a seed, what a two-element array means,
  which families are recognised — in `asterism-infra`. Extraction runs over
  stored rows without opening a file, touches neither the metadata digest nor
  its canonical form, and its values reach the C2PA manifest's custom assertion
  as `model` and `seed`, riding the prompt's own withholding switch because they
  are read out of the very blob that switch was written to contain. Workflow
  identity is not extractable from either family and is not pretended otherwise.

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
  cut from wherever the line is at that moment, and writes rounds that never
  read the line — the operation that happens most often is the one that cannot
  contend with anybody. What a round asks for only means something measured
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
  round. Five rules ship and a line names one — keep the line's version and
  carry this work's onto a new entry; keep this work's under the contested name
  and move the line's aside; put both aside and take the old entry off; write
  this work's version down and then remove it, so what was tried stays readable;
  or write nothing and leave the collision standing for somebody to answer.
  Rules say what they do, so choosing one is a choice somebody makes rather than
  a default they inherit. What a rule returns is checked rather than trusted:
  the model folds it in and refuses the rule if the collisions it was asked
  about are still there. What a rule writes is recorded as the server rather
  than as the person who asked for it.

  Who did a thing is the forge's own word now: a handle, and whether it was a
  person or the server. What the handle stands for — which authenticated user,
  which instance — is asked through a new face and answered outside, because the
  binding has not happened yet and a node that recorded today's answer would
  have to be rewritten the day the real one arrives. The cost is stated where it
  is paid: a forge node no longer records which agent carried an operation out.
  Time is asked for rather than read, alone among this codebase's services,
  because a timestamp here is evidence in a record that never moves and nothing
  orders anything by it — so a wrong one breaks nothing and misleads for good.

  Work can be discussed as well as done. A thread hangs off a pursuit, one
  round, one entry as one round had it, or what landed — the four things worth
  remarking on — and it is the forge's own rather than the annotation surface
  the layer below has, which anchors to snapshots and cards and could not learn
  these four without learning what a pursuit is. The entry anchor names the
  round as well as the entry, so a remark about one attempt does not follow that
  entry into every other pursuit it is ever carried into. Nothing is
  overwritten: a correction appends a revision and every earlier wording stays
  readable. Nothing is resolved either — whether a remark is dealt with is a
  word people use about their work rather than a shape the record has, so a
  later message says it.

  Lines can be listed, work can be found by the line it is against, and work
  filed under a larger piece of work can be found by its parent — the last one a
  plain omission, since a pursuit has named its parent since it was written and
  nothing could read it back. Ended work is in those listings, because a listing
  that showed only what is open would hide what was tried and abandoned.

  Still domain and application only: no storage, no transport. The service the
  new one replaces has been renamed `LegacyPursuitService`, since what is
  leaving should carry the qualified name.

- **JSON documents share an identity across member order** (#16). A whole
  `.json` file now declares `application/json` and takes a content-axis digest
  (`cr1-`) over a stated canonical reading: member order and inter-token
  whitespace normalised, members sorted by decoded name, and every scalar token
  copied from the source verbatim — so two files a serialiser reordered are one
  document, while `1.50` / `1.5`, `-0.0` / `0` and integers above 2^53 stay
  distinct (the collisions RFC 8785 accepts, declined deliberately: on a
  duplicate-detection axis a false positive is folded and destroys, a false
  negative costs a row). A document carrying a duplicate member name is refused
  rather than silently resolved. Two migrations rename the rows imported under
  `text/plain` and hand their refused content axis back to the fingerprint walk;
  `.jsonl` stays a record container, and JSON stays in the body cache and the
  full-text index.

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
