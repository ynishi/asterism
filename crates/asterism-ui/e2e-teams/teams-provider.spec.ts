// Signing in through the team's identity provider, from the drawer to a
// session, across three real processes (#163).
//
// Its subject is
// the seam between the legs — that the command the button reaches starts
// an attempt the server accepts, hands the window the page it sent the
// browser to, listens where it said it would, and collects what the
// server sends back through that port. This spec walks exactly that.
//
// # The browser
//
// Is this process. `connect_team_server_provider` always tells the
// window where it sent the browser; built with the `wdio` feature it
// does not open one — a real browser opening on a runner is a window
// nothing drives. The spec reads the URL off the
// drawer and walks the page over HTTP with `fetch`, following each
// redirect by hand the way the server's suite does: the start page,
// its button, the provider's consent, the callback, the redirect to
// the app's loopback port — which is the app under test answering —
// and the done page. Everything on that walk is a real process except
// the person clicking "Continue". Before the walk it presses once and
// cancels, so the way out of a wait is driven too.
//
// # What it meets and leaves
//
// It meets the window as the roster spec left it: signed in as the
// fixture's first account, the drawer closed. It opens the drawer and
// disconnects before anything else, because the form it needs is what
// a signed-in drawer does not show. The account it then signs in as
// holds no password and was bound by `onPrepare` to the email address
// the provider vouches for; the drawer names a session by the
// account's id, and `onPrepare` passes the id `create-user` reported,
// so which account signed in is asserted rather than inferred. It
// disconnects at the end, which revokes nothing — the box was not
// ticked, so no device token was minted — and the database goes with
// the run.
import { browser } from "@wdio/globals";
import fs from "node:fs";
import path from "node:path";

const DRIVER_MS = 15_000;
const ROUND_TRIP_MS = 30_000;
const POLL_GAP_MS = 250;

const SHARED_ROW = 'aside.sidebar button[title^="Lines a team hosts"]';
const DRAWER = '[role="dialog"][aria-label="Shared lines"]';

/** What `onPrepare` put up, or a failure that says it did not. */
function fixture(): { baseUrl: string; providerName: string; ssoId: string } {
  const read = (name: string): string => {
    const value = process.env[name];
    if (!value) {
      throw new Error(
        `${name} is not set — \`onPrepare\` in wdio.teams.conf.ts is what ` +
          `provides it, so this spec was run through the wrong config.`,
      );
    }
    return value;
  };
  return {
    baseUrl: read("E2E_TEAMS_BASE_URL"),
    providerName: read("E2E_TEAMS_PROVIDER_NAME"),
    ssoId: read("E2E_TEAMS_SSO_ID"),
  };
}

async function stage<T>(
  trail: string[],
  what: string,
  ms: number,
  run: () => Promise<T>,
): Promise<T> {
  const start = Date.now();
  try {
    const value = await Promise.race([
      run(),
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error(`timed out after ${ms} ms`)), ms),
      ),
    ]);
    trail.push(`${what} (${Date.now() - start} ms)`);
    return value;
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(
      `${what}: ${why}\n  passed already: ${trail.join(" -> ") || "(nothing)"}`,
    );
  }
}

async function pollUntil(
  probe: () => Promise<boolean>,
  what: string,
  ms: number,
): Promise<void> {
  const deadline = Date.now() + ms;
  for (;;) {
    if (await probe()) return;
    if (Date.now() > deadline) {
      throw new Error(`${what} (polled for ${ms} ms)`);
    }
    await new Promise((resolve) => setTimeout(resolve, POLL_GAP_MS));
  }
}

function drawerText(): Promise<string | null> {
  return browser.execute((sel: string) => {
    const drawer = document.querySelector(sel);
    return drawer === null ? null : (drawer.textContent ?? "");
  }, DRAWER);
}

async function clickIn(selector: string): Promise<void> {
  const hit = await browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    if (el === null) return false;
    (el as HTMLElement).click();
    return true;
  }, selector);
  if (!hit) throw new Error(`nothing matched ${selector}`);
}

/** Presses the button whose text carries `text`, inside `container`. */
async function clickCarrying(container: string, text: string): Promise<void> {
  const hit = await browser.execute(
    (sel: string, want: string) => {
      const scope = document.querySelector(sel);
      if (scope === null) return false;
      const button = Array.from(scope.querySelectorAll("button")).find(
        (candidate) => (candidate.textContent ?? "").includes(want),
      );
      if (button === undefined) return false;
      (button as HTMLElement).click();
      return true;
    },
    container,
    text,
  );
  if (!hit) throw new Error(`nothing in ${container} carries "${text}"`);
}

/** Types into a field the way a person does — see `teams-connect.spec.ts`. */
async function fill(selector: string, value: string): Promise<void> {
  const field = await $(selector);
  await field.waitForExist({ timeout: DRIVER_MS });
  await field.setValue(value);
}

