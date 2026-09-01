// A team, from inside somebody's own library — the lines it hosts, who
// is in it, and what it recorded.
//
// This catalog exists because #148 decision 16 says a shared line is
// served through rather than mirrored: reads go to the server, there is
// no local copy, and therefore no staleness to reason about. The three
// sections after this one were written when the lines were all this
// plane showed.
//
// What follows them is the frame #171's surfaces attach to, written
// ahead of them for the reason `forge.svelte.ts` gives for its own: a
// frame decided from inside whichever surface is built first ends up
// shaped by that one. As each surface lands, what it decides for itself
// belongs in its own header — what stays here is what all of them stand
// on.
//
// # Why it is a catalog of its own rather than more fields on another
//
// Because the source is different, and the UI's job is to be honest
// about that. The decision puts shared lines "in their own panel rather
// than mixed into the local ones, which is what having two sources
// honestly looks like". A store that held both would be the mixing,
// one layer down from where it would be visible.
//
// # There is no cache, and `reset` is the whole story
//
// A `Resource` here is a *request in flight and its last answer*, not a
// copy of anything. When the connection drops, `reset` empties them —
// the panel then shows nothing, which is true, rather than the last
// thing the server said, which is a mirror with extra steps. That is
// also why nothing reloads on a timer: `openPanel` reads, selecting a
// line reads, and a write that changed the answer reads. Between those
// the panel is not showing a stale copy; it is showing the answer to
// the last question somebody asked.
//
// # Writes, and whether pressing one again is safe
//
// `clone` copies one entry onto this machine (decision 10). It is an
// import: the answer is an ordinary asset, and asking twice gets the
// same one back, so the button is safe to press again.
//
// `publish` seeds a team's line from a local one (decision 11) and is
// not safe to press again — each press opens another line on the team.
// The re-enactment option is chosen here at init and can never be
// chosen later, which is why the panel states its two costs before
// offering it rather than after.
//
// The writes that came after these two answer the same question where
// they are defined: the work verbs below, and `promote`, which the
// promotion section further down is about.
//
// # Three subjects under a team, and the frame that follows
//
// Three things sit under a team. #148's model draws two of them — the
// memberships that say who is in it, and the lines it hosts — and its
// decisions 17 and 18 keep the third, #83's ledger of what was done in
// what capacity, putting a cursor on the read rather than introducing
// the table. That is where this frame parts from `forge.svelte.ts`'s,
// which answers about a line because a line is all the local plane
// has. A frame here that answered only about lines would leave two of
// the three nowhere to be.
//
// They are three answers about one team, so they are tabs on one frame
// rather than three places to go: somebody moving between them is
// changing the question rather than the subject, which is the reading
// `ForgePanel` gives its own three.
//
// Selecting a line inside the first of them brings the forge's frame
// with it, because decision 19 mirrors the local surface path for path
// and the same DTOs come back, so a shared line is the same subject a
// local one is. Two levels of tab, because the model has two subjects
// here and not one.
//
// A subset of that frame, and the subset is part of the design rather
// than an omission in it. `ForgePanel` mounts `ForgeTalk` under
// whichever of its tabs is showing, and the member's client states of
// itself that the conversation verbs are not among what it carries.
// The server mirrors the thread routes; nothing on this side calls
// them. So contents, work and history come across and conversations do
// not, and whether they should is a question for whichever child wants
// one rather than something this frame has already answered.
//
// The frame, with a line open:
//
//   ┌─ team ── ▾ studio ───────────────── signed in as ytk ─────────┐
//   │ lines │ members │ ledger                                      │
//   │ ───────────────────────────────────────────────────────────── │
//   │ ← the team's lines   ROOT   open                              │
//   │ 1 change point since this line began                          │
//   │ on the line │ work │ history                                  │
//   │ ───────────────────────────────────────────────────────────── │
//   │ key visual                                          [ Clone ] │
//   └───────────────────────────────────────────────────────────────┘
//
// The list is not beside it: whether the two share the width or take
// turns is the panel's to decide, and its header decides it — this
// drawer is narrow, so a line takes the place of the list and the
// header carries the way back.
//
// Which of the three leads is a choice rather than a consequence.
// Lines lead because a team is joined in order to work with what it
// holds, and the roster and the ledger answer about the team rather
// than about the work. **That is an assumption about intended use and
// not a measurement**; if it is wrong, the order of the tabs is what
// moves.
//
// # Two kinds of empty, and what tells them apart
//
// Every read here is a request to a server, which gives "nothing to
// show" two meanings a screen must not merge: nobody has been asked
// yet, and there is nobody to ask. The forge has only the first, so
// its panel can let an empty list speak for itself and this one cannot
// — an empty list under a dropped connection would say the team hosts
// nothing, which is a claim about a team nobody is talking to.
//
// `phase` is where they are told apart, and it is the frame's own
// state rather than a per-resource one. Each resource already knows
// whether it is loading and whether it failed; what none of them can
// know is whether there is a server behind it at all, because that is
// a fact about the connection rather than about any read. A screen
// deriving it from `lines.data.length` would be reading the answer to
// a question nobody asked.
//
// # What the picker changed, and what it did not
//
// The team id was typed because nothing answered "the teams I am in".
// `teams` below is that read (#202), and naming a team is a choice
// from a list now — `teamId` is set by pressing a row rather than by
// filling a field, and it was already state every surface reads, so
// nothing else moved.
//
// **The field to type an id stayed.** The read answers membership and
// not reach, so an instance admin — who acts inside teams without a
// membership row — gets an empty list, and a surface with only a list
// would have no way in for them. The panel argues where it put the
// two.
//
// The phase between — connected, with no team chosen — is not
// something the picker introduced. A window that has just connected is
// already in it, because `teamId` starts empty, and it stays in it
// until somebody names a team either way. What the picker changed is
// what that phase shows: a list to choose from above the field.
//
// It is where a window begins rather than where every session does.
// `disconnect` leaves `teamId` alone, so connecting again in the same
// window returns to the team it was last looking at — the id is what
// somebody typed, and dropping it because a connection dropped would
// be making them type it twice. That is why the frame draws this as a
// state rather than a moment it passes through: it is the first thing
// a window shows and the one thing a reconnection skips.
//
// # Where #171's surfaces attach
//
// **The roster** — who is in the team — is the second tab, over the
// member's client's `roster`. It is a read and nothing more, and that
// is the routes' doing rather than a scope somebody chose: #171's body
// asks for four verbs beside it, and they answer to four different
// rules at four different depths. Only the read and team creation are
// wired end to end. **Joining has no verb at all**, so a tab offering
// one would be offering something with nothing behind it, and
// `RegistrationPolicy` — which #171 hangs all four on — is consulted
// by exactly one of them.
//
// Founding a team is the write that came with this tab and does not
// sit on it. Every tab is an answer about the team named above them,
// and founding is about none; the panel's header argues where the
// control goes.
//
// **The ledger** is the last of the three, over `events`. What it
// decides for itself is in the panel's header; what it leaves here is
// the walk, below.
//
// The rule that governed both: a surface arrives with whatever desktop
// command it needs, because what a surface asks of a command is known
// where the surface is written and guessed anywhere else. A command
// that fetches from the team server maps the wire's shapes to
// `asterism-contract::teams` on the
// way, so a screen holds one vocabulary rather than two — the rule the
// boundary test in `src-tauri/tests/boundary.rs` enforces.
//
// **Working a shared line** is the forge's `work` tab, in the inner
// frame above. Decision 10 is why there is no copy step in front of
// it: working on a shared line needs no clone, so the verb a person
// reaches for is the same one they reach for on a line of their own.
//
// **The promotion** is not on this frame at all — see below.
//
// # A promotion does not start here
//
// #152's client converts a local Asset into a TeamAsset, so the
// subject is the asset and the team is where it goes. A verb placed on
// this frame would ask somebody to name their asset from a screen that
// is not showing it. The asset detail pane is where it belongs, which
// is #171's own answer.
//
// What the model adds is that it cannot be started from there either
// without work already open: decision 5 gives content exactly one
// entry point, a verb scoped to an open pursuit, so that the team
// never holds an Asset that is not attached to work. Which pursuit,
// and what the detail pane does when there is none, is that surface's
// to decide and belongs in its header. What belongs here is that the
// open pursuit is a read over the member's client, and like the
// roster's and the ledger's it arrives with the child that needs it.
//
// It is also a write that is not safe to press twice, and the one
// where that is least visible: decision 7 mints a TeamAsset per
// promotion. What stops a second press from making a second one is a
// relation row this machine wrote, which the client reads before
// anything is sent — so a repeat costs nothing on the wire and says
// so, though it still reads and hashes the material, which is why it
// can report a digest at all. Two *members* promoting the same bytes
// still get one each, which no row on this machine can see. `publish`
// carries the same asymmetry and `clone` does not.
//
// # The ledger is paged rather than listed
//
// #149 gave the read a keyset cursor over `seq`, because #148 turns a
// table that grew by the occasional membership gesture into one that
// gains a row per push. A surface over it is therefore a page through
// a log and not a list, and a resource holding "the events" would be
// holding the first page while claiming the name of all of them.
//
// Which control pages it was taste, and the tab settled it: one
// control at the foot, bringing `LEDGER_PAGE` more. An infinite scroll
// would keep asking a server on somebody's behalf for a record they
// may only have glanced at, and a range needs a total this read has no
// way to give.
//
// What was never taste is that the cursor is part of what the read
// takes, so it cannot be added to a resource shaped without it — which
// is why the walk below is fields on this catalog rather than a
// `Resource`. A `Resource` holds one answer; this holds a sequence of
// them, and only the caller knows they are the same walk.
//
// # Where a credential lives is not settled
//
// #167 kept the connection to the window because a stored credential
// has no designed home yet, and #171 makes designing that home its
// own. It is deferred here rather than answered, because the answer
// interacts with a provider path this plane does not have: #163 adds
// one to the connect form, and a home chosen before it lands is a home
// chosen for one of the two kinds of credential.
//
// The alternatives, so the deferral names them: the OS keychain
// through Tauri's own plugin, which is where a password belongs; the
// profile directory, which is where this app's state lives and which
// would therefore put a password beside it; and the window, which is
// what #167 chose and what the deferral leaves in place.
//
// What the frame does meanwhile is meet a person with a connection
// form once per window rather than once per opening: the session lives
// in the backend for as long as the window does, so reopening the
// drawer while connected shows lines rather than the form again.
// `phase` is what says which of the two somebody is looking at.
//
// # The head pull is not a tab here
//
// #171 carries #130's fetch-for-me — the head a model panel pastes as
// an artifact today should be fetchable from the team holding it. That
// is a verb the model panel gains rather than a subject this frame
// grows: what somebody is looking at is the encoder they are training,
// and the team is where the bytes come from, which is the promotion's
// direction read backwards. It attaches to `SettingsModel`, and what
// it needs from this plane is the connection the first section of this
// frame is about.
//
// # Why this grows rather than a second catalog beside it
//
// `forge.svelte.ts` was written before any screen read it, so its
// frame could arrive with the first one. A panel reads this catalog,
// so a second catalog carrying the frame would be two stores answering
// about one team — the shape the first section of this file refuses
// one layer down. The panel is what grew into the frame, and the
// ledger is what grew it: the design said the first surface to land
// would build the tabs, and this is that surface. What each tab
// decides for itself is in the panel's header from here on; what stays
// above is what all of them stand on.
//
// Where it sits is not among the questions this design opens. #181
// moved `shared lines` beside the forge in the sidebar and named
// revisiting that as this umbrella's, and revisiting a placement is a
// different act from choosing one.
import { api } from "../api";
import { clashingNames, projectWork } from "../forge-projection";
import type { ForgeProjectedEntry } from "../forge-projection";
import { mutate } from "../mutate";
import { Resource } from "./_resource.svelte";
import type {
  AssetDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
  ForgeOpDto,
  ForgePursuitDto,
  MyTeamDto,
  MyTeamsDto,
  PromotedAssetDto,
  TeamCreatedDto,
  TeamLedgerEventDto,
  TeamLedgerPageDto,
  TeamRosterDto,
} from "../../bindings";

