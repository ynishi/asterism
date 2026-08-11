import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { installWebviewDiag } from "./lib/diag";

// Before the mount, so an exception during first render is already
// captured — the blank-window case is the one that most needs a
// record on the backend.
installWebviewDiag();

// Frontend half of the WebDriver surface, loaded only for `just ui-e2e`
// (which sets `VITE_WDIO=1`). It is what lets a spec reach
// `browser.tauri.execute()` and mock an `invoke` — the Rust plugins
// alone drive the DOM but cannot see the IPC. A dynamic import behind
// the flag keeps it out of every other bundle, matching the `wdio`
// cargo feature on the Rust side: neither half exists in a shipped app.
// Not `await`ed: the build target predates top-level await, and the
// plugin installs itself on `window` — nothing below reads it, so
// racing the mount is fine.
if (import.meta.env.VITE_WDIO === "1") {
  void import("@wdio/tauri-plugin");
}

const app = mount(App, {
  target: document.getElementById("app")!,
});

export default app;