/** The text of one element, or null when it is not there. */
function textOf(selector: string): Promise<string | null> {
  return browser.execute((sel: string) => {
    const el = document.querySelector(sel);
    return el === null ? null : (el.textContent ?? "").trim();
  }, selector);
}

async function snap(name: string): Promise<void> {
  const dir = process.env.E2E_TEAMS_SCREENS_DIR;
  if (!dir) return;
  try {
    fs.mkdirSync(dir, { recursive: true });
    await browser.saveScreenshot(path.join(dir, `${name}.png`));
  } catch {
    // Diagnostics must not cascade a failure.
  }
}

/**
 * One hop of the walk: a request that is expected to redirect, whose
 * `Location` is what the browser would follow next. `fetch` follows
 * nothing here, because the loopback hop is the assertion — a browser
 * that followed it silently would hide which process answered.
 */
async function hop(
  url: string,
  init: RequestInit,
  what: string,
): Promise<{ status: number; location: string | null; body: string }> {
  let response: Response;
  try {
    response = await fetch(url, { ...init, redirect: "manual" });
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(`${what} could not be reached at ${url}: ${why}`);
  }
  const body = await response.text();
  const location = response.headers.get("location");
  return { status: response.status, location, body };
}

/** The page's form token, read out of the HTML the way a browser would. */
function pageToken(page: string): string {
  const marker = 'name="token" value="';
  const start = page.indexOf(marker);
  if (start < 0) throw new Error("the start page carries no form token");
  const from = start + marker.length;
  const end = page.indexOf('"', from);
  return page.slice(from, end);
}

