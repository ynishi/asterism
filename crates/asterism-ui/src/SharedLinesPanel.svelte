<script lang="ts">
  // SharedLinesPanel — the lines a team hosts, in their own drawer.
  //
  // The separation is the requirement (#148 decision 16): shared lines
  // list here rather than mixed into the local ones, because they come
  // from somewhere else and a surface that hid that would be claiming
  // one library where there are two. Nothing on this panel is a copy of
  // anything — each read is a request to the server, and when the
  // connection goes the panel goes empty rather than stale.
  //
  // 0-prop by design, like DispatchHistoryPanel: it reads
  // `sharedCatalog` and `activeFilter.activePersona` directly, and the
  // App only mounts it.
  //
  // A team is picked from the ones this account is in, or named by id.
  // What the picker changed and what it did not is argued in the
  // catalog's header; what this panel decides is where the two go, and
  // why the id field is still here at all — both under "Two columns"
  // below and beside the control.
  //
  // What this panel reads from that header is `phase`: there is nobody
  // to ask, there is nobody chosen to ask about, or there is. The three
  // branches below are those three, and the empty list belongs to the
  // last of them alone — under either of the others it would be
  // answering for a team on nobody's behalf.
  // Tabs rather than one column, argued in the catalog: the lines a
  // team hosts, its roster and its ledger are three answers about one
  // team, so moving between them changes the question rather than the
  // subject.
  //
  // What this component adds is where everything else goes. The
  // connection and the team are what the tabs are answers about, so
  // they are picked from before the tabs are read. Publishing is
  // gated on the lines tab, because it seeds a line and a line is what
  // that tab is for — though the form itself sits in the rail, at the
  // lines list's own foot, argued under "Two columns" below. Founding
  // a team sits with the teams, argued where the control is.
  //
  // # Two columns, the forge's width (#217)
  //
  // The same shape `ForgePanel` draws, at the same `min(52rem, 96vw)`:
  // a rail on the left and the thing being read on the right. This
  // drawer was one column at 30rem, and a line replaced the list when
  // it opened — which was the trade that width forced, and what it
  // cost was the line. Everything a person had to pass (the note, the
  // session, the devices, the teams, the typed id, the founding
  // button, the team's tabs, the line's header, the line's tabs) stayed
  // on screen above the line's body, and with a pursuit open the body
  // began in the bottom tenth.
  //
  // The rail holds what is picked from: who is signed in, the teams,
  // and — once a team is on — its lines, with the form that publishes
  // one of this machine's own at their foot (#217). The body holds
  // what is read about the pick: the team's three tabs, and inside
  // `lines`, once a team has one to open, the open line's frame.
  // Typing a team id, which
  // the instance admin still needs (their list is empty while their
  // reach is not), sits behind a disclosure in the rail rather than as
  // a form beside a list that already holds the same rows.
  //
  // Not a component shared with `ForgePanel`, and not the same store
  // (#148 decision 16, #170 §1): two sources, two panels. What they
  // share is the shape — and, where the shape is code rather than CSS,
  // the code: `axes()` reads a change row the same way for both and
  // lives in `lib/forge-projection.ts`; the fold's rows are
  // `ForgeRoundLog.svelte`, shared between `SharedLineWork` and
  // `ForgeWork`; and both of this file's tab rows are `TabStrip.svelte`,
  // shared with `ForgePanel`'s. The shell stays as each file's own
  // markup, because a component for a flex row of two columns would
  // carry less than its props do, and the two rails hold nothing
  // alike enough for a snippet to be worth passing in — that much of
  // the departure holds. What did not hold, checked the same way the
  // tab strip's reasoning was, is the two shells' actual values: the
  // gap and the rail's fixed width had drifted (1.2rem/15rem here,
  // `ForgePanel`'s 1rem/12rem there) with nothing arguing for the
  // difference, the same shape the tab strip's drift took.
  // `--drawer-shell-gap` and `--drawer-rail-width` in `app.css` hold
  // the one answer both files read now, unified to `ForgePanel`'s
  // values — `ForgePanel`'s `.lines` picked up the `min-width: 0` this
  // file's `.rail` already had, so the rule sets match as well as the
  // values do. The shell's markup stays a departure from #217's Shape
  // section; its values no longer are.
  //
  // One more of #217's asks is not built as written, on purpose. The
  // signed-in row keeps Disconnect beside it and the devices behind
  // their own disclosure rather than both behind a menu, security
  // over the literal ask: a menu hides the one verb that ends the
  // connection and the list that says which machines can open one
  // behind an extra click, which is the wrong side to add friction to.
  //
  // The publish form, by contrast, is built where #217 asked: at the
  // lines list's own foot in the rail (below), gated the same way it
  // always was — a team on (`ready`), the lines tab open, and no line
  // picked — rather than by anything about sitting in the rail now.
  // Everywhere else the rail draws the same no matter which of the
  // body's tabs is open; this is the one thing in it that reads `tab`
  // at all, and the reason is the same one #217's own reasoning for
  // not putting it there gave: offering to seed a line is a thing to
  // do from where the lines are read, not from the roster or the
  // ledger, so the guard the departure needed stays even though the
  // position no longer does. The local line it seeds from is picked
  // from the forge's own list rather than typed as an id, since this
  // machine knows every one of them.
  import { untrack } from "svelte";
  import SharedLineWork from "./SharedLineWork.svelte";
  import TabStrip from "./TabStrip.svelte";
  import { confirmCatalog } from "./lib/stores/confirm.svelte";
  import { isDeparture, sharedCatalog } from "./lib/stores/shared.svelte";
  import { forgeCatalog } from "./lib/stores/forge.svelte";
  import { activeFilter } from "./lib/stores/filter.svelte";
  import { fmtDateTime } from "./lib/formatters";
  import { axes } from "./lib/forge-projection";

  let baseUrl = $state("http://127.0.0.1:8787");
  let login = $state("");
  let password = $state("");
  // Unticked to begin with, and it stays a choice rather than a
  // default: ticking it puts a credential on this machine for months,
  // and that is not something to arrive at by not reading a form.
  let remember = $state(false);

  // What this machine remembered, put back in the fields (#204).
  //
  // An effect rather than an initialiser because the catalog reads it
  // when the panel opens, which is after this component is built. It
  // runs when `stored` changes and not when somebody types, so a
  // half-typed server is never overwritten — and a rejected credential
  // leaves `stored` in place precisely so the login survives into the
  // form the person is about to use.
  $effect(() => {
    const held = sharedCatalog.stored;
    if (held === null) return;
    baseUrl = held.base_url;
    login = held.login;
  });

  // Whether the server typed here signs people in through a provider
  // (#163), asked as the URL settles rather than on every keystroke —
  // a half-typed server is not one to ask, and the answer is a fact
  // about the server rather than about what was typed.
  $effect(() => {
    const url = baseUrl;
    // Only while the form is showing: a connected window has no
    // button to decide about, and no reason to knock on a server.
    if (sharedCatalog.phase !== "disconnected") return;
    const timer = setTimeout(() => void sharedCatalog.probeProvider(url), 400);
    return () => clearTimeout(timer);
  });

  // The provider button is only shown for the server the answer was
  // about: a URL edited after the probe answered is a server nobody
  // has asked yet.
  const provider = $derived(
    sharedCatalog.providerFor === baseUrl.trim() ? sharedCatalog.provider : null,
  );

  // Whether the device list is showing. Closed to begin with: it
  // answers a question about the account rather than about the work,
  // which is the same reason the ledger's payloads are behind a
  // toggle.
  let devicesOpen = $state(false);

  // Whether the typed-id form is showing (#217). Closed to begin with:
  // the list above it holds every team this account is in, and the
  // reader who needs the field — the instance admin, whose list is
  // empty while their reach is not — opens it once.
  let byIdOpen = $state(false);

  // What "Start a team of your own" asks for (#218) — a team is named
  // at founding rather than left to read as an id.
  let newTeamName = $state("");

  // The local lines the publish form picks from. Read once, the first
  // time a team is on, because that is when the form can be shown;
  // `forgeCatalog` reads the same list when its own drawer opens, and
  // one more read here costs one call over this machine's store.
  //
  // Once, by a flag rather than by looking at what the list holds: a
  // successful read writes a fresh array to `data` even when it is
  // empty, and an effect that read `data` to decide would be re-run by
  // its own answer.
  let localLinesAsked = false;
  $effect(() => {
    if (sharedCatalog.phase !== "ready" || localLinesAsked) return;
    localLinesAsked = true;
    if (untrack(() => forgeCatalog.lines.data.length) > 0) return;
    void forgeCatalog.lines.load();
  });

  // The field is this component's, not the catalog's.
  //
  // Bound straight to `sharedCatalog.teamId` it changed the team every
  // read is made against as somebody typed, so a ledger walk started on
  // one team would continue against another — the next page requested
  // from team B with team A's cursor, and its answer appended to team
  // A's list. Typing is not the naming act — `lookAt` is, and says so
  // — so what is half-typed here reaches nothing until a gesture does.
  //
  // Seeded from the catalog, because a connection outlives this drawer
  // and reopening it should show the team it was last looking at.
  let teamField = $state(sharedCatalog.teamId);

  let tab = $state<"lines" | "roster" | "ledger">("lines");
  // Which of the line's three answers is showing, once one is open.
  //
  // Component state rather than the catalog's, and reset when a line
  // is opened rather than remembered per line: the reads behind all
  // three arrive together, so this says which is drawn and nothing
  // more. A reader who left the last line on its history is not asking
  // to meet the next one there.
  let lineTab = $state<"contents" | "work" | "history">("contents");
  // One event's payload open at a time, by `event_id`.
  let openPayload = $state<string | null>(null);
  // One change point's rows open at a time, by change point id.
  let openPoint = $state<string | null>(null);

  /// The line the frame is about, or null when the list is showing.
  ///
  /// Found in the list rather than held beside the selection, so a
  /// re-read that no longer carries it takes the frame down with it
  /// rather than leaving a header over somebody else's line.
  const current = $derived(
    sharedCatalog.selected === null
      ? null
      : (sharedCatalog.lines.data.find(
          (line) => line.id === sharedCatalog.selected,
        ) ?? null),
  );

  // Publishing asks for more than a click, and all of it is init-time.
  let publishLineId = $state("");
  let publishName = $state("");
  let reenact = $state(false);

  const STRATEGY = "mainline-first";

  async function connect(event: Event) {
    event.preventDefault();
    await sharedCatalog.connect(baseUrl, login, password, remember);
    password = "";
    // Back to unticked, like the password. Disconnecting revoked the
    // token this ticking minted, so a box left ticked would mint
    // another on the next connect from a choice somebody made about a
    // connection they have since ended.
    remember = false;
    if (teamField) await sharedCatalog.lookAt(teamField);
  }

  // The other way in (#163): the browser is where the person signs
  // in, and nothing typed here goes with them but the server. The
  // box means the same thing it means above, and is unticked after
  // for the same reason — when a session was opened; a cancel opened
  // none and minted nothing, so the box stays as it was.
  async function connectWithProvider() {
    const opened = await sharedCatalog.connectWithProvider(baseUrl, remember);
    if (!opened) return;
    remember = false;
    if (teamField) await sharedCatalog.lookAt(teamField);
  }

  // The list reads on demand, like the roster and the ledger: what
  // this account has stored on its other machines is a question asked
  // apart from working with what a team holds.
  async function toDevices() {
    devicesOpen = !devicesOpen;
    if (devicesOpen && sharedCatalog.deviceTokens.data.length === 0) {
      await sharedCatalog.deviceTokens.load({});
    }
  }

  async function look(event: Event) {
    event.preventDefault();
    // Everything naming a team has to let go of is `lookAt`'s, written
    // once there rather than at each caller.
    await sharedCatalog.lookAt(teamField);
    // Folded away once it has done its one job: the team it named is
    // the one being read now, and a form standing open under the list
    // is what #217 took out of the main path.
    byIdOpen = false;
    await refreshOpenTab();
  }

  // The second gesture that reaches the naming act, and `lookAt` says
  // the two are equal. It goes the same way `look` does — `lookAt`,
  // then whichever tab is open — and writes the field as well, so
  // pressing a row leaves it showing what was picked rather than what
  // somebody typed last.
  async function choose(teamId: string) {
    teamField = teamId;
    await sharedCatalog.lookAt(teamId);
    await refreshOpenTab();
  }

  // Naming a team drops what the on-demand tabs held, because what they
  // held was about the team that was named before. Whichever of them is
  // showing has to ask again, or it shows an unread state under a tab
  // the reader never left.
  async function refreshOpenTab() {
    if (tab === "ledger") await sharedCatalog.readLedgerPage();
    if (tab === "roster") {
      await sharedCatalog.roster.load({ teamId: sharedCatalog.teamId });
    }
  }

  // The ledger reads on demand rather than beside the lines: it answers
  // what the team did, which is a question asked apart from working
  // with what it holds.
  async function toLedger() {
    tab = "ledger";
    if (!sharedCatalog.ledgerRead) await sharedCatalog.readLedgerPage();
  }

  // The roster reads on demand for the same reason: who is in a team
  // is a question about the team rather than about the work.
  async function toRoster() {
    tab = "roster";
    if (sharedCatalog.roster.data === null) {
      await sharedCatalog.roster.load({ teamId: sharedCatalog.teamId });
    }
  }

  // Founding a team names it too (#218) — asked for here rather than
  // left to read as an id. Somebody who just made one wants to be
  // looking at it, and the alternative is copying an id out of a
  // message into the field directly above.
  async function makeTeam(event: Event) {
    event.preventDefault();
    const name = newTeamName.trim();
    if (name === "") return;
    const teamId = await sharedCatalog.createTeam(name);
    newTeamName = "";
    teamField = teamId;
    await sharedCatalog.lookAt(teamId);
    await refreshOpenTab();
  }

  // The roster writes (#210). Whether to draw them at all is one
  // question asked once: an owner's verbs are not a member's, and a
  // member shown a control it cannot press is being offered a refusal.
  // The server decides regardless — this only decides what to offer.
  //
  // An instance admin who holds no membership row has no role, so
  // `iOwn` is false and the four member-shaped controls stay off their
  // screen — which is right, because #83 §1 grants an admin no
  // implicit invite, remove or role change inside a team they do not
  // own. Deleting is the one their standing does carry, so it asks a
  // second question.
  //
  // An admin who is also a member holds a row, so `iOwn` reads it like
  // anybody else's — and `mayDelete` asks the standing besides,
  // because the server does: a verb the row's role does not permit
  // falls through to the admin capacity rather than stopping there.
  // The two are separate fields for that reason.
  const iOwn = $derived(sharedCatalog.myRole === "owner");
  const mayDelete = $derived(iOwn || sharedCatalog.iAmAdmin);

  // The team on's own name, or its id for a team from before #218 —
  // read off the same list the rail's rows draw from rather than a
  // second read, since founding or picking one already put it there.
  const currentTeamName = $derived(
    sharedCatalog.teams.data.find((t) => t.team_id === sharedCatalog.teamId)
      ?.name ?? sharedCatalog.teamId,
  );

  // The rename field, seeded from the team on and re-seeded when
  // *which team* changes — switching teams mid-edit is switching what
  // a typed name would rename, so there is nothing to preserve across
  // that. Keyed on `teamId` rather than on `currentTeamName` itself:
  // a successful rename of the team already on also changes
  // `currentTeamName`, by reloading the list the name is read from,
  // and re-seeding on that would overwrite whatever the field holds
  // — the name just submitted, or the start of a second edit typed
  // before the first request answered — with what is now stale
  // input read back as if it were fresh.
  let renameValue = $state("");
  $effect(() => {
    void sharedCatalog.teamId;
    renameValue = untrack(() => currentTeamName);
  });

  let inviteLogin = $state("");
  let inviteId = $state("");
  let inviteRole = $state("member");
  // Closed to begin with (#218), the same reason `byIdOpen` is: the
  // login form above it is the one a person reaches for first, and
  // this is reachable for when the login is not known.
  let inviteByIdOpen = $state(false);

  async function inviteByLogin(event: Event) {
    event.preventDefault();
    await sharedCatalog.inviteMemberByLogin(inviteLogin.trim(), inviteRole);
    inviteLogin = "";
  }

  async function invite(event: Event) {
    event.preventDefault();
    await sharedCatalog.inviteMember(inviteId.trim(), inviteRole);
    inviteId = "";
  }

  // Removing somebody, deleting a team, and leaving (askLeave, below)
  // all ask first, and are not undone by anything. A role change is
  // the one row verb that does not: it is undone by the button
  // beside it.
  async function askRemove(userId: string, login: string) {
    const ok = await confirmCatalog.open({
      title: "Remove this member?",
      body: `${login} loses everything this team holds. What they did stays in the ledger, under the name it read at the time.`,
      confirmLabel: "Remove",
      danger: true,
    });
    if (!ok) return;
    await sharedCatalog.removeMember(userId);
    // The reader's own row offers `leave` instead of this, so the
    // branch is unreachable from the roster as it stands. It is here
    // because the store handles the same case for the same reason —
    // the server permits an owner to remove themself, and a path
    // nobody draws today is still a path.
    if (userId === sharedCatalog.session) teamField = "";
  }

  async function askLeave() {
    const ok = await confirmCatalog.open({
      title: "Leave this team?",
      body: "You lose everything it holds. What you did stays in its ledger, and getting back in takes an invitation.",
      confirmLabel: "Leave",
      danger: true,
    });
    if (!ok) return;
    await sharedCatalog.leaveTeam();
    teamField = "";
  }

  async function askDeleteTeam() {
    const ok = await confirmCatalog.open({
      title: "Delete this team?",
      body: "Its lines, everything it holds, and its whole ledger go with it. This cannot be undone.",
      confirmLabel: "Delete Forever",
      danger: true,
    });
    if (!ok) return;
    await sharedCatalog.deleteTeam();
    teamField = "";
  }


  // Opening a line lands on its contents, which is what somebody
  // pressed a line to see. `show` is what reads all three.
  async function openLine(lineId: string) {
    lineTab = "contents";
    openPoint = null;
    await sharedCatalog.show(lineId);
  }

  async function publish(event: Event) {
    event.preventDefault();
    await sharedCatalog.publish(publishLineId, publishName, STRATEGY, reenact);
    publishLineId = "";
    publishName = "";
    reenact = false;
  }
