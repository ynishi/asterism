// The forge on this machine — the lines it holds and how each got there.
//
// This catalog is written ahead of the screens that read it (#170), and
// its doc carries the shape they share. That is deliberate: four
// surfaces attach to one frame, and deciding the frame from inside
// whichever one is built first is how it ends up shaped by that one.
// As each surface lands, what it decides for itself belongs in its own
// header — what stays here is what all of them stand on.
//
// # What the model forbids, and what follows for a screen
//
// Five properties of `asterism-core`'s `domain::forge` decide most of
// this, and each rules something out:
//
// **The history is the only record.** What is on a line is folded out
// of the chain on every ask; there is no stored copy. So `states` is a
// read like any other and not a cache to keep fresh, and a panel that
// held its answer across a write would be showing a derivation of a
// chain that has moved.
//
// **The chain does not fork.** `History::record` refuses a change point
// whose parent is not the head. So there is no branch graph to draw —
// which removes the pattern most version-history UIs are built around,
// and leaves a single ordered list.
//
// **A name is a handle only among the living.** The same `record`
// refuses a table that would leave two live entries answering to one
// name — and only live ones: the check skips entries that are off the
// line, and the fold keeps a removed entry's last name. So an entry on
// the line and one off it can carry the same name, and a screen showing
// both has to say which is which by something other than the name.
//
// **Nothing is removed.** Taking an entry off a line is a change point
// that says so, and the name and content it had stay readable. `states`
// therefore answers with entries that are off the line as well as on
// it, and a screen showing only the living ones would discard half of
// what the record is for.
//
// The model draws a further line a screen cannot: "was taken off" and
// "was never here" are different answers there, and
// `ForgeEntryStateDto` carries one boolean for both. `offTheLine` says
// what that costs.
//
// **A change point carries axes, not verbs.** Each row states only the
// axes it moved — existence, content, name. "Added" and "renamed" are
// readings of a row rather than kinds of it, so a history view chooses
// how to phrase a row rather than reading a verb off it.
//
// **There are two logs, and a change point holds two handles into the
// other one.** The history says what changed; a pursuit says what
// somebody proposes. `History` and `Pursuit` do not read each other —
// three model modules read both at once and say so — but a change point
// names the pursuit it came out of *and* the node that ended it, which
// `history.rs` keeps as separate questions: neither is derivable from
// the other without going to the other log. A pursuit names its base
// change point in return. So a screen joining the two has three ids to
// work with, not one, and can land on the close a change point was born
// with rather than only on the work it came from.
//
// # Contents first, history second
//
// Working a line is the common path: look at what is on it, open a
// pursuit, add and remove, say something, close it. Reading the chain
// is the occasional one — it answers how the line arrived where it is,
// which is a question asked after the fact rather than while working.
//
// That is an assumption about intended use and not a measurement.
// Nothing here is instrumented; if it turns out to be wrong the layout
// below is what has to move, so it is written where somebody would
// look.
//
// It is why `states` and `history` are separate resources rather than
// one read of `ForgeLineHistoryDto` that a screen picks apart.
// Selecting a line loads its contents; the chain loads when somebody
// asks for it. A layout that leads with the history makes the common
// verb something you reach through a log — which is what the first
// draft of this design did, and the reason it was redrawn:
//
//   ┌─ lines ──────┐┌─ ROOT ──────────────── open · mainline-first ──┐
//   │ ▸ ROOT       ││ ● on the line │ work │ history │[open a pursuit]│
//   │   drafts     ││ ────────────────────────────────────────────── │
//   │              ││ key visual                                     │
//   │  archived    ││ cut 04                                         │
//   │   old-cuts   ││ board 01                                       │
//   │              ││                         7 on the line          │
//   │              ││                         3 off the line  ▸      │
//   └──────────────┘└────────────────────────────────────────────────┘
//
// Drawn as a list, rendered as a grid of tiles: the drawing is the
// arrangement, and looking at what is on a line is the reason to open
// this.
//
// A tile is a thumbnail where there is one to show. Most of what a
// line holds is images and it is worth optimising for that, but *only*
// images was an assumption this catalog made and the model does not —
// a line refers to assets, and an asset is as likely to be a recording
// or a conversation. `cards` is what a tile falls back to.
//
// `onTheLine` and `offTheLine` are separate deriveds for the same reason
// the panel draws them apart. Digital-asset tooling states the
// constraint without solving it: something no longer held has to stay
// findable for the record and be visually distinct so it is not reused
// as if it were still held. One list with a flag on some rows is the
// shape that loses the second half.
//
// # Where #170's four surfaces attach
//
// A **lines panel** builds the frame above and carries a line's whole
// lifecycle (#180): the reads, the tabs, the button the next child
// fills, and the verbs that open, rename, re-point, archive, reopen
// and discard one. #170 put the last of those in a later child and
// named no verb for the first — a panel over lines nobody can create
// shows an empty list forever, so both landed together.
// **Working a line** fills that button: opening a pursuit, pushing a
// round, closing it, with a grid selection becoming a round's content.
// It landed as a tab on the frame rather than a surface beside it,
// which `ForgePanel`'s header argues for where the tabs are.
// **The line verbs** sit on the header, discard among them — and its
// response is the only place the assets it released are ever named, so
// a caller that ignores the body has lost them. **Threads** anchor to
// forge work, and opened with a question this catalog could not answer
// for them — whether one surface can hold two records that answer to
// different fields, rather than whether to teach the drawer one more
// anchor kind. `ForgeTalk` answers it, and says why where it does.
//
// # Two questions this left open, and how #180 answered them
//
// **How a line's contents relate to the asset grid.** The forge is a
// place you go, not a way you filter. The grid's facets are properties
// an asset *has* — persona, modality, tag — and a line is not one: it
// refers to assets and names them its own way, so one asset sits on two
// lines under two names. Narrowing the grid by a line would replace
// what it lists rather than filter it, and the name under each card
// would stop being the asset's. The panel has its own grid instead,
// opened from the sidebar.
//
// **How much of a row a person sees at rest.** A change point's line
// carries how many rows it moved, who landed it and when; the rows
// themselves open one point at a time. Every axis inline makes a
// history that is mostly table, and letting every point stand open is
// the same wall reached from the other side.
//
// A row is phrased from the axes it states rather than as a verb,
// because that is what the model stores — "renamed" is a reading of a
// row and not a kind of one, and a row moving two axes is a reading no
// single verb has.
import { api } from "../api";
import { mutate } from "../mutate";
import { Resource } from "./_resource.svelte";
import type {
  AssetCardDto,
  ForgeCollisionDto,
  ForgeDiscardedDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
  ForgeOpDto,
  ForgePursuitDto,
  ForgeResolvedDto,
  ForgeStrategyDto,
  ForgeThreadDto,
} from "../../bindings";

