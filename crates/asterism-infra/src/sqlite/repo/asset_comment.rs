//! SQLite adapter for the `AssetCommentRepository` port.

use asterism_core::domain::asset_comment::{AssetComment, CommentAuthor};
use asterism_core::domain::repository::AssetCommentRepository;
use asterism_core::domain::value::{AssetCommentId, AssetId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime};

/// SQLite adapter for `AssetCommentRepository`.
#[derive(Clone)]
pub struct SqliteAssetCommentRepository {
    isle: AsyncIsle,
}

impl SqliteAssetCommentRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

struct CommentRow {
    id: Uuid,
    asset_id: Uuid,
    author_kind: String,
    author_persona_id: Option<Uuid>,
    body: String,
    created_at: i64,
    edited_at: Option<i64>,
}

impl CommentRow {
    const COLUMNS: &'static str =
        "id, asset_id, author_kind, author_persona_id, body, created_at, edited_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            asset_id: row.get(1)?,
            author_kind: row.get(2)?,
            author_persona_id: row.get(3)?,
            body: row.get(4)?,
            created_at: row.get(5)?,
            edited_at: row.get(6)?,
        })
    }

    fn into_domain(self) -> Result<AssetComment, DomainError> {
        let author = match self.author_kind.as_str() {
            "user" => CommentAuthor::User,
            // A NULL id here is not a broken row. It is what
            // `ON DELETE SET NULL` leaves behind when the Persona is
            // purged (schema V68), and reading it back as
            // `DeletedPersona` is what keeps the body — refusing it
            // would lose the comment to the death of its author, which
            // is the outcome V68 exists to avoid.
            "persona" => match self.author_persona_id {
                Some(pid) => CommentAuthor::Persona {
                    persona_id: PersonaId::from_uuid(pid),
                },
                None => CommentAuthor::DeletedPersona,
            },
            other => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "unknown author_kind: {other:?}"
                )));
            }
        };
        Ok(AssetComment {
            id: AssetCommentId::from_uuid(self.id),
            asset_id: AssetId::from_uuid(self.asset_id),
            author,
            body: self.body,
            created_at: ms_to_datetime(self.created_at)?,
            edited_at: self.edited_at.map(ms_to_datetime).transpose()?,
        })
    }
}

