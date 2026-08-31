<script lang="ts">
  // PromoteToTeam — handing this asset to a team, from the pane that is
  // showing it.
  //
  // The act #66 exists for, and the one way content reaches a team's
  // line: #148 decision 5 gives it exactly one entry point, a verb
  // scoped to open work, so the team never holds an asset that is not
  // attached to work.
  //
  // # Why it is here and not on the shared-lines drawer
  //
  // `shared.svelte.ts` argues the placement under "A promotion does not
  // start here" and leaves this surface the rest. What it does not
  // consider is the arrangement the local plane uses: `ForgeWork`
  // builds a round out of the grid selection, and the drawer could have
  // taken a promotion the same way, making one gesture where this makes
  // two. What decided against it is decision 7 — a `TeamAsset` is
  // minted per promotion — so a selection of five behind one press is
  // five acts that cannot be pressed twice, and nothing on the way
  // would have said so.
  //
  // # Which pursuit, and what this shows when there is none
  //
  // Whichever the shared-lines drawer has open. The three ids a
  // promotion needs — team, line, pursuit — are `sharedCatalog`'s
  // already, and it holds them whether or not the drawer is showing, so
  // a person opens work once and promotes from as many assets as they
  // like. This surface reads them and never sets them; a picker here
  // would be a second place naming the same three, and then two places
  // could disagree about which team an asset was going to.
  //
  // With no work open there is nothing to promote *to*, and this says
  // so rather than offering a disabled button — a control that cannot
  // act is worse than a sentence that says where to go. The drawer is
  // one press away, and the button below opens it.
  //
  // # What travels is said before it goes
  //
  // #171 asks for this in as many words. The rule is decision 4's and
  // the client's promotion module states it; what is new here is that
  // a person reads it, in their own words, **above** the control rather
  // than beside the result — it is a thing to know before pressing.
  //
  // # What this does not pre-empt
  //
  // A Collection and a multi-material asset are refused by the client,
  // each with a message naming which of the two it is and why the
  // team's shape for it is undecided (decision 3). This surface does
  // not ask those questions first: the refusal's message is the whole
  // answer, and a second copy of that rule here would be a copy to
  // drift. `mutate` puts it on screen.
  import { untrack } from "svelte";
  import { sharedCatalog } from "./lib/stores/shared.svelte";
  import type { PromotedAssetDto } from "./bindings";

  let { assetId, defaultName }: { assetId: string; defaultName: string } =
    $props();

  // What the entry answers to on the line — the caller's, not the
  // asset's title. A line's names are the team's namespace, and what
  // somebody called a thing in their own library is not automatically
  // what it should be called on somebody else's line. Seeded from the
  // title because that is the likeliest answer, and editable because it
  // is a guess.
  let named = $state("");
  let sending = $state(false);
  let promoted = $state<PromotedAssetDto | null>(null);

  // Reset on the asset rather than on the name, so a pane moved to
  // another asset does not show the last one's outcome under it — and
  // so that renaming the asset in the pane above does not overwrite a
  // half-typed name here. That is why the seed is read untracked: this
  // effect is about which asset is showing.
  $effect(() => {
    void assetId;
    promoted = null;
    named = untrack(() => defaultName);
  });

  const line = $derived(
    sharedCatalog.lines.data.find((one) => one.id === sharedCatalog.selected) ??
      null,
  );
  const work = $derived(sharedCatalog.work);
  // Ended work is read like any other — the drawer lists it and shows
  // what was asked for — so a pursuit being selected does not mean
  // anything may enter against it. Kept apart here because the cost of
  // getting it wrong is not a refusal: the content verb streams the
  // whole body into the team's blob store and only then asks whether
  // the work has closed.
  const ended = $derived(work !== null && work.close !== null);

  async function promote(event: Event) {
    event.preventDefault();
    sending = true;
    try {
      promoted = await sharedCatalog.promote(assetId, named.trim());
    } catch {
      // Two kinds of failure arrive here and neither wants anything
      // added. A refusal from the team is already on screen — `mutate`
      // puts it there, and the client's message names which refusal it
      // is. A refusal from the catalog's own guards is not, and cannot
      // be: those fire when a caller skipped what this surface checks
      // above, so what they say is for a stack trace rather than for a
      // person. Caught so that neither becomes an unhandled rejection.
    } finally {
      sending = false;
    }
  }
</script>

