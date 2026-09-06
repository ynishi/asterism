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
// The frame, with a line open — the part of the drawer that reads
// about the pick; who is signed in, the teams and the lines are picked
// from beside it, and the panel draws where:
//
//   ┌───────────────────────────────────────────────────────────────┐
//   │ lines │ members │ ledger                                      │
//   │ ───────────────────────────────────────────────────────────── │
//   │ ← the team's lines   ROOT   open                              │
//   │ 1 change point since this line began                          │
//   │ on the line │ work │ history                                  │
//   │ ───────────────────────────────────────────────────────────── │
//   │ key visual                                          [ Clone ] │
//   └───────────────────────────────────────────────────────────────┘
//
// Whether the list sits beside it or the two take turns is the panel's
// to decide, and its header decides it. The header's "← the team's
// lines" lets go of the line, which is the catalog's `closeLine`
// either way.
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
// whether it is loading, whether it failed, and — since #219 — whether
// any load has answered since its last reset, which is the first kind
// of empty told apart at the read itself; what none of them can know
// is whether there is a server behind it at all, because that is a
// fact about the connection rather than about any read. A screen
// deriving it from `lines.data.length` would be reading the answer to
// a question nobody asked.
//
// # What the picker changed, and what it did not
//
// The team id was typed because nothing answered "the teams I am in".
// `teams` below is that read (#202), and a team can be chosen from a
// list now: `lookAt` gained a second gesture and is where the two are
// argued to be equal. `teamId` was already state every surface reads,
// so it did not move.
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
// member's client's `roster`. It reads and it writes: #210 brought the
// five roster writes up from the routes they had stopped at, so
// inviting, removing and the two role changes sit on the tab beside
// the read, and deleting the team sits under them. **Joining has no
// verb at all**, so a tab offering one would be offering something
// with nothing behind it, and `RegistrationPolicy` — which #171 hung
// it on — gates who may found a team rather than who may enter one.
// Leaving has no verb either: an owner may take themself out, which
// the ledger records as a removal.
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
// What the model adds is that it lands on open work: decision 5 gives
// content exactly one entry point, a verb scoped to an open pursuit,
// so that the team never holds an Asset that is not attached to work.
// The pane does not open one on a caller's behalf (#219) — a pursuit
// is the record that a person chose to start work, and `Promotion`'s
// own doc, on the client, says why opening one as a step of a
// promotion the team might still refuse is not that. So opening work
// for an entry is its own press on the pane, before the promotion.
// Which pursuit, and what the detail pane offers, is that surface's
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
// # Where a credential lives, and why the answer is not a password
//
// #204 settled it by fixing the invariant instead of picking a store:
// **the disk never holds a primary credential.** What it may hold is
// one thing, a device token the server minted — expiring, listable,
// revocable — and that shape is the same whichever verifier said yes,
// which is what made the question answerable before #163's provider
// path existed rather than after it.
//
// So both alternatives #167 weighed are used, for different halves.
// The OS keychain holds the token. The profile directory holds the
// server, the login and the token's revocation handle, none of which
// authenticates anybody. The password is in neither, in any encoding.
// Which half is where, and what each store does when it fails, is on
// `stored_connection` in the desktop binary; this catalog only calls
// the commands over it.
//
// This section used to record the opposite — that the home was
// deferred, and that the window was where a connection stayed — and
// the sentence is kept in the past tense because the reason for the
// deferral is what #204 answered rather than worked around.
//
// What the frame does with it is `resume`: a panel opening with no
// session tries what this machine remembers, silently, and falls back
// to the form when there is nothing or when the server refuses it.
// The session itself is still the window's — a device token opens a
// new one rather than being one — so reopening the drawer while
// connected shows lines rather than the form again. `phase` is what
// says which of the two somebody is looking at.
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
import { listen } from "@tauri-apps/api/event";
import { api } from "../api";
import { clashingNames, projectWork } from "../forge-projection";
import type { ForgeProjectedEntry } from "../forge-projection";
import { mutate } from "../mutate";
import { Resource } from "./_resource.svelte";
import type {
  AssetDto,
  ForgeCollisionDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
  ForgeOpDto,
  ForgePursuitDto,
  MyTeamDto,
  MyTeamsDto,
  PromotedAssetDto,
  RenamedTeamDto,
  StoredTeamConnectDto,
  StoredTeamConnectionDto,
  TeamCreatedDto,
  TeamDeviceTokenDto,
  TeamDeviceTokensDto,
  TeamIdentityDto,
  TeamLedgerEventDto,
  TeamLedgerPageDto,
  TeamProviderDto,
  TeamRosterDto,
} from "../../bindings";

