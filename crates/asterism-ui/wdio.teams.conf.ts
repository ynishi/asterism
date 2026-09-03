// WebdriverIO config for the team plane's e2e suite (`just ui-e2e-teams`).
//
// A sibling of `wdio.conf.ts` and `wdio.bench.conf.ts` rather than a
// variant of either. What makes it one is that this run needs more
// than the app: every read on the team plane is a request to a
// `teams-server`, the sign-in spec needs an identity provider beside
// it, and a sibling config starts the app and nothing else.
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
// A database of its own, the accounts — two with passwords, and for
// the sign-in spec a third that holds none, with an identity provider
// process to vouch for it, started before the server — and a team, in
// that order, because `teams-server` has no verb that makes a team
// and `POST /teams/create` is behind a session. So the fixture logs
// in over HTTP before the app is ever driven.
//
// Then the teams a spec cannot make for itself. One the second
// account founds and invites the first into, because leaving needs a
// team the window's account did not found. One with a line holding an
// entry, for the work spec — `seedWorkableTeam` says why the app
// cannot seed that one. `prepareFixture` is the order they are made
// in; a spec reaches each by the environment name given it there.
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
// of 9989 and out of reach of an instance somebody is running. 19990
// for the stand-in identity provider, next to the server's.
//
// A child that cannot bind stops the run here rather than letting
// every spec fail at the connect form. That failure mode is not
// hypothetical — the app spent two configs' worth of comments claiming
// a port it was not using, and in a window a failed bind is a warning
// the app survives, so no spec ever reported it.
//
// Which is why readiness is each child's own line rather than the port
// answering: see `waitForMark`.

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
// A second account, so the roster has somebody to let in and take back
// out (#210). It joins none of the teams a spec invites it to, which
// is the point — an account already on that roster would have nothing
// to prove. It founds one of its own, because leaving needs a team the
// window's account did not found.
const OTHER_LOGIN = "e2e-other";

/** What the second team's line and its one entry are called. Named
 *  here because the spec asserts on both. */
const WORK_LINE_NAME = "ROOT";
const WORK_ENTRY_NAME = "cut-01";

/** The specs this suite runs, in the order argued for where `specs`
 *  is set below. */
const SPECS = [
  "./e2e-teams/teams-connect.spec.ts",
  "./e2e-teams/teams-work.spec.ts",
  "./e2e-teams/teams-promote.spec.ts",
  "./e2e-teams/teams-roster.spec.ts",
  // After the roster spec: it signs in as a third account, and the
  // window it meets is signed in as the first, which it has to end
  // before the form it needs shows — see its header.
  "./e2e-teams/teams-provider.spec.ts",
];

/**
 * How many hits a minute the server lets one address make on the
 * auth routes its limiter covers. Its default is sized for one
 * person guessing (#83 §5); this suite is one address making every
 * spec's sign-ins, sign-outs and the provider round trip. The number
 * is what the run needs with room, not a claim about the limiter —
 * the server's own tests hold that.
 */
const AUTH_RATE_LIMIT = "100";

/** Everything the server owns, removed and remade on every run. */
const serverHome = path.join(repoRoot, "workspace/runtime/e2e-teams-server");
/** The app's profile home, kept apart from the e2e suite's own. */
const appHome = path.join(repoRoot, "workspace/runtime/e2e-teams");

const screensRoot = path.join(
  repoRoot,
  "workspace/test-logs/e2e-teams-screens",
);

const appBinary = path.join(repoRoot, "target/debug/asterism-ui");
const serverBinary = path.join(repoRoot, "target/debug/teams-server");
/** The stand-in identity provider (#163), an example of the server's
 *  crate — `just ui-e2e-teams` builds it beside the binary. */
const providerBinary = path.join(
  repoRoot,
  "target/debug/examples/fake_oidc_provider",
);

// The third process: the identity provider the sign-in spec walks
// through. Its port sits beside the server's; the client id and secret
// are what the server is started with and what the provider checks at
// the exchange — the secret generated per run for the password's
// reason, the id a constant, since an id is a name and not a
// credential. The account it vouches for holds no password and is
// bound to the address below by `create-user --oidc-email`.
const PROVIDER_PORT = "19990";
const PROVIDER_URL = `http://${SERVER_HOST}:${PROVIDER_PORT}`;
const PROVIDER_NAME = "Example IdP";
const OIDC_CLIENT_ID = "asterism-e2e";
const SSO_LOGIN = "e2e-sso";
const SSO_EMAIL = "e2e-sso@example.test";

/** The running provider, ended beside the server. */
let provider: ChildProcess | null = null;

/** What the provider prints once it is bound and serving. */
const PROVIDER_MARK = "fake-oidc-provider: http://";