<section class="promote">
  <h4>Hand to a team</h4>

  {#if sharedCatalog.session === null}
    <p class="quiet">
      Not connected to a team server. The shared-lines drawer is where a
      connection is made.
    </p>
    <button type="button" onclick={() => sharedCatalog.openPanel()}>
      open shared lines
    </button>
  {:else if work === null || line === null}
    <p class="quiet">
      A promotion goes onto open work, so there has to be some: open a line
      in the shared-lines drawer and open a pursuit against it. What is
      promoted lands on that work, and reaches the line when it closes
      satisfied.
    </p>
    <button type="button" onclick={() => sharedCatalog.openPanel()}>
      open shared lines
    </button>
  {:else if ended}
    <!-- Its own sentence rather than the one above, because the reader
         is not in the same place: something *is* selected, and what is
         wrong with it is that it has ended. -->
    <p class="quiet">
      The work showing in the shared-lines drawer has ended, and nothing
      enters against work that has. Open another pursuit against
      <strong>{line.name}</strong> to hand this over.
    </p>
    <button type="button" onclick={() => sharedCatalog.openPanel()}>
      open shared lines
    </button>
  {:else}
    <p class="quiet target">
      Onto <strong>{work.title ?? "(untitled work)"}</strong>, against
      <strong>{line.name}</strong>.
    </p>

    <!-- Before the control, because it is a thing to know before
         pressing rather than after. -->
    <p class="quiet travels">
      What goes: the file itself, and the marks you wrote on it. What
      stays: thumbnails, anything indexed from the file, and marks the
      import or a machine made — the team can make those again.
    </p>

    <form onsubmit={promote}>
      <label>
        Call it, on the line
        <input type="text" bind:value={named} required />
      </label>
      <button type="submit" disabled={sending || named.trim() === ""}>
        {sending ? "Handing over…" : "Promote"}
      </button>
    </form>

    {#if promoted !== null}
      <!-- What only the promotion knows. `sharedCatalog.said` carries
           the sentence; this carries the three facts a person may want
           to check afterwards. -->
      <dl class="outcome">
        {#if promoted.already_promoted}
          <div>
            <dt>Sent</dt>
            <dd>
              Nothing. This machine had already promoted it onto this line —
              which says what this machine did rather than what the team
              holds.
            </dd>
          </div>
          <!-- Labelled for what it is on this path. The client hashes
               before it reads the relation, so this is the file as it
               is now — not what the team took, which was hashed
               whenever the first promotion happened. -->
          <div>
            <dt>This file, now</dt>
            <dd class="mono">{promoted.digest}</dd>
          </div>
        {:else}
          <div>
            <dt>Entry</dt>
            <dd class="mono">{promoted.entry_id}</dd>
          </div>
          <div>
            <dt>The team's copy</dt>
            <dd class="mono">{promoted.team_asset_id ?? "—"}</dd>
          </div>
          <div>
            <dt>Digest</dt>
            <dd class="mono">{promoted.digest}</dd>
          </div>
        {/if}
        <div>
          <dt>Already there</dt>
          <dd>
            <!-- Three states, and the third is not "no". Nobody asked on
                 a repeat, because nothing was going to be sent. -->
            {#if promoted.bytes_already_held === null}
              not asked
            {:else if promoted.bytes_already_held}
              the team already held these bytes
            {:else}
              the team did not have these bytes
            {/if}
          </dd>
        </div>
      </dl>
    {/if}
  {/if}
</section>

<style>
  .promote {
    border-top: 1px solid rgba(128, 128, 128, 0.25);
    margin-top: 0.9rem;
    padding-top: 0.7rem;
  }
  h4 {
    margin: 0 0 0.3rem;
    font-size: 0.82rem;
    font-weight: 500;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.78rem;
    line-height: 1.45;
    margin: 0.3rem 0;
  }
  .target strong {
    opacity: 1;
    font-weight: 600;
  }
  .travels {
    border-left: 2px solid rgba(128, 128, 128, 0.4);
    padding-left: 0.5rem;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-top: 0.5rem;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.75rem;
    opacity: 0.85;
  }
  button {
    align-self: flex-start;
    background: none;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.78rem;
    padding: 0.15rem 0.6rem;
  }
  .outcome {
    margin: 0.6rem 0 0;
    font-size: 0.75rem;
  }
  .outcome div {
    display: flex;
    gap: 0.5rem;
    padding: 0.1rem 0;
  }
  .outcome dt {
    flex: 0 0 7rem;
    opacity: 0.6;
  }
  .outcome dd {
    margin: 0;
  }
  .mono {
    font-family: ui-monospace, monospace;
    font-size: 0.7rem;
    overflow-wrap: anywhere;
  }
</style>