/// What a read of one line needs to name it.
type LineArgs = { lineId: string };

/// What a read of one piece of work needs to name it.
type PursuitArgs = { pursuitId: string };

/// What a conversation is being held about, as a caller names it.
///
/// Ids and a kind rather than a `ForgeAnchorDto`: that shape is what a
/// read answers with, and `ForgeAnchorDto`'s own doc says a caller does
/// not build one — the service resolves an anchor from the ids and the
/// model makes it, so an entry a round never touched is refused rather
/// than recorded.
///
/// `about` is the one field that is not an id. A conversation is opened
/// from wherever it is about — a round in the log, an entry in a row, a
/// change point in the chain — and shown in one place below, which
/// would otherwise have to work out what it is looking at from ids
/// alone.
export interface ForgeTalkAnchor {
  kind: "pursuit" | "round" | "entry" | "change";
  about: string;
  pursuitId?: string;
  lineId?: string;
  nodeId?: string;
  entryId?: string;
  changePointId?: string;
}

/// One entry as a satisfied close would leave it.
export interface ForgeProjectedEntry {
  entryId: string;
  name: string | null;
  assetId: string | null;
  alive: boolean;
  /// Whether it is this work that takes it off.
  ///
  /// Not the same as `!alive`, and the difference decides what can
  /// usefully be asked next. An entry the *line* is not holding can
  /// still be renamed by this work and the rename lands; one this work
  /// takes off cannot, because the fold gives a departing entry a row
  /// that states existence and nothing else, so a rename beside it is
  /// discarded rather than refused.
  leaving: boolean;
}

class ForgeCatalog {
  /// Whether the panel is showing.
  ///
  /// A panel rather than a facet on the grid. The grid narrows by
  /// properties an asset *has* — persona, modality, tag — and a line is
  /// not one of them: it is a thing that refers to assets and names
  /// them its own way, so the same asset sits on two lines under two
  /// names. Narrowing the grid by a line would replace what the grid
  /// lists rather than filter it, and the name under each card would
  /// stop being the asset's.
  ///
  /// `shared lines` sits beside this in the sidebar on a different
  /// reason reaching the same place — a team's lines are not this
  /// library either.
  open = $state(false);

  /// The line whose contents are showing, if one is open.
  selected = $state<string | null>(null);

  /// The piece of work being read, if one is open.
  ///
  /// A second selection under the first, and it narrows rather than
  /// replaces: work is against a line, so the line stays selected while
  /// one of its pursuits is open and the panel keeps showing which line
  /// this is work on. Nothing derives it from `pursuit.data` — that
  /// resource is empty for the moment between asking and arriving, and
  /// a screen that read emptiness as "no work open" would flash the
  /// list of pursuits back over the one being opened.
  working = $state<string | null>(null);