/// What the two reads need to name a line on a server.
type TeamArgs = { teamId: string };
type LineArgs = { teamId: string; lineId: string };

/// How many events one press of the ledger's foot control brings back.
///
/// Stated here rather than left to the server's default because the
/// number is what the control means to a person: press once, get this
/// many more. Small enough that a first page arrives while somebody is
/// still looking at the tab.
const LEDGER_PAGE = 50;

class SharedCatalog {
  /// Whether the panel is showing. The panel reads this itself; the
  /// App only mounts it.
  open = $state(false);
  /// The user id the server answered with, or `null` when this window
  /// is talking to no team.
  session = $state<string | null>(null);
  /// Which team is being looked at — picked from `teams` or typed,
  /// which the header and the panel argue between them. Kept across a
  /// disconnect on purpose; see `phase`.
  teamId = $state("");
  /// The line whose contents are showing, if one is open.
  selected = $state<string | null>(null);
  /// What the last write said, for the panel to report. Cleared when a
  /// new one starts.
  said = $state<string | null>(null);

  lines = new Resource<TeamArgs, ForgeLineDto[]>(
    async (args) =>
      api<ForgeLineDto[]>("list_shared_lines", { teamIdRaw: args.teamId }),
    [] as ForgeLineDto[],
    "sharedCatalog.lines",
  );

