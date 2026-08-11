// Duplicates catalog — the report of byte-identical originals, and
// the questions about them that are still waiting on a person.
//
// Not a facet like FORMAT / COLOR: those describe the grid, this is a
// work list. It is fetched when the panel opens and re-fetched after
// the user resolves a group, rather than following the grid's filter
// state, so it stays off the sidebar reload chain entirely.
//
// `unhashed` is carried beside the groups on purpose. An empty report
// means two different things — "no duplicates" and "nothing has been
// fingerprinted yet" — and the panel must not present the second as
// the first.
//
// Reload orchestration still follows the catalog rule: this store
// never decides when to load, the panel asks.
//
// # The axis lives here, not in the panel
//
// The report answers about one fingerprint at a time — every byte of
// the file, or only the bytes that decide the decoded picture — and
// `axis` is which one the last read asked for. It is store state
// because switching it is a **refetch with different arguments**, and
// that is the rule worth a test: a toggle that changed a label without
// re-reading, or re-read the old axis, would show one axis's groups
// under the other's name. The panel owns the control; the store owns
// what the control means.
//
// Two axes, one section, one at a time. The alternative considered was
// reading the content axis alone and showing the byte-identical sets as
// a breakdown inside each group — a content group does contain the
// artefact group whenever both rows carry a region digest. They do not
// always carry one: a format with no walker, and every material that
// predates the column, hold a marker instead, and those rows group on
// the artefact axis and nowhere else. Deriving one axis from the other would drop
// exactly those findings. One axis at a time also means a pair appears
// once in the report rather than in two places offering the same button
// (the `unwalked` note below is the other half of not lying by shape).
//
// # `unwalked` is not a smaller `unhashed`
//
// `unhashed` counts materials with no answer yet, and it drains as the
// fingerprint walk runs. `unwalked` counts materials whose content
// column still says `not-walked`. That marker is written over every
// pre-existing row by the migration that adds the column and cleared by
// the next step of the same chain, which reads the files — both before
// the app answers anything, so this is never a queue that is about to
// start. What is left is the originals that step could not open: files
// moved or deleted since import, a disk that was not connected during
// the upgrade. It does not drain with time; the files have to come
// back. `contentBacklogNote` is that sentence, and the panel renders it
// whether or not there are groups, because a partial answer is
// incomplete in both directions.
//
// # Why the conflicts live here rather than in a catalog of their own
//
// Two Resources on one catalog, not two catalogs. They are separate
// endpoints on separate refetch schedules — which the Resource
// primitive already gives each of them (own generation guard, own
// `loading` / `error`), so splitting the class buys nothing there.
// What splitting would cost is the one thing this file exists to get
// right: `unhashed_count` rides on the *report* and the conflicts
// call deliberately does not repeat it, so "no open questions" and
// "nothing fingerprinted yet" can only be told apart by reading both.
// Across two singletons that join happens in the panel, which is the
// surface the tests cannot reach (node env, no DOM) — the same trap
// the `unhashed` note above was written for, moved somewhere it stops
// being checkable.
//
// # Answered rows
//
// `answered` holds conflicts resolved in this session so the list
// collapses without waiting for the refetch. It is store state rather
// than panel state for one reason: a row must leave the list only
// when the backend accepted the answer, and that rule needs a test.
// Entries are added after the call resolves, never before — a
// failed answer leaves the question on screen where the error
// message is.

import type {
  DuplicateAxis,
  DuplicateConflictDto,
  DuplicateGroupDto,
  DuplicateReportDto,
  DuplicateResolutionDto,
  MergeAssetsCommand,
  MergeAssetsDto,
  ResolveDuplicateConflictCommand,
} from "../../bindings";
import { untrack } from "svelte";
import { SvelteSet } from "svelte/reactivity";
import { api } from "../api";
import { Resource } from "./_resource.svelte";

const EMPTY: DuplicateReportDto = { groups: [], unhashed_count: 0, unwalked_count: 0 };
const NO_CONFLICTS: DuplicateConflictDto[] = [];

/**
 * The axes the panel offers, in the order it draws them — strongest
 * claim about sameness first, which is also the order detection reports
 * them in.
 */
export const DUPLICATE_AXES: readonly DuplicateAxis[] = ["artefact", "content", "meta"];

