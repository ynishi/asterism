<script lang="ts">
  // DuplicatesPanel — the report of byte-identical originals, the
  // questions the import left open about them, and the one place to
  // answer either.
  //
  // Asset identity is the file *path*, so the same photograph copied
  // into two folders has always been two assets: rated twice, tagged
  // twice, shown twice. The `material_hash` job fingerprints the bytes;
  // this panel is where that fingerprint becomes something a person can
  // act on.
  //
  // Resolution in the report is deliberately narrow: "keep this one"
  // trashes the other members and nothing else. It does not merge tags,
  // ratings or group filing — deciding which side of a conflict wins is
  // a judgement the app does not get to make silently, and the trash is
  // reversible while a merge is not.
  //
  // # Two sections, and two different words for "resolve"
  //
  // One pair of rows can be in both sections at the same time, and
  // what the two offer is not the same act:
  //
  //   - Questions (top) answers one conflict the import raised.
  //     Folding leaves one row and a tombstone in place of the other,
  //     and the copy does not come back; or the pair is ruled two
  //     separate things and both rows stay.
  //   - Report (below) is the standing listing on one fingerprint, and
  //     its resolution is the reversible one: keep one, trash the rest.
  //
  // Kept as two sections with each button naming its own act, rather
  // than making the report fold as well. The reason the report resolves
  // by trashing is that trash is reversible and a fold is not — so
  // promoting it would quietly turn a reversible click into an
  // irreversible one across a whole list, which is exactly the
  // judgement the paragraph above says this panel does not get to make
  // on its own. Renaming its button ("Keep this, trash the rest") is
  // the part that had to change: two buttons reading "keep" that mean
  // different things is the state this section split exists to avoid.
  //
  // # …and now two fingerprints, which does not become a third section
  //
  // The report answers about one axis at a time and the toggle picks
  // which: identical files, or identical pictures (two exports of one
  // image differing only in a metadata chunk). Drawing both axes at
  // once would put the same pair in two places under one heading, each
  // with its own "Keep this, trash the rest" — the same shape the split
  // above exists to avoid, one level down, and worse because both
  // buttons would do the same thing while sitting under different
  // claims. The toggle replaces the list instead, and the section
  // heading says which question is on screen.
  //
  // The Questions section keeps every axis together, and does not grow
  // a toggle of its own. A question is about one pair on the axis
  // detection found it on, so the axis is a property of the row rather
  // than of the list — the badge on each question says which. Filtering
  // a work list by axis would let somebody answer everything on screen
  // and still have questions waiting.
  //
  // # The lead says the axes are not one claim, because they are not
  //
  // Detection reports the strongest agreement it finds and stops, so a
  // row badged "Made the same way" is a pair that matched on *neither*
  // of the other two: its two pictures are different. That row is in
  // this list under a fold button, and folding it discards a picture.
  //
  // The lead used to read "Pairs found byte-identical on the way in",
  // which was true of one badge out of three and false of the row that
  // most needs reading carefully. It cannot be fixed by narrowing the
  // sentence to the artefact axis either, because all three are drawn
  // here. So it names the axis instead and says the one thing a person
  // cannot infer from a label: that the third badge is not a claim
  // about sameness.
  //
  // **The list should not contain that row at all**, and this is copy
  // rather than a fix. The axes are `Artefact = Content + Meta`, so
  // there are two independent ones and metadata-alone agreement is a
  // Series observation; that restructure is designed but has not
  // landed.
  //
  // The relationship between the two sections is unchanged by any of
  // this: a pair can be a question *and* a group, the acts differ, and
  // switching axis does not move a row between them.
  //
  // # A third act in the report, and why it is allowed to be there
  //
  // Ticking rows and merging them is a fold — irreversible, the same as
  // answering a question above — sitting in the section whose whole
  // argument is that its own resolution is the reversible one. The
  // sentence that argument turns on is "**quietly** turn a reversible
  // click into an irreversible one": what it rules out is the report's
  // existing button growing teeth, so that the click somebody already
  // knows means "trash" starts meaning "fold". This is not that. It is
  // reached by ticking rows first, its button says what it costs, and
  // it goes through a dialog that will not confirm anything until a
  // preview of that exact plan has come back. Nothing about the
  // existing button changes.
  //
  // It belongs in the report rather than in Questions for the same
  // reason the two sections exist at all: Questions is a queue of pairs
  // detection raised, and a merge is exactly the act that needs no
  // detection to have raised anything — any rows a person has decided
  // are one thing. Putting it up there would put a second, differently
  // shaped fold button beside "Fold onto this one", which is the state
  // the section split exists to avoid.
  //
  // Ticks are per group and switching group clears them. A merge over
  // rows from two groups is a coherent thing to ask for and the verb
  // would take it, but this list draws one fingerprint at a time and a
  // selection spanning groups would be a ruling whose members are not
  // all on screen together — see `MergePlan::declare` on why the set
  // somebody looked at is the thing that stops being knowable.
  //
  // # Choosing the keeper
  //
  // A fold asks for the keeper explicitly — one button per side, no
  // preselection, no default. The two sides are "the one that just
  // arrived" and "the one already here", and neither is the obviously
  // right survivor; a default would turn an irreversible fold into a
  // one-click accept of a guess. The automatic path does have a rule
  // (oldest, not trashed), but it applies where nobody was asked. Here
  // somebody is being asked.
  //
  // # What a queued fold looks like
  //
  // Folding closes the question and enqueues the fold; the worker runs
  // it afterwards. So the question leaves this list at once — it is
  // genuinely answered, and answered questions are what this list drops
  // — while the grid is left alone: `onResolved` fires for neither
  // answer, because nothing has left the live set yet. "Kept" never
  // removes a row and "folded" has not removed one *yet*; calling it
  // would drop still-live ids out of the grid selection. In place of
  // that the panel keeps a line per queued fold, so a copy still
  // sitting in the grid reads as pending rather than as a click that
  // did nothing.
  //
  // State: `duplicatesCatalog` (0-prop). The two callbacks are the
  // allowed prop category — both mutate App-owned
  // state: the grid must reload once rows are trashed, and the panel's
  // own open flag lives in App beside the other overlays.
  import { SvelteSet } from "svelte/reactivity";
  import type { AssetCardDto, DuplicateConflictDto, DuplicateGroupDto } from "./bindings";
  import MergeDialog from "./MergeDialog.svelte";
  import { mutate } from "./lib/mutate";
  import { summariseBulk } from "./lib/bulk-status";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { mergeDialog } from "./lib/stores/merge-dialog.svelte";
  import {
    DUPLICATE_AXES,
    axisLabel,
    contentBacklogNote,
    duplicatesCatalog,
    foldExclusionNote,
    groupKey,
    noConflictsLine,
    noGroupsLine,
  } from "./lib/stores/duplicates.svelte";
  import { thumbCatalog } from "./lib/stores/thumb.svelte";
  import { fmtBytes } from "./lib/formatters";

  /**
   * Enough of the path to tell two copies apart: the parent folder and
   * the file name, extension kept.
   *
   * The file name alone is the wrong label here — duplicates are
   * usually the *same* name in different places, which is exactly the
   * case a bare basename cannot show. The full path stays in the
   * `title` for when the parent is not enough either.
   */
  function fileLabel(locator: string): string {
    const parts = locator.split("/").filter((p) => p.length > 0);
    return parts.slice(-2).join("/") || locator;
  }

  interface Props {
    /** Close the panel (App owns the flag). */
    onClose: () => void;
    /**
     * Rows left the live set — App reloads the grid and its counts.
     * Receives their ids so App can drop them from the grid selection:
     * a selected id that is no longer live would send a dead target on
     * the next bulk action.
     *
     * Two ways in, one signal: rows trashed by "Keep this, trash the
     * rest", and rows folded by a merge. They leave the listing for
     * different reasons — one is in the trash, the other is a marker
     * pointing at its keeper — but what App has to do about it is the
     * same, and a second prop would only be a second place to forget.
     */
    onResolved: (goneIds: string[]) => void;
  }

  let { onClose, onResolved }: Props = $props();

  // Groups the user has just resolved. Kept locally so the list
  // collapses immediately instead of waiting for the refetch — the
  // refetch still runs, this only removes the flicker. `SvelteSet`
  // rather than `Set` because a plain Set's mutations are invisible to
  // the `$derived` below.
  let resolved = new SvelteSet<string>();
  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);

  // Answering a question is its own busy / error pair: the two
  // sections fail independently, and a trash that failed must not put
  // its message under a conflict the user is still reading.
  let conflictBusy = $state<string | null>(null);
  let conflictError = $state<string | null>(null);
  // Folds enqueued while this panel has been open. Kept until it
  // closes — see the header: the rows are still live until the worker
  // gets to them, and this line is what says so.
  let queuedFolds = $state<string[]>([]);

  // Rows ticked for a merge, and the group they belong to. Two pieces
  // rather than one keyed map because the rule is that there is only
  // ever one group's worth: `selectionKey` is what makes ticking in a
  // second group clear the first rather than build a ruling whose
  // members are not all on screen together (see the header).
  let selection = new SvelteSet<string>();
  let selectionKey = $state<string | null>(null);
  // The rows the merge dialog draws. Frozen when the dialog opens, for
  // the same reason the store freezes the ids: a refetch underneath a
  // dialog must not change what it is showing.
  let mergeCards = $state<AssetCardDto[]>([]);

  let visibleGroups = $derived(
    duplicatesCatalog.groups.filter((g) => !resolved.has(groupKey(g))),
  );

  /** Ticks or unticks one row, dropping any ticks in another group. */
  function toggleSelected(group: DuplicateGroupDto, id: string) {
    const key = groupKey(group);
    if (selectionKey !== key) {
      selection.clear();
      selectionKey = key;
    }
    if (selection.has(id)) selection.delete(id);
    else selection.add(id);
    if (selection.size === 0) selectionKey = null;
  }

  /**
   * Opens the merge dialog over the ticked rows of `group`.
   *
   * The ids are taken in the group's own order rather than the order
   * they were ticked in: the fold runs in the order it is given and the
   * keeper's note is concatenated in it, so the sequence the list is
   * drawn in is the one somebody can actually see.
   */
  function startMerge(group: DuplicateGroupDto) {
    const members = group.members.filter((m) => selection.has(m.id));
    if (members.length < 2) return;
    mergeCards = members;
    mergeDialog.start(members.map((m) => m.id));
  }

  /**
   * A merge went through: the folded rows are gone from the live set.
   *
   * Both halves are the caller's job by contract — `mergeAssets` writes
   * nothing to the store on purpose, because the backend's next read is
   * the authority on which rows survived, and the persona to read it
   * for is known here.
   */
  async function onMerged(foldedIds: string[]) {
    selection.clear();
    selectionKey = null;
    mergeCards = [];
    onResolved(foldedIds);
    await duplicatesCatalog.load(activeFilter.activePersona);
  }

  // The backlog rides on the report call, so a failed report is passed
  // through as "not known" rather than as its fallback zero — the same
  // handling `noConflictsLine` gets, for the same reason.
  let backlogNote = $derived(
    contentBacklogNote(duplicatesCatalog.report.error ? null : duplicatesCatalog.unwalked),
  );

  $effect(() => {
    void duplicatesCatalog.load(activeFilter.activePersona);
  });

  /**
   * Trashes every member of the group except `keepId`.
   *
   * Sequential rather than parallel: these are destructive writes on
   * one SQLite writer, and a partial failure should stop rather than
   * race the rest through.
   *
   * Takes the group rather than its digest: the digest is the key of a
   * group *on one axis*, and the list underneath can change axis. It is
   * re-found by `groupKey` for the same reason it was re-found before —
   * the click may be stale — and the axis is part of what has to still
   * match.
   */
  async function keepOnly(clicked: DuplicateGroupDto, keepId: string) {
    const key = groupKey(clicked);
    const group = duplicatesCatalog.groups.find((g) => groupKey(g) === key);
    // The kept id must still be in the group the click referred to.
    // Without this a stale click could trash every member and keep
    // nothing — the one outcome this panel must never produce.
    if (!group || !group.members.some((m) => m.id === keepId)) return;
    busy = key;
    error = null;
    const trashed: string[] = [];
    try {
      for (const member of group.members) {
        if (member.id === keepId) continue;
        await mutate(
          "trash_asset",
          { command: { asset_id: member.id } },
          "move this to the trash",
        );
        trashed.push(member.id);
      }
      resolved.add(key);
    } catch {
      // `mutate` has the reason on screen. What is left for this panel
      // is the count, which the refusal cannot give: the loop stops at
      // the first one refused, so some members left the live set and
      // the rest did not.
      // The same helper `App.svelte` uses. It was a hand-kept copy for
      // two rounds and diverged in both directions — once the copy was
      // right and the original wrong, once the reverse — which is what
      // moved it into `lib/`.
      const asked = group.members.length - 1;
      error = summariseBulk(trashed.length, asked, {
        verb: "moved",
        into: "to trash",
      });
    } finally {
      busy = null;
      // Both run even on a partial failure: rows really did leave the
      // live set, and leaving the panel and the grid showing them
      // would be a lie about a destructive action. The refetch also
      // re-reads what actually survived rather than trusting the loop.
      if (trashed.length > 0) onResolved(trashed);
      await duplicatesCatalog.load(activeFilter.activePersona);
    }
  }

  /**
   * Answers one question: `keeperId` names the side that survives a
   * fold, `null` rules the two separate things.
   *
   * The refetch runs either way. On success it confirms what the
   * catalog already dropped; on failure it is how a question answered
   * from somewhere else (or with a side since trashed) leaves the list
   * — the error text says what happened, the stale row does not stay.
   */
  async function answerConflict(conflict: DuplicateConflictDto, keeperId: string | null) {
    conflictBusy = conflict.id;
    conflictError = null;
    try {
      if (keeperId === null) {
        await duplicatesCatalog.keepConflictApart(conflict.id);
      } else {
        await duplicatesCatalog.foldConflict(conflict.id, keeperId);
        const keepsNewcomer = keeperId === conflict.newcomer.id;
        const keeper = keepsNewcomer ? conflict.newcomer : conflict.incumbent;
        const gone = keepsNewcomer ? conflict.incumbent : conflict.newcomer;
        queuedFolds.push(
          `Fold queued: ${fileLabel(gone.source_locator)} → ${fileLabel(
            keeper.source_locator,
          )}. Both are still in the grid until the fold job runs.`,
        );
      }
    } catch (err) {
      conflictError = `could not answer: ${(err as { message?: string })?.message ?? err}`;
    } finally {
      conflictBusy = null;
      await duplicatesCatalog.loadConflicts(activeFilter.activePersona);
    }
  }