  states = new Resource<LineArgs, ForgeEntryStateDto[]>(
    async (args) =>
      api<ForgeEntryStateDto[]>("shared_line_states", {
        teamIdRaw: args.teamId,
        lineId: args.lineId,
      }),
    [] as ForgeEntryStateDto[],
    "sharedCatalog.states",
  );

  history = new Resource<LineArgs, ForgeLineHistoryDto | null>(
    async (args) =>
      api<ForgeLineHistoryDto>("shared_line_history", {
        teamIdRaw: args.teamId,
        lineId: args.lineId,
      }),
    null,
    "sharedCatalog.history",
  );

  /// The teams this window's account belongs to.
  ///
  /// The read the typed team id was waiting for. It names no team,
  /// because it is what a caller asks before they have one — so it
  /// belongs to the connection rather than to a team: read on
  /// connecting, on opening the panel, and after founding a team,
  /// dropped when the connection goes, and left alone when another
  /// team is named.
  ///
  /// **It answers membership, not reach**, which the route and the
  /// command both state. An admin who joined nothing gets an empty
  /// list while retaining every capacity they had — so a screen must
  /// not read an empty list as "no way in", and the panel keeps the
  /// field that names a team directly for exactly that reader.
  teams = new Resource<Record<string, never>, MyTeamDto[]>(
    async () => (await api<MyTeamsDto>("my_teams")).teams,
    [] as MyTeamDto[],
    "sharedCatalog.teams",
  );

