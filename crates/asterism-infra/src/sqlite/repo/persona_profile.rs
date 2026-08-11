//! SQLite adapter for the `PersonaProfileRepository` port.

use asterism_core::domain::persona_profile::PersonaProfile;
use asterism_core::domain::repository::PersonaProfileRepository;
use asterism_core::domain::value::{AssetId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `PersonaProfileRepository` (writer isle).
#[derive(Clone)]
pub struct SqlitePersonaProfileRepository {
    isle: AsyncIsle,
}

impl SqlitePersonaProfileRepository {
    /// Wraps a writer `AsyncIsle` handle.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

struct ProfileRow {
    persona_id: Uuid,
    avatar_asset_id: Option<Uuid>,
    bio_short: Option<String>,
    role_tag: Option<String>,
    updated_at: i64,
}

impl ProfileRow {
    const COLUMNS: &'static str = "persona_id, avatar_asset_id, bio_short, role_tag, updated_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            persona_id: row.get(0)?,
            avatar_asset_id: row.get(1)?,
            bio_short: row.get(2)?,
            role_tag: row.get(3)?,
            updated_at: row.get(4)?,
        })
    }

    fn into_domain(self) -> Result<PersonaProfile, DomainError> {
        Ok(PersonaProfile {
            persona_id: PersonaId::from_uuid(self.persona_id),
            avatar_asset_id: self.avatar_asset_id.map(AssetId::from_uuid),
            bio_short: self.bio_short,
            role_tag: self.role_tag,
            updated_at: ms_to_datetime(self.updated_at)?,
        })
    }
}

#[async_trait]
impl PersonaProfileRepository for SqlitePersonaProfileRepository {
    async fn get(&self, persona_id: &PersonaId) -> Result<Option<PersonaProfile>, DomainError> {
        let uuid = *persona_id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM persona_profile WHERE persona_id = ?1",
                        ProfileRow::COLUMNS
                    ),
                    params![uuid],
                    ProfileRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(ProfileRow::into_domain).transpose()
    }

    async fn upsert(&self, profile: &PersonaProfile) -> Result<(), DomainError> {
        let persona_id = *profile.persona_id.as_uuid();
        let avatar_asset_id = profile.avatar_asset_id.map(|a| *a.as_uuid());
        let bio_short = profile.bio_short.clone();
        let role_tag = profile.role_tag.clone();
        let updated = datetime_to_ms(&profile.updated_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO persona_profile
                         (persona_id, avatar_asset_id, bio_short, role_tag, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(persona_id) DO UPDATE SET
                         avatar_asset_id = excluded.avatar_asset_id,
                         bio_short       = excluded.bio_short,
                         role_tag        = excluded.role_tag,
                         updated_at      = excluded.updated_at",
                    params![persona_id, avatar_asset_id, bio_short, role_tag, updated],
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
                    "DELETE FROM persona_profile WHERE persona_id = ?1",
                    params![uuid],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}
