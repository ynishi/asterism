//! SQLite adapter for the `ImportStateRepository` port.
//!
//! Two statements over one table, and the interest is in what is absent
//! from them. Nothing reads inside `offset_json` — no `json_extract`, no
//! comparison, no ordering by anything in it. The column goes in as text
//! and comes out as text; why that rule exists is
//! [`import_state`](asterism_core::domain::import_state).
//!
//! The one row a key can have is the primary key's doing, so `upsert`
//! is an `ON CONFLICT` over the whole key rather than a read followed by
//! a write: two runs of the same importer finishing at once would
//! otherwise both read "nothing", both insert, and one would lose to a
//! constraint the other had not seen.
//!
//! A timestamp this build cannot represent is *not* treated the way
//! `app_setting` treats one. There, an uninterpretable row reads as
//! "not overridden" and the settings screen still renders; here the
//! equivalent would be a resumption point silently reading as absent,
//! and an importer told a source it has read is a source it has not
//! would re-import the whole thing. So the read fails and says so —
//! `updated_at` is a stamp for a person, and losing a position over one
//! is the trade that runs the wrong way.

use asterism_core::domain::import_state::{ImportState, ImportStateKey};
use asterism_core::domain::repository::ImportStateRepository;
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `ImportStateRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteImportStateRepository {
    isle: AsyncIsle,
}

impl SqliteImportStateRepository {
    /// Wraps a writer `AsyncIsle` handle (same discipline as the sibling
    /// adapters).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Row shape scanned inside the isle closure.
struct StateRow {
    persona_id: String,
    partition: String,
    offset_json: String,
    updated_at: i64,
}

impl StateRow {
    const COLUMNS: &'static str = "persona_id, partition, offset_json, updated_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            persona_id: row.get(0)?,
            partition: row.get(1)?,
            offset_json: row.get(2)?,
            updated_at: row.get(3)?,
        })
    }

    fn into_domain(self) -> Result<ImportState, DomainError> {
        Ok(ImportState {
            key: ImportStateKey::new(self.persona_id, self.partition),
            offset_json: self.offset_json,
            updated_at: ms_to_datetime(self.updated_at)?,
        })
    }
}

#[async_trait]
impl ImportStateRepository for SqliteImportStateRepository {
    async fn find(&self, key: &ImportStateKey) -> Result<Option<ImportState>, DomainError> {
        let persona_id = key.persona_id.clone();
        let partition = key.partition.clone();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM import_state \
                         WHERE persona_id = ?1 AND partition = ?2",
                        StateRow::COLUMNS
                    ),
                    params![persona_id, partition],
                    StateRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    // No row is the first-run state, not a failure.
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(StateRow::into_domain).transpose()
    }

    async fn upsert(&self, state: &ImportState) -> Result<(), DomainError> {
        let persona_id = state.key.persona_id.clone();
        let partition = state.key.partition.clone();
        let offset_json = state.offset_json.clone();
        let updated = datetime_to_ms(&state.updated_at);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO import_state (persona_id, partition, offset_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(persona_id, partition) DO UPDATE SET
                         offset_json = excluded.offset_json,
                         updated_at = excluded.updated_at",
                    params![persona_id, partition, offset_json, updated],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}