</script>

{#if sharedCatalog.open}
  <!-- Backdrop absorbs outside-click and Escape; the drawer itself
       stopPropagation so an interior click never closes. -->
  <div
    class="drawer-backdrop"
    onclick={() => sharedCatalog.closePanel()}
    onkeydown={(e) => e.key === "Escape" && sharedCatalog.closePanel()}
    role="button"
    tabindex="-1"
    aria-label="Close the team"
  >
    <div
      class="drawer"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      tabindex="-1"
      aria-label="Team"
    >
      <header class="drawer-head">
        <h3>Team</h3>
        <button
          class="drawer-close"
          onclick={() => sharedCatalog.closePanel()}
          aria-label="Close"
        >✕</button>
      </header>

      <!-- These lines are on a team's server. They are read from it
           every time this panel opens; none of them is kept. The
           qualifier is #204's: a device token may be on this machine,
           in the keychain, and it is not one of these lines. One line
           under the title rather than a paragraph above everything:
           it is the drawer's standing, not a step on the way in.

           Three verbs cross this boundary and each lives where it
           acts rather than here — clone on a shared line's contents,
           publish under the shared lines list, promote on the asset
           — so this sentence names all three and which is which
           (#220), where the drawer that stands for the boundary is
           the one place a person reading it fresh would look. -->
      <p class="drawer-sub">
        Hosted by a team and read from it — none of the work shown here
        is stored on this machine. Three verbs cross this boundary:
        clone takes a copy of what a shared line holds, publish sends a
        line of yours the other way, and promote — on the asset itself
        — hands one over.
      </p>

      {#if sharedCatalog.providerAttempt !== null}
        <!-- A sign-in through the provider waiting for the browser
             (#163), above the phase switch on purpose: the wait owns
             the connection while it runs — the store opens no session
             under it — and it outlives this drawer being closed and
             opened again, so its way out has to be wherever the drawer
             is and not only inside the form. The URL is shown rather
             than only opened: a browser that did not open, or opened
             as somebody else, leaves the person with the page to reach
             by hand. The cancel is the way out before the wait's own
             end — a tab closed without finishing is otherwise a wait. -->
        <p class="drawer-note" data-testid="provider-waiting">
          Finish signing in in your browser. If it did not open, go to
          <code data-testid="provider-start-url"
            >{sharedCatalog.providerAttempt.startUrl}</code
          >.
          <button
            type="button"
            onclick={() => sharedCatalog.cancelProviderSignIn()}
          >
            Cancel
          </button>
        </p>
      {/if}

      {#if sharedCatalog.phase === "disconnected"}
        {#if sharedCatalog.storedRejected}
          <!-- The one outcome of a silent reconnect worth a sentence.
               Nothing was stored and the stored thing was refused look
               identical — this form — and only the second is somebody
               else's doing. Which end it met is the server's word
               (#163, #213), and is what makes the sentence something
               to act on: a token that sat unused is not one somebody
               took back, and one the server's admin took back is not
               one the person did. -->
          <p class="drawer-said">
            This machine's saved sign-in was refused, and has been
            forgotten.
            {#if sharedCatalog.storedRejectedReason === "expired"}
              It reached the end of its life.
            {:else if sharedCatalog.storedRejectedReason === "idle"}
              It went unused for longer than the server allows.
            {:else if sharedCatalog.storedRejectedReason === "revoked"}
              It was revoked.
            {:else if sharedCatalog.storedRejectedReason === "revoked_by_instance"}
              Whoever runs the server signed this device out.
            {:else if sharedCatalog.storedRejectedReason === "locked"}
              The account is locked on that server; ask whoever runs
              it, and sign in again once the lock is lifted.
            {:else}
              It was either revoked or it expired.
            {/if}
            Signing in again does not replace it &mdash; tick
            &ldquo;Remember this device&rdquo; below to save a new one.
          </p>
        {/if}
        <!-- Every control is off while a sign-in through the provider
             waits for the browser (#163): a password sign-in landing
             under it would be written over when the wait ended — the
             store refuses one too — and the wait has a cancel of its
             own above. -->
        <form class="drawer-form" onsubmit={connect}>
          <label>
            Server
            <input
              type="url"
              bind:value={baseUrl}
              required
              disabled={sharedCatalog.providerBusy}
            />
          </label>
          <label>
            Login
            <input
              type="text"
              bind:value={login}
              required
              autocomplete="username"
              disabled={sharedCatalog.providerBusy}
            />
          </label>
          <label>
            Password
            <input
              type="password"
              bind:value={password}
              required
              autocomplete="current-password"
              disabled={sharedCatalog.providerBusy}
            />
          </label>
          <!-- What ticking this does, said before it is ticked: the
               server mints a token for this machine and the keychain
               holds it. The password is not stored under any key, and
               signing out gives the token back. -->
          <label class="drawer-check">
            <input
              type="checkbox"
              bind:checked={remember}
              disabled={sharedCatalog.providerBusy}
            />
            Remember this device
          </label>
          <p class="drawer-cost">
            {#if remember}
              This server will issue a token for this device, kept in
              the system keychain so the next window signs in without
              asking. Your password is not stored. Disconnecting
              revokes it; closing the window keeps it.
            {:else}
              You will be asked for this password again next time.
            {/if}
          </p>
          <button type="submit" disabled={sharedCatalog.providerBusy}>Connect</button>
          {#if provider !== null}
            <!-- Shown only when the server said it offers one (#163).
                 A button rather than a second form: the login and the
                 password above are not part of this, and the browser
                 is where the person proves who they are. The "remember"
                 box above applies here too, and says so. -->
            <p class="drawer-note">
              Or sign in in your browser through <strong>{provider.name}</strong>.
              The device is remembered the same way if the box above is ticked.
            </p>
            <button
              type="button"
              onclick={connectWithProvider}
              disabled={sharedCatalog.providerBusy}
            >
              Sign in with {provider.name}
            </button>
          {/if}
        </form>
      {:else}
        <section class="team-plane" aria-label="The team and its lines">
        <!-- The rail: what is picked from. Its order is the order a
             person meets things — who they are here, which teams they
             are in, and which lines the team on has. -->
        <div class="rail">
        <div class="drawer-session">
          <span title={sharedCatalog.session ?? undefined}>
            Signed in as {sharedCatalog.identity?.login ?? sharedCatalog.session}
          </span>
          <button type="button" onclick={() => sharedCatalog.disconnect()}>
            Disconnect
          </button>
        </div>

        <!-- The devices this account has signed in from, behind a
             toggle for the ledger payloads' reason: it answers about
             the account rather than about a team, so it is not one of
             the tabs, and it is not what somebody opened this drawer
             to read. -->
        <button
          type="button"
          class="event-payload-toggle"
          aria-expanded={devicesOpen}
          onclick={toDevices}
        >
          {devicesOpen ? "▾" : "▸"} devices signed in
        </button>

        {#if devicesOpen}
          <p class="drawer-note">
            Each of these is a machine that can sign in as you without a
            password. Revoking one takes that back; sessions it already
            opened run out on their own.
          </p>
          {#if sharedCatalog.deviceTokens.loading}
            <p class="drawer-empty">loading…</p>
          {:else if sharedCatalog.deviceTokens.error}
            <p class="drawer-empty drawer-error">
              Could not read your devices: {sharedCatalog.deviceTokens.error}
            </p>
          {:else if sharedCatalog.deviceTokens.data.length === 0}
            <p class="drawer-empty">
              No device is remembering this account.
            </p>
          {:else}
            <ul class="drawer-list devices" role="list">
              {#each sharedCatalog.deviceTokens.data as token (token.id)}
                <li
                  class="device"
                  class:you={sharedCatalog.stored?.token_id === token.id}
                >
                  <span class="device-label">
                    {token.label}{#if sharedCatalog.stored?.token_id === token.id}
                      &nbsp;· this one{/if}
                  </span>
                  <span class="device-when">
                    minted {fmtDateTime(token.created_at_ms)} ·
                    {#if token.last_used_at_ms === null}
                      never used
                    {:else}
                      last used {fmtDateTime(token.last_used_at_ms)}
                    {/if}
                    · expires {fmtDateTime(token.expires_at_ms)}
                  </span>
                  <button
                    type="button"
                    onclick={() => sharedCatalog.revokeDevice(token.id)}
                    title="Stop this device signing in without a password"
                  >Revoke</button>
                </li>
              {/each}
            </ul>
          {/if}
        {/if}

        <!-- The teams this account is in, which is what the field
             below used to be the only way to name. Named now (#218):
             a team founded before the migration that added the column
             reads `name: null` and falls back to its id, the same
             shortage every team read as before. The role is shown
             beside each because it is the fact a reader chooses on;
             the creation time is what the rows are ordered by and is
             not drawn. -->
        <h4 class="rail-head">Teams</h4>
        {#if sharedCatalog.teams.loading}
          <p class="drawer-empty">reading your teams…</p>
        {:else if sharedCatalog.teams.error}
          <p class="drawer-empty drawer-error">
            Could not read your teams: {sharedCatalog.teams.error}
          </p>
        {:else if sharedCatalog.teams.data.length > 0}
          <ul class="drawer-list teams" role="list">
            {#each sharedCatalog.teams.data as team (team.team_id)}
              <li>
                <!-- `aria-current` beside the weight, because the
                     marking is the answer to "which team is this
                     window on" and weight alone says that to one kind
                     of reader. The e2e reads the class for the same
                     fact. -->
                <button
                  type="button"
                  class="drawer-row"
                  class:active={sharedCatalog.teamId === team.team_id}
                  aria-current={sharedCatalog.teamId === team.team_id
                    ? "true"
                    : undefined}
                  onclick={() => choose(team.team_id)}
                  title={team.team_id}
                >
                  <span class="row-title truncate">{team.name ?? team.team_id}</span>
                  <span class="row-standing">{team.role}</span>
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <!-- Empty is not "no way in". The read answers membership
               and an instance admin belongs to nothing by being one
               (#83 §1), so the field below is the whole surface for
               that reader. -->
          <p class="drawer-empty">
            You are not a member of any team on this server.
          </p>
        {/if}

        <!-- Founding a team sits at the foot of the list rather than on
             a tab, because every tab is an answer about the team named
             and this is the one act about no team in particular. Shown
             in every phase with a connection, not only in `no-team`:
             naming a team is still a one-way trip on this surface —
             picking a row or submitting the field both name one, and
             neither has an undo — so an offer that only appeared before
             the first would be an offer somebody could take exactly
             once per window.

             Asks for a name (#218) rather than the bare press it used
             to be — a team is named at founding, not left to read as
             the id underneath. -->
        <form class="drawer-form make-team" onsubmit={makeTeam}>
          <label>
            Start a team of your own
            <input
              type="text"
              bind:value={newTeamName}
              placeholder="team name"
              required
            />
          </label>
          <button type="submit">Found it</button>
        </form>

        <!-- Kept, and folded away. It names a team the list above does
             not hold, and the reader that has is the instance admin:
             they act inside teams without a membership row, so their
             list is empty while their reach is not. Without this the
             plane would have no entrance for them at all — and with it
             open beside the list, everybody else read two ways to do
             one thing (#217). -->
        <button
          type="button"
          class="disclose by-id-toggle"
          aria-expanded={byIdOpen}
          onclick={() => (byIdOpen = !byIdOpen)}
        >
          {byIdOpen ? "▾" : "▸"} open a team by id
        </button>
        {#if byIdOpen}
          <form class="drawer-form by-id" onsubmit={look}>
            <label>
              Team id
              <input
                type="text"
                bind:value={teamField}
                placeholder="team id"
                required
              />
            </label>
            <button type="submit">List its lines</button>
          </form>
        {/if}

        {#if sharedCatalog.phase === "ready"}
          <!-- The team's lines, in the rail because they are what the
               body reads about next. Named `lines` so a selector can
               name this one; the class has no CSS of its own beyond
               the marking, and the e2e specs read it. -->
          <h4 class="rail-head">Lines</h4>
          {#if sharedCatalog.lines.loading}
            <p class="drawer-empty">loading…</p>
          {:else if sharedCatalog.lines.error}
            <p class="drawer-empty drawer-error">
              Could not read the team's lines: {sharedCatalog.lines.error}
            </p>
          {:else if sharedCatalog.lines.data.length === 0}
            <p class="drawer-empty">This team hosts no lines.</p>
          {:else}
            <ul class="drawer-list lines" role="list">
              {#each sharedCatalog.lines.data as line (line.id)}
                <li>
                  <button
                    type="button"
                    class="drawer-row"
                    class:active={current !== null && current.id === line.id}
                    aria-current={current !== null && current.id === line.id
                      ? "true"
                      : undefined}
                    onclick={() => openLine(line.id)}
                    title="Open this line"
                  >
                    <span class="row-title">{line.name}</span>
                    <span class="row-standing">{line.standing}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}

          <!-- Publishing, at the lines list's own foot (#217). The
               re-enactment is chosen here or never: a line seeded with
               its current state cannot be given its history
               afterwards.

               Three conditions, unchanged from before this moved:
               `sharedCatalog.phase === "ready"` from the `{#if}` this
               sits inside, since offering to seed a line on a team
               nobody has named is offering to publish to nobody;
               `tab === "lines"`, since this is the one thing in the
               rail that reads `tab` at all, and only because seeding a
               line is a thing to do from where the lines are read, not
               from the roster or the ledger; and `current === null`,
               since a line the rail is already reading needs no second
               one seeded beside it. -->
          {#if tab === "lines" && current === null}
            <form class="drawer-form drawer-publish" onsubmit={publish}>
              <h4>Publish a line of mine</h4>
              <!-- Picked from this machine's lines rather than typed as
                   an id (#217): the forge knows every one of them. The
                   typed field stays for the case the list is empty or
                   unread, because a line that exists and is not listed
                   should still be publishable. -->
              <label>
                Local line
                {#if forgeCatalog.lines.data.length > 0}
                  <select bind:value={publishLineId} required>
                    <option value="" disabled>choose…</option>
                    {#each forgeCatalog.lines.data as line (line.id)}
                      <option value={line.id}>{line.name} · {line.standing}</option>
                    {/each}
                  </select>
                {:else}
                  <input
                    type="text"
                    bind:value={publishLineId}
                    placeholder="line id"
                    required
                  />
                {/if}
              </label>
              <label>
                Call it
                <input type="text" bind:value={publishName} required />
              </label>
              <label class="drawer-check">
                <input type="checkbox" bind:checked={reenact} />
                Re-enact the whole chain
              </label>
              <p class="drawer-cost">
                {#if reenact}
                  The team's line will be <strong>re-enacted</strong>: one
                  change point for each of mine, every act stamped to me
                  rather than to whoever made the work, and every content
                  the line ever named sent — including what has since been
                  replaced. Work logs and conversations do not go.
                {:else}
                  The team gets what the line holds now, as a single change
                  point. Choose re-enactment before publishing if you want
                  the chain; it cannot be added to the line afterwards.
                {/if}
              </p>
              <button type="submit">Publish</button>
            </form>
          {/if}
        {/if}
        </div>

        <!-- The body: what is read about the pick. -->
        <div class="body">
        {#if sharedCatalog.said}
          <p class="drawer-said">{sharedCatalog.said}</p>
        {/if}

        {#if sharedCatalog.phase === "no-team"}
          <p class="drawer-empty">
            Pick a team, or open one by id, to see the lines it hosts.
          </p>
        {:else}
          <!-- Row shared with `ForgePanel` as `TabStrip` (#217). -->
          <div class="drawer-tabs">
            <TabStrip
              ariaLabel="What to read about this team"
              tabs={[
                { key: "lines", label: "lines", onSelect: () => (tab = "lines") },
                { key: "roster", label: "members", onSelect: toRoster },
                { key: "ledger", label: "ledger", onSelect: toLedger },
              ]}
              active={tab}
            />
          </div>
        {/if}

        {#if sharedCatalog.phase === "no-team" || tab !== "lines"}
          <!-- The line's frame is what this chain renders, and this arm
               is what keeps it off the other tabs. The publish form is
               in the rail now, on its own condition, not here. -->
        {:else if current !== null}
          <!-- A line, beside the list it was picked from, argued in
               this component's header.

               The three tabs are the forge's three answers about one
               line, which decision 19 makes the same three here — a
               shared line is the same subject a local one is. What
               does not come across is the conversation `ForgePanel`
               mounts under whichever of its tabs is showing: the
               member's client carries no thread verbs, which the
               catalog's header records. -->
          <header class="line-head">
            <!-- Lets go of the line; the list beside it never left.
                 What letting go of a line ends is the catalog's,
                 written once there: the work open under it goes too,
                 because a piece of work belongs to the line it is
                 against. -->
            <button
              type="button"
              class="back"
              onclick={() => sharedCatalog.closeLine()}
            >
              ← the team's lines
            </button>
            <strong>{current.name}</strong>
            <span class="row-standing">{current.standing}</span>
          </header>

          <!-- How the chain reads is the visible difference between the
               two seedings: published as it stands is one change point
               however long the private line was, re-enacted is as many
               as it had. -->
          {#if sharedCatalog.changePoints !== null}
            <p class="drawer-chain">
              {sharedCatalog.changePoints} change point{sharedCatalog.changePoints ===
              1
                ? ""
                : "s"} since this line began
            </p>
          {/if}

          <!-- Row shared with `ForgePanel` as `TabStrip` (#217). -->
          <div class="drawer-tabs line-tabs">
            <TabStrip
              ariaLabel="What to read about this line"
              tabs={[
                { key: "contents", label: "on the line", onSelect: () => (lineTab = "contents") },
                { key: "work", label: "work", onSelect: () => (lineTab = "work") },
                { key: "history", label: "history", onSelect: () => (lineTab = "history") },
              ]}
              active={lineTab}
            />
          </div>

          {#if lineTab === "contents"}
            {#if sharedCatalog.states.loading}
              <p class="drawer-empty">loading…</p>
            {:else if sharedCatalog.states.error}
              <p class="drawer-empty drawer-error">
                {sharedCatalog.states.error}
              </p>
            {:else if sharedCatalog.onTheLine.length === 0}
              <p class="drawer-empty">Nothing is on this line.</p>
            {:else}
              <ul class="drawer-entries" role="list">
                {#each sharedCatalog.onTheLine as entry (entry.entry_id)}
                  <li class="entry">
                    <span class="entry-name">{entry.name ?? entry.entry_id}</span>
                    <button
                      type="button"
                      disabled={activeFilter.activePersona === null}
                      title={activeFilter.activePersona === null
                        ? "Pick a single persona to clone into"
                        : "Take a detached copy into this library"}
                      onclick={() =>
                        sharedCatalog.clone(
                          entry.entry_id,
                          activeFilter.activePersona!,
                        )}
                    >Clone</button>
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if lineTab === "work"}
            <SharedLineWork />
          {:else if sharedCatalog.history.loading}
            <p class="drawer-empty">loading…</p>
          {:else if sharedCatalog.history.error}
            <p class="drawer-empty drawer-error">
              Could not read this line's history: {sharedCatalog.history.error}
            </p>
          {:else if sharedCatalog.history.data === null}
            <!-- Opening a line reads all three of its answers, and a
                 read that failed is caught above, so nothing arrives
                 here today. It is the arm a `Resource` has whether or
                 not anything reaches it: `null` is what it holds
                 before a first answer, and a branch that assumed
                 otherwise would be reading a state as a promise. -->
            <p class="drawer-empty">No history read yet.</p>
          {:else}
            <!-- Newest first, which is the question a history answers:
                 what happened last. The work log next door is oldest
                 first, and `ForgeWork`'s header says why the two
                 differ. -->
            <ol class="drawer-list chain" role="list">
              {#each [...sharedCatalog.history.data.changes].reverse() as point (point.id)}
                <li>
                  <button
                    type="button"
                    class="point"
                    aria-expanded={openPoint === point.id}
                    onclick={() =>
                      (openPoint = openPoint === point.id ? null : point.id)}
                  >
                    <span>
                      {openPoint === point.id ? "▾" : "▸"}
                      {point.table.length}
                      {point.table.length === 1 ? "row" : "rows"}
                    </span>
                    <span class="row-standing">{point.actor_id}</span>
                    <span class="row-standing">{fmtDateTime(point.at_ms)}</span>
                  </button>
                  {#if openPoint === point.id}
                    <ul class="drawer-entries" role="list">
                      {#each point.table as row (row.entry_id)}
                        <li class="entry">
                          <span class="entry-name">{axes(row)}</span>
                          <span class="row-standing">
                            {row.name ?? row.entry_id}
                          </span>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </li>
              {/each}
            </ol>
            <!-- The genesis is not a change point and the model keeps
                 the two apart, so folding it into the chain would claim
                 something the record does not. -->
            <p class="drawer-empty">
              genesis · {fmtDateTime(sharedCatalog.history.data.genesis_at_ms)}
            </p>
          {/if}
        {:else if sharedCatalog.lines.data.length > 0}
          <!-- The list is in the rail; this is the body with nothing
               picked from it yet. -->
          <p class="drawer-empty">
            Pick a line on the left to read what is on it, the work
            against it, and its history.
          </p>
        {:else if !sharedCatalog.lines.answered || sharedCatalog.lines.loading || sharedCatalog.lines.error !== null}
          <!-- An empty `data` array is not the same claim as an
               answered, error-free read of zero lines (the rail's own
               chain above draws this same distinction with `loading`
               and `error` first): a read still in flight or one that
               failed is not "this team hosts no lines," and saying so
               here would repeat the mistake `Resource.answered` exists
               to catch. The rail already shows its own loading and
               error states; the body says nothing rather than guess. -->
        {:else}
          <!-- Answered, without error, and empty: this team really
               does host no lines yet. The rail's own empty state says
               so too; the body's job here is only to point at where
               the fix for that is, since the publish form moved out
               from under it and left nothing else to show. -->
          <p class="drawer-empty">
            Publish one of this machine's lines from the rail on the
            left to give this team its first.
          </p>
        {/if}

        {#if sharedCatalog.phase === "ready" && tab === "roster"}
          <!-- A membership row's login and display name are read live
               at roster time (#218), not stamped — the note says so
               where a reader would otherwise compare this tab with
               the ledger and wonder: a ledger event's name is a
               snapshot the act took, a different question from what
               a name is now. -->
          <p class="drawer-note">
            Who is in this team, by their current login and display
            name — the ledger keeps what a name read when an act
            happened, which this does not.
          </p>

          {#if iOwn}
            <!-- The team's own name, changeable here (#218) — an
                 owner's verb, per the authority table. Nowhere else on
                 this tab depends on `iOwn` reading before this does,
                 since the roster read that resolves it is what put the
                 form on screen at all. -->
            <form
              class="drawer-form rename-team"
              onsubmit={(event) => {
                event.preventDefault();
                const name = renameValue.trim();
                if (name === "" || name === currentTeamName) return;
                void sharedCatalog.renameTeam(name);
              }}
            >
              <label>
                This team's name
                <input type="text" bind:value={renameValue} required />
              </label>
              <button type="submit" disabled={renameValue.trim() === ""}
                >Rename</button
              >
            </form>

            <!-- Login first (#218): the form a person reaches for to
                 let somebody in by the name they know them under,
                 resolved on the server. The id form stays reachable
                 below for when the login is not known — a team has no
                 directory to search for somebody who is not in it
                 yet, so an id is sometimes all there is. -->
            <form class="drawer-form drawer-invite" onsubmit={inviteByLogin}>
              <label>
                Let somebody in
                <input
                  type="text"
                  bind:value={inviteLogin}
                  placeholder="login"
                  required
                />
              </label>
              <label>
                As
                <select bind:value={inviteRole}>
                  <option value="member">member</option>
                  <option value="owner">owner</option>
                </select>
              </label>
              <button type="submit">Invite</button>
            </form>

            <button
              type="button"
              class="disclose invite-by-id-toggle"
              aria-expanded={inviteByIdOpen}
              onclick={() => (inviteByIdOpen = !inviteByIdOpen)}
            >
              {inviteByIdOpen ? "▾" : "▸"} invite by id instead
            </button>
            {#if inviteByIdOpen}
              <form class="drawer-form invite-by-id" onsubmit={invite}>
                <label>
                  Account id
                  <input
                    type="text"
                    bind:value={inviteId}
                    placeholder="account id"
                    required
                  />
                </label>
                <label>
                  As
                  <select bind:value={inviteRole}>
                    <option value="member">member</option>
                    <option value="owner">owner</option>
                  </select>
                </label>
                <button type="submit">Invite</button>
              </form>
            {/if}
          {/if}

          {#if sharedCatalog.roster.loading}
            <p class="drawer-empty">loading…</p>
          {:else if sharedCatalog.roster.error}
            <p class="drawer-empty drawer-error">
              Could not read the team's roster: {sharedCatalog.roster.error}
            </p>
          {:else if sharedCatalog.roster.data === null}
            <!-- An unread state with a way out of it, the same as the
                 ledger's foot. Reachable when a read failed and was
                 dismissed, or if the tab is ever shown without the
                 load its opening performs. -->
            <p class="drawer-empty">Nothing read yet.</p>
            <button
              type="button"
              onclick={() =>
                sharedCatalog.roster.load({ teamId: sharedCatalog.teamId })}
            >Read the roster</button>
          {:else}
            <ul class="drawer-list roster" role="list">
              {#each sharedCatalog.roster.data.members as member (member.user_id)}
                <li class="member" class:you={member.user_id === sharedCatalog.session}>
                  <!-- Login and display name, read live (#218) — the
                       id is still there, on the title, for whoever
                       needs to match a row against one. -->
                  <span class="member-id" title={member.user_id}>
                    {member.login}{#if member.display_name !== member.login}
                      &nbsp;({member.display_name}){/if}
                  </span>
                  <span class="member-role">
                    {member.role}{#if member.user_id === sharedCatalog.session}
                      &nbsp;· you{/if}
                  </span>
                  {#if member.user_id === sharedCatalog.session}
                    <!-- The reader's own row, and what it offers is
                         leaving rather than removing. A member acting
                         on their own membership asks no authority over
                         anybody, so the verb is there whether or not
                         they own the team — an owner also gets the
                         step down beside it. The last owner is refused
                         either way, and that is the team's state to
                         refuse rather than this row's to guess ahead
                         of. -->
                    <span class="member-acts">
                      {#if iOwn}
                        <button
                          type="button"
                          onclick={() =>
                            sharedCatalog.revokeOwner(member.user_id)}
                          title="Step down to being a member of this team"
                        >make member</button>
                      {/if}
                      <button
                        type="button"
                        onclick={askLeave}
                        title="Take yourself out of this team"
                      >leave</button>
                    </span>
                  {:else if iOwn}
                    <span class="member-acts">
                      {#if member.role === "owner"}
                        <button
                          type="button"
                          onclick={() =>
                            sharedCatalog.revokeOwner(member.user_id)}
                          title="Put this owner back to being a member"
                        >make member</button>
                      {:else}
                        <button
                          type="button"
                          onclick={() =>
                            sharedCatalog.grantOwner(member.user_id)}
                          title="Make this member an owner"
                        >make owner</button>
                      {/if}
                      <button
                        type="button"
                        onclick={() => askRemove(member.user_id, member.login)}
                        title="Remove this member from the team"
                      >remove</button>
                    </span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}

          {#if mayDelete}
            <!-- Under the roster rather than on a tab of its own. Every
                 tab is an answer about the team named above, and this
                 is an act about that same team — the one somebody is
                 looking at when they are administering it rather than
                 working it. Founding sits outside the tabs because it
                 is about no team in particular; this is not that.

                 The one control on this tab an instance admin reaches
                 by standing alone: an admin holding no row in this
                 team is offered nothing above, because #83 §1 gives
                 them no implicit membership verbs. One who is also a
                 member gets what their role gets above, and this
                 besides — the server grants the delete to the standing
                 whether or not a row is held. -->
            <button type="button" class="delete-team" onclick={askDeleteTeam}>
              Delete this team
            </button>
          {/if}
        {/if}

        {#if sharedCatalog.phase === "ready" && tab === "ledger"}
          <p class="drawer-note">
            What this team did, and in what capacity. Oldest first, and
            the names are as they read when each act was recorded.
          </p>

          {#if sharedCatalog.ledgerError}
            <p class="drawer-empty drawer-error">
              Could not read the team's ledger: {sharedCatalog.ledgerError}
            </p>
          {/if}

          {#if sharedCatalog.ledger.length > 0}
            <ul class="drawer-list ledger" role="list">
              <!-- `kind` and `payload_json` are rendered as stored, and
                   that is a decision rather than an omission. The kinds
                   are namespaced and versioned by the server and
                   `forge.*` is still growing them, so a screen mapping
                   each to a sentence would be a second place every new
                   kind has to be learned — going stale where nobody is
                   looking, which is the trap #148 decision 14 names for
                   the projection. It costs a reader some fluency, and
                   means a kind this screen has never seen still arrives
                   intact. -->
              {#each sharedCatalog.ledger as event (event.event_id)}
                <li class="event">
                  <div class="event-head">
                    <span class="event-kind">{event.kind}</span>
                    <!-- The kind covers leaving and being removed
                         alike, and neither of the two fields that
                         separate them is drawn on this row — the actor
                         shows as a name, and the subjects not at all —
                         so a reader could not tell without being told.
                         The kind itself stays verbatim beside the
                         note: a screen rewriting one would be
                         answering for a stream it does not own. -->
                    {#if isDeparture(event)}
                      <span class="event-note">left of their own accord</span>
                    {/if}
                    <span class="event-when"
                      >{fmtDateTime(event.occurred_at_ms)}</span
                    >
                  </div>
                  <div class="event-who">
                    {event.actor_display_name}
                    <!-- The capacity, not just the name. An admin acting
                         inside a team without a membership row is stamped
                         as one and never disguised as a member (#83 §1). -->
                    <span class="event-kind-of-actor">{event.actor_kind}</span>
                  </div>
                  <button
                    type="button"
                    class="event-payload-toggle"
                    onclick={() =>
                      (openPayload =
                        openPayload === event.event_id ? null : event.event_id)}
                  >
                    {openPayload === event.event_id ? "hide" : "what it says"}
                  </button>
                  {#if openPayload === event.event_id}
                    <pre class="event-payload">{event.payload_json}</pre>
                  {/if}
                </li>
              {/each}
            </ul>
          {:else if sharedCatalog.ledgerRead && !sharedCatalog.ledgerLoading}
            <!-- Unreachable against a server behaving itself: founding a
                 team appends its own event, so a team that answered with
                 nothing answered wrongly. -->
            <p class="drawer-empty drawer-error">
              This team's ledger came back empty, which should not be
              possible — creating a team records itself.
            </p>
          {/if}

          <!-- The foot. A null cursor is not an end: the read says only
               that nothing lay past here when the page was taken, and a
               ledger has no final page. So neither branch below claims
               one. -->
          <div class="ledger-foot">
            {#if sharedCatalog.ledgerLoading}
              <p class="drawer-empty">reading…</p>
            {:else if !sharedCatalog.ledgerRead}
              <!-- Nothing has come back, so there is nothing to say
                   about what lies past it. A page that failed lands
                   here, under the error above. -->
              <button type="button" onclick={() => sharedCatalog.readLedgerPage()}>
                Read the ledger
              </button>
            {:else if sharedCatalog.ledgerCursor !== null}
              <button type="button" onclick={() => sharedCatalog.readLedgerPage()}>
                Read more
              </button>
            {:else}
              <button type="button" onclick={() => sharedCatalog.readLedgerPage()}>
                Ask again
              </button>
              <span class="drawer-empty">
                Nothing more had been recorded when this was read.
              </span>
            {/if}
          </div>
        {/if}
        </div>
        </section>
      {/if}
    </div>
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    z-index: 60;
    border: 0;
    padding: 0;
  }
  .drawer {
    position: absolute;
    top: 0;
    right: 0;
    height: 100%;
    /* The forge's width, for the forge's reason: two columns, and the
       lists stay in view while a line is read (#217). */
    width: min(52rem, 96vw);
    overflow-y: auto;
    background: var(--panel-bg, #1b1b1e);
    color: var(--panel-fg, #e8e8ea);
    box-shadow: -0.5rem 0 1.5rem rgba(0, 0, 0, 0.4);
    padding: 1rem 1.15rem 2rem;
    box-sizing: border-box;
  }
  .drawer-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .drawer-head h3 {
    margin: 0;
    font-size: 1rem;
  }
  .drawer-close {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.9rem;
  }
  .drawer-note,
  .drawer-cost {
    font-size: 0.78rem;
    opacity: 0.72;
    line-height: 1.45;
  }
  .drawer-sub {
    font-size: 0.74rem;
    opacity: 0.6;
    margin: 0.1rem 0 0.9rem;
  }
  .team-plane {
    display: flex;
    gap: var(--drawer-shell-gap);
    align-items: flex-start;
  }
  .rail {
    flex: 0 0 var(--drawer-rail-width);
    min-width: 0;
  }
  .body {
    flex: 1 1 auto;
    min-width: 0;
  }
  .rail-head {
    margin: 1rem 0 0.2rem;
    font-size: 0.78rem;
    font-weight: 500;
    opacity: 0.7;
  }
  .rail .drawer-form {
    margin: 0.4rem 0 0.6rem;
  }
  .disclose {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.74rem;
    opacity: 0.6;
    padding: 0.4rem 0 0.1rem;
    text-align: left;
  }
  .disclose:hover {
    opacity: 1;
  }
  .lines .drawer-row.active {
    font-weight: 600;
  }
  .body > .drawer-tabs {
    margin-top: 0;
  }
  .drawer-said {
    font-size: 0.8rem;
    border-left: 2px solid currentColor;
    padding-left: 0.5rem;
    opacity: 0.85;
  }
  .drawer-form {
    display: flex;
    flex-direction: column;
    gap: 0.45rem;
    margin: 0.9rem 0;
  }
  .drawer-form label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.78rem;
  }
  .drawer-check {
    flex-direction: row !important;
    align-items: center;
    gap: 0.4rem;
  }
  .drawer-publish {
    border-top: 1px solid rgba(255, 255, 255, 0.12);
    padding-top: 0.9rem;
  }
  .drawer-publish h4 {
    margin: 0;
    font-size: 0.85rem;
  }
  .drawer-session {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-size: 0.78rem;
    opacity: 0.85;
  }
  /* One row in the rail. The id is long and the rail is not, so the
     whole of it is on the title. */
  .drawer-session span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .drawer-empty {
    font-size: 0.8rem;
    opacity: 0.7;
  }
  .drawer-error {
    color: #ff9d9d;
  }
  .drawer-tabs {
    margin: 0.8rem 0 0.2rem;
  }
  .roster .member {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.5rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    padding: 0.4rem 0.1rem;
    font-size: 0.78rem;
  }
  .roster .member.you {
    font-weight: 600;
  }
  /* Takes the slack so the role and the acts stay at the right edge.
     Without it a row with three children spreads them evenly and the
     role drifts into the middle, which reads as a column that is not
     one. */
  .member-id {
    flex: 1;
    min-width: 0;
    font-family: ui-monospace, monospace;
    font-size: 0.72rem;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .member-role {
    opacity: 0.6;
    font-size: 0.72rem;
    white-space: nowrap;
  }
  .member-acts {
    display: flex;
    gap: 0.3rem;
    white-space: nowrap;
  }
  .member-acts button {
    padding: 0.1rem 0.35rem;
    font-size: 0.68rem;
    opacity: 0.75;
  }
  .member-acts button:hover {
    opacity: 1;
  }
  .delete-team {
    margin-top: 0.9rem;
    font-size: 0.72rem;
    opacity: 0.7;
  }
  .delete-team:hover {
    opacity: 1;
  }
  /* Two rows rather than the roster's one line: a device's three
     times are a sentence, and at this drawer's width putting them
     beside the label leaves neither readable. Placed explicitly
     because the reading order and the layout differ — the times come
     after the label in the markup, where somebody hearing this row
     wants them, and under it on screen. */
  .devices .device {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: baseline;
    gap: 0.15rem 0.5rem;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    padding: 0.45rem 0.1rem;
    font-size: 0.78rem;
  }
  .devices .device.you {
    font-weight: 600;
  }
  .device-label {
    grid-area: 1 / 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .devices .device button {
    grid-area: 1 / 2;
  }
  .device-when {
    grid-area: 2 / 1 / 3 / -1;
    opacity: 0.6;
    font-size: 0.72rem;
    font-weight: 400;
  }
  .ledger .event {
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    padding: 0.45rem 0.1rem;
    font-size: 0.78rem;
  }
  .event-head {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .event-note {
    opacity: 0.6;
    font-size: 0.68rem;
    font-style: italic;
    white-space: nowrap;
  }
  .event-kind {
    font-family: ui-monospace, monospace;
    font-size: 0.72rem;
  }
  .event-when,
  .event-kind-of-actor {
    opacity: 0.6;
    font-size: 0.72rem;
  }
  .event-who {
    display: flex;
    gap: 0.4rem;
    align-items: baseline;
    opacity: 0.85;
  }
  .event-payload-toggle {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.72rem;
    opacity: 0.6;
    padding: 0.1rem 0;
  }
  .event-payload {
    font-size: 0.7rem;
    margin: 0.2rem 0 0;
    padding: 0.35rem 0.45rem;
    background: rgba(255, 255, 255, 0.05);
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .ledger-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding-top: 0.7rem;
  }
  .drawer-list,
  .drawer-entries {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .drawer-row {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 0.5rem;
    background: none;
    border: 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    color: inherit;
    cursor: pointer;
    padding: 0.45rem 0.1rem;
    text-align: left;
    font-size: 0.82rem;
  }
  .teams .drawer-row.active {
    font-weight: 600;
  }
  /* One line, cut at the right edge: a team name — or, for one from
     before #218, its id — drawn in the rail would otherwise wrap or
     overflow. The whole thing is the row's title. */
  .truncate {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line-head {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    margin-top: 0.8rem;
  }
  .line-head strong {
    font-size: 0.9rem;
  }
  .back {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
    font-size: 0.78rem;
    opacity: 0.75;
    padding: 0;
  }
  .line-tabs {
    margin-top: 0.4rem;
  }
  .chain .point {
    display: flex;
    width: 100%;
    justify-content: space-between;
    gap: 0.5rem;
    background: none;
    border: 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    color: inherit;
    cursor: pointer;
    font-size: 0.78rem;
    padding: 0.4rem 0.1rem;
    text-align: left;
  }
  .row-standing {
    opacity: 0.6;
    font-size: 0.72rem;
  }
  .entry {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.25rem 0 0.25rem 0.8rem;
    font-size: 0.78rem;
  }
  .entry-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .drawer-chain {
    font-size: 0.72rem;
    opacity: 0.6;
    margin: 0.2rem 0 0.2rem 0.8rem;
  }
</style>