  /// Who is in the team now named.
  ///
  /// A `Resource` rather than a walk, because a roster is one answer:
  /// the whole membership set comes back at once, and the read has no
  /// cursor because a team's members are not a stream.
  roster = new Resource<TeamArgs, TeamRosterDto | null>(
    async (args) =>
      api<TeamRosterDto>("team_roster", { teamIdRaw: args.teamId }),
    null,
    "sharedCatalog.roster",
  );

  /// The work against the open line, open and ended alike.
  ///
  /// Read from the server like everything else here. A pursuit on this
  /// plane belongs to a line somebody else may also be working, so
  /// what this held a moment ago is not what the line has been asked
  /// for now — the list is re-read after every write rather than
  /// patched from what a write answered.
  pursuits = new Resource<LineArgs, ForgePursuitDto[]>(
    async (args) =>
      api<ForgePursuitDto[]>("shared_line_pursuits", {
        teamIdRaw: args.teamId,
        lineId: args.lineId,
      }),
    [] as ForgePursuitDto[],
    "sharedCatalog.pursuits",
  );

  /// The piece of work being read, if one is open.
  ///
  /// A second selection under the line, and it narrows rather than
  /// replaces: work is against a line, so the line stays selected
  /// while one of its pursuits is open. Nothing derives it from the
  /// list — a screen reading emptiness as "no work open" would flash
  /// the whole list back over the one being opened, which is the trap
  /// `forge.svelte.ts` names at its own `working`.
  working = $state<string | null>(null);