  /// What the last discard released, until something replaces or clears
  /// it.
  ///
  /// Held by the catalog rather than the panel because the catalog owns
  /// when it stops being true: it answers about a line that no longer
  /// exists, so selecting another, closing the panel, or dismissing it
  /// all end it — and only one of those three is a thing the panel
  /// notices. A component field would need the same clear written at
  /// each of them, which is how one of them gets missed.
  released = $state<string[] | null>(null);

  /// Every line on this machine, without its history. `get_forge_line`
  /// answers with the whole chain, which is the wrong read for a list.
  lines = new Resource<void, ForgeLineDto[]>(
    async () => api<ForgeLineDto[]>("list_forge_lines", {}),
    [] as ForgeLineDto[],
    "forgeCatalog.lines",
  );

  /// What the chain folds to: every entry it ever named, on the line or
  /// off it.
  states = new Resource<LineArgs, ForgeEntryStateDto[]>(
    async (args) =>
      api<ForgeEntryStateDto[]>("get_forge_line_states", {
        lineId: args.lineId,
      }),
    [] as ForgeEntryStateDto[],
    "forgeCatalog.states",
  );

  /// The rules a line can be pointed at, built from what this
  /// deployment carries. Read with the list, because opening a line
  /// needs one and there is no sensible default to offer: a strategy
  /// decides how the line settles a collision, which is not a thing to
  /// pick on somebody's behalf.
  strategies = new Resource<void, ForgeStrategyDto[]>(
    async () => api<ForgeStrategyDto[]>("list_forge_strategies", {}),
    [] as ForgeStrategyDto[],
    "forgeCatalog.strategies",
  );

  /// The chain itself. Loaded when somebody asks for it rather than
  /// with the contents, per the ratio above.
  history = new Resource<LineArgs, ForgeLineHistoryDto | null>(
    async (args) =>
      api<ForgeLineHistoryDto>("get_forge_line", { lineId: args.lineId }),
    null,
    "forgeCatalog.history",
  );

  /// Every piece of work against the selected line, open and ended
  /// alike. Ended work stays in the list because it is the record of
  /// what was proposed and what became of it — an abandoned pursuit
  /// puts nothing on the line and is still the answer to why something
  /// is not there.
  pursuits = new Resource<LineArgs, ForgePursuitDto[]>(
    async (args) =>
      api<ForgePursuitDto[]>("list_forge_pursuits_of_line", {
        lineId: args.lineId,
      }),
    [] as ForgePursuitDto[],
    "forgeCatalog.pursuits",
  );

  /// One piece of work, whole — its rounds and how it ended if it has.
  ///
  /// One read where the line needs two. `ForgePursuitDto`'s own doc
  /// says why: a pursuit is an opening, a handful of rounds and at most
  /// one close, so there is no split to make and no summary shape to
  /// keep in step with it.
  pursuit = new Resource<PursuitArgs, ForgePursuitDto | null>(
    async (args) =>
      api<ForgePursuitDto>("get_forge_pursuit", { pursuitId: args.pursuitId }),
    null,
    "forgeCatalog.pursuit",
  );

  /// What this work asks for that the line has already moved.
  ///
  /// A separate `Resource` from the work rather than a field folded
  /// into it, because the answer is about the pair and moves when
  /// either side does — which is also why nothing here has to be
  /// invalidated by hand. `change.rs` defines the term and says what
  /// ends one.
  collisions = new Resource<PursuitArgs, ForgeCollisionDto[]>(
    async (args) =>
      api<ForgeCollisionDto[]>("get_forge_pursuit_collisions", {
        pursuitId: args.pursuitId,
      }),
    [] as ForgeCollisionDto[],
    "forgeCatalog.collisions",
  );

  /// The landings this work has not seen, oldest first.
  ///
  /// Its own `Resource` beside `collisions` rather than a count taken
  /// off it, because they answer different questions —
  /// `get_forge_pursuit_behind` says which, and most of what this
  /// counts touches nothing the work is doing. A screen with only the
  /// other one cannot explain a close that comes back asking to be
  /// resolved first.
  behind = new Resource<PursuitArgs, string[]>(
    async (args) =>
      api<string[]>("get_forge_pursuit_behind", { pursuitId: args.pursuitId }),
    [] as string[],
    "forgeCatalog.behind",
  );

  /// What the library knows about the assets on screen, by asset id.
  ///
  /// **A line does not hold images.** It holds assets, and the first
  /// card in this repository's own fixture is a video — so a panel that
  /// draws every entry as a thumbnail draws an empty box for anything
  /// that has none, forever: `thumbById` answers with a transparent
  /// pixel while a thumb is being made *and* when there is never going
  /// to be one, and those are the same grey square on screen. An e2e
  /// waited twenty seconds at one before this existed.
  ///
  /// `media` is what settles it, and it is projected onto the card
  /// precisely so a UI stops deriving it from a mime type. The forge
  /// asks the same question the grid asks, through the same command.
  ///
  /// Kept as a plain map rather than a `Resource` because it is
  /// cumulative and keyed: entries arrive as rounds are pushed, and
  /// what has already been read stays read.
  cards = $state<Record<string, AssetCardDto>>({});

