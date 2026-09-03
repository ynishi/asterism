// Resource primitive unit tests. Exercises the
// four cross-cutting concerns with fake fetchers: stale-response
// guard (generation), loading flag, error normalization, reset —
// plus the two H2 contract extensions (accepted boolean, isStale
// callback). No Tauri / DOM involved.
import { describe, expect, it, vi } from "vitest";
import { Resource } from "./_resource.svelte";

// Manually-resolvable promise so tests control response ordering.
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("Resource", () => {
  it("writes data and returns true on success", async () => {
    const r = new Resource(
      async (n: number) => [n],
      [] as number[],
      "test",
    );
    const accepted = await r.load(7);
    expect(accepted).toBe(true);
    expect(r.data).toEqual([7]);
    expect(r.loading).toBe(false);
    expect(r.error).toBeNull();
  });

  it("normalizes errors: warn + initial data + error field + false", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const r = new Resource(
      async (): Promise<number[]> => {
        throw new Error("boom");
      },
      [1, 2],
      "test.err",
    );
    const accepted = await r.load(undefined);
    expect(accepted).toBe(false);
    expect(r.data).toEqual([1, 2]);
    expect(r.error).toContain("boom");
    expect(r.loading).toBe(false);
    expect(warn).toHaveBeenCalledWith(
      "[test.err] load failed:",
      expect.any(Error),
    );
    warn.mockRestore();
  });

  it("clears error on the next successful load", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    let fail = true;
    const r = new Resource(
      async () => {
        if (fail) throw new Error("boom");
        return ["ok"];
      },
      [] as string[],
      "test",
    );
    await r.load(undefined);
    expect(r.error).not.toBeNull();
    fail = false;
    await r.load(undefined);
    expect(r.error).toBeNull();
    expect(r.data).toEqual(["ok"]);
    warn.mockRestore();
  });

  it("drops a stale response when a newer load supersedes it", async () => {
    const first = deferred<string[]>();
    const second = deferred<string[]>();
    const queue = [first.promise, second.promise];
    const r = new Resource(
      () => queue.shift()!,
      [] as string[],
      "test",
    );
    const p1 = r.load(undefined);
    const p2 = r.load(undefined);
    // Newest resolves first, then the superseded one lands late.
    second.resolve(["new"]);
    expect(await p2).toBe(true);
    first.resolve(["old"]);
    expect(await p1).toBe(false);
    expect(r.data).toEqual(["new"]);
    expect(r.loading).toBe(false);
  });

  it("keeps loading true until the NEWEST in-flight load settles", async () => {
    const first = deferred<string[]>();
    const second = deferred<string[]>();
    const queue = [first.promise, second.promise];
    const r = new Resource(() => queue.shift()!, [] as string[], "test");
    const p1 = r.load(undefined);
    const p2 = r.load(undefined);
    first.resolve(["old"]);
    await p1;
    expect(r.loading).toBe(true); // old completion must not clear it
    second.resolve(["new"]);
    await p2;
    expect(r.loading).toBe(false);
  });

  it("reset() restores initial data and invalidates in-flight loads", async () => {
    const slow = deferred<string[]>();
    const r = new Resource(() => slow.promise, [] as string[], "test");
    const p = r.load(undefined);
    r.reset();
    expect(r.loading).toBe(false);
    slow.resolve(["late"]);
    expect(await p).toBe(false);
    expect(r.data).toEqual([]);
    expect(r.error).toBeNull();
  });

  it("says whether any load has answered, apart from what it answered", async () => {
    // An initial value and an empty answer read the same on `data`;
    // `answered` is what tells them apart (#219). Set by success and
    // by failure alike, cleared by reset, and never by a response
    // that was dropped as stale.
    const r = new Resource(async (tag: string) => {
      if (tag === "fail") throw new Error("no");
      return [] as string[];
    }, [] as string[], "test");
    expect(r.answered).toBe(false);

    await r.load("ok");
    expect(r.data).toEqual([]);
    expect(r.answered).toBe(true);

    r.reset();
    expect(r.answered).toBe(false);

    vi.spyOn(console, "warn").mockImplementation(() => {});
    await r.load("fail");
    expect(r.answered).toBe(true);

    r.reset();
    const slow = deferred<string[]>();
    const s = new Resource(() => slow.promise, [] as string[], "test");
    const p = s.load(undefined);
    s.reset();
    slow.resolve(["late"]);
    await p;
    expect(s.answered).toBe(false);
  });

  it("exposes staleness to the fetcher via the isStale callback", async () => {
    const observed: boolean[] = [];
    const gate = deferred<void>();
    const r = new Resource(
      async (tag: string, isStale) => {
        observed.push(isStale());
        if (tag === "slow") {
          await gate.promise;
          observed.push(isStale());
        }
        return [tag];
      },
      [] as string[],
      "test",
    );
    const p1 = r.load("slow");
    const p2 = r.load("fast");
    gate.resolve();
    await Promise.all([p1, p2]);
    // slow saw fresh at entry, stale after the newer load started.
    expect(observed).toEqual([false, false, true]);
    expect(r.data).toEqual(["fast"]);
  });
});