/// What the backend says when a sign-in through the provider has a
/// page to go to — the event `connect_team_server_provider` emits
/// before it waits. Hand-typed on purpose: the generated bindings are
/// a projection of `asterism-contract`, and this shape lives in the
/// app crate beside the command that emits it. Its name and this
/// shape have to match `PROVIDER_SIGN_IN_EVENT` and
/// `ProviderSignInStarted` in `src-tauri/src/commands.rs`.
type ProviderSignInStarted = { attempt_id: string; start_url: string };

/// What the two reads need to name a line on a server.
type TeamArgs = { teamId: string };
type LineArgs = { teamId: string; lineId: string };
/// What `collisions` and `behind` need to name a piece of work.
type PursuitArgs = { teamId: string; pursuitId: string };

/// How many events one press of the ledger's foot control brings back.
///
/// Stated here rather than left to the server's default because the
/// number is what the control means to a person: press once, get this
/// many more. Small enough that a first page arrives while somebody is
/// still looking at the tab.
const LEDGER_PAGE = 50;

/// Whether a membership event records somebody going of their own
/// accord rather than being taken out (#210).
///
/// This plane's reading of a rule stated on the kind itself:
/// `teams.membership.removed/1` covers a member leaving and being
/// removed alike, and its doc is where the reading lives and where the
/// argument against a second kind is kept. This is that reading in
/// TypeScript, over the two fields the entry already carries.
export function isDeparture(event: TeamLedgerEventDto): boolean {
  if (event.kind !== "teams.membership.removed/1") return false;
  return event.subjects.some(
    (subject) =>
      subject.ref_type === "user" && subject.value === event.actor_user_id,
  );
}

class SharedCatalog {
  /// Whether the panel is showing. The panel reads this itself; the
  /// App only mounts it.
  open = $state(false);
  /// The user id the server answered with, or `null` when this window
  /// is talking to no team.
  session = $state<string | null>(null);
  /// The signed-in account's login and display name, or `null`
  /// (#218) — "Signed in as" reads this rather than `session`, which
  /// stays the bare id every own-row equality check compares against.
  /// Read alongside `session` at every place that sets it, the same
  /// "read back rather than assumed" rule `stored` already follows.
  identity = $state<TeamIdentityDto | null>(null);
  /// Which team is being looked at — picked from `teams` or typed,
  /// which the header and the panel argue between them. Kept across a
  /// disconnect on purpose; see `phase`.
  teamId = $state("");
  /// The line whose contents are showing, if one is open.
  selected = $state<string | null>(null);
  /// What the last write said, for the panel to report. Cleared when a
  /// new one starts.
  said = $state<string | null>(null);

  /// What this machine remembers about a server, or `null` (#204).
  ///
  /// Not a credential: the device token is in the OS keychain and
  /// never crosses to a window. What is here is the server, the login
  /// and the stored token's revocation handle, which the connect form
  /// pre-fills from and the token list marks a row with.
  stored = $state<StoredTeamConnectionDto | null>(null);

  /// Whether the stored connection was tried and refused.
  ///
  /// A fact worth keeping apart from "nothing was stored", because the
  /// two look identical on screen — the password form — and only one
  /// of them is worth a sentence. Cleared by a connection, since what
  /// it describes is over.
  storedRejected = $state(false);

  /// Why the stored token was refused, when it was: `expired`, `idle`
  /// or `revoked` as the server said it (#163), `revoked_by_instance`
  /// or `locked` when an admin did it (#213), or null from a server
  /// too old to say. What the drawer tells the person, and nothing
  /// the store acts on: what the app did with the token is the same
  /// for every reason, on the terms `connect_team_server_stored`
  /// gives. Cleared wherever `storedRejected` is: by every connection
  /// that lands, and by a disconnect.
  storedRejectedReason = $state<string | null>(null);

  /// The device tokens this account holds, on whatever machines.
  ///
  /// Read from the server on demand like everything else here: what
  /// another machine minted or revoked since this window opened is not
  /// something a copy could know.
  deviceTokens = new Resource<Record<string, never>, TeamDeviceTokenDto[]>(
    async () =>
      (await api<TeamDeviceTokensDto>("list_team_device_tokens")).tokens,
    [] as TeamDeviceTokenDto[],
    "sharedCatalog.deviceTokens",
  );

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
  /// **It answers membership, not reach**, which the route decides and
  /// `MyTeamsDto` restates for screens. An admin who joined nothing gets an empty
  /// list while retaining every capacity they had — so a screen must
  /// not read an empty list as "no way in", and the panel keeps the
  /// field that names a team directly for exactly that reader.
  teams = new Resource<Record<string, never>, MyTeamDto[]>(
    async () => (await api<MyTeamsDto>("my_teams")).teams,
    [] as MyTeamDto[],
    "sharedCatalog.teams",
  );