#[async_trait]
impl AssetCommentRepository for SqliteAssetCommentRepository {
    async fn save(&self, comment: &AssetComment) -> Result<(), DomainError> {
        let id = *comment.id.as_uuid();
        let asset_id = *comment.asset_id.as_uuid();
        let author_kind = comment.author.kind_slug().to_string();
        let author_persona_id = comment.author.persona_id().map(|p| *p.as_uuid());
        let body = comment.body.clone();
        let created = datetime_to_ms(&comment.created_at);
        let edited = comment.edited_at.as_ref().map(datetime_to_ms);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO asset_comment
                         (id, asset_id, author_kind, author_persona_id, body,
                          created_at, edited_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                         body = excluded.body,
                         edited_at = excluded.edited_at",
                    params![
                        id,
                        asset_id,
                        author_kind,
                        author_persona_id,
                        body,
                        created,
                        edited
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn list_by_asset(&self, asset_id: &AssetId) -> Result<Vec<AssetComment>, DomainError> {
        let aid = *asset_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM asset_comment
                        WHERE asset_id = ?1
                        ORDER BY created_at, id",
                    CommentRow::COLUMNS
                ))?;
                let rows: Vec<CommentRow> = stmt
                    .query_map(params![aid], CommentRow::from_row)?
                    .collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(CommentRow::into_domain).collect()
    }

    async fn find(&self, id: &AssetCommentId) -> Result<Option<AssetComment>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM asset_comment WHERE id = ?1",
                        CommentRow::COLUMNS
                    ),
                    params![uuid],
                    CommentRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(CommentRow::into_domain).transpose()
    }

    async fn delete(&self, id: &AssetCommentId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM asset_comment WHERE id = ?1", params![uuid])?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_core::domain::repository::PersonaRepository;
    use chrono::DateTime;

    /// Seeds one persona. `pack_id` is UNIQUE, so it is derived from
    /// the id — a test needing two personas calls this twice.
    async fn seed_persona(isle: &AsyncIsle) -> PersonaId {
        let pid = Uuid::now_v7();
        let pack = format!("pack-{pid}");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, 'P', 0, 0)",
                params![pid, pack],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        PersonaId::from_uuid(pid)
    }

    /// Seeds one asset under `persona`.
    async fn seed_asset(isle: &AsyncIsle, persona: PersonaId) -> AssetId {
        let aid = Uuid::now_v7();
        let owner = *persona.as_uuid();
        let locator = format!("a-{aid}.md");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'dialogue', 0, 0, 0)",
                params![aid, owner, locator],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        AssetId::from_uuid(aid)
    }

    fn comment(asset: AssetId, author: CommentAuthor, body: &str) -> AssetComment {
        // A fixed millisecond rather than `Utc::now()`: the column
        // stores epoch milliseconds, so a clock read carrying
        // microseconds would not survive the round trip.
        let at = DateTime::from_timestamp_millis(1_700_000_000_000).unwrap();
        AssetComment::new(asset, author, body, at).unwrap()
    }

    /// The purge path end to end, through the adapter rather than
    /// through raw SQL: the author goes, the comment stays, and the
    /// row that comes back reads as `DeletedPersona` instead of
    /// failing as a broken row.
    ///
    /// Before V68 this could not be written — the `DELETE` aborted, so
    /// there was no orphaned row for `into_domain` to be asked about.
    #[tokio::test]
    async fn a_purged_author_leaves_a_comment_that_still_reads() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetCommentRepository::new(isle.clone());
        let personas = crate::sqlite::repo::SqlitePersonaRepository::new(isle.clone());

        // The asset belongs to someone else, so the author FK is the
        // only path from the author to the comment.
        let owner = seed_persona(&isle).await;
        let author = seed_persona(&isle).await;
        let asset = seed_asset(&isle, owner).await;

        let posted = comment(
            asset,
            CommentAuthor::Persona { persona_id: author },
            "worth keeping",
        );
        repo.save(&posted).await.unwrap();

        // `purge` refuses a live persona; the trash flag is the gate.
        isle.call({
            let pid = *author.as_uuid();
            move |conn| {
                conn.execute(
                    "UPDATE persona SET trashed_at = 1 WHERE id = ?1",
                    params![pid],
                )?;
                Ok(())
            }
        })
        .await
        .unwrap();
        personas.purge(&author).await.unwrap();

        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(listed.len(), 1, "the comment outlives its author");
        assert_eq!(listed[0].body, "worth keeping");
        assert_eq!(
            listed[0].author,
            CommentAuthor::DeletedPersona,
            "a persona row with no id is the purged author, not a broken row"
        );
        assert_eq!(listed[0].author.kind_slug(), "persona");
        assert_eq!(listed[0].author.persona_id(), None);

        driver.shutdown().await.unwrap();
    }

    /// An orphaned comment is still editable, which means `save` has to
    /// be able to write the shape `into_domain` reads. `persona_id()`
    /// answering `None` puts a NULL beside `author_kind = 'persona'` —
    /// the exact pair V15's CHECK rejected.
    #[tokio::test]
    async fn an_orphaned_comment_can_still_be_saved() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteAssetCommentRepository::new(isle.clone());
        let owner = seed_persona(&isle).await;
        let asset = seed_asset(&isle, owner).await;

        let mut orphan = comment(asset, CommentAuthor::DeletedPersona, "written by nobody");
        repo.save(&orphan).await.unwrap();

        orphan.body = "edited afterwards".into();
        repo.save(&orphan).await.unwrap();

        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(listed, vec![orphan]);

        driver.shutdown().await.unwrap();
    }
}
