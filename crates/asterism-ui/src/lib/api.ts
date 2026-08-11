// api — single choke point over Tauri `invoke`.
// All catalogs / components call `api<T>()` instead of importing
// `invoke` directly. Thin and transparent on purpose: error
// normalization is the Resource primitive's job, not this layer's.
//
// Why it exists (even as a pass-through):
//   (a) one mock point for vitest,
//   (b) one place to add logging / retry / timing later,
//   (c) `grep "@tauri-apps/api/core"` mechanically finds files
//       that haven't migrated yet.
//
// Migration policy: files switch to `api()` when they are touched
// by a hardening wave (H1-H3) — no dedicated sweep wave.
import { invoke } from "@tauri-apps/api/core";

export async function api<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return invoke<T>(cmd, args);
}
