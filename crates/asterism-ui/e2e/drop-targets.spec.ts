// The drop surface, asserted in the DOM.
//
// Everything about a drag that persists is already reachable over HTTP:
// `POST /asterism/groups/reorder` writes the arrangement,
// `GET /asterism/assets/index?sort=…` reads it back, and
// `sorted_list_e2e` pins the two against each other. None of that says
// whether the *gesture* is wired — whether a row advertises itself as a
// drop target at all, and whether it stops advertising when the drop
// would be refused.
//
// That is the gap these specs cover, and the reason they are here
// rather than in Rust. The drag router (`App.svelte:onCardDropTarget`)
// finds its target by asking the DOM what is under the pointer:
//
//     elementFromPoint(x, y).closest('[data-drop-kind]')
//
// so a target that renders without those two attributes is invisible to
// a drop no matter how correct the backend write is. Reading the same
// attributes the router reads is therefore the assertion — not a proxy
// for it.
//
// Deliberately *not* here: synthesising a pointer drag to prove a card
// lands where it was dropped. The landing is `reorderOnto` writing
// `asset_bucket.position`, which the Rust e2e already covers with a
// fixture that crosses the axis against arrival order. Driving a
// 4px-threshold pointer gesture through WebDriver to re-check a
// database write would be the slowest possible way to ask a question
// already answered.
//
// # Why every `$$` here goes through `getElements()`
//
// It is the same value either way at runtime: `$$` hands back a Proxy
// that forwards `then` to the underlying promise, so awaiting it
// resolves to a real `ElementArray` [measured 2026-08-12,
// @wdio/utils/build/index.js:922 `PROMISE_METHODS`, forwarded at
// :1063-1065]. The *type* stopped following in webdriverio 9:
// `ChainablePromiseArray` declares `length: Promise<number>` and
// `[n: number]: ChainablePromiseElement` and no longer extends
// `Promise<ElementArray>` [webdriverio/build/types.d.ts:92-121], so
// `Awaited<>` leaves the chainable in place.
//
// Awaiting directly therefore made the `dirs.length === 0` fallback
// below read as a comparison with no overlap (TS2367) and `trash[0]`
// as unassignable to `WebdriverIO.Element` (TS2345). Both are artefacts
// of the declaration rather than dead code — the fallback branch runs,
// on a profile with no dirs — and both went unseen until
// `tsconfig.e2e.json` first pointed a compiler at this file.
// `getElements()` is declared `Promise<WebdriverIO.ElementArray>`
// [same file], so the awaited value is finally the one the runtime was
// producing all along.

import { $, $$ } from "@wdio/globals";

/** The two attributes the drag router reads, for one element. */
async function dropTarget(el: WebdriverIO.Element) {
  return {
    kind: await el.getAttribute("data-drop-kind"),
    id: await el.getAttribute("data-drop-id"),
  };
}

describe("drop targets", () => {
  before(async () => {
    // The shell mounts before the first page load resolves, so waiting
    // on the sidebar is waiting on Svelte, not on SQLite.
    await $("aside.sidebar").waitForExist({ timeout: 60_000 });
  });

  it("advertises every modality row that accepts a card", async () => {
    const rows = await $$("aside.sidebar li").getElements();
    expect(rows.length).toBeGreaterThan(0);

    // `$$` resolves to a wdio ElementArray, not a plain Array — its
    // `map` is wdio's own and already awaits, so handing the result to
    // `Promise.all` gets a non-iterable.
    const targets = [];
    for (const row of rows) targets.push(await dropTarget(row));
    const modality = targets.filter((t) => t.kind === "modality");
    expect(modality.length).toBeGreaterThan(0);

    // A `kind` without an `id` is the failure mode that matters: the
    // router would resolve a target and then have nothing to move the
    // card into.
    for (const t of modality) {
      expect(t.id).not.toBe(null);
      expect(t.id).not.toBe("");
    }
  });

  it("leaves Unclassified without a drop kind", async () => {
    // `ModalityList.accepts()` withholds the attribute rather than
    // refusing the drop later, so the row must never light up. Asserted
    // by absence, which is exactly what the router sees.
    const rows = await $$("aside.sidebar li").getElements();
    for (const row of rows) {
      const label = (await row.getText()).trim();
      if (!label.startsWith("Unclassified")) continue;
      const t = await dropTarget(row);
      expect(t.kind).toBe(null);
    }
  });

  it("gives dir rows a drop kind, including the Root row", async () => {
    const dirs = await $$("aside.sidebar .dir-row").getElements();
    if (dirs.length === 0) {
      // No dirs in this profile: the Root row still has to be a target,
      // since it is how a Group gets filed back to the top level.
      const roots = await $$('[data-drop-kind="dir"]').getElements();
      expect(roots.length).toBeGreaterThan(0);
      return;
    }
    for (const dir of dirs) {
      const t = await dropTarget(dir);
      expect(t.kind).toBe("dir");
      expect(t.id).not.toBe(null);
    }
  });

  it("advertises the Trash row on the live side", async () => {
    // The default view is the live set, where dragging a card onto
    // Trash must work. The attribute is withheld on the trash side
    // (a trashed card is not re-trashable), but flipping the view
    // toggle here would leave state behind for the other specs —
    // the positive half is the wiring the issue was about.
    const trash = await $$(
      'aside.sidebar [data-drop-kind="trash"]',
    ).getElements();
    expect(trash.length).toBe(1);
    const t = await dropTarget(trash[0]);
    expect(t.id).not.toBe(null);
    expect(t.id).not.toBe("");
  });

  it("does not advertise cards as drop targets outside the arrangement view", async () => {
    // `reorderActive` gates the card→card target on four conditions
    // (one manual Group, `Group` + `As arranged`, no search, not
    // reversed). The default view satisfies none of them, so no card
    // may carry the attribute. This is the negative half of the gate
    // the Rust side checks positively.
    const cards = await $$(
      '.grid-wrapper [data-drop-kind="card"]',
    ).getElements();
    expect(cards.length).toBe(0);
  });
});
