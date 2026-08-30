// WebdriverIO config for the team plane's e2e suite (`just ui-e2e-teams`).
//
// A sibling of `wdio.conf.ts` and `wdio.bench.conf.ts` rather than a
// variant of either. What makes it one is that this run needs a second
// process: every read on the team plane is a request to a
// `teams-server`, and a sibling config starts the app and nothing else.
//
// # Why the specs do not join the e2e suite's run
//
// Because a run may hold one stateful fixture or two, and #188 is what
// two looks like: `card-trash.spec.ts` fails in a full run and passes
// alone, with the signature of a fixture left in one of two states. A
// second fixture in the same run turns one error message into two
// causes.
//
// Here the separation costs nothing, because this fixture is created
// empty per run and thrown away. The app's profile cannot be: the
// e2e suite's specs provoke verbs against seeded content and put back
// what they take, which is the arrangement `card-trash.spec.ts`
// argues for where it restores what it trashed.
//
// # What `onPrepare` puts up, and how a spec reaches it
//
// A database of its own, an account, and a team, in that order,
// because `teams-server` has no verb that makes a team and
// `POST /teams/create` is behind a session. So the fixture logs in
// over HTTP before the app is ever driven.
//
// Then a second team, with a line holding one entry, for the work
// spec — `seedWorkableTeam` says why it is a second one and why the
// app cannot seed it.
//
// A spec reads the values it needs from `process.env`. That is
// the channel that works: the worker re-evaluates this module, so
// anything written into the exported `config` object here is not what
// the worker runs — the screenshot directory already travels by
// environment for the same reason.
//
// The app is given nothing. The base URL is typed into the connect
// form, so it reaches the backend as `connect_team_server`'s argument
// rather than as an environment variable, and `capabilities[].env`
// stays about the app's own profile.
//
// # The password
//
// Generated per run. The database is disposable, so a fixed one would
// leak nothing — but a literal credential in the tree is a thing every
// reader has to classify, and generating it costs one line.
// `create-user` reads it from the environment and refuses placeholders
// outright (#83 §5: the instance has no default credentials).
//
// # Ports
//
// 19897 for the app: not the e2e suite's 19899, not the bench's 19898,
// not a profile default. 19989 for the server, beside its own default
// of 9989 and out of reach of an instance somebody is running.
//
// A server that cannot bind stops the run here rather than letting
// every spec fail at the connect form. That failure mode is not
// hypothetical — the app spent two configs' worth of comments claiming
// a port it was not using, and in a window a failed bind is a warning
// the app survives, so no spec ever reported it.
//
// Which is why readiness is the server's own line rather than the port
// answering: see `waitForServer`.

import type { TauriCapabilities } from "@wdio/tauri-service";
import { SevereServiceError } from "webdriverio";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { createHash, randomBytes, randomUUID } from "node:crypto";
import path from "node:path";
import fs from "node:fs";

const here = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(here, "../..");

/** Ports of their own — see the header. */
const APP_PORT = "19897";
const SERVER_PORT = "19989";
const SERVER_HOST = "127.0.0.1";
const BASE_URL = `http://${SERVER_HOST}:${SERVER_PORT}`;

/** The account this fixture provisions. Its password is generated. */
const LOGIN = "e2e-member";

/** What the second team's line and its one entry are called. Named
 *  here because the spec asserts on both. */
const WORK_LINE_NAME = "ROOT";
const WORK_ENTRY_NAME = "cut-01";

/** The specs this suite runs, in the order the header argues for. */
const SPECS = [
  "./e2e-teams/teams-connect.spec.ts",
  "./e2e-teams/teams-work.spec.ts",
];

/** Everything the server owns, removed and remade on every run. */
const serverHome = path.join(repoRoot, "workspace/runtime/e2e-teams-server");
/** The app's profile home, kept apart from the e2e suite's own. */
const appHome = path.join(repoRoot, "workspace/runtime/e2e-teams");

const screensRoot = path.join(repoRoot, "workspace/test-logs/e2e-teams-screens");

const appBinary = path.join(repoRoot, "target/debug/asterism-ui");
const serverBinary = path.join(repoRoot, "target/debug/teams-server");

/** Same window and same reasoning as the two sibling configs. */
const WINDOW_LABEL = "main";

/** The running server, so `onComplete` and the exit hook can end it. */
let server: ChildProcess | null = null;