  /// The open piece of work as the list has it, or `null`.
  ///
  /// Read out of `pursuits` rather than through a read of one pursuit.
  /// The list answers with whole pursuits — rounds and close included
  /// — so a second read of one of them would be a second copy of what
  /// is already here, and the two would disagree for as long as one of
  /// them was in flight. A read of one is what a surface that arrives
  /// at a pursuit without its line would need; this one always has the
  /// line.
  get work(): ForgePursuitDto | null {
    return this.pursuits.data.find((item) => item.id === this.working) ?? null;
  }

  /// Work nobody has ended. The only work a round can be written to.
  get openWork(): ForgePursuitDto[] {
    return this.pursuits.data.filter((item) => item.close === null);
  }

  /// Work that has ended, either way. Kept and shown apart, because
  /// what was asked for is readable after it stops being askable.
  get endedWork(): ForgePursuitDto[] {
    return this.pursuits.data.filter((item) => item.close !== null);
  }

  /// The line as the open work would leave it.
  ///
  /// The fold is `lib/forge-projection`, which argues why it is one
  /// copy. What this site adds is which two reads it is made of: the
  /// open pursuit's rounds, and the states of the line it is against.
  get projection(): ForgeProjectedEntry[] {
    return projectWork(this.work?.rounds ?? [], this.states.data);
  }

  /// Names that would be on the line twice if the open work landed.
  get wouldClash(): string[] {
    return clashingNames(this.projection);
  }

  /// Reads one of the line's pursuits.
  ///
  /// Nothing is fetched: the list carries whole pursuits, so opening
  /// one is choosing which of them the surface is about.
  selectPursuit(pursuitId: string): void {
    this.working = pursuitId;
    this.said = null;
  }

  /// Lets go of the work being read, keeping the line.
  clearWork(): void {
    this.working = null;
    this.said = null;
  }

  /// Lets go of the line, and of the work under it.
  ///
  /// Here rather than on the panel for the reason `lookAt` gives: a
  /// piece of work belongs to the line it is against, so the two are
  /// let go together, and a screen writing both fields is a second
  /// place that pairing has to be remembered.
  closeLine(): void {
    this.selected = null;
    this.clearWork();
  }

  /// What is on the line, and only what is on it. An entry the line
  /// took off is in the answer and is not something to show under
  /// "what this line holds" — nor something a clone will take.
  get onTheLine(): ForgeEntryStateDto[] {
    return this.states.data.filter((state) => state.alive);
  }

  /// How many change points the open line has, not counting its
  /// genesis. Worth showing beside a shared line because it is the
  /// visible difference between the two seedings: a line published as
  /// it stands has one however long its private history was, and a
  /// re-enacted one has as many as the private line did.
  get changePoints(): number | null {
    return this.history.data?.changes.length ?? null;
  }

  /// Which of the frame's three states this window is in: there is
  /// nobody to ask, there is somebody to ask and no team chosen, or a
  /// team is chosen and its reads can be made.
  ///
  /// The frame reads this rather than reading a resource, because the
  /// two kinds of empty a served-through view has are not a resource's
  /// to tell apart — a `Resource` knows whether it is loading and
  /// whether it failed, and neither answers whether there is a server
  /// behind it. See the header.
  ///
  /// `no-team` is where a window begins: the field starts empty, so a
  /// window that has just connected is in it until somebody names a
  /// team. It is not where every session begins — `disconnect` leaves
  /// the field alone, so connecting again in the same window goes
  /// straight back to `ready` on the team it was last looking at. A
  /// picker populates this state rather than introducing it.
  get phase(): "disconnected" | "no-team" | "ready" {
    if (this.session === null) return "disconnected";
    if (this.teamId === "") return "no-team";
    return "ready";
  }

  /// Opening the panel reads. A served-through view that showed the
  /// last answer it happened to have would be a mirror with extra
  /// steps, which is the thing decision 16 refuses.
  async openPanel(): Promise<void> {
    this.open = true;
    // Everything an on-demand tab holds goes with the panel, on the
    // same rule as the lines: a served-through view that showed what
    // it last had would be a mirror. The panel is mounted for the
    // window's lifetime rather than for the drawer's, so nothing is
    // dropped by the component going away — there is no such moment.
    this.forgetLedger();
    this.roster.reset();
    await this.refreshSession();
    if (this.session === null) return;
    // The teams a person may choose from, on the same rule as the
    // lines: a served-through view that showed what it last had would
    // be a mirror. Read whenever there is somebody to ask rather than
    // only where a team is missing — the list stays on screen after
    // one is named, marking which.
    await this.teams.load({});
    if (this.phase === "ready") await this.lines.load({ teamId: this.teamId });
  }