describe("the team plane, through the identity provider", () => {
  it("signs in from the button, through the browser, back on loopback", async () => {
    const trail: string[] = [];
    const { baseUrl, providerName, ssoId } = fixture();

    await stage(trail, "open the shared-lines drawer", DRIVER_MS, async () => {
      if ((await drawerText()) === null) await clickIn(SHARED_ROW);
      await pollUntil(
        async () => (await drawerText()) !== null,
        "the drawer never mounted",
        DRIVER_MS,
      );
    });

    // The window arrives signed in as the spec before this one left it.
    // The form is what a signed-in drawer does not show, so the session
    // is ended first — which is the password account's, not the one
    // this spec is about.
    await stage(
      trail,
      "start from a disconnected window",
      ROUND_TRIP_MS,
      async () => {
        if (((await drawerText()) ?? "").includes("Signed in as")) {
          await clickCarrying(DRAWER, "Disconnect");
        }
        await pollUntil(
          async () => !((await drawerText()) ?? "").includes("Signed in as"),
          "the drawer never returned to its form",
          ROUND_TRIP_MS,
        );
      },
    );

    // The form, pointed at the server; the button appears once the
    // server has been asked and said it has a provider.
    await stage(
      trail,
      "the drawer offers the provider",
      ROUND_TRIP_MS,
      async () => {
        await $(`${DRAWER} form input[type="url"]`).waitForExist({
          timeout: DRIVER_MS,
        });
        await fill(`${DRAWER} form input[type="url"]`, baseUrl);
        await pollUntil(
          async () =>
            ((await drawerText()) ?? "").includes(
              `Sign in with ${providerName}`,
            ),
          "the drawer never offered the provider's button",
          ROUND_TRIP_MS,
        );
      },
    );
    await snap("10-provider-offered");

    // Press it. The command binds the listener, starts the attempt and
    // tells the window where the browser went; under the e2e build it
    // opens none, and the drawer shows the page.
    const pressAndRead = async (): Promise<string> => {
      await clickCarrying(`${DRAWER} form`, `Sign in with ${providerName}`);
      await pollUntil(
        async () =>
          (await textOf('[data-testid="provider-start-url"]')) !== null,
        "the drawer never showed the page the browser was sent to",
        ROUND_TRIP_MS,
      );
      const url = await textOf('[data-testid="provider-start-url"]');
      if (url === null || !url.startsWith(baseUrl)) {
        throw new Error(`the start URL is not on the team server: ${url}`);
      }
      return url;
    };
    const abandoned = await stage(
      trail,
      "press the button and read where the browser was sent",
      ROUND_TRIP_MS,
      pressAndRead,
    );
    await snap("11-waiting-for-browser");

    // A wait a person walks away from: the cancel beside the page ends
    // it, the command answers with no session, and the form comes
    // back with nothing said — which is the drawer's own way out, and
    // the one the header of `commands.rs`'s cancel argues for. The
    // attempt it abandons is left to the server to expire.
    await stage(
      trail,
      "cancel the wait from the drawer",
      ROUND_TRIP_MS,
      async () => {
        await clickCarrying('[data-testid="provider-waiting"]', "Cancel");
        await pollUntil(
          async () =>
            (await textOf('[data-testid="provider-start-url"]')) === null,
          "the drawer kept waiting after the cancel",
          ROUND_TRIP_MS,
        );
        const text = (await drawerText()) ?? "";
        if (text.includes("Signed in as")) {
          throw new Error("a cancelled sign-in opened a session");
        }
        // A refusal is a toast outside the drawer, so it is read there.
        const toast = await textOf(".refusal-toast");
        if (toast !== null) {
          throw new Error(`a cancel was reported as a failure: ${toast}`);
        }
      },
    );

    // Press it again: the second attempt is the one walked, which is
    // also what says the first left nothing behind that blocks a new
    // one.
    const startUrl = await stage(
      trail,
      "press the button again for the attempt to walk",
      ROUND_TRIP_MS,
      pressAndRead,
    );
    if (startUrl === abandoned) {
      throw new Error("the second press reused the abandoned attempt");
    }

    // The browser's walk, by hand. Each hop asserts which process
    // answered and where it sent the browser next.
    await stage(trail, "walk the browser's leg", ROUND_TRIP_MS, async () => {
      const page = await hop(startUrl, { method: "GET" }, "the start page");
      if (page.status !== 200) {
        throw new Error(`the start page answered ${page.status}: ${page.body}`);
      }
      if (!page.body.includes("Asterism on ")) {
        throw new Error("the start page does not name the device asking");
      }
      const button = await hop(
        `${startUrl}/authorize`,
        {
          method: "POST",
          headers: { "content-type": "application/x-www-form-urlencoded" },
          body: `token=${encodeURIComponent(pageToken(page.body))}`,
        },
        "the button",
      );
      if (button.status !== 303 || button.location === null) {
        throw new Error(`the button answered ${button.status}: ${button.body}`);
      }
      const consent = await hop(
        button.location,
        { method: "GET" },
        "the provider",
      );
      if (consent.status !== 303 || consent.location === null) {
        throw new Error(
          `the provider answered ${consent.status}: ${consent.body}`,
        );
      }
      if (!consent.location.startsWith(`${baseUrl}/teams/auth/oidc/callback`)) {
        throw new Error(`the provider sent the browser to ${consent.location}`);
      }
      const callback = await hop(
        consent.location,
        { method: "GET" },
        "the callback",
      );
      if (callback.status !== 303 || callback.location === null) {
        throw new Error(
          `the callback answered ${callback.status}: ${callback.body}`,
        );
      }
      // The leg that ties the answer to this machine: the server sends
      // the browser to a port on 127.0.0.1, and what answers there is
      // the app under test.
      if (
        !/^http:\/\/127\.0\.0\.1:\d+\/teams\/auth\/oidc\/loopback\?attempt=/.test(
          callback.location,
        )
      ) {
        throw new Error(
          `the callback sent the browser to ${callback.location}`,
        );
      }
      if (!callback.location.includes("&grant=")) {
        throw new Error(`the callback carried no grant: ${callback.location}`);
      }
      const loopback = await hop(
        callback.location,
        { method: "GET" },
        "the app's listener",
      );
      if (loopback.status !== 303 || loopback.location !== `${startUrl}/done`) {
        throw new Error(
          `the app's listener answered ${loopback.status} to ${loopback.location}: ${loopback.body}`,
        );
      }
      const done = await hop(
        loopback.location,
        { method: "GET" },
        "the done page",
      );
      if (done.status !== 200 || !done.body.includes("Signed in.")) {
        throw new Error(`the done page answered ${done.status}: ${done.body}`);
      }
    });

    // Back in the window: the command collected the session and the
    // drawer names it by the account's id, for an account nobody typed.
    // The id is the one `create-user` reported for the bound account,
    // so this is the account the provider vouched for and not the one
    // the roster spec left.
    await stage(
      trail,
      "the drawer reports the bound account's session",
      ROUND_TRIP_MS,
      async () => {
        await pollUntil(
          async () => ((await drawerText()) ?? "").includes("Signed in as"),
          "the drawer never reported a session",
          ROUND_TRIP_MS,
        );
        const text = (await drawerText()) ?? "";
        if (text.includes(`Sign in with ${providerName}`)) {
          throw new Error("the form is still showing beside the session");
        }
        if (!text.includes(ssoId)) {
          throw new Error(
            `the drawer is signed in as somebody other than ${ssoId}: ${text}`,
          );
        }
      },
    );
    await snap("12-signed-in-through-provider");

    await stage(trail, "disconnect", ROUND_TRIP_MS, async () => {
      await clickCarrying(DRAWER, "Disconnect");
      await pollUntil(
        async () => {
          const text = (await drawerText()) ?? "";
          return !text.includes("Signed in as");
        },
        "the drawer never returned to its form",
        ROUND_TRIP_MS,
      );
    });
    await snap("13-disconnected");
  });
});
