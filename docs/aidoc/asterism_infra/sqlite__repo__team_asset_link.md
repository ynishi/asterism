# asterism-infra::sqlite::repo::team_asset_link

SQLite adapter for the `AssetLinkRepository` port — what a
promotion left at home (#148 decisions 8 and 9).

The table carries no foreign key, so the check the schema declines
to enforce is written here instead: [`dangling_locally`] is an
anti-join against `asset` — a `NOT EXISTS` looking for the rows a
delete left behind — and nothing else in this file reaches outside
`team_asset_link`. The V104 migration argues why the key is absent;
this module is the other half of that argument, the part that goes
looking.

[`dangling_locally`]: SqliteAssetLinkRepository::dangling_locally

## Types

- `SqliteAssetLinkRepository` — SQLite adapter for `AssetLinkRepository`.

