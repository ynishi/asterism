// Merge dialog — the steps a person goes through to rule that several
// rows are one thing, and the rules about the order they go in.
//
// The verb underneath is `duplicatesCatalog.mergeAssets`, which takes a
// whole ruling in one call and runs it in one transaction. What it does
// not do is remember anything between the preview and the run: it is
// handed a command and answers it. Everything that has to hold *across*
// those two calls — that the plan confirmed is the plan previewed, that
// the rows named are the rows somebody looked at — lives here.
//
// # Why this is a store and not component state
//
// The rules below are the ones worth being wrong about, and the panel
// is DOM: this suite runs in node, so a rule kept in the component is a
// rule nothing checks (the same reasoning the duplicates catalog
// records for its own `answered` set). Two in particular:
//
//   - a commit is refused unless a preview of *this* plan came back
//     first, and
//   - changing the keeper throws the preview away.
//
// Both exist because the preview is the only moment the warnings are
// computed — the application verb says so, and returns an empty
// warnings list on the commit branch precisely because the caller has
// already been shown them. A dialog that let somebody preview keeping A
// and then confirm keeping B would put an irreversible fold behind a
// screen describing a different one.
//
// # What is frozen at `open`
//
// `members` is copied when the ruling starts and never recomputed.
// `MergePlan::declare` refuses a command whose `member_ids` is not
// exactly the keeper plus the discarded rows, and the reason it takes
// that argument at all is that "these three of five" and "I lost track
// of two" produce the same call otherwise. The set a person was looking
// at stops being knowable the moment the call is made, so this is where
// it is held.
//
// # What this does not own
//
// The reload. On a committed run the panel re-reads the report — the
// catalog's `mergeAssets` docstring leaves that to the caller because
// the caller is the one that knows the persona, and this state machine
// deliberately does not reach for one.

import type { MergeAssetsCommand, MergeAssetsDto, MergeTotalsDto } from "../../bindings";
import { duplicatesCatalog } from "./duplicates.svelte";

/**
 * What each number in a merge preview counts, singular and plural.
 *
 * Typed as a `Record` over the DTO's own keys on purpose: the day the
 * backend adds a tenth count, this file stops compiling. The
 * alternative — a list of the ones somebody remembered — fails the
 * other way, by quietly showing a person a smaller preview of an
 * operation that has no undo.
 */
const TOTAL_LABELS: Record<keyof MergeTotalsDto, [string, string]> = {
  edges_repointed: ["connection re-pointed", "connections re-pointed"],
  edges_dropped: ["connection dropped", "connections dropped"],
  buckets_moved: ["group filing moved", "group filings moved"],
  children_repointed: ["derived row re-pointed", "derived rows re-pointed"],
  tags_moved: ["tag moved", "tags moved"],
  comments_moved: ["comment moved", "comments moved"],
  threads_reanchored: ["thread re-anchored", "threads re-anchored"],
  columns_merged: ["field combined", "fields combined"],
  values_discarded: ["value set aside", "values set aside"],
};

/**
 * The non-zero counts of a preview, in a fixed order, ready to list.
 *
 * Zero rows are dropped rather than drawn as "0": a preview is read to
 * find out what will happen, and nine lines of zero is where the one
 * line that matters gets lost. Empty means nothing but the fold itself.
 */
export function mergeTotalLines(totals: MergeTotalsDto): { label: string; count: number }[] {
  const lines: { label: string; count: number }[] = [];
  for (const [key, [one, many]] of Object.entries(TOTAL_LABELS)) {
    const count = totals[key as keyof MergeTotalsDto];
    if (count > 0) lines.push({ label: count === 1 ? one : many, count });
  }
  return lines;
}

/**
 * How many rows a preview would fold, and what is already true.
 *
 * `already_folded_ids` is not a refusal — the port treats a row already
 * folded into *this* keeper as the plan already holding — so it is said
 * separately rather than counted in. A person who ticked five rows and
 * is told "3 rows fold" needs the other sentence to know the remaining
 * two are not being forgotten.
 */
export function mergeRowsLine(dto: MergeAssetsDto): string {
  const folding = dto.folded_ids.length;
  const already = dto.already_folded_ids.length;
  const head =
    folding === 1 ? "1 row folds into the one you kept." : `${folding} rows fold into the one you kept.`;
  if (already === 0) return head;
  return `${head} ${
    already === 1 ? "1 more is" : `${already} more are`
  } already folded into it — nothing changes for ${already === 1 ? "it" : "them"}.`;
}

/**
 * What an automatic fold would have objected to, said to somebody who
 * is about to fold by hand anyway.
 *
 * Deliberately not `foldExclusionNote`'s wording, though the slugs are
 * the same two. That one is read in the questions queue, where the
 * exclusion is why a pair is still waiting, and it ends by handing the
 * decision over. This one is read with a confirm button on screen: the
 * decision has been made, and what is left to say is what it costs.
 *
 * An unrecognised slug is reported verbatim for the same reason its
 * sibling does it: a warning the panel cannot phrase still happened.
 */
export function mergeWarningNote(kind: string): string {
  switch (kind) {
    case "lineage":
      return "These are connected by lineage — one came from the other, or both descend from something in common. Folding them turns that link into one pointing at itself, so it is dropped; what it recorded is written into the surviving row's history instead of staying a link you can follow.";
    case "dispatch":
      return "One of these is the output of an export run. The automatic fold leaves those alone; doing it by hand is allowed, and this is the notice that the rule was there.";
    default:
      return `The automatic fold would have declined this pair (${kind}). Doing it by hand is allowed; this is the notice that the rule was there.`;
  }
}