/** Runs one `teams-server` subcommand to completion, or throws saying which. */
function serverCli(args: string[], env: NodeJS.ProcessEnv = {}): void {
  const done = spawnSync(serverBinary, args, {
    env: { ...process.env, ...env },
    encoding: "utf8",
  });
  if (done.error) {
    throw new Error(
      `teams-server ${args[0]} could not be run (${done.error.message}). ` +
        `Expected the binary at ${serverBinary} — \`just ui-e2e-teams\` builds it.`,
    );
  }
  if (done.status !== 0) {
    throw new Error(
      `teams-server ${args[0]} exited ${done.status}: ` +
        `${(done.stderr || done.stdout || "").trim()}`,
    );
  }
}

/** What `teams-server serve` prints once it is bound and serving. */
const SERVING_MARK = "teams-server: http://";

/**
 * Resolves when the server says it is serving, or throws saying what
 * stopped it.
 *
 * It waits for the server's own line rather than for the port to
 * accept a connection, and the difference is the whole point of the
 * check. A TCP probe answers "somebody is listening", which is exactly
 * what is true when the port is already taken — so the version of this
 * that probed the port treated another process holding it as its own
 * server coming up, and the run went on to fail at the login with a
 * shape nobody would trace back to the port. Reading the line means a
 * bind failure is what it is: the child exits, and this rejects.
 */
function waitForServer(child: ChildProcess, deadlineMs: number): Promise<void> {
  return new Promise((resolve, reject) => {
    let settled = false;
    const settle = (error?: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolve();
    };
    const timer = setTimeout(
      () =>
        settle(
          new Error(
            `teams-server did not report itself serving within ${deadlineMs} ms.`,
          ),
        ),
      deadlineMs,
    );
    // The last thing it said, so the rejection can name the cause
    // rather than guess at it. A taken port is the likely reason to
    // exit early and not the only one — a migration the binary refuses
    // and a blob root it cannot open leave through the same door, and
    // the server has already said which on this stream.
    let lastSaid = "";
    child.stderr?.on("data", (chunk: Buffer) => {
      const text = chunk.toString();
      // The stream is piped so this can read it; forwarding keeps the
      // server's own log where an `inherit` would have put it.
      process.stderr.write(text);
      lastSaid = text.trim() || lastSaid;
      if (text.includes(SERVING_MARK)) settle();
    });
    child.once("exit", (code) =>
      settle(
        new Error(
          `teams-server exited ${code} before it began serving on ` +
            `${SERVER_HOST}:${SERVER_PORT}: ${lastSaid || "(it said nothing)"}`,
        ),
      ),
    );
  });
}

async function putBytes<T>(
  route: string,
  bytes: Buffer,
  token: string,
): Promise<T> {
  const response = await fetch(`${BASE_URL}${route}`, {
    method: "PUT",
    headers: {
      "content-type": "application/octet-stream",
      authorization: `Bearer ${token}`,
    },
    body: new Uint8Array(bytes),
  });
  if (!response.ok) {
    throw new Error(
      `PUT ${route} answered ${response.status}: ${(await response.text()).trim()}`,
    );
  }
  return (await response.json()) as T;
}