/** Same window and same reasoning as the two sibling configs. */
const WINDOW_LABEL = "main";

/** The running server, so `onComplete` and the exit hook can end it. */
let server: ChildProcess | null = null;

/** Runs one `teams-server` subcommand to completion and hands back
 *  its stdout, or throws saying which. */
function serverCli(args: string[], env: NodeJS.ProcessEnv = {}): string {
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
  return done.stdout;
}

/** The id `create-user` reports, off the line it prints. */
function createdUserId(said: string): string {
  const match = /\(user_id ([^)]+)\)/.exec(said);
  if (match === null) {
    throw new Error(
      `create-user did not report a user_id; it said: ${said.trim()}`,
    );
  }
  return match[1];
}

/** What `teams-server serve` prints once it is bound and serving. */
const SERVING_MARK = "teams-server: http://";

/** Resolves when the team server says it is serving — `waitForMark`
 *  for its line. */
function waitForServer(child: ChildProcess, deadlineMs: number): Promise<void> {
  return waitForMark(child, SERVING_MARK, "teams-server", deadlineMs);
}

/**
 * Resolves when the child prints the line that says it is serving, or
 * throws saying what stopped it.
 *
 * It waits for the child's own line rather than for the port to
 * accept a connection, and the difference is the whole point of the
 * check. A TCP probe answers "somebody is listening", which is exactly
 * what is true when the port is already taken — so the version of this
 * that probed the port treated another process holding it as its own
 * server coming up, and the run went on to fail at the login with a
 * shape nobody would trace back to the port. Reading the line means a
 * bind failure is what it is: the child exits, and this rejects.
 */
