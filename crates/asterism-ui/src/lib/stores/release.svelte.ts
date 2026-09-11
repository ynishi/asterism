// Releases, and what became of the copies one wrote out.
//
// The forge's history tab anchors a change point; this catalog owns
// everything after the verb on it. A release freezes what a change
// point carried, copies those files into a directory and stamps each
// copy on the way out; a send puts exactly that stamped set on a
// destination's host. Both are recorded, and both hand the bytes to a
// dispatch — so what a person wants to see afterwards is on two records
// this reads and on none it keeps.
//
// # Two verbs, and why the second is not a checkbox on the first
//
// Writing out and sending are separate because the release is signed
// before it travels: re-rendering per destination would invalidate the
// manifest, so a send has to be a transport over the frozen files
// rather than a second export. The backend is already shaped that way
// — `ReleaseService` and `SendService` are two verbs over two records
// — and this catalog is the same shape rather than a screen that folds
// them back together.
//
// # Progress is read from the record, never held beside it
//
// The runner writes the file stamps onto the release and the host's
// answers onto the send's dispatch. Neither is pushed anywhere, so the
// only way to be right after a reload is to read them again — which is
// what this does, on `dispatchCatalog.pollDispatch`'s own cadence
// through its `onTick`. Nothing here caches a run's state: the poll
// says when to look, the record says what is true. A channel would be
// faster and is not what makes a reload correct, which is why #280
// leaves it out.
//
// # The dispatch rows, kept by id
//
// A release names one dispatch and each send names another, and what a
// screen needs off them is different: the release's says whether the
// copies have been written yet, and a send's carries `attempt_json`,
// the per-file record of what the host said. They are held in a plain
// map rather than a `Resource` each, for `forgeCatalog.cards`' reason
// — the set grows as sends arrive and what has been read stays read.
//
// # What is not here
//
// No profile editing. A destination profile is a JSON file in a
// directory, listed and validated by `list_transfer_profiles` and
// chosen; the app never writes one. `asterism-server`'s
// `transfer_profiles` is where profiles are described and why they are
// files rather than rows.
import { api } from "../api";
import { mutate } from "../mutate";
import { dispatchCatalog } from "./dispatch.svelte";
import { undoToastCatalog } from "./undo-toast.svelte";
import { Resource } from "./_resource.svelte";
import type {
  DispatchDto,
  ForgeReleaseDto,
  ForgeSendDto,
  TransferProfileListDto,
} from "../../bindings";

/// What a read of one release needs to name it.
type ReleaseArgs = { releaseId: string };

/// A dispatch nobody is waiting on any more.
///
/// The runner parks a row in one of these and nothing moves it
/// afterwards, so a poll that sees one stops and a screen that sees one
/// can say the run is over rather than "still reading".
const TERMINAL = new Set(["done", "failed", "cancelled"]);

class ReleaseCatalog {
  /// The release the drawer is showing, if one is open.
  ///
  /// An id rather than the record: the record is a `Resource` that is
  /// empty for the moment between asking and arriving, and a drawer
  /// that read emptiness as "nothing open" would close itself on every
  /// re-read.
  openId = $state<string | null>(null);

  /// The release being read, its file stamps included.
  ///
  /// `files` is empty until the run has written them, and that is a
  /// state rather than missing information — `ForgeReleaseDto::files`
  /// says so. The drawer asks the dispatch which of the two it is
  /// looking at.
  release = new Resource<ReleaseArgs, ForgeReleaseDto | null>(
    async (args) =>
      api<ForgeReleaseDto>("get_forge_release", { releaseId: args.releaseId }),
    null,
    "releaseCatalog.release",
  );

  /// Every time this release went out, most recent first.
  ///
  /// A list because nothing refuses a second send: a re-submission
  /// after a rejection is the case the record exists to hold.
  sends = new Resource<ReleaseArgs, ForgeSendDto[]>(
    async (args) =>
      api<ForgeSendDto[]>("list_forge_release_sends", {
        releaseId: args.releaseId,
      }),
    [] as ForgeSendDto[],
    "releaseCatalog.sends",
  );

  /// The destination profiles this machine holds, and where from.
  ///
  /// The directory travels with the list because it is what an empty
  /// list has to explain; `asterism-server`'s `transfer_profiles` says
  /// why a missing directory is an empty list rather than a failure.
  profiles = new Resource<void, TransferProfileListDto | null>(
    async () => api<TransferProfileListDto>("list_transfer_profiles", {}),
    null,
    "releaseCatalog.profiles",
  );

  /// Where the next release writes its copies, resolved by the backend.
  ///
  /// Not derived from the setting here. Resolving it reads the
  /// environment and checks a marker, which a webview cannot do, and a
  /// second copy of the rule in TypeScript is the copy nobody would
  /// edit.
  outputDir = new Resource<void, string>(
    async () => api<string>("release_output_dir", {}),
    "",
    "releaseCatalog.outputDir",
  );