  closePanel(): void {
    this.open = false;
  }

  async refreshSession(): Promise<void> {
    this.session = await api<string | null>("team_server_session");
  }

  async connect(
    baseUrl: string,
    login: string,
    password: string,
  ): Promise<void> {
    this.said = null;
    this.session = await mutate<string>(
      "connect_team_server",
      { baseUrl, login, password },
      "connect to that team server",
    );
    // A connection is what makes this answerable, so it is read here
    // rather than left for the next time the panel opens — the phase
    // this lands in is the one the list is for.
    await this.teams.load({});
  }

  /// The ledger as far as it has been read, oldest first.
  ///
  /// Not a `Resource`, because a `Resource` holds one answer and this
  /// holds a walk: each page is appended to what came before, and the
  /// cursor below says where the next one resumes. Read afresh on every
  /// team, since a ledger belongs to one.
  ledger = $state<TeamLedgerEventDto[]>([]);

  /// Where the next page resumes, or `null`.
  ///
  /// **`null` is not "that was the end".** The read's own shape says
  /// so: a page that filled its limit always carries a cursor, and a
  /// short page carries none — which means nothing lay past there *when
  /// the page was taken*, rather than that nothing ever will. A ledger
  /// has no final page. Whatever a screen puts at the foot has to be
  /// phrased as asking again.
  ledgerCursor = $state<number | null>(null);

  /// Whether a page is in flight, and what the last one failed with.
  ledgerLoading = $state(false);
  ledgerError = $state<string | null>(null);

  /// Whether any page has come back for the team now named.
  ///
  /// Distinct from `ledger.length === 0`, which is unreachable for a
  /// team that answered: creating one appends its own event, so a team
  /// with an empty first page is a team that answered wrongly rather
  /// than one nothing has happened to.
  ledgerRead = $state(false);

  async disconnect(): Promise<void> {
    await api("disconnect_team_server");
    this.session = null;
    this.selected = null;
    // Not a cache being invalidated — a served-through view losing the
    // thing it was served through.
    this.lines.reset();
    this.states.reset();
    this.history.reset();
    this.roster.reset();
    this.pursuits.reset();
    this.teams.reset();
    this.working = null;
    this.forgetLedger();
  }

  /// Founds a team owned by the signed-in account.
  ///
  /// Answers with the id so a caller can name what it made. The one
  /// write here that is about no team in particular, which is why the
  /// control for it does not sit on a tab — every tab is an answer
  /// about the team named above them.
  async createTeam(): Promise<string> {
    this.said = null;
    const created = await mutate<TeamCreatedDto>(
      "create_team",
      {},
      "create a team",
    );
    this.said = `Created team ${created.team_id}.`;
    // The list is one shorter than the truth until this lands, and the
    // person who just founded a team is the likeliest to pick it.
    await this.teams.load({});
    return created.team_id;
  }

  /// Drops the walk. The ledger belongs to a team and to a connection,
  /// so both losing one and naming another end it.
  forgetLedger(): void {
    this.ledger = [];
    this.ledgerCursor = null;
    this.ledgerError = null;
    this.ledgerRead = false;
  }

  /// Reads the next page, or the first when the walk has not started.
  ///
  /// The page size is `LEDGER_PAGE`, argued where it is defined.
  async readLedgerPage(): Promise<void> {
    if (this.ledgerLoading) return;
    this.ledgerLoading = true;
    this.ledgerError = null;
    try {
      // Where to resume when there is no cursor is the last seq this
      // walk saw, which is what the read's own doc prescribes for a
      // caller following a live stream. Passing nothing would re-read
      // from the beginning and append a second copy of everything —
      // the cost of reading a null cursor as a starting point rather
      // than as "nothing had been recorded past here".
      const after = this.ledgerCursor ?? this.ledger.at(-1)?.seq ?? null;
      const page = await api<TeamLedgerPageDto>("team_ledger_page", {
        teamIdRaw: this.teamId,
        after,
        limit: LEDGER_PAGE,
      });
      this.ledger = [...this.ledger, ...page.events];
      this.ledgerCursor = page.next_after;
      this.ledgerRead = true;
    } catch (err) {
      this.ledgerError = err instanceof Error ? err.message : String(err);
    } finally {
      this.ledgerLoading = false;
    }
  }

