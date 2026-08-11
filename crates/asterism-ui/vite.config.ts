/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [svelte()],

  // Vitest. The svelte plugin above compiles the
  // runes in `.svelte.ts` store modules, so tests import catalogs /
  // Resource directly.
  //
  // Node is still the default environment, and almost every suite wants
  // it: the surface those tests cover (Resource, encodeToSearch, catalog
  // deriveds) is DOM-free by design, and a document costs setup time per
  // file for nothing.
  //
  // A file that needs a document opts in for itself with a
  // `@vitest-environment happy-dom` docblock — added deliberately in
  // 2026-08-05 for the merge dialog, where the thing worth checking is
  // the wiring between a component and a store and there is no
  // DOM-free way to press a checkbox. Opting in per file rather than
  // switching the default keeps that cost on the handful of suites
  // that asked for it, and keeps "does this need a document?" an
  // answer written at the top of each file.
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },

  // Svelte ships separate server and client builds. Under Vitest the
  // client one is the one under test — without this, a component
  // rendered into jsdom gets the SSR build and never mounts. Guarded on
  // `VITEST` so the app's own build resolves exactly as it did before.
  // @ts-expect-error process is a nodejs global
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`.
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