  /// Reads what it does not have yet. Safe to call on every render
  /// pass: it fetches only the ids it is missing, and answers nothing
  /// when it is missing none.
  async ensureCards(ids: (string | null)[]): Promise<void> {
    const wanted = [...new Set(ids.filter((id): id is string => id !== null))];
    const missing = wanted.filter((id) => this.cards[id] === undefined);
    if (missing.length === 0) return;
    // Ids the library will not answer for drop out of the response, so
    // a card that never arrives stays missing and the screen falls back
    // to what it can say without one.
    const got = await api<AssetCardDto[]>("hydrate_cards", {
      ids: missing,
      viewerSubject: null,
    });
    const next = { ...this.cards };
    for (const card of got) next[card.id] = card;
    this.cards = next;
  }

  /// What is being talked about, if anything is.
  ///
  /// One at a time, and held here rather than in whichever component
  /// offered it: the conversations are shown in one place below the
  /// tabs, and what they are about is opened from three different ones
  /// — a piece of work, a round or one of its entries, a change point
  /// in the chain.
  talkingAbout = $state<ForgeTalkAnchor | null>(null);

  /// The conversations about it. More than one can hang off the same
  /// thing — two people starting separate ones about a round is not a
  /// mistake to merge — so this is a list.
  ///
  /// Each arrives whole, with every message and every correction to
  /// each. That is `ForgeThreadDto`'s decision and not this store's,
  /// and it is stated there.
  threads = new Resource<ForgeTalkAnchor, ForgeThreadDto[]>(
    async (anchor) =>
      api<ForgeThreadDto[]>("list_forge_threads_about", {
        anchorKind: anchor.kind,
        pursuitId: anchor.pursuitId ?? null,
        lineId: anchor.lineId ?? null,
        nodeId: anchor.nodeId ?? null,
        entryId: anchor.entryId ?? null,
        changePointId: anchor.changePointId ?? null,
      }),
    [] as ForgeThreadDto[],
    "forgeCatalog.threads",
  );

  /// Opening the panel reads the list and the rules. Nothing reloads on
  /// a timer: the lines a person opened are not something a background
  /// write moves, and a panel that refreshed under a selection would
  /// lose it.
  async openPanel(): Promise<void> {
    this.open = true;
    await Promise.all([this.lines.load(), this.strategies.load()]);
  }

  /// Opens a line.
  ///
  /// #170 names four children and this verb is in none of them: the
  /// first is read-only and the third is rename, re-point, standing and
  /// discard. Creating a line falls between them, and its absence shows
  /// on any machine that has never had one — which is every machine
  /// until somebody runs the command by hand. A surface that can only
  /// read a thing nobody can create is not a surface, so it came here
  /// with the rest of the lifecycle (#180).
  ///
  /// Safe to press again in the sense that matters — each press opens a
  /// separate line, and `Name` carries no claim of uniqueness, so two
  /// lines may share one. That is the model's position rather than an
  /// oversight: "unique among what?" needs an owner to answer, and the
  /// owner is outside the forge.
  async openLine(name: string, strategyId: string): Promise<void> {
    await mutate(
      "open_forge_line",
      { command: { name, strategy_id: strategyId } },
      "open a line",
    );
    await this.lines.load();
  }

  /// Shows a line: what it holds, and the work against it.
  ///
  /// Every clear a move off the previous line owes is here rather than
  /// at each site that moves. A pursuit id belongs to the line it is
  /// against and a change point to the chain it is on, so none of what
  /// this store holds about the previous line survives the move — and
  /// writing that out at each call site is how one of them gets missed,
  /// which is what happened to `released` before this store's tests
  /// existed. What the panel holds is the panel's to clear.
  ///
  /// The chain is reset rather than read. It answers a question asked
  /// after the fact, so it loads when somebody asks; what would be
  /// wrong is leaving the previous line's chain under this line's name.
  async selectLine(lineId: string): Promise<void> {
    this.selected = lineId;
    // The discard notice answers about a line that is gone. Selecting
    // another one is a new question, and leaving the old answer above
    // it reads as though it were about this one.
    this.released = null;
    this.history.reset();
    // Before `clearWork`, which leaves a conversation about a change
    // point standing on the grounds that the line has not moved. Here
    // it has.
    this.stopTalking();
    this.clearWork();
    await Promise.all([
      this.states.load({ lineId }),
      this.pursuits.load({ lineId }),
    ]);
  }

