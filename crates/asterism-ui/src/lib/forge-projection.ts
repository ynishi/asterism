// What a piece of work would leave on the line it is against.
//
// A pure fold over two contract shapes — the rounds of a
// `ForgePursuitDto` and a line's `ForgeEntryStateDto[]` — and it lives
// here rather than on either catalog because both planes ask it. The
// forge is the same on both (#148 decision 19 mirrors it path for path
// and the same DTOs come back), so the answer to "what would closing
// this leave" is the same answer, made of the same two reads.
//
// **This is not the two stores sharing state.** Decision 16 keeps
// `forge.svelte.ts` and `shared.svelte.ts` apart because their sources
// differ — one reads a service on this machine, the other is served
// through a team's server — and nothing here reads anything. It takes
// what a caller already has and returns rows. A second copy of this
// fold would be the drift the separation is not asking for: every rule
// below is one the model gets right and a screen has to predict, and
// two predictions of one rule diverge on the case nobody tested.
import type { ForgeEntryStateDto, ForgeRoundDto } from "../bindings";

/// One entry as a satisfied close would leave it.
export interface ForgeProjectedEntry {
  entryId: string;
  name: string | null;
  assetId: string | null;
  /// What the line ends up holding — `normalise`'s answer, and the
  /// second of the model's two steps.
  alive: boolean;
  /// What this work said about the entry being there — `fold`'s answer,
  /// and the first of the two steps. Null when the work said nothing
  /// about it either way.
  ///
  /// **Both steps, because the row has two kinds of reader and they
  /// want different ones.** Drawing an entry as gone, and offering to
  /// put it back, are questions about what a close leaves: `alive`.
  /// Offering a rename or a refill is a question about what a close
  /// will even look at: an entry whose winning existence is absent gets
  /// a row stating existence and nothing else, so a rename beside it is
  /// discarded rather than refused, and a screen that offers one is
  /// offering something that cannot land.
  ///
  /// One boolean answered both for a while, and it answered with
  /// `normalise`'s step. The two agree everywhere except on an entry
  /// the line is *not* holding: there a removal survives the fold and
  /// dies in `normalise`, so the verbs came back on a row where they
  /// still could not land — two presses away, `put back` then `remove`.
  /// This is the shape `forge.svelte.ts` warns about for `offTheLine`:
  /// one boolean standing for two model facts, with the screen left to
  /// guess them apart.
  stated: "present" | "absent" | null;
}

/// The line as the work being read would leave it: every entry the
/// chain has named, with the work's operations applied over it.
///
/// A fold over both reads because it is a question about the pair, and
/// neither log answers it alone — the line says what it has said, the
/// work says what it asks for, and what a close leaves is the second
/// applied to the first.
///
/// **In two steps, because the model takes two.** `op.rs`'s `fold`
/// reduces the whole work to one row per entry *before* anything meets
/// the line: per axis the last operation wins, and then the winning
/// existence decides what the row says — present carries the content
/// and name that won, absent says existence and nothing else, and an
/// entry no operation placed keeps whatever axes were written. Applying
/// operations to a line one at a time instead is wrong wherever a
/// removal has an operation after it or beside it, and each of those is
/// two presses away: remove-then-rename and remove-then-replace show
/// what the landing will discard, and an entry added, removed and
/// renamed inside one piece of work becomes a row no close produces.
/// The tests hold the list.
///
/// Only then does the line come in, which is `change.rs`'s `normalise`:
/// a removal of something the line is not holding has nothing left to
/// do, so it leaves nothing here — while one the line knows and let go
/// stays exactly as the line has it. An entry the line has never heard
/// of is otherwise on this list like any other, which is `table.rs`'s
/// position rather than a choice made here: an entry appears as soon as
/// anything names it, on the line or off it.
///
/// **Not the model's answer, and it cannot be.** A landing arriving
/// meanwhile changes what this is folded onto — most of them touch
/// nothing this work asks for and the close lands anyway, and the ones
/// that collide are refused. Reading it early is what lets somebody fix
/// a name before the close tells them to.
export function projectWork(
  rounds: ForgeRoundDto[],
  states: ForgeEntryStateDto[],
): ForgeProjectedEntry[] {
  // Step one: the work alone, per axis, last operation winning.
  const existence = new Map<string, boolean>();
  const content = new Map<string, string | null>();
  const named = new Map<string, string | null>();
  for (const round of rounds) {
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
  for (const state of states) {
    rows.set(state.entry_id, {
      entryId: state.entry_id,
      name: state.name,
      assetId: state.content_asset_id,
      alive: state.alive,
      stated: null,
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
      // Existence standing alone. The line goes on saying what it said,
      // minus the entry — and if it was not holding it, the removal
      // leaves the line's own row exactly as it was.
      //
      // `stated` is written either way, because the fold said absent
      // either way: the other axes are gone from what a close will look
      // at whether or not `normalise` keeps the row.
      if (before !== undefined) {
        rows.set(entryId, { ...before, alive: false, stated: "absent" });
      }
      continue;
    }
    rows.set(entryId, {
      entryId,
      name: named.get(entryId) ?? before?.name ?? null,
      assetId: content.get(entryId) ?? before?.assetId ?? null,
      alive: goes === true ? true : (before?.alive ?? false),
      stated: goes === true ? "present" : null,
    });
  }
  return [...rows.values()];
}

/// Names that would be on the line twice if the projected work landed.
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
/// either backend. It is here so the comparison cannot drift from the
/// rule it is predicting.
export function clashingNames(rows: ForgeProjectedEntry[]): string[] {
  const seen = new Map<string, number>();
  for (const row of rows) {
    if (!row.alive || row.name === null) continue;
    const name = row.name.trim();
    seen.set(name, (seen.get(name) ?? 0) + 1);
  }
  return [...seen.entries()]
    .filter(([, count]) => count > 1)
    .map(([name]) => name);
}