  /// The dispatch rows this surface is watching, by id.
  dispatches = $state<Record<string, DispatchDto>>({});

  /// Which change points have been written out, by change point id.
  ///
  /// Cumulative and keyed, like `forgeCatalog.cards`: the history tab
  /// asks for every point it is drawing and only the ones it is
  /// missing are fetched.
  ///
  /// One read per change point, because that is the read there is:
  /// releases are listed under the point they name.
  byPoint = $state<Record<string, ForgeReleaseDto[]>>({});

  /// Which line `byPoint` is about.
  ///
  /// A change point id belongs to the chain it is on, so nothing
  /// remembered about one line answers for another. Held here and
  /// checked on the way in rather than cleared by whoever changes the
  /// selection: a clear written at each site that moves off a line is
  /// how one of them gets missed, which `forgeCatalog.selectLine` says
  /// happened to its own `released` before its tests existed.
  #heldFor: string | null = null;

  /// Reads the releases of every point it does not have yet.
  ///
  /// Safe on every render pass, for `ensureCards`' reason: it asks only
  /// about what is missing and answers nothing when it is missing
  /// nothing.
  async ensureReleasesOf(lineId: string, pointIds: string[]): Promise<void> {
    if (this.#heldFor !== lineId) {
      this.#heldFor = lineId;
      this.byPoint = {};
    }
    const missing = [...new Set(pointIds)].filter(
      (id) => this.byPoint[id] === undefined,
    );
    if (missing.length === 0) return;
    const got = await Promise.all(
      missing.map(async (changePointId) => {
        try {
          return [
            changePointId,
            await api<ForgeReleaseDto[]>("list_forge_releases_of_change_point", {
              lineId,
              changePointId,
            }),
          ] as const;
        } catch (err) {
          // A point whose releases could not be read is left unknown
          // rather than recorded as none: "no releases" is a claim, and
          // a failed read is not evidence for it.
          console.warn("[releaseCatalog] releases of a point failed", err);
          return null;
        }
      }),
    );
    const next = { ...this.byPoint };
    for (const entry of got) {
      if (entry !== null) next[entry[0]] = entry[1];
    }
    this.byPoint = next;
  }

  /// Shows one release, and reads everything about it.
  async open(releaseId: string): Promise<void> {
    this.openId = releaseId;
    await Promise.all([
      this.release.load({ releaseId }),
      this.sends.load({ releaseId }),
    ]);
    await this.watchAll();
  }

  /// Stops showing it, and lets go of everything it produced.
  ///
  /// `byPoint` stays: it answers about the chain behind the drawer,
  /// which is still on screen, and the count on a change point row does
  /// not stop being true because a drawer closed. What ends *that* is
  /// the line changing, which `ensureReleasesOf` notices for itself.
  close(): void {
    this.openId = null;
    this.release.reset();
    this.sends.reset();
    this.profiles.reset();
  }

  /// Writes a change point out.
  ///
  /// The directory is the backend's answer rather than this screen's,
  /// and it is read here rather than passed in so that the value the
  /// command gets is the one the setting resolves to at the moment it
  /// is pressed.
  ///
  /// The drawer opens on the new release straight away, which is
  /// before its files exist: the dispatch has been started and runs
  /// afterwards, so what the drawer shows first is a release with a run
  /// in flight. That is the honest first frame, and the poll fills it.
  async writeOut(
    lineId: string,
    changePointId: string,
    personaId: string,
  ): Promise<void> {
    await this.outputDir.load(undefined);
    // **A directory nobody answered for is not a directory.** `Resource`
    // falls back to its initial value when a read fails, which here is
    // the empty string — and sending that on would have the exporter
    // refuse a path it cannot write to, after the release row had
    // already been recorded. The refusal is the same either way; what
    // differs is that this one leaves no release behind.
    if (!this.outputDir.answered || this.outputDir.data.trim() === "") {
      // Raised the way `mutate` raises one, because it is the same kind
      // of answer: the thing the person asked for did not happen, and a
      // refusal only the console hears is the defect `lib/mutate.ts`
      // exists to prevent. It is not routed *through* `mutate` because
      // nothing was invoked — there is no command to blame.
      undoToastCatalog.refuse(
        "Could not write this change point out.",
        this.outputDir.error ??
          "the directory to write into could not be read from the settings",
      );
      throw new Error("no output directory");
    }
    const released = await mutate<ForgeReleaseDto>(
      "release_forge_change_point",
      {
        command: {
          line_id: lineId,
          change_point_id: changePointId,
          persona_id: personaId,
          output_dir: this.outputDir.data,
        },
      },
      "write this change point out",
    );
    // The point now has one more release than whatever is remembered
    // about it, and re-reading is how the count stays the record's
    // rather than this store's arithmetic.
    const next = { ...this.byPoint };
    delete next[changePointId];
    this.byPoint = next;
    await this.ensureReleasesOf(lineId, [changePointId]);
    await this.open(released.id);
  }