  /// Names a team and reads the lines it hosts.
  ///
  /// The catalog owns what naming a team ends, rather than the panel:
  /// a ledger walk and a line selection both belong to the team that
  /// was named, and three clears written at three call sites is how one
  /// gets missed — which `forge.svelte.ts` learned the same way.
  async lookAt(teamId: string): Promise<void> {
    this.teamId = teamId;
    this.selected = null;
    this.working = null;
    this.forgetLedger();
    this.states.reset();
    this.history.reset();
    this.roster.reset();
    this.pursuits.reset();
    await this.lines.load({ teamId });
  }

  async show(lineId: string): Promise<void> {
    this.selected = lineId;
    // A piece of work belongs to the line it is against, so opening
    // another line ends whatever was open under the last one.
    this.working = null;
    await Promise.all([
      this.states.load({ teamId: this.teamId, lineId }),
      this.history.load({ teamId: this.teamId, lineId }),
      this.pursuits.load({ teamId: this.teamId, lineId }),
    ]);
  }

  /// Opens work against the line now showing.
  ///
  /// Decision 10 is why nothing is copied first: working on a shared
  /// line needs no clone.
  async openPursuit(title: string, note: string): Promise<ForgePursuitDto> {
    const lineId = this.requireLine();
    this.said = null;
    const pursuit = await mutate<ForgePursuitDto>(
      "open_shared_pursuit",
      {
        teamIdRaw: this.teamId,
        lineId,
        title: title || null,
        note: note || null,
      },
      "open work against that line",
    );
    await this.pursuits.load({ teamId: this.teamId, lineId });
    this.working = pursuit.id;
    return pursuit;
  }

  /// Writes a round into the open work.
  ///
  /// Nothing reaches the line here. A round is a request, and the only
  /// moment anything lands is a satisfied close — which is what lets
  /// two members work one line without contending, and why the
  /// contents are not re-read after this.
  async pushRound(ops: ForgeOpDto[], note: string): Promise<void> {
    const pursuitId = this.requireWork();
    this.said = null;
    await mutate<ForgePursuitDto>(
      "push_shared_round",
      { teamIdRaw: this.teamId, pursuitId, ops, note: note || null },
      "push that round",
    );
    await this.reloadWork();
  }

  /// Ends the open work, landing it or abandoning it.
  ///
  /// A satisfied close is the one moment the line moves, so the
  /// contents and the chain are re-read after it and not before.
  async closePursuit(outcome: string, note: string): Promise<void> {
    const lineId = this.requireLine();
    const pursuitId = this.requireWork();
    this.said = null;
    await mutate<ForgePursuitDto>(
      "close_shared_pursuit",
      { teamIdRaw: this.teamId, pursuitId, outcome, note: note || null },
      "close that work",
    );
    this.said =
      outcome === "satisfied"
        ? "Closed as satisfied — what the work asked for is on the line."
        : "Abandoned. The line did not move.";
    this.working = null;
    await Promise.all([
      this.states.load({ teamId: this.teamId, lineId }),
      this.history.load({ teamId: this.teamId, lineId }),
      this.pursuits.load({ teamId: this.teamId, lineId }),
    ]);
  }

  /// Re-reads the work list, which is where a pursuit's own state
  /// lives once it is written.
  async reloadWork(): Promise<void> {
    const lineId = this.requireLine();
    await this.pursuits.load({ teamId: this.teamId, lineId });
  }

  /// The open line, or a refusal naming what is missing.
  private requireLine(): string {
    if (this.selected === null) throw new Error("no line is open");
    return this.selected;
  }

  /// The open work, or a refusal naming what is missing.
  private requireWork(): string {
    if (this.working === null) throw new Error("no work is open");
    return this.working;
  }

