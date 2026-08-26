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
// The contents are a list here because a grid is the shape of the
// second open question below, not a decision this drawing gets to make
// by drawing it.
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
// A **lines panel** builds the frame above and writes nothing — the
// three reads, the two tabs, and the button the next one fills.
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
// # Two questions this does not answer
//
// **How a line's contents relate to the asset grid.** Both are images,
// but an entry is a reference to an asset plus the name the line gave
// it, and that name is the line's rather than the asset's. Whether a
// line becomes another axis the grid narrows by, or a panel with its
// own grid, decides whether the forge is somewhere you go or something
// you filter to. Left open because the answer belongs with the panel
// that has to live with it.
//
// **How much of a row a person sees at rest.** A change point carries a
// table of rows and each row up to three axes. Every axis inline makes
// a history that is mostly table; too little makes a log that has to be
// opened to say anything. `ForgePanel` currently sits at the second
// pole with no opening — it prints how many rows a change point moved
// and offers no way to see them — which is where a deferred decision
// leaves a screen rather than where one should stay.
import { api } from "../api";
import { Resource } from "./_resource.svelte";
import type {
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
} from "../../bindings";

/// What a read of one line needs to name it.
type LineArgs = { lineId: string };

class ForgeCatalog {
  /// The line whose contents are showing, if one is open.
  selected = $state<string | null>(null);

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

  /// The chain itself. Loaded when somebody asks for it rather than
  /// with the contents, per the ratio above.
  history = new Resource<LineArgs, ForgeLineHistoryDto | null>(
    async (args) =>
      api<ForgeLineHistoryDto>("get_forge_line", { lineId: args.lineId }),
    null,
    "forgeCatalog.history",
  );

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
