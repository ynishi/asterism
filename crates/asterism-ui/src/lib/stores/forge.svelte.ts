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
// Nothing here is instrumented, and no screen has existed to
// instrument; if it turns out to be wrong the layout below is what has
// to move, so it is written where somebody would look.
//
// It is why `states` and `history` are separate resources rather than
// one read of `ForgeLineHistoryDto` that a screen picks apart.
// Selecting a line loads its contents; the chain loads when somebody
// asks for it. A layout that leads with the history makes the common
// verb something you reach through a log — which is what the first
// draft of this design did, and the reason it was redrawn:
//
//   ┌─ lines ──────┐┌─ ROOT ──────────────── open · mainline-first ──┐
//   │ ▸ ROOT       ││ ● on the line │ history │      [open a pursuit]│
//   │   drafts     ││ ────────────────────────────────────────────── │
//   │              ││ key visual                                     │
//   │  archived    ││ cut 04                                         │
//   │   old-cuts   ││ board 01                                       │
//   │              ││                         7 on the line          │
//   │              ││                         3 off the line  ▸      │
//   └──────────────┘└────────────────────────────────────────────────┘
//
// Drawn as a list, rendered as a grid of thumbnails: the drawing is
// the arrangement, and what a line holds is images.
//
// `onTheLine` and `takenOff` are separate deriveds for the same reason
// the panel draws them apart. Digital-asset tooling states the
// constraint without solving it: something no longer held has to stay
// findable for the record and be visually distinct so it is not reused
// as if it were still held. One list with a flag on some rows is the
// shape that loses the second half.
//
// # Where #170's four surfaces attach
//
// A **lines panel** builds the frame above and carries a line's whole
// lifecycle (#180): the three reads, the two tabs, the button the next
// child fills, and the verbs that open, rename, re-point, archive,
// reopen and discard one. #170 put the last of those in a later child
// and named no verb for the first — a panel over lines nobody can
// create shows an empty list forever, so both landed together.
// **Working a line** fills that button: opening a pursuit, pushing a
// round, closing it, with a grid selection becoming a round's content.
// **The line verbs** sit on the header, discard among them — and its
// response is the only place the assets it released are ever named, so
// a caller that ignores the body has lost them. **Threads** anchor to
// forge work, and open with a question this catalog cannot answer for
// them. It is not only that no app-level anchor kind is a forge node:
// the two are separate aggregates with different shapes — a forge
// thread carries revisions and what was first said, an app-level one
// carries archived, role and refs. So the choice is not "teach the
// drawer one more anchor" but whether one surface can hold two records
// that answer to different fields.
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
  ForgeDiscardedDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
  ForgeStrategyDto,
} from "../../bindings";

/// What a read of one line needs to name it.
type LineArgs = { lineId: string };

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

  /// Closing ends the question rather than pausing it.
  ///
  /// Everything a line's selection produced goes with it: what is on
  /// it, its chain, and the answer a discard left. Keeping any of that
  /// across a close would mean the next open shows a derivation of a
  /// chain that may have moved in between — and it will move, because
  /// #170's second child lands rounds on it. The alternative was to
  /// re-read on open instead, which is the same work at a worse moment:
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
}

export const forgeCatalog = new ForgeCatalog();