  /// Work that is being read *and* has not ended, or a refusal.
  ///
  /// Stricter than `requireWork` because one caller has more to lose
  /// than a refusal. Reading an ended pursuit is ordinary — the drawer
  /// lists ended work and shows what was asked for — so `working` is
  /// set for one as readily as for an open one, and the verbs that
  /// write to it are kept apart on the screen instead.
  ///
  /// That is enough for a round, where nothing but the op list crosses
  /// before the model refuses it. It is not enough for a promotion:
  /// `enter_content`
  /// streams the body into the team's blob store and only then asks
  /// whether the work is still open, so a promotion onto ended work
  /// sends the whole file and is told afterwards. This refuses here,
  /// where nothing has been read off disk yet.
  /// Two refusals rather than one, because "not in the list" and
  /// "ended" are different facts. The first is what a failed re-read
  /// leaves — `Resource` puts its data back to the initial and the
  /// reason on `.error` — and not what one in flight leaves, which
  /// keeps the previous list until it resolves.
  ///
  /// **These three `require`s are backstops and their messages reach
  /// nobody.** They throw before `mutate` is called, and `mutate` is
  /// the only thing that puts a refusal in front of a person. That is
  /// the arrangement rather than a gap: a screen offers this verb only
  /// when it has a line, work, and work that has not ended, so a guard
  /// firing means a caller skipped what the screen checks. The message
  /// is for whoever is reading the stack, and the tests assert which
  /// of them fired.
  private requireOpenWork(): string {
    const pursuitId = this.requireWork();
    const work = this.work;
    if (work === null) {
      throw new Error("the work being read is not in this line's list");
    }
    if (work.close !== null) {
      throw new Error("that work has ended, so nothing can enter against it");
    }
    return pursuitId;
  }

  /// Hands one of this library's assets to the team, onto the open
  /// work.
  ///
  /// The verb the detail pane reaches for, and it is here rather than
  /// there because the three ids it needs are this catalog's: the
  /// team, the line, and the pursuit content may enter against (#148
  /// decision 5). A pane holding its own copies of those would be a
  /// second answer to a question this frame already answers, and the
  /// place they could disagree.
  ///
  /// What comes back is what only the promotion knows. The work is
  /// re-read here rather than taken from what the write answered, on
  /// the rule the rest of this catalog follows — a pursuit belongs to
  /// a line somebody else may also be working. Nothing re-reads the
  /// contents, because a round is a request: the entry this pushed is
  /// on the line when the work is closed satisfied and not before.
  async promote(assetId: string, named: string): Promise<PromotedAssetDto> {
    const lineId = this.requireLine();
    const pursuitId = this.requireOpenWork();
    this.said = null;
    const promoted = await mutate<PromotedAssetDto>(
      "promote_asset_to_team",
      { teamIdRaw: this.teamId, lineId, pursuitId, assetId, named },
      "promote that asset to the team",
    );
    this.said = promoted.already_promoted
      ? "This machine had already promoted that asset onto this line, so nothing was sent."
      : "Promoted. It is on the work — closing it as satisfied is what puts it on the line.";
    await this.reloadWork();
    return promoted;
  }

  /// Copies one entry onto this machine.
  async clone(entryId: string, personaId: string): Promise<AssetDto> {
    const lineId = this.selected;
    if (lineId === null) throw new Error("no line is open");
    this.said = null;
    const asset = await mutate<AssetDto>(
      "clone_shared_entry",
      { teamIdRaw: this.teamId, lineId, entryId, personaId },
      "clone that entry",
    );
    this.said = `Cloned into this library as ${asset.id}.`;
    return asset;
  }

  /// Seeds a team line from a local one. `reenact` is the init-time
  /// option and there is no later one.
  async publish(
    lineId: string,
    name: string,
    strategyId: string,
    reenact: boolean,
  ): Promise<ForgeLineDto> {
    this.said = null;
    const line = await mutate<ForgeLineDto>(
      "publish_line_to_team",
      { teamIdRaw: this.teamId, lineId, name, strategyId, reenact },
      "publish that line to the team",
    );
    this.said = reenact
      ? `Published “${line.name}” — the chain was re-enacted, so every act on it is stamped to you.`
      : `Published “${line.name}” as it stands.`;
    // The team has a line it did not have.
    await this.lines.load({ teamId: this.teamId });
    return line;
  }
}

export const sharedCatalog = new SharedCatalog();