async function postJson<T>(
  route: string,
  body: unknown,
  token?: string,
): Promise<T> {
  const response = await fetch(`${BASE_URL}${route}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(token === undefined ? {} : { authorization: `Bearer ${token}` }),
    },
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    throw new Error(
      `POST ${route} answered ${response.status}: ${(await response.text()).trim()}`,
    );
  }
  return (await response.json()) as T;
}

/** Ends the server if it is still up. Safe to call more than once. */
function stopServer(): void {
  if (server === null) return;
  const child = server;
  server = null;
  child.kill("SIGTERM");
}

// Two hooks, because neither covers the other's case — and the belief
// that spawning attached is enough covers neither. A child spawned
// with the default `detached: false` is not killed when its parent
// exits; it is reparented. So the orderly exits are the `exit` hook's,
// which is exactly where Node fires it. A Ctrl-C in the terminal
// reaches the child directly, because it shares the process group, and
// the `SIGINT` hook is what keeps this side from outliving it. A
// `SIGTERM` or `SIGKILL` of the launcher is covered by neither, which
// is why `onPrepare` removes the database before making one rather
// than trusting the run before it to have finished.
process.on("exit", stopServer);
process.on("SIGINT", () => {
  stopServer();
  process.exit(130);
});

export const config: WebdriverIO.Config & {
  capabilities: TauriCapabilities[];
} = {
  runner: "local",
  // Named and ordered rather than globbed, because the order is load
  // bearing and a glob would decide it by filename.
  //
  // The two specs share one app process — the service starts the binary
  // once and hands each spec a session against it — so what one leaves
  // in the window, the next one meets. Two things survive: the backend
  // holds the team-server session for as long as the window does, and
  // the catalog keeps the team id across a disconnect on purpose.
  //
  // The first of those each spec can undo, and both do: they disconnect
  // at the end. The second has no undo on this surface — the team field
  // is `required`, so naming a team is a one-way trip and there is no
  // gesture that returns a window to having named none. `no-team` is
  // therefore reachable only by the spec that meets the window first,
  // and `teams-connect.spec.ts` is the one that asserts about it.
  //
  // A spec added here goes in this list. One that needs `no-team` goes
  // first, or gets a window of its own — and `assertEverySpecIsListed`
  // in `onPrepare` is what says so, because a list is the one shape
  // where forgetting to add a file is a spec that silently never runs.
  specs: SPECS,
  maxInstances: 1,

  services: ["@wdio/tauri-service"],

  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": {
        application: appBinary,
        args: ["--port", APP_PORT],
      },
      "wdio:tauriServiceOptions": {
        // The key the service spawns from; `tauri:options.args` above
        // reaches only a debug log inside it. The full measurement is
        // written out once in `wdio.conf.ts`, and both keys are kept in
        // sync here for the reason the bench config gives.
        appArgs: ["--port", APP_PORT],
        env: {
          ASTERISM_PROFILE: "dev",
          ASTERISM_HOME: appHome,
        },
      },
    },
  ],

  logLevel: "warn",
  framework: "mocha",
  reporters: ["spec"],
  // The app opens a real window and a real SQLite core, and this suite
  // also waits on a second process it did not start until `onPrepare`.
  mochaOpts: { ui: "bdd", timeout: 300_000 },

  // Identical to the two sibling configs and for the identical reason:
  // without it every element command pays a ~5 s "core.invoke not
  // available" timeout. The argument is written out once in
  // `wdio.conf.ts`; the three must not drift.
  beforeSuite: async () => {
    const tauri: WebdriverIO.Browser["tauri"] | undefined = browser.tauri;
    try {
      if (!tauri) {
        throw new Error("browser.tauri is missing (service before() hook)");
      }
      await tauri.switchWindow(WINDOW_LABEL);
    } catch (error) {
      console.warn(
        `[wdio.teams.conf] auto-focus opt-out failed (${String(error)}) — ` +
          `every element command now pays the ~5 s core.invoke timeout.`,
      );
    }
  },

  // Wrapped, because the class of the error decides whether the run
  // stops.
  //
  // A launcher hook that rejects with anything else is logged and the
  // specs start regardless — `Error in hook: …`, and then a window
  // opens against a fixture that was never built [measured 2026-08-29
  // against a port held by another `teams-server`]. `SevereServiceError`
  // is the one the launcher rethrows, which rejects the awaited call
  // before any worker spawns, so no spec runs at all. `onComplete`
  // still runs from the launcher's `finally`, so the server is stopped
  // and the database removed on this path exactly as on the other.
  onPrepare: async () => {
    try {
      await prepareFixture();
    } catch (error) {
      throw new SevereServiceError(
        error instanceof Error ? error.message : String(error),
      );
    }
  },

  onComplete: () => {
    stopServer();
    fs.rmSync(serverHome, { recursive: true, force: true });
  },

  afterTest: async (test, _context, result) => {
    const dir = process.env.E2E_TEAMS_SCREENS_DIR;
    if (!dir || result.passed) return;
    try {
      const safe = test.title.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 80);
      await browser.saveScreenshot(path.join(dir, `FAIL_${safe}.png`));
    } catch {
      // Diagnostics must not cascade a failure.
    }
  },
};

/**
 * Fails the run when a spec file is not in `SPECS`.
 *
 * The list replaced a glob because the order is load bearing, and it
 * took a failure mode with it: a glob cannot miss a file and a list
 * can. An unlisted spec does not fail — it does not run, and
 * `just ui-e2e-teams` reports the suite green, which is the worst
 * shape a gate has.
 *
 * Named rather than counted, so the message says which file.
 */
function assertEverySpecIsListed(): void {
  const dir = path.join(here, "e2e-teams");
  const listed = new Set(SPECS.map((spec) => path.basename(spec)));
  const missing = fs
    .readdirSync(dir)
    .filter((name) => name.endsWith(".spec.ts"))
    .filter((name) => !listed.has(name));
  if (missing.length > 0) {
    throw new Error(
      `e2e-teams/${missing.join(", ")} is not in this config's \`specs\` list, ` +
        `so it would not run. The list is ordered on purpose — see the ` +
        `comment above it — so add the file where its fixture needs it.`,
    );
  }
}

