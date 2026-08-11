//! SQLite adapter for the `PersonaThemeRepository` port.

use asterism_core::domain::persona_theme::PersonaTheme;
use asterism_core::domain::repository::PersonaThemeRepository;
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `PersonaThemeRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqlitePersonaThemeRepository {
    isle: AsyncIsle,
}

impl SqlitePersonaThemeRepository {
    /// Wraps a writer `AsyncIsle` handle (same discipline as the
    /// sibling adapters — writers only for now, WAL reader pool is a
    /// follow-up when contention shows up).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Row shape scanned inside the isle closure.
struct ThemeRow {
    persona_id: Uuid,
    wallpaper_asset_id: Option<Uuid>,
    updated_at: i64,
}

impl ThemeRow {
    const COLUMNS: &'static str = "persona_id, wallpaper_asset_id, updated_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            persona_id: row.get(0)?,
            wallpaper_asset_id: row.get(1)?,
            updated_at: row.get(2)?,
        })
    }

    fn into_domain(self) -> Result<PersonaTheme, DomainError> {
        Ok(PersonaTheme {
            persona_id: PersonaId::from_uuid(self.persona_id),
            wallpaper_asset_id: self.wallpaper_asset_id.map(AssetId::from_uuid),
            updated_at: ms_to_datetime(self.updated_at)?,
        })
    }
}

#[async_trait]
impl PersonaThemeRepository for SqlitePersonaThemeRepository {
    async fn get(&self, persona_id: &PersonaId) -> Result<Option<PersonaTheme>, DomainError> {
        let uuid = *persona_id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM persona_theme WHERE persona_id = ?1",
                        ThemeRow::COLUMNS
                    ),
                    params![uuid],
                    ThemeRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(ThemeRow::into_domain).transpose()
    }

    async fn upsert(&self, theme: &PersonaTheme) -> Result<(), DomainError> {
        let persona_id = *theme.persona_id.as_uuid();
        let wallpaper_asset_id = theme.wallpaper_asset_id.map(|a| *a.as_uuid());
        let updated = datetime_to_ms(&theme.updated_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO persona_theme (persona_id, wallpaper_asset_id, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(persona_id) DO UPDATE SET
                         wallpaper_asset_id = excluded.wallpaper_asset_id,
                         updated_at = excluded.updated_at",
                    params![persona_id, wallpaper_asset_id, updated],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn delete(&self, persona_id: &PersonaId) -> Result<(), DomainError> {
        let uuid = *persona_id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "DELETE FROM persona_theme WHERE persona_id = ?1",
                    params![uuid],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}