  /// Everything the work half is showing, cleared as one.
  ///
  /// Called wherever the work being shown stops being the work to
  /// show — a move off the line, a pursuit replacing the last one, a
  /// person going back to the list. One method rather than a clear
  /// written at each of them, for the reason `selectLine` gives.
  clearWork(): void {
    this.working = null;
    this.pursuit.reset();
    this.collisions.reset();
    this.behind.reset();
    // Three of the four anchors name a node of the work being let go
    // and cannot outlive it. The fourth is a change point, which is the
    // line's: it was landed by work that may be long closed, and the
    // chain it sits on is still on screen. So this ends a conversation
    // about the work and leaves one about the line alone — the moves
    // that end *that* are the ones that change or drop the line.
    if (this.talkingAbout?.kind !== "change") this.stopTalking();
  }

  /// Opens work against a line.
  ///
  /// Nothing is read back about what it collides with or how far behind
  /// it is, and that is not an omission: `open` cuts the work from where
  /// the line is now, so a pursuit that has just been opened is level
  /// with the line by construction and both answers are empty. They are
  /// cleared instead, which is the same answer without the two calls.
  async openPursuit(
    lineId: string,
    title: string | null,
    note: string | null,
  ): Promise<void> {
    const opened = await mutate<ForgePursuitDto>(
      "open_forge_pursuit",
      { command: { line_id: lineId, title, note } },
      "open a pursuit",
    );
    this.clearWork();
    this.working = opened.id;
    await Promise.all([
      this.pursuit.load({ pursuitId: opened.id }),
      this.pursuits.load({ lineId }),
    ]);
  }

  /// Shows one piece of work, and what stands between it and the line.
  async selectPursuit(pursuitId: string): Promise<void> {
    this.working = pursuitId;
    await Promise.all([
      this.pursuit.load({ pursuitId }),
      this.collisions.load({ pursuitId }),
      this.behind.load({ pursuitId }),
    ]);
  }

  /// Writes a round — what this work asks the line to say.
  ///
  /// **Nothing lands here.** `push` does not read the line at all,
  /// which is what lets work go on while somebody else is finishing
  /// theirs; what a round asks for reaches the line only when the work
  /// is closed satisfied.
  ///
  /// The write answers with the work whole and this re-reads it anyway.
  /// The two are the same shape, and taking the read keeps `Resource`'s
  /// generation guard the only thing that decides what is on screen —
  /// assigning `data` behind it would let a read already in flight land
  /// on top of the round that was just written.
  ///
  /// Collisions are re-read and `behind` is not. What a round asks for
  /// is half of what a collision is made of, so writing one can make or
  /// clear one; how far behind the work is says how many landings it
  /// has not seen, which nothing this side writes can move.
  async pushRound(
    pursuitId: string,
    ops: ForgeOpDto[],
    note: string | null,
  ): Promise<void> {
    await mutate(
      "push_forge_round",
      { pursuitId, command: { pursuit_id: pursuitId, ops, note } },
      "push a round",
    );
    await Promise.all([
      this.pursuit.load({ pursuitId }),
      this.collisions.load({ pursuitId }),
    ]);
  }

  /// Asks the line's rule to answer whatever this work collides with.
  ///
  /// **Returns what the rule did rather than throwing**, because a rule
  /// that writes nothing has not failed — `react.rs` says so at the
  /// site, and lists the two ways it happens. A screen reporting either
  /// as a failure would be telling somebody to retry the one thing that
  /// cannot help.
  async resolve(pursuitId: string): Promise<boolean> {
    const answer = await mutate<ForgeResolvedDto>(
      "resolve_forge_pursuit",
      { pursuitId },
      "let the rule settle this",
    );
    await Promise.all([
      this.pursuit.load({ pursuitId }),
      this.collisions.load({ pursuitId }),
    ]);
    return answer.round !== null;
  }

  /// Ends the work, and puts what it says on the line if it says
  /// anything.
  ///
  /// The only verb on this side that moves a line, so it is the only
  /// one that invalidates what the line half is showing: satisfied puts
  /// every round on the chain as one landing, which moves the contents
  /// and the history both. Abandoned moves neither and re-reads them
  /// anyway — the alternative is a branch on an outcome to save one
  /// read, and it would be wrong the first time a landing arrived from
  /// somewhere else while this was open.
  ///
  /// `behind` is the one read not refreshed. A satisfied close puts a
  /// landing on the line that the work is then behind — its own — which
  /// is a true number about a question nobody is asking any more. The
  /// surface stops offering it for work that has ended rather than
  /// re-reading to say that.
  ///
  /// A refusal from here reaches the caller, and what to do about it
  /// is not something `mutate` reads. The reason separates retrying
  /// from asking for something else from neither; `blocked` covers two
  /// situations that want different actions and only its message says
  /// which, which `close_forge_pursuit` in `asterism-server`'s `http`
  /// module spells out.
  async closePursuit(
    pursuitId: string,
    lineId: string,
    outcome: "satisfied" | "abandoned",
    note: string | null,
  ): Promise<void> {
    await mutate(
      "close_forge_pursuit",
      { pursuitId, command: { pursuit_id: pursuitId, outcome, note } },
      "close this pursuit",
    );
    this.history.reset();
    await Promise.all([
      this.pursuit.load({ pursuitId }),
      this.collisions.load({ pursuitId }),
      this.pursuits.load({ lineId }),
      this.states.load({ lineId }),
    ]);
  }