/**
 * What each of the toggle's positions claims, in the panel's own words.
 *
 * Here rather than in the markup so the sentences sit next to the note
 * that qualifies them: "content" promises to see past metadata, and
 * `contentBacklogNote` is what stops that promise being read as a
 * statement about files nothing has walked.
 *
 * A `switch` rather than the ternary this was: a third axis under a
 * two-way conditional is labelled as whichever one the `else` names,
 * and the badge on a conflict row (`axisLabel(conflict.axis)`) renders
 * whatever detection found rather than what the toggle selected — so
 * the wrong label would appear without anybody choosing it.
 */
export function axisLabel(axis: DuplicateAxis): string {
  switch (axis) {
    case "artefact":
      return "Identical files";
    case "content":
      return "Identical pictures";
    case "meta":
      return "Made the same way";
  }
}

/**
 * The identity of a group in a list whose axis changes underneath it.
 *
 * The digest alone would do today — the two axes tag their values
 * differently (`sha256:` / `cr1-sha256:`), so two keys cannot collide —
 * but the key is also what the panel's "already resolved" set is
 * remembered by, and that set outlives a toggle. Keying on the digest
 * alone would let a group resolved on one axis silently disappear from
 * the other, which is a claim about a pair that nobody made. Naming the
 * axis costs a prefix and stops the key depending on a property of the
 * digest vocabulary holding forever.
 */
export function groupKey(group: DuplicateGroupDto): string {
  return `${group.axis}/${group.content_hash}`;
}

/**
 * What the report says when it found nothing on `axis`.
 *
 * Only the "found nothing" half — the backlog is
 * `contentBacklogNote`'s, because it has to be said whether or not
 * there are groups.
 */
export function noGroupsLine(axis: DuplicateAxis): string {
  switch (axis) {
    case "artefact":
      return "No byte-identical originals found.";
    case "content":
      return "No two files here decode to the same picture.";
    case "meta":
      return "No two files here carry the same embedded metadata.";
  }
}

/**
 * The materials the content axis has no reading of, and why — or `null`
 * when there is nothing to say.
 *
 * Without this the content axis lies by omission: a row with no content
 * digest is in no content group, which from the outside is
 * indistinguishable from a row that has no duplicate. "No duplicates"
 * and "not looked at" are the two readings and only this number
 * separates them.
 *
 * What it says has changed with the pass that fills the column in.
 * Those values are computed by the migration that adds the column,
 * before the app answers anything, so a non-zero count here is never
 * "not started yet" — it is the originals the migration could not open,
 * which is a fact about missing files rather than about pending work.
 * The wording names the file so somebody can go and look, and does not
 * promise the number will fall on its own, because it will not.
 *
 * `null` is "could not be read", which is the state after a failed
 * report: the fallback report carries `unwalked_count: 0`, and passing
 * that zero through would turn a failed read into a clean bill of
 * health. Same trap, same handling, as `noConflictsLine`.
 */
export function contentBacklogNote(unwalkedCount: number | null): string | null {
  if (unwalkedCount === null) {
    return "How much of your library the content axis has looked at could not be read, so this answer may cover only part of it.";
  }
  if (unwalkedCount === 0) return null;
  return `${unwalkedCount} file${
    unwalkedCount === 1 ? "" : "s"
  } could not be read on this axis — the original${
    unwalkedCount === 1 ? " was" : "s were"
  } not where the library expects ${
    unwalkedCount === 1 ? "it" : "them"
  } when the content fingerprints were computed, so ${
    unwalkedCount === 1 ? "it is" : "they are"
  } left out of this answer either way. Putting ${
    unwalkedCount === 1 ? "that file" : "those files"
  } back is what changes it.`;
}

/**
 * Why the automatic fold declined this pair, in a sentence somebody
 * can act on — or `null` when nothing declined it.
 *
 * The exclusions stop the *automatic* fold and nothing else: a person
 * looking at both rows is allowed to fold them anyway. So the wording
 * says what the rule noticed and hands the decision over, rather than
 * reading as a refusal ("cannot be folded") the panel would then
 * contradict with its own button.
 *
 * An unrecognised slug is reported verbatim instead of being dropped.
 * A warning the panel silently swallows is worse than one it cannot
 * phrase: the pair really was held back by something.
 */
