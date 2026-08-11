// Webview diag capture unit tests.
//
// What is worth pinning is the *contract of non-interference* plus the
// storm guard, because both failure modes are invisible in a demo and
// destructive in production:
//
//   - the original console method still runs (a capture that eats the
//     message would make local debugging worse than no capture);
//   - a recording failure is swallowed (a rejected invoke that threw
//     would recurse into the console.error hook it came from);
//   - identical messages inside the throttle window forward once (a
//     per-frame render error must not firehose the diag sink);
//   - install is idempotent (dev HMR re-runs the entry module, and a
//     double wrap would double every record).
//
// The module is a singleton, so `beforeEach` resets it through the
// test seam (`_resetForTest`, same shape as the catalogs' Resource
// reset) and each test installs against its own console spies. The
// hooks mutate the real `console` of the test realm; originals are
// restored after each test because the suite's own reporters use the
// same console.
//
// The `window` listeners (`error` / `unhandledrejection`) are the
// untested half: vitest runs in a node environment (vite.config.ts),
// and the repo convention is that window-dependent surface stays
// untested until a DOM env is deliberately added.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { _resetForTest, installWebviewDiag } from "./diag";

vi.mock("./api", () => ({ api: vi.fn() }));

const apiMock = vi.mocked(api);

const nativeError = console.error;
const nativeWarn = console.warn;

beforeEach(() => {
  vi.useFakeTimers();
  apiMock.mockReset();
  apiMock.mockResolvedValue(undefined);
  _resetForTest();
});

afterEach(() => {
  console.error = nativeError;
  console.warn = nativeWarn;
  vi.useRealTimers();
});

describe("installWebviewDiag", () => {
  it("forwards console.error to record_diag and still calls the native console", () => {
    const spy = vi.fn();
    console.error = spy;
    installWebviewDiag();

    console.error("boom", { code: 500 });

    expect(spy).toHaveBeenCalledWith("boom", { code: 500 });
    expect(apiMock).toHaveBeenCalledTimes(1);
    expect(apiMock).toHaveBeenCalledWith("record_diag", {
      command: {
        level: "error",
        event: "webview.console_error",
        message: 'boom {"code":500}',
        attrs_json: null,
      },
    });
  });

  it("forwards console.warn at warn level", () => {
    installWebviewDiag();

    console.warn("odd");

    expect(apiMock).toHaveBeenCalledWith("record_diag", {
      command: {
        level: "warn",
        event: "webview.console_warn",
        message: "odd",
        attrs_json: null,
      },
    });
  });

  it("throttles identical messages inside the window, forwards again after it", () => {
    installWebviewDiag();

    console.error("boom");
    console.error("boom");
    console.error("boom");
    expect(apiMock).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(6_000);
    console.error("boom");
    expect(apiMock).toHaveBeenCalledTimes(2);
  });

  it("keeps distinct messages independent of each other's throttle", () => {
    installWebviewDiag();

    console.error("boom-a");
    console.error("boom-b");

    expect(apiMock).toHaveBeenCalledTimes(2);
  });

  it("swallows a rejected invoke without touching the console", () => {
    apiMock.mockRejectedValue(new Error("no backend"));
    const spy = vi.fn();
    console.error = spy;
    installWebviewDiag();

    console.error("boom");

    // The native call happened once (the user's own message) and the
    // rejection did not synthesize a second one — no recursion.
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it("is idempotent: a second install does not double-forward", () => {
    installWebviewDiag();
    installWebviewDiag();

    console.error("boom");

    expect(apiMock).toHaveBeenCalledTimes(1);
  });
});
