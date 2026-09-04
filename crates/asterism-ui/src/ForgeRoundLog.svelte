<script lang="ts">
  // The round log — "What was asked for" — factored out of `ForgeWork`
  // and `SharedLineWork` (#217): the two files carried byte-identical
  // `when` / `opName` / `summarise` functions and near-identical
  // markup for this one section. The row shape itself never differed;
  // what did was one optional verb and one CSS value, both named below
  // (`onTalkAboutRound`/`onTalkAboutOp`, `dividerColor`) rather than
  // picked once and imposed on both callers.
  //
  // `ForgeWork` lets a reader open the conversation about a round or an
  // entry within it (`forgeCatalog.talkAbout` — "shows what is said
  // about one thing, and reads it"; starting one from there is
  // `openTalk`'s to do). `SharedLineWork` has no such verb — the
  // member's client carries no thread commands, which `sharedCatalog`'s
  // own header records. So the "say something" buttons are the one
  // thing this component does not own outright: `onTalkAboutRound` /
  // `onTalkAboutOp` are optional, and a caller that omits them gets the
  // log without the buttons rather than buttons that do nothing.
  //
  // `projected` stays a prop rather than something this reads off a
  // store, because there are two catalogs to read it from (#148
  // decision 16): `forgeCatalog.projection` here, `sharedCatalog.projection`
  // in `SharedLineWork`, and only the caller knows which one names this
  // work's entries. Each is a getter over two `Resource`s, not a
  // `Resource` this component could ask for on its own — and a round
  // can in any case name an asset the line has never held, which is why
  // the projection carries its own answer rather than reading one off
  // `forgeCatalog.cards`/`sharedCatalog.cards` directly.
  //
  // `dividerColor` is a prop rather than a fixed rule for the same
  // reason the verb is optional: the two callers never agreed on it.
  // `ForgeWork` drew this divider in `rgba(128, 128, 128, 0.25)`,
  // `SharedLineWork` in `rgba(255, 255, 255, 0.14)` — a difference this
  // component would otherwise erase by picking one of the two.
  import type { ForgeOpDto, ForgeRoundDto } from "./bindings";
  import type { ForgeProjectedEntry } from "./lib/forge-projection";

  interface Props {
    rounds: ForgeRoundDto[];
    projected: ForgeProjectedEntry[];
    dividerColor: string;
    onTalkAboutRound?: (round: ForgeRoundDto) => void;
    onTalkAboutOp?: (round: ForgeRoundDto, op: ForgeOpDto) => void;
  }

  let { rounds, projected, dividerColor, onTalkAboutRound, onTalkAboutOp }: Props =
    $props();

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }

  /** What to call an entry an operation names — the op's own name when
   *  it carries one, and otherwise whatever the fold has for it, which
   *  may be a name another round of this work asked for rather than
   *  one the line has ever said. */
  function opName(op: ForgeOpDto): string {
    return (
      op.name ??
      projected.find((row) => row.entryId === op.entry_id)?.name ??
      op.entry_id
    );
  }

  function summarise(round: ForgeRoundDto): string {
    return `${round.ops.length} ${round.ops.length === 1 ? "operation" : "operations"}`;
  }
</script>

<!-- Oldest first, which is the order the chain holds and the order
     somebody reads a piece of work in — unlike a line's history, where
     the question is what happened last. -->
<h4>What was asked for</h4>
<ol class="rounds" style:--divider-color={dividerColor}>
  {#each rounds as round (round.id)}
    <li>
      <p class="round-head">
        <span>{summarise(round)}</span>
        <span class="quiet">{when(round.at_ms)}</span>
        {#if onTalkAboutRound}
          <button class="talk-about" onclick={() => onTalkAboutRound(round)}>
            say something
          </button>
        {/if}
      </p>
      {#if round.note !== null}
        <p class="quiet note">{round.note}</p>
      {/if}
      <ul class="ops">
        {#each round.ops as op (op.entry_id + op.kind)}
          <li>
            <!-- The verb is stated rather than read off what moved: an
                 operation carries one, which a change row does not. -->
            <span class="kind">{op.kind}</span>
            <span class="op-name">{opName(op)}</span>
            {#if onTalkAboutOp}
              <!-- An entry as *that round* had it, which is why the
                   round is named beside it: the same entry in two
                   rounds is two things to talk about. -->
              <button class="talk-about" onclick={() => onTalkAboutOp(round, op)}>
                say something
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    </li>
  {/each}
</ol>

{#if rounds.length === 0}
  <p class="quiet">Nothing asked for yet.</p>
{/if}

<style>
  h4 {
    margin: 0.9rem 0 0.3rem;
    font-size: 0.82rem;
    font-weight: 500;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.78rem;
    margin: 0.3rem 0;
  }
  .note {
    font-style: italic;
  }
  .rounds {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rounds > li {
    border-top: 1px solid var(--divider-color);
    padding: 0.4rem 0;
  }
  .round-head {
    display: flex;
    gap: 0.6rem;
    margin: 0;
    font-size: 0.78rem;
  }
  .ops {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .ops li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.78rem;
    padding: 0.15rem 0;
  }
  .kind {
    min-width: 4rem;
    opacity: 0.75;
  }
  .op-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .talk-about {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    margin-left: auto;
    opacity: 0.7;
    padding: 0;
    text-decoration: underline;
  }
</style>
