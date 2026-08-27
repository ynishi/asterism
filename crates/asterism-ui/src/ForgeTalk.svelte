<script lang="ts">
  // ForgeTalk — what was said about one thing in the forge.
  //
  // #170's fourth surface, and its first question was not whether but
  // which: teach the app's thread drawer a forge anchor, or give forge
  // conversations their own place. They are separate aggregates down to
  // the service, and their messages answer to different fields — a
  // forge message carries what it said first and every revision of it,
  // an app-level one carries role and refs — so one surface holding
  // both would be a component with two halves that never run together.
  // This is the second answer.
  //
  // **One place, four anchors.** A conversation hangs off a piece of
  // work, a round, an entry as that round had it, or a change point on
  // the line — the four `Anchor` has, and a fifth would not compile
  // past the resolver. They are opened from two components and shown
  // here, because a conversation is about something rather than beside
  // it: opening one from a round should not move the reader away from
  // the round.
  //
  // **Every correction is shown, and this is the surface that could
  // have shown only the latest.** `ForgeMessageDto` carries `said`,
  // `first_said` and every revision, and `ForgeThreadDto` says why it
  // is shaped to carry them. So a corrected message says that it was
  // corrected, and what it said before is one press away rather than
  // gone.
  //
  // **A conversation cannot be opened empty**, which is the model's
  // too — it is what was said in it, so the first thing said is part of
  // opening one.
  import { forgeCatalog } from "./lib/stores/forge.svelte";
  import type { ForgeMessageDto, ForgeThreadDto } from "./bindings";

  const anchor = $derived(forgeCatalog.talkingAbout);

  // The composers. One draft per thread rather than one shared: two
  // half-written replies in two conversations are two drafts, and a
  // single field would silently be the first one moved into the second.
  let starting = $state("");
  let replies = $state<Record<string, string>>({});
  let correcting = $state<string | null>(null);
  let correction = $state("");
  let busy = $state(false);
  // Which messages are showing what they said before.
  let unfolded = $state<Record<string, boolean>>({});

  async function start(event: Event) {
    event.preventDefault();
    if (anchor === null || starting.trim() === "") return;
    busy = true;
    try {
      await forgeCatalog.openTalk(anchor, starting.trim(), null);
      starting = "";
    } finally {
      busy = false;
    }
  }

  async function say(thread: ForgeThreadDto) {
    const said = (replies[thread.id] ?? "").trim();
    if (anchor === null || said === "") return;
    busy = true;
    try {
      await forgeCatalog.sayInTalk(anchor, thread.id, said, null);
      replies = { ...replies, [thread.id]: "" };
    } finally {
      busy = false;
    }
  }

  async function correct(thread: ForgeThreadDto, message: ForgeMessageDto) {
    if (anchor === null || correction.trim() === "") return;
    busy = true;
    try {
      await forgeCatalog.amendInTalk(
        anchor,
        thread.id,
        message.id,
        correction.trim(),
      );
      correcting = null;
      correction = "";
    } finally {
      busy = false;
    }
  }

  function when(ms: number): string {
    return new Date(ms).toLocaleString();
  }
</script>