export function foldExclusionNote(exclusion: string | null): string | null {
  const yours = "so it was left for you to decide — you can still fold it by hand.";
  switch (exclusion) {
    case null:
      return null;
    case "lineage":
      return `These two are connected by lineage — one came from the other, or both descend from something in common. The automatic fold does not touch that, ${yours}`;
    case "dispatch":
      return `One of these two is the output of an export run. The automatic fold does not touch those, ${yours}`;
    default:
      return `The automatic fold declined this pair (${exclusion}), ${yours}`;
  }
}

/**
 * What the conflicts section says when it has nothing to show.
 *
 * "No questions" and "no questions yet" are different answers and the
 * fingerprint backlog is what separates them — the same distinction
 * `unhashed` keeps for the report, applied to the work list.
 *
 * `null` is the third state and the reason this takes an argument at
 * all: the backlog rides on the *report* call, which can fail while
 * the conflicts call succeeds. Reading the empty-report fallback (0)
 * as "nothing left to fingerprint" would turn a failed read into a
 * clean bill of health, which is the one thing this line exists to
 * avoid.
 */
export function noConflictsLine(unhashedCount: number | null): string {
  if (unhashedCount === null) {
    return "Nothing to answer — but how much is still to be fingerprinted could not be read.";
  }
  if (unhashedCount > 0) {
    return `Nothing to answer yet — ${unhashedCount} file${
      unhashedCount === 1 ? "" : "s"
    } still to fingerprint.`;
  }
  return "Nothing to answer — no duplicate questions are open.";
}

class DuplicatesCatalog {
  report = new Resource(
    (args: { personaId: string | null; axis: DuplicateAxis }) =>
      api<DuplicateReportDto>("list_duplicate_groups", {
        personaId: args.personaId,
        axis: args.axis,
        limit: null,
      }),
    EMPTY,
    "duplicatesCatalog.report",
  );

  conflicts = new Resource(
    (personaId: string | null) =>
      api<DuplicateConflictDto[]>("list_duplicate_conflicts", {
        personaId,
        limit: null,
      }),
    NO_CONFLICTS,
    "duplicatesCatalog.conflicts",
  );

  /**
   * Which fingerprint the report currently answers about.
   *
   * Read by the panel to label the section and light the toggle; only
   * ever changed through `setAxis`, which refetches. Assigning it
   * directly would leave the label and the rows describing different
   * questions.
   */
  axis = $state<DuplicateAxis>("artefact");

  /** Conflict ids answered in this session (see file header). */
  answered = new SvelteSet<string>();

  groups = $derived(this.report.data.groups);
  /** Materials still waiting to be fingerprinted. */
  unhashed = $derived(this.report.data.unhashed_count);
  /** Materials the content axis has never looked at (file header). */
  unwalked = $derived(this.report.data.unwalked_count);
  /** Assets that would disappear if every group were resolved to one. */
  redundantCount = $derived(
    this.report.data.groups.reduce((acc, g) => acc + Math.max(0, g.members.length - 1), 0),
  );
  /** Questions still on the table, answered ones already dropped. */
  openConflicts = $derived(this.conflicts.data.filter((c) => !this.answered.has(c.id)));

  /**
   * Reads both lists for `personaId`, on whichever axis is current.
   *
   * The axis read is untracked on purpose. The panel calls this from an
   * `$effect` whose subject is the persona; letting the effect also
   * depend on `axis` would make `setAxis` fire two reads of the same
   * thing — its own, and the effect's — and re-read the conflicts queue
   * for a change that has nothing to do with it. Switching axis is
   * `setAxis`'s job and only its job.
   */
  async load(personaId: string | null): Promise<void> {
    const axis = untrack(() => this.axis);
    await Promise.all([
      this.report.load({ personaId, axis }),
      this.conflicts.load(personaId),
    ]);
  }