  /// Puts this release's stamped copies on a destination's host.
  ///
  /// **The button is not the guard.** A release whose files are not
  /// written yet, and one whose copies have moved, are both refused by
  /// the backend with a reason, and `mutate` puts that reason on
  /// screen. Duplicating the rule here would give the screen a second
  /// opinion about when a send is allowed, and the first time the two
  /// disagreed the screen would be the one that was wrong.
  async send(
    releaseId: string,
    destination: string,
    profileJson: string,
  ): Promise<void> {
    await mutate<ForgeSendDto>(
      "send_forge_release",
      {
        command: {
          release_id: releaseId,
          destination,
          profile_json: profileJson,
        },
      },
      "send this release",
    );
    await this.sends.load({ releaseId });
    await this.watchAll();
  }

  /// Reads the dispatch behind the release and behind each send, and
  /// keeps reading the ones still running.
  ///
  /// One poll per run that has not parked. A run already terminal is
  /// read once and left alone — there is nothing coming to move it.
  async watchAll(): Promise<void> {
    const ids = new Set<string>();
    const release = this.release.data;
    if (release !== null) ids.add(release.dispatch_id);
    for (const sent of this.sends.data) ids.add(sent.dispatch_id);
    await Promise.all([...ids].map((id) => this.watch(id)));
  }

  /// Which runs this is already following, so a second `watchAll` does
  /// not start a second loop over one of them.
  #watching = new Set<string>();

  /// Follows one run until it parks, re-reading the records it writes.
  ///
  /// **A run found already parked still costs one re-read.** A release
  /// is recorded before its files exist and the run that writes them is
  /// started immediately after, so a fast copy can finish in the gap
  /// between reading the release and reading its dispatch — leaving a
  /// record on screen with no file rows and nothing left to fetch them,
  /// because the poll that would have is never started. The drawer then
  /// says the copies are on their way forever. One read closes it, and
  /// it is the read the poll's own tick would have made.
  async watch(dispatchId: string): Promise<void> {
    if (this.#watching.has(dispatchId)) return;
    let dto: DispatchDto;
    try {
      dto = await api<DispatchDto>("get_dispatch", { id: dispatchId });
    } catch (err) {
      console.warn("[releaseCatalog] reading a dispatch failed", err);
      return;
    }
    this.noteDispatch(dto);
    if (TERMINAL.has(dto.state)) {
      await this.reread();
      return;
    }
    this.#watching.add(dispatchId);
    try {
      await dispatchCatalog.pollDispatch(dispatchId, async (tick) => {
        this.noteDispatch(tick);
        await this.reread();
      });
    } finally {
      this.#watching.delete(dispatchId);
    }
  }

  /// Reads the release again, if one is open.
  ///
  /// The record, not a copy of it. The runner writes the file stamps
  /// onto the release and the host's answers onto the send's own
  /// dispatch row; nothing pushes either anywhere, so reading again is
  /// the only thing that makes this drawer and a reload agree.
  async reread(): Promise<void> {
    const open = this.openId;
    if (open === null) return;
    await this.release.load({ releaseId: open });
  }

  /// Keeps one dispatch row.
  noteDispatch(dto: DispatchDto): void {
    this.dispatches = { ...this.dispatches, [dto.id]: dto };
  }

  /// The run that carried this release's copies out, if it has been
  /// read.
  get releaseRun(): DispatchDto | null {
    const release = this.release.data;
    if (release === null) return null;
    return this.dispatches[release.dispatch_id] ?? null;
  }

  /// Whether the release's own run has parked.
  ///
  /// Read off the dispatch rather than off `files.length`, because an
  /// empty file list is also what a run still going has — and the two
  /// lead somewhere different. A parked run with no rows is a release
  /// that wrote nothing; a running one with no rows is a release whose
  /// files are on the way.
  get releaseSettled(): boolean {
    const run = this.releaseRun;
    return run !== null && TERMINAL.has(run.state);
  }

  /// The run behind one send, if it has been read.
  runOf(send: ForgeSendDto): DispatchDto | null {
    return this.dispatches[send.dispatch_id] ?? null;
  }
}

export const releaseCatalog = new ReleaseCatalog();

/// Whether a dispatch state is one nothing will move.
export function isTerminal(state: string): boolean {
  return TERMINAL.has(state);
}
