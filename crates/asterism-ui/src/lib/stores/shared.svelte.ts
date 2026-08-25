// Shared lines — the lines a team hosts, which are not this machine's.
//
// This catalog exists because #148 decision 16 says a shared line is
// served through rather than mirrored: reads go to the server, there is
// no local copy, and therefore no staleness to reason about. Everything
// here follows from that one sentence.
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
// # Two writes, and they are not symmetrical
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
import { api } from "../api";
import { mutate } from "../mutate";
import { Resource } from "./_resource.svelte";
import type {
  AssetDto,
  ForgeEntryStateDto,
  ForgeLineDto,
  ForgeLineHistoryDto,
} from "../../bindings";

/// What the two reads need to name a line on a server.
type TeamArgs = { teamId: string };
type LineArgs = { teamId: string; lineId: string };

class SharedCatalog {
  /// Whether the panel is showing. The panel reads this itself; the
  /// App only mounts it.
  open = $state(false);
  /// The user id the server answered with, or `null` when this window
  /// is talking to no team.
  session = $state<string | null>(null);
  /// Which team is being looked at. Typed in, because there is no verb
  /// on the member's client for "the teams I am in" — see the panel.
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

  /// Opening the panel reads. A served-through view that showed the
  /// last answer it happened to have would be a mirror with extra
  /// steps, which is the thing decision 16 refuses.
  async openPanel(): Promise<void> {
    this.open = true;
    await this.refreshSession();
    if (this.session !== null && this.teamId !== "") {
      await this.lines.load({ teamId: this.teamId });
    }
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
  }

  async disconnect(): Promise<void> {
    await api("disconnect_team_server");
    this.session = null;
    this.selected = null;
    // Not a cache being invalidated — a served-through view losing the
    // thing it was served through.
    this.lines.reset();
    this.states.reset();
    this.history.reset();
  }

  async show(lineId: string): Promise<void> {
    this.selected = lineId;
    await Promise.all([
      this.states.load({ teamId: this.teamId, lineId }),
      this.history.load({ teamId: this.teamId, lineId }),
    ]);
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
