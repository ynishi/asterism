# asterism-contract::query

Query DTOs — inputs for read-side operations.

`viewer_subject` is the raw input for visibility enforcement:
`None` means the owner view (everything visible), and `Some(subject)`
means a restricted subject view (persona view, and so on).

## Types

- `DiagLevel` — Severity of a persisted diagnostic — the closed set `tracing` can
- `GetAssetDetailQuery` — Detail view (asset + tags + constellation edges).
- `GetJobStatusQuery` — Job status lookup (for progress polling; push updates travel over the
- `ListAssetsQuery` — Filter and pagination parameters for the asset grid.
- `ListDiagQuery` — Diagnostic listing — newest first, the read side of `diag_log`.
- `ListEventsQuery` — Telemetry event listing — newest first. All filters are optional so
- `ListJobLogQuery` — Job-run listing — newest first, the read side of `job_log`.
- `ListObservationsQuery` — Cross-stream listing — newest first, over the `observation` view.
- `ListPerfQuery` — Timing listing — newest first, the read side of `perf_log`.
- `RandomAssetsQuery` — A random handful out of the set a filter describes — the "🎲 Random"
- `SearchAssetsQuery` — Full-text / fuzzy search. Shares the same filter and pagination shape
- `TagMatch` — How the entries of [`ListAssetsQuery::tag_ids`] combine.