  /// Shows what is said about one thing, and reads it.
  async talkAbout(anchor: ForgeTalkAnchor): Promise<void> {
    this.talkingAbout = anchor;
    await this.threads.load(anchor);
  }

  /// Stops showing it. Not a write of any kind — a conversation is
  /// there whether or not anybody has it open.
  stopTalking(): void {
    this.talkingAbout = null;
    this.threads.reset();
  }

  /// Starts a conversation about it.
  ///
  /// **There is no opening one empty.** A conversation is what was said
  /// in it, so the first thing said is part of opening it rather than a
  /// message written afterwards.
  ///
  /// What the anchor can and cannot be is `ForgeTalkAnchor`'s to
  /// explain; nothing here is trusted with it.
  async openTalk(
    anchor: ForgeTalkAnchor,
    said: string,
    title: string | null,
  ): Promise<void> {
    await mutate(
      "open_forge_thread",
      {
        command: {
          anchor_kind: anchor.kind,
          pursuit_id: anchor.pursuitId ?? null,
          line_id: anchor.lineId ?? null,
          node_id: anchor.nodeId ?? null,
          entry_id: anchor.entryId ?? null,
          change_point_id: anchor.changePointId ?? null,
          title,
          said,
        },
      },
      "start a conversation",
    );
    await this.threads.load(anchor);
  }

  /// Says something in one.
  ///
  /// The write answers with the message it wrote and this re-reads the
  /// list, for the reason `pushRound` re-reads the work: `Resource`'s
  /// generation guard is what decides what is on screen, and a message
  /// spliced in beside it would be a second answer nothing arbitrates.
  async sayInTalk(
    anchor: ForgeTalkAnchor,
    threadId: string,
    said: string,
    replyingTo: string | null,
  ): Promise<void> {
    await mutate(
      "say_in_forge_thread",
      { threadId, command: { thread_id: threadId, replying_to: replyingTo, said } },
      "say this",
    );
    await this.threads.load(anchor);
  }

  /// Corrects something said. **Nothing is overwritten** — this appends
  /// a revision, which is why it answers with the correction rather
  /// than with the message as it now reads.
  async amendInTalk(
    anchor: ForgeTalkAnchor,
    threadId: string,
    messageId: string,
    said: string,
  ): Promise<void> {
    await mutate(
      "amend_forge_message",
      { threadId, command: { thread_id: threadId, message_id: messageId, said } },
      "correct this",
    );
    await this.threads.load(anchor);
  }

  /// Gets out of the way without ending anything.
  ///
  /// The drawer is an overlay, so while it is up the grid cannot be
  /// reached — and the grid is where the content of a round comes from.
  /// A person who opens the forge before picking is therefore told to
  /// go and select something, by a surface that is itself what stops
  /// them. The first build of this shipped that as a disabled button
  /// with the instruction on it, and pressing it does nothing, which is
  /// the state the button existed to avoid.
  ///
  /// So: step aside, keep everything. The line, the work, what is
  /// loaded about both — none of it is a question this answers, unlike
  /// `closePanel`, which ends the question deliberately. Opening the
  /// panel again lands back on the same work with the selection now
  /// made.
  stepAside(): void {
    this.open = false;
  }

  /// Whether the panel is out of the way with work still open on it.
  ///
  /// Derived rather than a flag, so nothing has to remember to clear
  /// it: the two things that end it are opening the panel again and
  /// letting go of the work, and both are already writes to what this
  /// reads.
  ///
  /// It exists because stepping aside is only half a gesture. The first
  /// build of it closed the drawer and said nothing more, which left
  /// somebody in the grid with no sign that the forge was waiting and
  /// no way back that was not a guess. What reads this is the way back.
  get steppedAside(): boolean {
    return !this.open && this.working !== null;
  }

  /// Closing ends the question rather than pausing it.
  ///
  /// Everything a line's selection produced goes with it. Keeping any of that
  /// across a close would mean the next open shows a derivation of a
  /// chain that may have moved in between, and a satisfied close is a
  /// thing anybody can do to it. The alternative was to re-read on
  /// open instead, which is the same work at a worse moment:
  /// a panel that opens onto a stale answer and corrects itself.
  ///
  /// The team's catalog empties on `disconnect` rather than on close,
  /// and that is a different event for a different reason — what it was
  /// served through can vanish. Nothing here vanishes; it moves.
  closePanel(): void {
    this.open = false;
    this.selected = null;
    this.states.reset();
    this.history.reset();
    this.pursuits.reset();
    this.stopTalking();
    this.clearWork();
    this.released = null;
  }