  /// Who is in the team now named, and what the reader may do there.
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

  /// The role the reader holds in the team now named, as the roster
  /// read said it.
  ///
  /// Said rather than derived from the rows, and the difference is an
  /// instance admin: they reach a team by standing outside it rather
  /// than by joining it (#83 §1), so no membership set describes them,
  /// and a getter searching the rows would read their absence as
  /// "nothing you may do" while what they may do is delete the team.
  /// `null` here means the reader holds no membership row, or the
  /// roster has not been read yet — for what an admin may still do,
  /// ask `iAmAdmin`.
  ///
  /// The server decides every one of these verbs regardless of what
  /// this says. Hiding a control is about not offering somebody a
  /// refusal, not about enforcement.
  get myRole(): string | null {
    return this.roster.data?.viewer.role ?? null;
  }

  /// Whether the reader is an instance admin, as the roster read said
  /// it. Independent of `myRole`: an admin may also be a member of the
  /// team they are administering.
  get iAmAdmin(): boolean {
    return this.roster.data?.viewer.admin ?? false;
  }

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

  /// What the open work asks for that the line has already moved
  /// (#211, mirroring `forgeCatalog.collisions`).
  ///
  /// A separate `Resource` from the work rather than a field folded
  /// into it, on `forge.svelte.ts`'s own reasoning: the answer is
  /// about the pair and moves when either side does.
  collisions = new Resource<PursuitArgs, ForgeCollisionDto[]>(
    async (args) =>
      api<ForgeCollisionDto[]>("shared_pursuit_collisions", {
        teamIdRaw: args.teamId,
        pursuitId: args.pursuitId,
      }),
    [] as ForgeCollisionDto[],
    "sharedCatalog.collisions",
  );

