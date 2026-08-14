# asterism-infra::sqlite::repo::series

SQLite adapter for the `SeriesRepository` port — the series axis's
two tables (`series_strategy` / `material_series`, V73).

Follows the crate-wide adapter convention: only `rusqlite` primitives
inside the isle closure, promotion into domain types outside it
([`StrategyRow::into_domain`]).

The one thing worth stating beyond that convention is where the path
lists are taken apart. `include` / `exclude` are JSON columns (the
V73 doc comment argues why they are not a side table), so the
`serde_json` call is the promotion step and a column this build
cannot parse is [`DomainError::Infra`] — a rule read as though it
selected nothing would derive keys nobody wrote.

## Types

- `SqliteSeriesRepository` — SQLite adapter for `SeriesRepository` (uses a writer isle).

