# asterism-infra::sqlite::repo::modality

SQLite adapter for the `ModalityRepository` port — the Modality
master (`modality` table), backed by rusqlite-isle.

Follows the crate-wide adapter convention: only `rusqlite`
primitives inside the isle closure; promotion into domain types
(parsing `kind` → `ContentKind`, `cover_template` → `CoverTemplate`)
happens outside via [`ModalityRow::into_domain`].

## Types

- `SqliteModalityRepository` — SQLite adapter for `ModalityRepository` (uses a writer isle).