/**
 * Where a ruling has got to.
 *
 * `refused` is its own resting phase rather than an error string on
 * `preview`: a refusal is an answer from the backend (200 with
 * `committed: false`), it names rows, and the dialog stays open on it
 * because the person has something new to read. A thrown call is the
 * other thing entirely — that is `error`, and it drops back to the
 * phase it came from so the same button can be pressed again.
 */
export type MergePhase =
  | "closed"
  | "choosing"
  | "previewing"
  | "preview"
  | "committing"
  | "refused";

class MergeDialog {
  /** The single source of truth for what is on screen. */
  phase = $state<MergePhase>("closed");

  /**
   * The rows this ruling is over, in the order the panel drew them.
   *
   * The order is not decoration: the merge folds in it and the keeper's
   * note is built by concatenating in fold order, so a set sorted by id
   * here would file somebody's rows in an order nobody chose.
   */
  members = $state<string[]>([]);

  /** The row the person picked to survive, or `null` before they have. */
  keeperId = $state<string | null>(null);

  /** The last dry run of the current plan, or `null` if there is none. */
  preview = $state<MergeAssetsDto | null>(null);

  /** A refused commit, kept so the dialog can list what it refused. */
  refusal = $state<MergeAssetsDto | null>(null);

  /** A call that threw (a malformed plan, a dead backend). */
  error = $state<string | null>(null);

  isOpen = $derived(this.phase !== "closed");

  /** Everything but the keeper, in `members` order. */
  discardIds = $derived(this.members.filter((id) => id !== this.keeperId));

  /**
   * A plan can be previewed once somebody has named a survivor and
   * there is at least one other row to fold into it — the two things
   * `MergePlan::declare` refuses a command for.
   */
  canPreview = $derived(
    (this.phase === "choosing" || this.phase === "refused") &&
      this.keeperId !== null &&
      this.discardIds.length > 0,
  );

  /** A commit needs a preview of the plan now on screen. See header. */
  canCommit = $derived(this.phase === "preview" && this.preview !== null);

  /**
   * Starts a ruling over `memberIds`.
   *
   * Copies rather than keeps the array: the panel's selection is live
   * and a ruling is about the rows that were on screen when it started.
   */
  start(memberIds: string[]): void {
    this.members = [...memberIds];
    this.keeperId = null;
    this.preview = null;
    this.refusal = null;
    this.error = null;
    this.phase = "choosing";
  }

  /** Abandons the ruling and leaves nothing behind for the next one. */
  close(): void {
    this.members = [];
    this.keeperId = null;
    this.preview = null;
    this.refusal = null;
    this.error = null;
    this.phase = "closed";
  }

  /**
   * Names the row that survives.
   *
   * Any preview is dropped, because it described a different plan — see
   * the header on why that is the rule this type exists to hold. A
   * refusal is dropped for the same reason: it named rows under the old
   * keeper.
   */
  chooseKeeper(id: string): void {
    if (this.phase === "previewing" || this.phase === "committing") return;
    this.keeperId = id;
    this.preview = null;
    this.refusal = null;
    this.error = null;
    this.phase = "choosing";
  }

  /**
   * Runs the plan as a dry run and holds the answer.
   *
   * This is the only moment the warnings are computed (the application
   * verb returns none on the commit branch), so it is also the only
   * moment somebody can be told that a fold they are about to confirm
   * loses a lineage record.
   */
  async runPreview(): Promise<void> {
    if (!this.canPreview) return;
    this.phase = "previewing";
    this.error = null;
    this.refusal = null;
    try {
      this.preview = await duplicatesCatalog.mergeAssets(this.#command(true));
      this.phase = "preview";
    } catch (err) {
      this.preview = null;
      this.error = `could not preview the merge: ${message(err)}`;
      this.phase = "choosing";
    }
  }

  /**
   * Runs the plan for real, and answers what happened.
   *
   * Returns the committed DTO — and **only** that. A refusal and a
   * thrown call both come back as `null` because neither wrote
   * anything, and the caller's job on this return value is exactly one
   * thing: re-read the panel because rows left the live set. What to
   * *show* for the other two outcomes is `phase` / `refusal` / `error`,
   * which is rendering rather than reloading.
   *
   * The dialog closes itself on a committed run. It stays open on a
   * refusal: the rows named there are the answer, and a dialog that
   * vanished would take them with it.
   */
  async commit(): Promise<MergeAssetsDto | null> {
    if (!this.canCommit) return null;
    this.phase = "committing";
    this.error = null;
    let result: MergeAssetsDto;
    try {
      result = await duplicatesCatalog.mergeAssets(this.#command(false));
    } catch (err) {
      this.error = `could not merge: ${message(err)}`;
      // Back to the preview the person already read, so confirming
      // again does not make them run the dry run a second time.
      this.phase = "preview";
      return null;
    }
    if (!result.committed) {
      this.refusal = result;
      this.phase = "refused";
      return null;
    }
    this.close();
    return result;
  }

  /**
   * The command for both calls, differing only in the flag.
   *
   * One builder rather than two: the preview and the run being the same
   * plan is the whole guarantee of this type, and two call sites
   * assembling the same fields is where that stops being true.
   */
  #command(dryRun: boolean): MergeAssetsCommand {
    return {
      keeper_id: this.keeperId as string,
      discard_ids: this.discardIds,
      member_ids: this.members,
      dry_run: dryRun,
    };
  }
}

function message(err: unknown): string {
  return (err as { message?: string })?.message ?? String(err);
}

export const mergeDialog = new MergeDialog();