/** Everything the run needs before a window opens. */
async function prepareFixture(): Promise<void> {
  assertEverySpecIsListed();
  const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  const dir = path.join(screensRoot, stamp);
  fs.mkdirSync(dir, { recursive: true });
  process.env.E2E_TEAMS_SCREENS_DIR = dir;
  const runs = fs
    .readdirSync(screensRoot)
    .filter((name) => !name.startsWith("."))
    .sort();
  for (const old of runs.slice(0, Math.max(0, runs.length - 10))) {
    fs.rmSync(path.join(screensRoot, old), { recursive: true, force: true });
  }

  // A database nothing has touched. Removing it first rather than
  // afterwards is what makes a killed run harmless to the next one.
  fs.rmSync(serverHome, { recursive: true, force: true });
  fs.mkdirSync(serverHome, { recursive: true });
  const db = path.join(serverHome, "teams.db");
  const blobs = path.join(serverHome, "blobs");
  const password = randomBytes(18).toString("base64url");

  serverCli(["init", "--db", db]);
  serverCli(["create-user", "--db", db, "--login", LOGIN], {
    ASTERISM_TEAMS_USER_PASSWORD: password,
  });

  const child = spawn(
    serverBinary,
    ["serve", "--db", db, "--blobs", blobs, "--port", SERVER_PORT],
    // stderr piped so the readiness line can be read; the handler
    // forwards it, so nothing is lost by not inheriting.
    { stdio: ["ignore", "inherit", "pipe"] },
  );
  server = child;
  await waitForServer(child, 30_000);

  const session = await postJson<{ token: string }>("/teams/auth/login", {
    login: LOGIN,
    password,
  });
  const team = await postJson<{ team_id: string }>(
    "/teams/create",
    { owner_user_id: null },
    session.token,
  );

  process.env.E2E_TEAMS_BASE_URL = BASE_URL;
  process.env.E2E_TEAMS_LOGIN = LOGIN;
  process.env.E2E_TEAMS_PASSWORD = password;
  process.env.E2E_TEAMS_ID = team.team_id;

  const worked = await seedWorkableTeam(session.token);
  process.env.E2E_TEAMS_WORK_ID = worked.teamId;
  process.env.E2E_TEAMS_WORK_LINE = WORK_LINE_NAME;
  process.env.E2E_TEAMS_WORK_ENTRY = WORK_ENTRY_NAME;
}

/**
 * A second team, with a line holding one entry, for the work spec.
 *
 * A second team rather than content on the first, because the first
 * team's emptiness is what `teams-connect.spec.ts` is about: it asserts
 * "This team hosts no lines." to tell the two kinds of empty apart, and
 * a line seeded into it would take that assertion away and leave the
 * bug it guards unwatched.
 *
 * Seeded over HTTP rather than through the app, because the app cannot
 * do this: putting content on a team's line is `enter_content`, which
 * is the promotion — #198's sibling and out of its scope. The work
 * surface under test moves entries the line already holds, so the
 * fixture is what puts one there.
 *
 * The order is decision 5's rather than a choice: content enters
 * against open work, and only then can a round name it. The same walk
 * is made in Rust by `forge_routes_e2e.rs`'s
 * `a_member_works_a_line_from_login_to_landing`, which reads more
 * around it than this needs.
 */
async function seedWorkableTeam(token: string): Promise<{ teamId: string }> {
  const team = await postJson<{ team_id: string }>(
    "/teams/create",
    { owner_user_id: null },
    token,
  );
  const teamId = team.team_id;
  const line = await postJson<{ id: string }>(
    `/teams/${teamId}/forge/lines`,
    { name: WORK_LINE_NAME, strategy_id: "mainline-first" },
    token,
  );
  const pursuit = await postJson<{ id: string }>(
    `/teams/${teamId}/forge/pursuits`,
    { line_id: line.id },
    token,
  );

  const bytes = Buffer.from("the seeded artefact");
  const digest = `sha256:${createHash("sha256").update(bytes).digest("hex")}`;
  const entered = await putBytes<{ asset_id: string }>(
    `/teams/${teamId}/forge/pursuits/${pursuit.id}/content?digest=${digest}`,
    bytes,
    token,
  );

  await postJson(
    `/teams/${teamId}/forge/pursuits/${pursuit.id}/push`,
    {
      ops: [
        {
          entry_id: randomUUID(),
          kind: "add",
          content_asset_id: entered.asset_id,
          name: WORK_ENTRY_NAME,
        },
      ],
    },
    token,
  );
  await postJson(
    `/teams/${teamId}/forge/pursuits/${pursuit.id}/close`,
    { outcome: "satisfied" },
    token,
  );
  return { teamId };
}