  /**
   * Switches which fingerprint the report is about, and re-reads it.
   *
   * The refetch is the whole operation — the two axes are two questions
   * and the rows on screen answer the old one. Only the report moves:
   * the conflicts list is a queue whose rows each carry the axis
   * detection found them on, so there is nothing there to re-select.
   *
   * The field is set before the call, so the panel's label and its
   * pending state describe the axis being fetched rather than the one
   * being left. A response from a superseded call is dropped by the
   * Resource's own generation guard.
   */
  async setAxis(axis: DuplicateAxis, personaId: string | null): Promise<void> {
    this.axis = axis;
    await this.report.load({ personaId, axis });
  }

  /** Re-reads the work list alone (after one question was answered). */
  async loadConflicts(personaId: string | null): Promise<void> {
    await this.conflicts.load(personaId);
  }

  /**
   * Rules the pair one thing and queues the fold onto `keeperId`.
   *
   * The keeper is a required argument all the way down — the command
   * validates it is one of the pair, and there is no side the UI is
   * entitled to pick on its own (see the panel docstring).
   */
  async foldConflict(
    conflictId: string,
    keeperId: string,
  ): Promise<DuplicateResolutionDto> {
    return this.#resolve({
      conflict_id: conflictId,
      resolution: "folded",
      keeper_id: keeperId,
    });
  }

  /**
   * Rules the pair two separate things. Both rows stay, so no keeper
   * is sent — the command refuses one, because a caller that passed a
   * keeper here would believe it had folded something.
   */
  async keepConflictApart(conflictId: string): Promise<DuplicateResolutionDto> {
    return this.#resolve({
      conflict_id: conflictId,
      resolution: "kept",
      keeper_id: null,
    });
  }

  async #resolve(
    command: ResolveDuplicateConflictCommand,
  ): Promise<DuplicateResolutionDto> {
    const result = await api<DuplicateResolutionDto>("resolve_duplicate_conflict", {
      command,
    });
    // Only now. A refusal (already answered, a side since trashed)
    // throws to the caller with the row still listed.
    this.answered.add(command.conflict_id);
    return result;
  }

  /**
   * Collapses `command.member_ids` into `command.keeper_id` — the
   * manual merge verb. Runs whether or not detection ever raised a
   * question about these rows.
   *
   * # Preview vs commit
   *
   * `command.dry_run: true` returns a preview and writes nothing;
   * `false` runs. Callers wanting the two-phase flow send the same
   * command twice, once with `dry_run: true` and once with `false`,
   * and read the answer back on the same DTO shape — the port doc
   * says why (a run following a preview reads on the same fields, and
   * [`MergeAssetsDto.committed`] is what tells them apart). The dry
   * run is where warnings appear; the commit branch always returns
   * an empty warnings list because the caller has already seen them.
   *
   * # State after a successful commit
   *
   * On a successful commit **the store does not touch its own state**
   * — no additions to [`answered`], no local drop of folded rows from
   * the report groups. Two reasons:
   *
   * 1. [`answered`] is keyed by conflict_id and the manual merge has
   *    no conflict_id. Adding a synthetic key would let a fold hide
   *    an actual unresolved conflict on a group that later shared it.
   * 2. Dropping rows from `report.groups` client-side by asset_id
   *    would duplicate what the backend already computes on a fresh
   *    read — the fold rewrites `folded_into` on the discards and
   *    those rows leave the report groups on the next load. The
   *    server's answer is authoritative.
   *
   * The caller (the panel driving the merge) knows the current
   * persona and re-runs [`load`] when it wants the panel to reflect
   * the fold. That is the same contract [`foldConflict`] leaves for
   * the caller to trigger when the fold's queued job finishes.
   *
   * On a preview / a refused commit (`refusals` non-empty and
   * `committed: false`) nothing is written server-side either, so
   * the same "no local mutation" rule leaves the caller reading the
   * refusals to rule again.
   */
  async mergeAssets(command: MergeAssetsCommand): Promise<MergeAssetsDto> {
    return api<MergeAssetsDto>("merge_assets", { command });
  }

  reset(): void {
    this.report.reset();
    this.conflicts.reset();
    this.answered.clear();
    // Back to the artefact axis with the data, not left where the last
    // session put it: a panel that opened showing "Identical pictures"
    // over an empty list would be reporting the axis's backlog as a
    // result, and the axis a person did not choose is the one they are
    // least likely to read the small print for.
    this.axis = "artefact";
  }
}

export const duplicatesCatalog = new DuplicatesCatalog();