  /// Renames a line. Not a landing — the history says what happened to
  /// what the line carries, and its own description is a separate
  /// record, so nothing goes on the chain and the head does not move.
  async rename(lineId: string, name: string): Promise<void> {
    await mutate(
      "rename_forge_line",
      { lineId, command: { line_id: lineId, name } },
      "rename a line",
    );
    await this.lines.load();
  }

  /// Points the line at a different rule, from here on. Also not a
  /// landing, and deliberately not retroactive: what a past collision
  /// settled to was settled under the rule in force then.
  async setStrategy(lineId: string, strategyId: string): Promise<void> {
    await mutate(
      "set_forge_line_strategy",
      { lineId, command: { line_id: lineId, strategy_id: strategyId } },
      "re-point a line",
    );
    await this.lines.load();
  }

  /// Finished with. An archived line takes no landing, and it is the
  /// only standing a discard can be reached from — so this is the step
  /// before dropping as well as a state in its own right.
  async archive(lineId: string): Promise<void> {
    await mutate("archive_forge_line", { lineId }, "archive a line");
    await this.lines.load();
  }

  /// Takes it back out of archived.
  async reopen(lineId: string): Promise<void> {
    await mutate("reopen_forge_line", { lineId }, "reopen a line");
    await this.lines.load();
  }

  /// Drops the line, its history, and every piece of work against it.
  ///
  /// **It refuses while any of that work is still open**, and the model
  /// says why where it refuses: dropping takes the history the work was
  /// cut from, so what would be left is a log against nothing. Closing
  /// the work first is the record of what happened to it, not a
  /// formality — which makes this the one line verb with a precondition
  /// the screen has to state before it is pressed.
  ///
  /// **The answer is the point.** It names the assets the forge was
  /// holding and is not holding any more, and after this write there is
  /// no record left to derive them from — so a caller that throws the
  /// response away has lost the only answer there will be. It goes to
  /// `released` rather than back to the caller, because it outlives the
  /// call: what ends it is a selection, a close or a dismissal, none of
  /// which the caller is in a position to notice.
  async discard(lineId: string): Promise<void> {
    const dropped = await mutate<ForgeDiscardedDto>(
      "discard_forge_line",
      { lineId },
      "discard a line",
    );
    this.released = dropped.released_asset_ids;
    if (this.selected === lineId) {
      this.selected = null;
      this.states.reset();
      this.history.reset();
      // Every pursuit against this line went with it, and so did every
      // change point and everything said about either. What was on
      // screen names records that no longer exist, and a read of one
      // now answers not-found rather than a list.
      this.pursuits.reset();
      this.stopTalking();
      this.clearWork();
    }
    await this.lines.load();
  }

  /// What the line holds now.
  get onTheLine(): ForgeEntryStateDto[] {
    return this.states.data.filter((state) => state.alive);
  }

  /// Every entry the chain named that the line does not hold now.
  ///
  /// **Not the same as "let go", and the wire cannot tell the two
  /// apart.** `alive` is false both for an entry a change point took
  /// off and for one a table named without ever putting it anywhere —
  /// `asterism-core`'s `table.rs` pins the second case with
  /// `an_entry_named_without_being_added_is_known_and_not_on_the_line`,
  /// because work has to be able to refer to an entry before anything
  /// has placed it. `ForgeEntryStateDto` carries one boolean, so a
  /// screen reading it can say "not on the line" and no more.
  ///
  /// That is a narrower answer than the record holds: the model does
  /// distinguish "was taken off" from "was never here", and telling
  /// them apart from here would need either another axis on the DTO or
  /// a read of the chain. Kept apart from `onTheLine` rather than
  /// flagged within it, because a record nobody can find is not a
  /// record and one that reads like contents is a mistake waiting to
  /// be made.
  get offTheLine(): ForgeEntryStateDto[] {
    return this.states.data.filter((state) => !state.alive);
  }