{#if anchor !== null}
  <section class="talk" aria-label="What was said">
    <header>
      <h4>Said about {anchor.about}</h4>
      <button type="button" onclick={() => forgeCatalog.stopTalking()}>
        close
      </button>
    </header>

    {#if forgeCatalog.threads.loading}
      <p class="quiet">Reading…</p>
    {:else if forgeCatalog.threads.data.length === 0}
      <p class="quiet">Nothing said about this yet.</p>
    {/if}

    {#each forgeCatalog.threads.data as thread (thread.id)}
      <article class="thread">
        {#if thread.title !== null}
          <h5>{thread.title}</h5>
        {/if}
        <ol>
          {#each thread.messages as message (message.id)}
            <li>
              <p class="said">{message.said}</p>
              <p class="quiet by">
                {when(message.at_ms)}
                {#if message.revisions.length > 0}
                  <!-- Said, and said differently since. The count is
                       the message's own and goes stale with nothing,
                       because it is read off the record each time. -->
                  · corrected {message.revisions.length}
                  {message.revisions.length === 1 ? "time" : "times"}
                  <button
                    type="button"
                    onclick={() =>
                      (unfolded = {
                        ...unfolded,
                        [message.id]: !unfolded[message.id],
                      })}
                  >
                    {unfolded[message.id] ? "hide what it said" : "what it said"}
                  </button>
                {/if}
                <button
                  type="button"
                  onclick={() => {
                    correcting = message.id;
                    correction = message.said;
                  }}
                >correct</button>
              </p>

              {#if unfolded[message.id]}
                <ol class="revisions">
                  <li>
                    <span class="quiet">first</span>
                    {message.first_said}
                  </li>
                  {#each message.revisions as revision (revision.at_ms)}
                    <li>
                      <span class="quiet">{when(revision.at_ms)}</span>
                      {revision.said}
                    </li>
                  {/each}
                </ol>
              {/if}

              {#if correcting === message.id}
                <form onsubmit={(e) => { e.preventDefault(); correct(thread, message); }}>
                  <input type="text" bind:value={correction} />
                  <button type="submit" disabled={busy}>save the correction</button>
                  <button type="button" onclick={() => (correcting = null)}>
                    cancel
                  </button>
                </form>
              {/if}
            </li>
          {/each}
        </ol>

        <form onsubmit={(e) => { e.preventDefault(); say(thread); }}>
          <input
            type="text"
            placeholder="say something"
            value={replies[thread.id] ?? ""}
            oninput={(e) =>
              (replies = { ...replies, [thread.id]: e.currentTarget.value })}
          />
          <button type="submit" disabled={busy}>say</button>
        </form>
      </article>
    {/each}

    <!-- Opening one and saying the first thing in it are one gesture,
         because the model has no empty conversation to open. -->
    <form class="start" onsubmit={start}>
      <input type="text" bind:value={starting} placeholder="start a conversation" />
      <button type="submit" disabled={busy || starting.trim() === ""}>
        start
      </button>
    </form>
  </section>
{/if}

<style>
  .talk {
    border-top: 1px solid rgba(128, 128, 128, 0.3);
    margin-top: 0.9rem;
    padding-top: 0.5rem;
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
  }
  h4 {
    margin: 0;
    font-size: 0.82rem;
    font-weight: 500;
  }
  h5 {
    margin: 0 0 0.2rem;
    font-size: 0.78rem;
    font-weight: 500;
  }
  header button {
    margin-left: auto;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .thread {
    border-left: 2px solid rgba(128, 128, 128, 0.3);
    margin: 0.5rem 0;
    padding-left: 0.5rem;
  }
  .thread > ol > li {
    padding: 0.2rem 0;
  }
  .said {
    margin: 0;
    font-size: 0.82rem;
  }
  .by {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    margin: 0.1rem 0 0;
  }
  .revisions {
    border-left: 1px solid rgba(128, 128, 128, 0.3);
    margin: 0.2rem 0 0.3rem 0.4rem;
    padding-left: 0.5rem;
    font-size: 0.76rem;
  }
  .revisions li {
    padding: 0.1rem 0;
  }
  .quiet {
    opacity: 0.7;
    font-size: 0.76rem;
    margin: 0.3rem 0;
  }
  form {
    display: flex;
    gap: 0.4rem;
    margin: 0.3rem 0;
  }
  input {
    flex: 1 1 auto;
    min-width: 0;
    box-sizing: border-box;
  }
  button {
    background: none;
    border: 1px solid rgba(128, 128, 128, 0.4);
    border-radius: 0.2rem;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    padding: 0.1rem 0.4rem;
  }
  .by button {
    border: 0;
    opacity: 0.75;
    padding: 0;
    text-decoration: underline;
  }
</style>
