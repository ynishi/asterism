# asterism-infra::dispatch

Outbound-dispatch runtime — `ExporterRegistry` plus the apalis
`DispatchRun` handler.

The registry is a simple `HashMap<slug, Arc<dyn Exporter>>` shared
through `JobDeps`. Server / Tauri boot picks the concrete exporters
and registers them here; the runner looks them up by slug on every
poll cycle.

`DispatchRun` drives one dispatch through `dispatch → poll →
harvest → reify`. Re-enqueue between polls is done by pushing a new
`DispatchRun` job with the same payload (apalis 0.7 does not
expose a native delayed-retry hook) — combined with the
terminal-state guard on the domain side, the loop is safe against
duplicated worker picks.