  /// The line as the work being read would leave it: every entry the
  /// chain has named, with the work's operations applied over it.
  ///
  /// A derived over both reads because it is a question about the pair,
  /// and neither log answers it alone — the line says what it has said,
  /// the work says what it asks for, and what a close leaves is the
  /// second applied to the first. It is here rather than on a screen
  /// because a fold nobody can see is a fold each screen writes again
  /// slightly differently, and every rule it has to get right is
  /// somebody else's.
  ///
  /// **In two steps, because the model takes two.** `op.rs`'s `fold`
  /// reduces the whole work to one row per entry *before* anything
  /// meets the line: per axis the last operation wins, and then the
  /// winning existence decides what the row says — present carries the
  /// content and name that won, absent says existence and nothing else,
  /// and an entry no operation placed keeps whatever axes were written.
  /// Applying operations to a line one at a time instead is wrong
  /// wherever a removal has an operation after it or beside it, and
  /// each of those is two presses away: remove-then-rename and
  /// remove-then-replace show what the landing will discard, and an
  /// entry added, removed and renamed inside one piece of work becomes
  /// a row no close produces. The tests hold the list.
  ///
  /// Only then does the line come in, which is `change.rs`'s
  /// `normalise`: a removal of something the line is not holding has
  /// nothing left to do, so it leaves nothing here — while one the line
  /// knows and let go stays exactly as the line has it. An entry the
  /// line has never heard of is otherwise on this list like any other,
  /// which is `table.rs`'s position rather than a choice made here: an
  /// entry appears as soon as anything names it, on the line or off it.
  ///
  /// **Not the model's answer, and it cannot be.** A landing arriving
  /// meanwhile changes what this is folded onto — most of them touch
  /// nothing this work asks for and the close lands anyway, and the
  /// ones that collide are refused. Reading it early is what lets
  /// somebody fix a name before the close tells them to.
  get projection(): ForgeProjectedEntry[] {
    // Step one: the work alone, per axis, last operation winning.
    const existence = new Map<string, boolean>();
    const content = new Map<string, string | null>();
    const named = new Map<string, string | null>();
    for (const round of this.pursuit.data?.rounds ?? []) {
      for (const op of round.ops) {
        if (op.kind === "add") {
          existence.set(op.entry_id, true);
          content.set(op.entry_id, op.content_asset_id);
          named.set(op.entry_id, op.name);
        } else if (op.kind === "replace") {
          content.set(op.entry_id, op.content_asset_id);
        } else if (op.kind === "rename") {
          named.set(op.entry_id, op.name);
        } else if (op.kind === "remove") {
          existence.set(op.entry_id, false);
        }
      }
    }

    // Step two: over the line, which is where an entry gets a name this
    // work never mentioned and where a removal of something already off
    // stops being anything at all.
    const rows = new Map<string, ForgeProjectedEntry>();
    for (const state of this.states.data) {
      rows.set(state.entry_id, {
        entryId: state.entry_id,
        name: state.name,
        assetId: state.content_asset_id,
        alive: state.alive,
        leaving: false,
      });
    }
    const touched = new Set([
      ...existence.keys(),
      ...content.keys(),
      ...named.keys(),
    ]);
    for (const entryId of touched) {
      const before = rows.get(entryId);
      const goes = existence.get(entryId);
      if (goes === false) {
        // Existence standing alone. The line goes on saying what it
        // said, minus the entry — and if it was not holding it, this
        // says nothing at all.
        if (before !== undefined && before.alive) {
          rows.set(entryId, { ...before, alive: false, leaving: true });
        }
        continue;
      }
      rows.set(entryId, {
        entryId,
        name: named.get(entryId) ?? before?.name ?? null,
        assetId: content.get(entryId) ?? before?.assetId ?? null,
        alive: goes === true ? true : (before?.alive ?? false),
        leaving: false,
      });
    }
    return [...rows.values()];
  }

  /// Names that would be on the line twice if this work landed.
  ///
  /// A line holds one live entry per name and `History::record` refuses
  /// the landing that would break it. Read from the fold above rather
  /// than from the names the work happens to have typed: the case that
  /// matters most is an added name meeting one the line already holds,
  /// and counting only what the work asked for stays silent on exactly
  /// that. Names default from filenames, and filenames repeat.
  /// Names are compared trimmed, which is agreement with `Name::new`
  /// rather than defence against anything: every name here has already
  /// been through it on the way out, so nothing untrimmed arrives from
  /// this backend. It is here so the comparison cannot drift from the
  /// rule it is predicting.
  get wouldClash(): string[] {
    const seen = new Map<string, number>();
    for (const row of this.projection) {
      if (!row.alive || row.name === null) continue;
      const name = row.name.trim();
      seen.set(name, (seen.get(name) ?? 0) + 1);
    }
    return [...seen.entries()]
      .filter(([, count]) => count > 1)
      .map(([name]) => name);
  }

  /// Work nobody has ended. The only work a round can be written to.
  get openWork(): ForgePursuitDto[] {
    return this.pursuits.data.filter((work) => work.close === null);
  }

  /// Work that ended, satisfied or abandoned.
  ///
  /// Kept apart from the open work rather than flagged among it, for
  /// the reason `offTheLine` is: a list that reads as things you can
  /// still push to, half of which refuse every write with "settled", is
  /// a list that has to be tried to be understood.
  get endedWork(): ForgePursuitDto[] {
    return this.pursuits.data.filter((work) => work.close !== null);
  }
}

export const forgeCatalog = new ForgeCatalog();