</script>

<!--
  One side of a question, with the button that keeps *this* side. The
  fold button belongs to the side rather than being one button plus a
  selection: the id it sends is the one under the thumbnail being
  looked at, and there is no preselected side to accept by mistake.
-->
{#snippet conflictSide(
  conflict: DuplicateConflictDto,
  side: DuplicateConflictDto["newcomer"],
  role: string,
)}
  <figure class="dup-member">
    <img src={thumbCatalog.thumbSrc(side)} alt={side.cover ?? ""} />
    <figcaption>
      <span class="dup-name" title={side.source_locator}>
        {fileLabel(side.source_locator)}
      </span>
      <span class="dup-meta">
        {side.file_size_bytes !== null ? fmtBytes(side.file_size_bytes) : "size unknown"}
        <span class="dup-role">{role}</span>
      </span>
      <button
        class="dup-keep"
        disabled={conflictBusy !== null}
        onclick={() => void answerConflict(conflict, side.id)}
      >
        Fold onto this one
      </button>
    </figcaption>
  </figure>
{/snippet}

<div class="dup-backdrop" role="presentation" onclick={onClose}></div>
<section class="dup-panel" aria-label="Duplicate originals">
  <header>
    <h2>Duplicates</h2>
    <button class="dup-close" onclick={onClose} aria-label="Close duplicates">✕</button>
  </header>

  <h3 class="dup-h3">Questions</h3>
  <p class="dup-lead">
    Pairs the import found matching on one fingerprint and did not fold
    automatically. Each says which it matched on — and
    <em>made the same way</em> means the pictures are different. Folding
    keeps one row and leaves a marker in place of the other — that one
    does not come back.
  </p>

  {#if duplicatesCatalog.conflicts.loading}
    <p class="dup-note">looking…</p>
  {:else if duplicatesCatalog.conflicts.error}
    <p class="dup-error">{duplicatesCatalog.conflicts.error}</p>
  {:else if duplicatesCatalog.openConflicts.length === 0}
    <!-- The backlog comes from the report call, so a failed report is
         passed through as "not known" rather than as its zero. -->
    <p class="dup-note">
      {noConflictsLine(duplicatesCatalog.report.error ? null : duplicatesCatalog.unhashed)}
    </p>
  {/if}

  {#if conflictError}
    <p class="dup-error">{conflictError}</p>
  {/if}

  {#each queuedFolds as line, index (index)}
    <p class="dup-queued">{line}</p>
  {/each}

  <div class="dup-groups">
    {#each duplicatesCatalog.openConflicts as conflict (conflict.id)}
      {@const exclusion = foldExclusionNote(conflict.fold_exclusion)}
      <article class="dup-group" class:busy={conflictBusy === conflict.id}>
        <!-- Which fingerprint raised this one. A property of the row,
             not of the list: the two below may be identical files, or
             the same picture written twice. -->
        <p class="dup-axis-badge">{axisLabel(conflict.axis)}</p>
        {#if exclusion}
          <p class="dup-exclusion">{exclusion}</p>
        {/if}
        <div class="dup-members">
          {@render conflictSide(conflict, conflict.newcomer, "just arrived")}
          {@render conflictSide(conflict, conflict.incumbent, "already here")}
        </div>
        <div class="dup-conflict-actions">
          <button
            class="dup-keep"
            disabled={conflictBusy !== null}
            onclick={() => void answerConflict(conflict, null)}
          >
            These are two different things
          </button>
        </div>
      </article>
    {/each}
  </div>

  <h3 class="dup-h3">Report</h3>
  <!--
    One list, one fingerprint. The toggle replaces what is below rather
    than adding to it — see the header on why both axes at once would
    put one pair under two buttons that do the same thing.
  -->
  <div class="dup-axes" role="group" aria-label="Which fingerprint to group on">
    {#each DUPLICATE_AXES as axis (axis)}
      <button
        class="dup-axis"
        class:on={duplicatesCatalog.axis === axis}
        aria-pressed={duplicatesCatalog.axis === axis}
        disabled={busy !== null}
        onclick={() => void duplicatesCatalog.setAxis(axis, activeFilter.activePersona)}
      >
        {axisLabel(axis)}
      </button>
    {/each}
  </div>
  <p class="dup-lead">
    {#if duplicatesCatalog.axis === "artefact"}
      Assets whose original files are byte-identical.
    {:else}
      Assets that decode to the same picture — the same image written
      twice, differing only in metadata such as an embedded workflow.
    {/if}
    Keeping one trashes the others — reversible from the Trash view.
  </p>

  {#if duplicatesCatalog.report.loading}
    <p class="dup-note">looking…</p>
  {:else if duplicatesCatalog.report.error}
    <p class="dup-error">{duplicatesCatalog.report.error}</p>
  {:else}
    <p class="dup-note">
      {#if visibleGroups.length === 0}
        {noGroupsLine(duplicatesCatalog.axis)}
      {:else}
        {visibleGroups.length} group{visibleGroups.length === 1 ? "" : "s"},
        {duplicatesCatalog.redundantCount} redundant cop{duplicatesCatalog.redundantCount === 1
          ? "y"
          : "ies"}.
      {/if}
      {#if duplicatesCatalog.unhashed > 0}
        <!-- The distinction that keeps an empty report honest: nothing
             found is not the same as nothing looked at yet. -->
        <span class="dup-pending">
          {duplicatesCatalog.unhashed} file{duplicatesCatalog.unhashed === 1 ? "" : "s"}
          not fingerprinted yet — this answer is incomplete.
        </span>
      {/if}
      {#if duplicatesCatalog.unreadable > 0}
        <!-- Deliberately not folded into the count above: that one
             converges to zero on its own, this one moves only when the
             files come back, and merging them is what used to keep the
             "still fingerprinting" notice open forever. -->
        <span class="dup-pending">
          {duplicatesCatalog.unreadable} original{duplicatesCatalog.unreadable === 1 ? "" : "s"}
          could not be read — moved, deleted, or on a disconnected disk.
        </span>
      {/if}
    </p>
  {/if}

  {#if duplicatesCatalog.axis === "content" && backlogNote}
    <!--
      The other half of the same honesty, and the one the report cannot
      infer: a material whose original the column's migration could not
      open carries no content digest, so it is in no group on this axis
      and its absence looks like a clean result. Rendered whether or not
      there are groups — a partial answer is partial in both directions
      — and outside the `error` branch above, because `backlogNote`
      already carries the "could not be read" wording for that case.
    -->
    <p class="dup-pending">{backlogNote}</p>
  {/if}

  {#if error}
    <p class="dup-error">{error}</p>
  {/if}

  <div class="dup-groups">
    {#each visibleGroups as group (groupKey(group))}
      {@const ticked = selectionKey === groupKey(group) ? selection.size : 0}
      <article class="dup-group" class:busy={busy === groupKey(group)}>
        <div class="dup-members">
          {#each group.members as member, index (member.id)}
            <figure class="dup-member">
              <img src={thumbCatalog.thumbSrc(member)} alt={member.cover ?? ""} />
              <figcaption>
                <label class="dup-pick">
                  <input
                    type="checkbox"
                    checked={selectionKey === groupKey(group) && selection.has(member.id)}
                    disabled={busy !== null}
                    onchange={() => toggleSelected(group, member.id)}
                  />
                  <span class="dup-name" title={member.source_locator}>
                    {fileLabel(member.source_locator)}
                  </span>
                </label>
                <span class="dup-meta">
                  {member.file_size_bytes !== null
                    ? fmtBytes(member.file_size_bytes)
                    : "size unknown"}
                  {#if index === 0}<span class="dup-oldest">oldest</span>{/if}
                </span>
                <button
                  class="dup-keep"
                  disabled={busy !== null}
                  onclick={() => void keepOnly(group, member.id)}
                >
                  Keep this, trash the rest
                </button>
              </figcaption>
            </figure>
          {/each}
        </div>
        <!--
          Only once two rows are ticked. One row is not a merge and the
          command that says so is refused a layer down (`MergePlan`);
          offering the button before then would be an invitation to a
          call that cannot succeed.
        -->
        {#if ticked >= 2}
          <div class="dup-conflict-actions">
            <button class="dup-keep" disabled={busy !== null} onclick={() => startMerge(group)}>
              Merge {ticked} into one…
            </button>
          </div>
        {/if}
      </article>
    {/each}
  </div>
</section>

{#if mergeDialog.isOpen}
  <MergeDialog cards={mergeCards} onCommitted={(ids) => void onMerged(ids)} />
{/if}

<style>
  .dup-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(20, 18, 16, 0.35);
    z-index: 40;
  }

  .dup-panel {
    position: fixed;
    top: 5vh;
    left: 50%;
    transform: translateX(-50%);
    width: min(880px, 90vw);
    max-height: 85vh;
    overflow-y: auto;
    background: #faf9f5;
    border-radius: 8px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
    padding: 1rem 1.25rem 1.5rem;
    z-index: 41;
  }

  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  h2 {
    font-size: 1rem;
    margin: 0 0 0.25rem;
  }

  .dup-close {
    background: none;
    border: none;
    font-size: 0.9rem;
    color: #777;
    cursor: pointer;
  }

  .dup-h3 {
    font-size: 0.85rem;
    margin: 1rem 0 0.15rem;
    color: #3a3733;
  }

  .dup-lead {
    margin: 0 0 0.5rem;
    font-size: 0.8rem;
    color: #666;
  }

  .dup-exclusion {
    margin: 0 0 0.5rem;
    font-size: 0.75rem;
    color: #8a6d3b;
  }

  .dup-axes {
    display: flex;
    gap: 0.3rem;
    margin: 0.35rem 0 0.5rem;
  }

  .dup-axis {
    font-family: inherit;
    font-size: 0.72rem;
    padding: 0.25rem 0.5rem;
    border: 1px solid #d8d4c8;
    border-radius: 4px;
    background: #f4f2ec;
    color: #555;
    cursor: pointer;
  }
  .dup-axis:hover:enabled {
    background: #eceae2;
  }
  .dup-axis.on {
    background: #e2ded1;
    border-color: #b9b3a1;
    color: #3a3733;
  }
  .dup-axis:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .dup-axis-badge {
    margin: 0 0 0.4rem;
    font-size: 0.7rem;
    color: #7a7594;
  }

  .dup-queued {
    margin: 0 0 0.4rem;
    font-size: 0.75rem;
    color: #4a6b52;
  }

  .dup-conflict-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.5rem;
  }

  .dup-role {
    margin-left: 0.3rem;
    color: #7a7594;
  }

  .dup-note {
    font-size: 0.8rem;
    color: #555;
    margin: 0 0 0.75rem;
  }

  .dup-pending {
    display: block;
    color: #8a6d3b;
  }

  .dup-error {
    font-size: 0.8rem;
    color: #a4423a;
    margin: 0 0 0.75rem;
  }

  .dup-groups {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .dup-group {
    border: 1px solid #e6e3da;
    border-radius: 6px;
    padding: 0.6rem;
    background: #fff;
  }
  .dup-group.busy {
    opacity: 0.6;
  }

  .dup-members {
    display: flex;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .dup-member {
    margin: 0;
    width: 160px;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .dup-member img {
    width: 160px;
    height: 110px;
    object-fit: cover;
    border-radius: 4px;
    background: #efefe9;
  }

  figcaption {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.72rem;
    color: #555;
  }

  .dup-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dup-meta {
    color: #999;
  }

  .dup-oldest {
    margin-left: 0.3rem;
    color: #7a7594;
  }

  .dup-pick {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
    cursor: pointer;
  }

  .dup-keep {
    font-family: inherit;
    font-size: 0.72rem;
    padding: 0.25rem 0.4rem;
    border: 1px solid #d8d4c8;
    border-radius: 4px;
    background: #f4f2ec;
    cursor: pointer;
  }
  .dup-keep:hover:enabled {
    background: #eceae2;
  }
  .dup-keep:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