  /// The landings the open work has not seen, oldest first (#211,
  /// mirroring `forgeCatalog.behind`).
  behind = new Resource<PursuitArgs, string[]>(
    async (args) =>
      api<string[]>("shared_pursuit_behind", {
        teamIdRaw: args.teamId,
        pursuitId: args.pursuitId,
      }),
    [] as string[],
    "sharedCatalog.behind",
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

  /// Shows one of the line's pursuits, and what stands between it and
  /// the line.
  ///
  /// The work itself is not fetched: the list already carries it, so
  /// opening one is choosing which of them the surface is about. The
  /// two answers about it are (#211): what it collides with, and how
  /// far behind the line it is.
  async selectPursuit(pursuitId: string): Promise<void> {
    this.working = pursuitId;
    this.said = null;
    await Promise.all([
      this.collisions.load({ teamId: this.teamId, pursuitId }),
      this.behind.load({ teamId: this.teamId, pursuitId }),
    ]);
  }

  /// Lets go of the work being read, keeping the line.
  clearWork(): void {
    this.working = null;
    this.said = null;
    this.collisions.reset();
    this.behind.reset();
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
    // And what was read for it, on the rule `show` states: the line
    // is the subject of these three, and a subject let go of takes
    // its reads with it rather than leaving them answered for a line
    // nothing is on.
    this.states.reset();
    this.history.reset();
    this.pursuits.reset();
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
  /// to tell apart — a `Resource` knows whether it is loading, whether
  /// it failed and whether it has answered, and none of those answers
  /// whether there is a server behind it. See the header.
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
    this.deviceTokens.reset();
    await this.refreshSession();
    // Nothing stored is read for, and nothing silent is attempted
    // against, a window that is already connected — the session it has
    // is the one it keeps, and a second login would replace it with an
    // identical one for no reason.
    if (this.session === null) await this.resume();
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

  /// What the asset pane needs before it can offer a target (#219): a
  /// session — the one this window has, or the one this machine
  /// remembers, tried silently as opening the drawer would, short of
  /// the one difference below — and the teams, the lines of the team
  /// that is on, and the work against the line that is on.
  ///
  /// The lists are read every time, on the rule the drawer opens under
  /// (decision 16): a served-through view that showed what it last had
  /// would be a mirror, and an empty list is an answer rather than an
  /// absence of one. What is not repeated is the silent sign-in: a
  /// window that has a session keeps it, and one whose stored sign-in
  /// was refused is not made to hear the refusal again per asset —
  /// `storedRejected` is the field that already holds that.
  async readyForPromotion(): Promise<void> {
    if (this.session === null) await this.refreshSession();
    if (this.session === null && !this.storedRejected) await this.resume();
    if (this.session === null) return;
    await this.teams.load({});
    if (this.phase !== "ready") return;
    const teamId = this.teamId;
    const lineId = this.selected;
    await Promise.all([
      this.lines.load({ teamId }),
      lineId === null
        ? Promise.resolve(true)
        : this.pursuits.load({ teamId, lineId }),
    ]);
  }

  async refreshSession(): Promise<void> {
    this.session = await api<string | null>("team_server_session");
    await this.refreshIdentity();
  }

  /// Reads the signed-in account's login and display name back from
  /// the connection (#218), the same "read back" reason `readStored`
  /// gives: this window's own state does not know what a mint left
  /// behind, only the command does. Called at every place that sets
  /// `session` outside `refreshSession`, and clears `identity` on the
  /// same `null` `session` does.
  async refreshIdentity(): Promise<void> {
    this.identity =
      (await api<TeamIdentityDto | null>("team_server_identity")) ?? null;
  }

  /// Re-reads what this machine remembers.
  ///
  /// Read back rather than inferred at each site that changes it — a
  /// connection that minted, a revoke that may have been this
  /// machine's own row, the silent attempt — because only the command
  /// knows which of them left anything behind.
  ///
  /// `?? null` because absent and null are the same fact here and
  /// only one of them is a value the rest of this file tests for.
  private async readStored(): Promise<void> {
    this.stored =
      (await api<StoredTeamConnectionDto | null>("stored_team_connection")) ??
      null;
  }

  /// Reads what this machine remembers and, if there is anything,
  /// tries it — without asking anybody anything (#204).
  ///
  /// `api` rather than `mutate`: nobody asked for this, so a refusal
  /// has nothing to report to. The three ends it can reach are all
  /// ordinary, and the command's `outcome` is what tells them apart —
  /// only a server that is unreachable or shouting throws, and that
  /// one is swallowed here for the same reason. A window whose silent
  /// attempt failed is a window showing the connect form, which is
  /// where it would have been anyway.
  ///
  /// Not while a sign-in through the provider waits (#163), for the
  /// reason `providerBusy` gives; opening the panel again with a wait
  /// running is the way here.
  async resume(): Promise<void> {
    if (this.providerBusy) return;
    await this.readStored();
    if (this.stored === null) return;
    let attempt: StoredTeamConnectDto;
    try {
      attempt = await api<StoredTeamConnectDto>("connect_team_server_stored");
    } catch {
      return;
    }
    if (attempt?.outcome === "connected") {
      this.session = attempt.user;
      await this.refreshIdentity();
      this.storedRejected = false;
      this.storedRejectedReason = null;
      return;
    }
    // `rejected` forgot both halves before answering, and `stored` is
    // kept anyway: the login it carries is what the person is about to
    // type a password beside. The reason travels with it (#163, #213)
    // — one of the values `storedRejectedReason` names, or null — for
    // the sentence the drawer shows, and for nothing the store does.
    this.storedRejected = attempt?.outcome === "rejected";
    this.storedRejectedReason = this.storedRejected
      ? (attempt.reason ?? null)
      : null;
    if (this.storedRejected) return;
    // `nothing` is the one that has to be read back rather than
    // inferred. It covers a file that was dropped — its entry was
    // gone — and a file that was kept because the keychain would not
    // answer, and the command carries no field saying which, on the
    // reasoning `StoredTeamConnectOutcome` gives. Assuming the first
    // would make one dismissed keychain prompt empty the form of a
    // connection this machine still remembers.
    await this.readStored();
  }

  /// The identity provider the server last asked about offers, or
  /// null — and which server the answer is for, so a form does not
  /// show one server's button beside another server's URL.
  ///
  /// Read rather than guessed: whether a server signs people in
  /// through a provider is the server's fact (#163), and the form
  /// asks it as the URL settles.
  provider = $state<TeamProviderDto | null>(null);
  providerFor = $state<string | null>(null);

  /// Asks a server what it offers besides a password.
  ///
  /// A server that does not answer is a server with no button, not a
  /// refusal to report: the form is still usable with a password, and
  /// the same URL typed into the password path will say what is wrong
  /// with it.
  async probeProvider(baseUrl: string): Promise<void> {
    const url = baseUrl.trim();
    if (url === "") {
      this.provider = null;
      this.providerFor = null;
      return;
    }
    let provider: TeamProviderDto | null = null;
    try {
      provider =
        (await api<TeamProviderDto | null>("team_auth_provider", {
          baseUrl: url,
        })) ?? null;
    } catch {
      provider = null;
    }
    this.provider = provider;
    this.providerFor = url;
  }

  /// Whether a sign-in through the provider is under way, from the
  /// press to the command's answer. While it is, the wait owns the
  /// connection: `connect` and `resume` do nothing, so no session is
  /// opened under it for its answer to write over, and the form is
  /// off. True before the attempt has an id — the round trip that
  /// starts it is the window in which a second press would start a
  /// second wait, which the backend refuses and this keeps from being
  /// asked; `providerAttempt` is what the drawer shows once the id is
  /// known. Outlives the drawer, which is the point: closing it is not
  /// ending the wait.
  providerBusy = $state(false);

  /// The sign-in through the provider that is waiting for the browser
  /// — which attempt, and where the browser was sent — or null when
  /// none is. Set by the backend's event, which is also what makes the
  /// attempt cancellable: a cancel names the id. Why the drawer shows
  /// the page is said where it is shown.
  providerAttempt = $state<{ id: string; startUrl: string } | null>(null);

  /// Signs in through the server's identity provider, in the system
  /// browser, and optionally asks to be remembered (#163).
  ///
  /// No login and no password cross here: the browser is where the
  /// person proves who they are, and the session that comes back says
  /// which account it was. The rest is `connect` — the same
  /// connection, the same mint when `remember` is ticked.
  ///
  /// The command tells the window which attempt and where it sent the
  /// browser, as an event, before it waits; the listener is registered
  /// before the command is called so the event cannot be missed, and
  /// removed however the command ends. One at a time: a call while
  /// one is under way does nothing, on the ground `providerBusy`
  /// gives. A `null` answer is a cancel from this drawer — no session
  /// and nothing to report. Answers whether a session was opened, so
  /// the drawer knows whether to untick the box.
  async connectWithProvider(
    baseUrl: string,
    remember: boolean,
  ): Promise<boolean> {
    if (this.providerBusy) return false;
    this.providerBusy = true;
    this.said = null;
    this.providerAttempt = null;
    let session: string | null = null;
    try {
      const unlisten = await listen<ProviderSignInStarted>(
        "team-provider-sign-in",
        (event) => {
          this.providerAttempt = {
            id: event.payload.attempt_id,
            startUrl: event.payload.start_url,
          };
        },
      );
      try {
        session = await mutate<string | null>(
          "connect_team_server_provider",
          { baseUrl, remember },
          "sign in through the team server's provider",
        );
      } finally {
        unlisten();
      }
    } finally {
      this.providerAttempt = null;
      this.providerBusy = false;
    }
    if (session === null) return false;
    this.session = session;
    await this.refreshIdentity();
    this.storedRejected = false;
    this.storedRejectedReason = null;
    await this.readStored();
    await this.teams.load({});
    return true;
  }

  /// Ends the sign-in through the provider that is waiting, by the
  /// attempt's id (#163). The command answers `null` to
  /// `connectWithProvider`, which is where the drawer's state goes
  /// back — up to the collect; a press that lands after the session
  /// has been collected is too late, and the person is signed in,
  /// which the command's doc is where that boundary is stated.
  /// Nothing to cancel is nothing to do.
  async cancelProviderSignIn(): Promise<void> {
    const attempt = this.providerAttempt;
    if (attempt === null) return;
    await mutate<null>(
      "cancel_provider_sign_in",
      { attemptId: attempt.id },
      "cancel the sign-in through the team server's provider",
    );
  }

  /// Logs in with a password, and optionally asks to be remembered.
  ///
  /// `remember` is the mint: the session this opens is used once more
  /// to ask for a device token, which lands in this machine's
  /// keychain. What it stores and why the password is not part of it
  /// is on the command.
  ///
  /// Refused while a sign-in through the provider waits, for the
  /// reason `providerBusy` gives; the form is off then, and this is
  /// the guard behind the form.
  async connect(
    baseUrl: string,
    login: string,
    password: string,
    remember: boolean,
  ): Promise<void> {
    if (this.providerBusy) return;
    this.said = null;
    this.session = await mutate<string>(
      "connect_team_server",
      { baseUrl, login, password, remember },
      "connect to that team server",
    );
    await this.refreshIdentity();
    this.storedRejected = false;
    this.storedRejectedReason = null;
    // What the mint wrote, read back rather than assumed: a connection
    // made without ticking the box leaves whatever was stored before
    // exactly as it was, and this is the one read that says which.
    await this.readStored();
    // A connection is what makes this answerable, so it is read here
    // rather than left for the next time the panel opens — the phase
    // this lands in is the one the list is for.
    await this.teams.load({});
  }

  /// Revokes one device token, which may be this machine's own.
  ///
  /// The listing is re-read rather than patched, on the rule the rest
  /// of this catalog follows: what the server holds is the answer, and
  /// a row removed here would be this side guessing at it. `stored` is
  /// re-read for the same reason — revoking this machine's own row
  /// forgets it on this side too, and the form has to know.
  async revokeDevice(tokenId: string): Promise<void> {
    this.said = null;
    await mutate<void>(
      "revoke_team_device_token",
      { tokenId },
      "revoke that device",
    );
    await this.readStored();
    await this.deviceTokens.load({});
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

  /// Signs out, which also gives up being remembered.
  ///
  /// The command revokes this machine's device token and drops both
  /// halves of what was stored — when what was stored is the
  /// connection being ended, which its own doc is where that rule is
  /// argued. `stored` goes with the session here either way: it is
  /// this store's copy of an answer that is now at best stale, and the
  /// next `readStored` is what it comes back from.
  ///
  /// That is a statement about the store and not about the screen. The
  /// panel's server and login fields are its own `$state`, seeded from
  /// `stored` by an effect that only writes when there is something to
  /// write, so clearing this leaves what was typed where it was — and
  /// a person signing out to sign in elsewhere is not helped by a form
  /// that empties itself. Closing the window does none of this, which
  /// the command's own doc argues.
  async disconnect(): Promise<void> {
    await api("disconnect_team_server");
    this.session = null;
    this.identity = null;
    this.stored = null;
    this.storedRejected = false;
    this.storedRejectedReason = null;
    this.deviceTokens.reset();
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

  /// Founds a team owned by the signed-in account, named (#218) —
  /// asked for at founding rather than left to read as an id.
  ///
  /// Answers with the id so a caller can name what it made. The one
  /// write here that is about no team in particular, which is why the
  /// control for it does not sit on a tab — every tab is an answer
  /// about the team named above them.
  async createTeam(name: string): Promise<string> {
    this.said = null;
    const created = await mutate<TeamCreatedDto>(
      "create_team",
      { name },
      "create a team",
    );
    this.said = `Created ${created.name}.`;
    // The list is one shorter than the truth until this lands, and the
    // person who just founded a team is the likeliest to pick it.
    await this.teams.load({});
    return created.team_id;
  }

  /// Renames the team on — an owner's verb (#218).
  async renameTeam(name: string): Promise<void> {
    this.said = null;
    const renamed = await mutate<RenamedTeamDto>(
      "rename_team",
      { teamIdRaw: this.teamId, name },
      "rename this team",
    );
    await this.teams.load({});
    this.said = `Renamed to ${renamed.name}.`;
  }

  /// Lets an account into the team, in the role named.
  ///
  /// Re-reads the roster afterwards rather than adding a row from what
  /// the write answered, for the reason `pursuits` gives: this is
  /// somebody else's server, and what it holds now is its answer to
  /// give rather than one worked out here.
  async inviteMember(userId: string, role: string): Promise<void> {
    this.said = null;
    await mutate<void>(
      "invite_team_member",
      { teamIdRaw: this.teamId, userId, role },
      "invite that account",
    );
    await this.roster.load({ teamId: this.teamId });
    // The id form is reached for when the login is not known
    // (`inviteMemberByLogin`'s own doc, below), but the roster read
    // just above answers with it anyway — the account just invited
    // is a row on it now.
    const memberLogin =
      this.roster.data?.members.find((m) => m.user_id === userId)?.login ?? userId;
    this.said = `Invited ${memberLogin} as ${role}.`;
  }

  /// Lets an account into the team by login rather than by id (#218)
  /// — the form "Let somebody in" reaches for first, the id form
  /// staying reachable through [`inviteMember`] for when the login is
  /// not known.
  async inviteMemberByLogin(login: string, role: string): Promise<void> {
    this.said = null;
    await mutate<void>(
      "invite_team_member_by_login",
      { teamIdRaw: this.teamId, login, role },
      "invite that account",
    );
    await this.roster.load({ teamId: this.teamId });
    this.said = `Invited ${login} as ${role}.`;
  }

  /// Takes a member out of the team.
  ///
  /// The last owner cannot go, and the server says so — what arrives
  /// here is a refusal `mutate` puts on screen, worded by the server
  /// rather than guessed at before asking.
  ///
  /// The reader's own row offers `leaveTeam` rather than this, since
  /// #210 gave departing a verb of its own. The server still permits
  /// an owner to remove themself, so the case is handled here rather
  /// than assumed away: it ends this window's membership, and takes
  /// `stopReading`'s path rather than the re-read.
  ///
  /// Not because the re-read would always be refused — an instance
  /// admin who was also a member still passes the gate afterwards, on
  /// the standing that is not a row — but because the panel would
  /// otherwise keep drawing a team this reader is no longer in, which
  /// is true of both. For the reader who holds nothing else the gate
  /// does refuse it, and `Resource.load` turns that refusal into an
  /// error message under a panel still pointed at the team, still
  /// drawing its lines, with the picker still offering it.
  async removeMember(userId: string): Promise<void> {
    const team = this.teamId;
    const teamName = (this.teams.data ?? []).find((t) => t.team_id === team)?.name ?? team;
    const memberLogin =
      this.roster.data?.members.find((m) => m.user_id === userId)?.login ?? userId;
    this.said = null;
    await mutate<void>(
      "remove_team_member",
      { teamIdRaw: team, userId },
      "remove that member",
    );
    if (userId === this.session) {
      this.stopReading();
      await this.teams.load({});
      this.said = `You are no longer in ${teamName}.`;
      return;
    }
    await this.roster.load({ teamId: team });
    this.said = `Removed ${memberLogin}.`;
  }

  /// Makes a member an owner.
  async grantOwner(userId: string): Promise<void> {
    const memberLogin =
      this.roster.data?.members.find((m) => m.user_id === userId)?.login ?? userId;
    this.said = null;
    await mutate<void>(
      "grant_team_owner",
      { teamIdRaw: this.teamId, userId },
      "make that member an owner",
    );
    await this.roster.load({ teamId: this.teamId });
    this.said = `${memberLogin} is an owner.`;
  }

  /// Puts an owner back to being a member, which the last owner cannot
  /// do even to themself.
  async revokeOwner(userId: string): Promise<void> {
    const memberLogin =
      this.roster.data?.members.find((m) => m.user_id === userId)?.login ?? userId;
    this.said = null;
    await mutate<void>(
      "revoke_team_owner",
      { teamIdRaw: this.teamId, userId },
      "take back the owner role",
    );
    await this.roster.load({ teamId: this.teamId });
    this.said = `${memberLogin} is a member.`;
  }

  /// Takes the reader out of the team, and stops looking at it.
  ///
  /// Distinct from removing yourself, which the roster also allows an
  /// owner: this one is the departure verb, and the server refuses it
  /// to somebody holding no membership rather than treating them as a
  /// removable row. The last owner cannot go by either.
  async leaveTeam(): Promise<void> {
    const team = this.teamId;
    const teamName = (this.teams.data ?? []).find((t) => t.team_id === team)?.name ?? team;
    this.said = null;
    await mutate<void>("leave_team", { teamIdRaw: team }, "leave the team");
    this.stopReading();
    await this.teams.load({});
    this.said = `You have left ${teamName}.`;
  }

  /// Deletes the team, and stops looking at what is no longer there.
  async deleteTeam(): Promise<void> {
    const gone = this.teamId;
    const goneName = (this.teams.data ?? []).find((t) => t.team_id === gone)?.name ?? gone;
    this.said = null;
    await mutate<void>("delete_team", { teamIdRaw: gone }, "delete the team");
    this.stopReading();
    await this.teams.load({});
    this.said = `Deleted team ${goneName}.`;
  }

  /// Lets go of everything read about the team now named, and stops
  /// naming it.
  ///
  /// Whatever ends a reader's relationship with a team, what the panel
  /// has to forget is the same: it would otherwise keep drawing a
  /// roster, a line list and a ledger belonging to something it can no
  /// longer ask about. Written once because each new way of ending it
  /// arrives after the last, and a caller that skips the forgetting is
  /// the defect this exists to make impossible.
  stopReading(): void {
    this.teamId = "";
    this.closeLine();
    this.roster.reset();
    this.lines.reset();
    this.forgetLedger();
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
  /// **This is the naming act, and the gestures that reach it are
  /// equal.** A screen may offer several — pressing a team in the
  /// picker and submitting the id field are the two today — and
  /// neither is more the act than the other: what makes a team named
  /// is arriving here. A surface that treated one gesture as the act
  /// would have to answer what the other one is.
  ///
  /// What that means for a field: it may hold what somebody is typing
  /// without the catalog moving, because typing is not a gesture that
  /// arrives here. The panel's `teamField` is that, and says what it
  /// costs to bind it straight to `teamId` instead.
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
    // The list of lines too, and before the read rather than when it
    // answers: naming a team changes what the list is about, and a
    // list kept from the team before would answer — and read as
    // answered — for a team it was never asked about (#219). The rule
    // is the same for every read under a subject: whoever changes the
    // subject resets what was read for the last one, so `answered`
    // means "answered for what is on now" everywhere it is read.
    this.lines.reset();
    await this.lines.load({ teamId });
  }

  async show(lineId: string): Promise<void> {
    this.selected = lineId;
    // A piece of work belongs to the line it is against, so opening
    // another line ends whatever was open under the last one.
    this.working = null;
    // And what was read about the last one goes before the next is
    // read, not when it answers. `Resource.load` keeps the previous
    // answer until the new one lands, which is right for a list being
    // refreshed and wrong for a list whose subject changed: between
    // the two, `selected` named one line while the states, the chain
    // and the work under it were another's — and a picker built from
    // that list offered another line's work against this one (#219).
    // `lookAt` clears the same way for the same reason, one level up.
    this.states.reset();
    this.history.reset();
    this.pursuits.reset();
    // No work is showing under a line that has just been named, so
    // whatever these last answered about is gone with it.
    this.collisions.reset();
    this.behind.reset();
    await Promise.all([
      this.states.load({ teamId: this.teamId, lineId }),
      this.history.load({ teamId: this.teamId, lineId }),
      this.pursuits.load({ teamId: this.teamId, lineId }),
    ]);
  }

  /// Opens work against the line now showing.
  ///
  /// Decision 10 is why nothing is copied first: working on a shared
  /// line needs no clone. Nothing is read back about what it collides
  /// with or how far behind it is, on `forge.svelte.ts`'s own
  /// reasoning: a pursuit cut from where the line is now is level with
  /// it by construction, so both answers are empty and reset says the
  /// same thing without the two calls.
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
    this.collisions.reset();
    this.behind.reset();
    return pursuit;
  }

  /// Writes a round into the open work.
  ///
  /// Nothing reaches the line here. A round is a request, and the only
  /// moment anything lands is a satisfied close — which is what lets
  /// two members work one line without contending, and why the
  /// contents are not re-read after this. What a round asks for is
  /// half of what a collision is made of, so writing one can make or
  /// clear one (#211) — `behind` is not re-read, on `forge.svelte.ts`'s
  /// own reasoning: nothing this side writes moves how many landings
  /// the line has gained.
  async pushRound(ops: ForgeOpDto[], note: string): Promise<void> {
    const pursuitId = this.requireWork();
    this.said = null;
    await mutate<ForgePursuitDto>(
      "push_shared_round",
      { teamIdRaw: this.teamId, pursuitId, ops, note: note || null },
      "push that round",
    );
    await Promise.all([
      this.reloadWork(),
      this.collisions.load({ teamId: this.teamId, pursuitId }),
    ]);
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
    this.collisions.reset();
    this.behind.reset();
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
  /// Separate refusals rather than one, because "not in the list",
  /// "against another line" and "ended" are different facts. The
  /// first is what a failed re-read
  /// leaves — `Resource` puts its data back to the initial and the
  /// reason on `.error` — and not what one in flight leaves, which
  /// keeps the previous list until it resolves.
  ///
  /// **These three `require`s are backstops and their messages reach
  /// nobody.** They throw before `mutate` is called, and `mutate` is
  /// the only thing that puts a refusal in front of a person. That is
  /// the arrangement rather than a gap: a screen offers this verb only
  /// when it has a line and open work that has not ended, so a guard
  /// firing means a caller skipped what the screen checks. The message
  /// is for whoever is reading the stack, and the tests assert which
  /// of them fired.
  private requireOpenWork(): string {
    const pursuitId = this.requireWork();
    const work = this.work;
    if (work === null) {
      throw new Error("the work being read is not in this line's list");
    }
    // The list is reset when the line changes, so this cannot fire
    // from the catalog's own reads. It is the invariant said in one
    // more place than the reset, for the caller that sets `working`
    // by hand: content against work on one line, recorded at home
    // against another, is a link row naming a line the entry never
    // reached.
    if (work.line_id !== this.selected) {
      throw new Error("that work is against another line");
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
  /// there because the ids it needs are this catalog's: the team, the
  /// line, and the pursuit content may enter against (#148 decision
  /// 5). A pane holding its own copies of those would be a second
  /// answer to a question this frame already answers, and the place
  /// they could disagree. The work has to be open already —
  /// `Promotion::pursuit_id` on the client says why this does not
  /// open one for a caller with none: a pursuit is a decision to start
  /// work, and one opened as a step of a promotion the team then
  /// refuses is a record of a decision nobody made.
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