function waitForMark(
  child: ChildProcess,
  mark: string,
  name: string,
  deadlineMs: number,
): Promise<void> {
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
            `${name} did not report itself serving within ${deadlineMs} ms.`,
          ),
        ),
      deadlineMs,
    );
    // The last thing it said, so the rejection can name the cause
    // rather than guess at it. A taken port is the likely reason to
    // exit early and not the only one — for the team server, a
    // migration the binary refuses and a blob root it cannot open
    // leave through the same door — and the child has already said
    // which on this stream.
    let lastSaid = "";
    child.stderr?.on("data", (chunk: Buffer) => {
      const text = chunk.toString();
      // The stream is piped so this can read it; forwarding keeps the
      // child's own log where an `inherit` would have put it.
      process.stderr.write(text);
      lastSaid = text.trim() || lastSaid;
      if (text.includes(mark)) settle();
    });
    child.once("exit", (code) =>
      settle(
        new Error(
          `${name} exited ${code} before it began serving: ` +
            `${lastSaid || "(it said nothing)"}`,
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

/** Ends the server and the provider if they are still up. Safe to call
 *  more than once. */
function stopServer(): void {
  if (server !== null) {
    const child = server;
    server = null;
    child.kill("SIGTERM");
  }
  if (provider !== null) {
    const child = provider;
    provider = null;
    child.kill("SIGTERM");
  }
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
  // The specs share one app process — the service starts the binary
  // once and hands each spec a session against it — so what one leaves
  // in the window, the next one meets. What survives is everything the
  // window holds rather than a list somebody keeps here: the backend's
  // team-server session, the catalog's team id, which panel is open,
  // which tab it is on. Each spec puts back what it can and states at
  // its head what it met.
  //
  // The session every spec can undo, by disconnecting at the end; one
  // that leaves it leaves it for the next spec's header to state. The
  // team id is not undone by
  // naming another — picking a team and submitting the field both name
  // one — so a spec that needs a window that has never named a team
  // goes first, which is `teams-connect.spec.ts`. Leaving or deleting
  // the team named is what clears it, and a spec that meets the window
  // after one of those meets `no-team` again.
  //
  // A spec added here goes in this list. One that needs a window that
  // has never named a team goes first, or gets a window of its own —
  // and `assertEverySpecIsListed`
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
  // also waits on processes it did not start until `onPrepare`.
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
  // still runs from the launcher's `finally`, so the children are
  // stopped and the database removed on this path exactly as on the
  // other.
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
 *
 * Recursive, and compared on the path rather than the basename,
 * because the glob it replaced was `**` — a spec in a subdirectory was
 * covered before and has to stay covered, and two files of one name in
 * two directories are two specs.
 */
function assertEverySpecIsListed(): void {
  const dir = path.join(here, "e2e-teams");
  const listed = new Set(
    SPECS.map((spec) => path.relative(dir, path.resolve(here, spec))),
  );
  const missing = fs
    .readdirSync(dir, { recursive: true })
    .map((entry) => String(entry))
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
  const otherPassword = randomBytes(18).toString("base64url");
  serverCli(["create-user", "--db", db, "--login", OTHER_LOGIN], {
    ASTERISM_TEAMS_USER_PASSWORD: otherPassword,
  });
  // The account that signs in through the provider (#163): no
  // password, bound to the address the provider will vouch for. Its
  // id is what the drawer shows once signed in, so the spec gets it.
  const ssoId = createdUserId(
    serverCli([
      "create-user",
      "--db",
      db,
      "--login",
      SSO_LOGIN,
      "--oidc-email",
      SSO_EMAIL,
      "--oidc-issuer",
      PROVIDER_URL,
    ]),
  );

  // The provider first — though the server reaches it only at the
  // first sign-in, a provider that cannot bind should stop the run
  // here, for the reason the server's own bind does.
  const clientSecret = randomBytes(24).toString("base64url");
  const idp = spawn(
    providerBinary,
    [
      "--port",
      PROVIDER_PORT,
      "--client-id",
      OIDC_CLIENT_ID,
      "--client-secret",
      clientSecret,
      "--email",
      SSO_EMAIL,
    ],
    { stdio: ["ignore", "inherit", "pipe"] },
  );
  provider = idp;
  await waitForMark(idp, PROVIDER_MARK, "fake-oidc-provider", 30_000);

  const child = spawn(
    serverBinary,
    [
      "serve",
      "--db",
      db,
      "--blobs",
      blobs,
      "--port",
      SERVER_PORT,
      "--auth-rate-limit",
      AUTH_RATE_LIMIT,
      "--oidc-issuer",
      PROVIDER_URL,
      "--oidc-client-id",
      OIDC_CLIENT_ID,
      "--oidc-name",
      PROVIDER_NAME,
      "--public-url",
      BASE_URL,
    ],
    // stderr piped so the readiness line can be read; the handler
    // forwards it, so nothing is lost by not inheriting.
    {
      stdio: ["ignore", "inherit", "pipe"],
      env: { ...process.env, ASTERISM_TEAMS_OIDC_CLIENT_SECRET: clientSecret },
    },
  );
  server = child;
  await waitForServer(child, 30_000);

  process.env.E2E_TEAMS_PROVIDER_NAME = PROVIDER_NAME;
  process.env.E2E_TEAMS_SSO_ID = ssoId;

  const session = await postJson<{ token: string; user_id: string }>(
    "/teams/auth/login",
    { login: LOGIN, password },
  );
  const team = await postJson<{ team_id: string }>(
    "/teams/create",
    { owner_user_id: null },
    session.token,
  );

  process.env.E2E_TEAMS_BASE_URL = BASE_URL;
  process.env.E2E_TEAMS_LOGIN = LOGIN;
  process.env.E2E_TEAMS_PASSWORD = password;
  process.env.E2E_TEAMS_ID = team.team_id;

  // The invitee's id rather than its login. A membership row names an
  // account by id, so that is what the invite form asks for, and a
  // login is not something the roster could show back.
  const other = await postJson<{ token: string; user_id: string }>(
    "/teams/auth/login",
    { login: OTHER_LOGIN, password: otherPassword },
  );
  process.env.E2E_TEAMS_OTHER_ID = other.user_id;

  // A team the window's account is a member of rather than the owner
  // of. The roster spec needs one to leave from, and founding a team
  // makes you its owner — the last of which cannot go, by either verb.
  // So the second account founds this one and invites the first.
  const theirs = await postJson<{ team_id: string }>(
    "/teams/create",
    { owner_user_id: null },
    other.token,
  );
  await postJson(
    `/teams/${theirs.team_id}/members/invite`,
    { user_id: session.user_id, role: "member" },
    other.token,
  );
  process.env.E2E_TEAMS_LEAVE_ID = theirs.team_id;

  const worked = await seedWorkableTeam(session.token);
  process.env.E2E_TEAMS_WORK_ID = worked.teamId;
  process.env.E2E_TEAMS_WORK_LINE = WORK_LINE_NAME;
  process.env.E2E_TEAMS_WORK_ENTRY = WORK_ENTRY_NAME;

  // The app's own loopback HTTP surface, which is how the promotion
  // spec seeds the one asset it hands over. Passed by environment like
  // everything else here, and it is the app's port rather than the
  // team server's: every process this suite runs serves HTTP, and a
  // spec reaching the wrong one gets a 404 that says nothing.
  process.env.E2E_TEAMS_APP_URL = `http://${SERVER_HOST}:${APP_PORT}`;
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
 * Seeded over HTTP rather than through the app, because of when it is
 * needed rather than what the app can do: the work spec runs before
 * anything has promoted anything, and it needs a line that already
 * holds an entry to rename. The app grew the promotion in #200, and
 * `teams-promote.spec.ts` drives it — but that is the spec after this
 * fixture, not a way to build it.
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
