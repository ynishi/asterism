//! SQLite adapter for the `ChapterMarkRepository` port.

use asterism_core::domain::chapter_mark::ChapterMark;
use asterism_core::domain::material_mark::TimelineSpan;
use asterism_core::domain::repository::ChapterMarkRepository;
use asterism_core::domain::value::{ChapterMarkId, MaterialLayerId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{infra_err, opt_u64};

/// SQLite adapter for `ChapterMarkRepository`.
#[derive(Clone)]
pub struct SqliteChapterMarkRepository {
    isle: AsyncIsle,
}

impl SqliteChapterMarkRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Narrows a domain millisecond value to the signed range the column
/// stores.
///
/// `TimelineSpan::new` has already refused anything out of range, so
/// this cannot fire — written as a conversion rather than `as i64` for
/// the reason the mark adapter gives: a silent wrap would land as a
/// `CHECK (start_ms >= 0)` abort, reporting an out-of-range value under
/// the name of a different rule.
fn to_stored_ms(value: u64, column: &str) -> Result<i64, DomainError> {
    i64::try_from(value).map_err(|_| {
        DomainError::Infra(anyhow::anyhow!(
            "{column} = {value} is past the signed 64-bit range the column stores"
        ))
    })
}

/// The row's own columns, in the order the statements below bind them.
struct ChapterRow {
    id: Uuid,
    layer_id: Uuid,
    start_ms: i64,
    end_ms: Option<i64>,
    label: String,
    ord: i64,
}

impl ChapterRow {
    const COLUMNS: &'static str = "id, layer_id, start_ms, end_ms, label, ord";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            layer_id: row.get(1)?,
            start_ms: row.get(2)?,
            end_ms: row.get(3)?,
            label: row.get(4)?,
            ord: row.get(5)?,
        })
    }

    /// Promotes a row into the domain **through the constructor**.
    ///
    /// The span is the part that needs it: `start_ms` and `end_ms` are
    /// two independent columns in the row and one value object in the
    /// domain, and `TimelineSpan::new` is what refuses the pairings the
    /// schema's `CHECK (end_ms IS NULL OR end_ms > start_ms)` would
    /// admit only because it was written around.
    fn into_domain(self) -> Result<ChapterMark, DomainError> {
        let id = self.id;
        let corrupt = |e: DomainError| {
            DomainError::Infra(anyhow::anyhow!(
                "chapter_mark {id} holds a row the domain refuses: {e}"
            ))
        };
        let start_ms = opt_u64(Some(self.start_ms), "start_ms")?.ok_or_else(|| {
            DomainError::Infra(anyhow::anyhow!("chapter_mark {id} has no start_ms"))
        })?;
        let end_ms = opt_u64(self.end_ms, "end_ms")?;
        let span = TimelineSpan::new(start_ms, end_ms).map_err(corrupt)?;
        let ord = u32::try_from(self.ord).map_err(|_| {
            DomainError::Infra(anyhow::anyhow!(
                "chapter_mark {id} holds ord = {} , which is outside the range an ordinal takes",
                self.ord
            ))
        })?;
        ChapterMark::rehydrate(
            ChapterMarkId::from_uuid(id),
            MaterialLayerId::from_uuid(self.layer_id),
            span,
            self.label,
            ord,
        )
        .map_err(corrupt)
    }
}

/// The bound values one chapter row is written from.
///
/// Extracted because `save` and `replace_layer_content` write the same
/// row from the same rules, and the second does it inside a
/// transaction where the domain check has to have happened *before* the
/// first `DELETE` — a refusal discovered halfway through a replacement
/// would have already emptied the band.
struct StoredChapter {
    id: Uuid,
    layer_id: Uuid,
    start_ms: i64,
    end_ms: Option<i64>,
    label: String,
    ord: i64,
}

impl StoredChapter {
    fn encode(chapter: &ChapterMark) -> Result<Self, DomainError> {
        chapter.validate()?;
        Ok(Self {
            id: *chapter.id.as_uuid(),
            layer_id: *chapter.layer_id.as_uuid(),
            start_ms: to_stored_ms(chapter.span.start_ms(), "start_ms")?,
            end_ms: chapter
                .span
                .end_ms()
                .map(|v| to_stored_ms(v, "end_ms"))
                .transpose()?,
            label: chapter.label.clone(),
            ord: i64::from(chapter.ord),
        })
    }

    const INSERT: &'static str = "INSERT INTO chapter_mark
             (id, layer_id, start_ms, end_ms, label, ord)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
             layer_id = excluded.layer_id,
             start_ms = excluded.start_ms,
             end_ms = excluded.end_ms,
             label = excluded.label,
             ord = excluded.ord";
}

#[async_trait]
impl ChapterMarkRepository for SqliteChapterMarkRepository {
    async fn save(&self, chapter: &ChapterMark) -> Result<(), DomainError> {
        let row = StoredChapter::encode(chapter)?;
        self.isle
            .call(move |conn| {
                conn.execute(
                    StoredChapter::INSERT,
                    params![
                        row.id,
                        row.layer_id,
                        row.start_ms,
                        row.end_ms,
                        row.label,
                        row.ord
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn list_by_layer(
        &self,
        layer_id: &MaterialLayerId,
    ) -> Result<Vec<ChapterMark>, DomainError> {
        let lid = *layer_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM chapter_mark
                        WHERE layer_id = ?1
                        ORDER BY ord, start_ms, id",
                    ChapterRow::COLUMNS
                ))?;
                let rows: Vec<ChapterRow> = stmt
                    .query_map(params![lid], ChapterRow::from_row)?
                    .collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(ChapterRow::into_domain).collect()
    }

    async fn delete(&self, id: &ChapterMarkId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM chapter_mark WHERE id = ?1", params![uuid])?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    /// Empties the band and refills it, in one transaction.
    ///
    /// The delete and the inserts are one unit because the alternative
    /// is a window in which the file's chapter list is empty — visible
    /// to any concurrent read, and permanent if the process stops
    /// mid-way, with nothing left to re-derive it from until the next
    /// probe.
    ///
    /// Everything that can be refused is refused **before** the
    /// transaction opens: `encode` runs the domain check and the range
    /// conversions over the whole slice, and the layer-membership check
    /// below is a comparison of values already in hand. A refusal
    /// discovered after the `DELETE` would have already destroyed the
    /// contents it was protecting.
    async fn replace_layer_content(
        &self,
        layer_id: &MaterialLayerId,
        chapters: &[ChapterMark],
    ) -> Result<(), DomainError> {
        let lid = *layer_id.as_uuid();
        let rows: Vec<StoredChapter> = chapters
            .iter()
            .map(StoredChapter::encode)
            .collect::<Result<_, _>>()?;
        // A chapter naming another band is two disagreeing statements
        // about where the rows go, not an instruction to move it: the
        // caller has said "replace this band's content" and "this row
        // lives elsewhere" in one argument.
        if let Some(stray) = rows.iter().find(|r| r.layer_id != lid) {
            return Err(DomainError::Validation(format!(
                "chapter {} belongs to layer {}, not to the layer being replaced ({})",
                stray.id, stray.layer_id, lid
            )));
        }
        self.isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                tx.execute("DELETE FROM chapter_mark WHERE layer_id = ?1", params![lid])?;
                {
                    let mut stmt = tx.prepare(StoredChapter::INSERT)?;
                    for row in &rows {
                        stmt.execute(params![
                            row.id,
                            row.layer_id,
                            row.start_ms,
                            row.end_ms,
                            row.label,
                            row.ord
                        ])?;
                    }
                }
                tx.commit()?;
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
    use crate::sqlite::repo::SqliteMaterialLayerRepository;
    use asterism_core::domain::material_layer::{LayerOrigin, LayerRole, MaterialLayer};
    use asterism_core::domain::repository::MaterialLayerRepository;
    use asterism_core::domain::value::AssetId;

    /// Seeds a persona, an asset, and one imported structure band over
    /// it — the shape every fixture below needs.
    async fn seed_layer(isle: &AsyncIsle) -> (AssetId, MaterialLayerId) {
        let pid = Uuid::now_v7();
        let pack = format!("pack-{pid}");
        let aid = Uuid::now_v7();
        let locator = format!("v-{aid}.mkv");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, pack_id, name, created_at, updated_at) \
                 VALUES (?1, ?2, 'P', 0, 0)",
                params![pid, pack],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'video', 0, 0, 0)",
                params![aid, pid, locator],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let asset = AssetId::from_uuid(aid);
        let layers = SqliteMaterialLayerRepository::new(isle.clone());
        let layer = MaterialLayer::new(
            asset,
            0,
            LayerOrigin::Imported,
            LayerRole::Structure,
            true,
            0,
        )
        .unwrap();
        layers.save(&layer).await.unwrap();
        (asset, layer.id)
    }

    /// A chapter with an id chosen by hand, for the same
    /// memcmp-versus-`Ord` reason the mark adapter's fixture gives.
    fn chapter_with(
        first_byte: u8,
        layer: MaterialLayerId,
        start_ms: u64,
        end_ms: Option<u64>,
        label: &str,
        ord: u32,
    ) -> ChapterMark {
        let span = TimelineSpan::new(start_ms, end_ms).unwrap();
        let mut chapter = ChapterMark::new(layer, span, label, ord).unwrap();
        chapter.id = ChapterMarkId::from_uuid(Uuid::from_bytes([first_byte; 16]));
        chapter
    }

    /// A chapter round-trips, an untitled one survives the round trip
    /// as untitled, and `delete` removes exactly one.
    ///
    /// The empty label is the load-bearing case: it is the value the
    /// sibling aggregate refuses, and a `NOT NULL` column plus an
    /// over-eager decoder could easily turn it into a `NULL` read
    /// failure instead of an empty string.
    #[tokio::test]
    async fn save_lists_and_deletes_a_chapter() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let (_asset, layer) = seed_layer(&isle).await;

        let opening = chapter_with(0x11, layer, 0, Some(30_000), "", 0);
        let second = chapter_with(0x22, layer, 30_000, None, "Two", 1);
        repo.save(&opening).await.unwrap();
        repo.save(&second).await.unwrap();

        let listed = repo.list_by_layer(&layer).await.unwrap();
        assert_eq!(listed, vec![opening.clone(), second.clone()]);
        assert_eq!(listed[0].label, "", "an untitled section stays untitled");
        assert!(
            listed[1].span.is_instant(),
            "a start-only section stays one"
        );

        // Same id, edited label: the upsert path, not a second row.
        let mut retitled = second.clone();
        retitled.label = "Chapter Two".into();
        repo.save(&retitled).await.unwrap();
        let listed = repo.list_by_layer(&layer).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].label, "Chapter Two");

        repo.delete(&opening.id).await.unwrap();
        assert_eq!(repo.list_by_layer(&layer).await.unwrap(), vec![retitled]);
        // Idempotent.
        repo.delete(&opening.id).await.unwrap();

        driver.shutdown().await.unwrap();
    }

    /// `list_by_layer` reads in the container's declared order (`ord`),
    /// not in timeline order and not in arrival order.
    ///
    /// The fixture puts all three axes in disagreement: the rows go in
    /// at ascending `start_ms` while their `ord` descends, so both the
    /// rowid scan and a `start_ms` sort would give the reverse of the
    /// expected answer.
    #[tokio::test]
    async fn list_by_layer_reads_in_the_declared_order() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let (_asset, layer) = seed_layer(&isle).await;

        repo.save(&chapter_with(0x11, layer, 1_000, None, "last", 2))
            .await
            .unwrap();
        repo.save(&chapter_with(0x22, layer, 2_000, None, "middle", 1))
            .await
            .unwrap();
        repo.save(&chapter_with(0x33, layer, 3_000, None, "first", 0))
            .await
            .unwrap();

        let labels: Vec<String> = repo
            .list_by_layer(&layer)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["first", "middle", "last"]);

        driver.shutdown().await.unwrap();
    }

    /// Chapters sharing an `ord` fall back to the timeline, then to the
    /// id.
    ///
    /// Separate from the test above because that one never reaches the
    /// tie-break: with three distinct `ord` values the trailing terms
    /// can be deleted and it still passes. A container that leaves
    /// every `ord` at zero is the case this pins, and it is not
    /// hypothetical — it is what a writer that only had start times
    /// produces.
    #[tokio::test]
    async fn chapters_sharing_an_ord_fall_back_to_the_timeline() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let (_asset, layer) = seed_layer(&isle).await;

        // Arrival order is the reverse of the expected order.
        repo.save(&chapter_with(0x11, layer, 9_000, None, "late", 0))
            .await
            .unwrap();
        repo.save(&chapter_with(0x22, layer, 1_000, None, "early", 0))
            .await
            .unwrap();

        let labels: Vec<String> = repo
            .list_by_layer(&layer)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, vec!["early", "late"]);

        driver.shutdown().await.unwrap();
    }

    /// `replace_layer_content` makes the band exactly what it is
    /// handed — including nothing at all — and leaves other bands
    /// alone.
    ///
    /// The second band is what makes the assertion mean anything: a
    /// `DELETE FROM chapter_mark` with no `WHERE` would pass every
    /// other check in this file.
    #[tokio::test]
    async fn replace_layer_content_is_wholesale_and_scoped() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let layers = SqliteMaterialLayerRepository::new(isle.clone());
        let (asset, imported) = seed_layer(&isle).await;

        let mine = MaterialLayer::new(asset, 0, LayerOrigin::User, LayerRole::Structure, false, 1)
            .unwrap();
        layers.save(&mine).await.unwrap();
        let of_mine = chapter_with(0x99, mine.id, 500, None, "my own", 0);
        repo.save(&of_mine).await.unwrap();

        repo.save(&chapter_with(0x11, imported, 0, Some(10), "stale", 0))
            .await
            .unwrap();

        let fresh = vec![
            chapter_with(0x21, imported, 0, Some(60_000), "One", 0),
            chapter_with(0x22, imported, 60_000, None, "Two", 1),
        ];
        repo.replace_layer_content(&imported, &fresh).await.unwrap();
        assert_eq!(repo.list_by_layer(&imported).await.unwrap(), fresh);
        assert_eq!(
            repo.list_by_layer(&mine.id).await.unwrap(),
            vec![of_mine.clone()],
            "another band's chapters are not this band's to replace"
        );

        // An empty reading is a reading: a file that used to declare
        // chapters and no longer does ends with an empty band.
        repo.replace_layer_content(&imported, &[]).await.unwrap();
        assert!(repo.list_by_layer(&imported).await.unwrap().is_empty());
        assert_eq!(repo.list_by_layer(&mine.id).await.unwrap(), vec![of_mine]);

        driver.shutdown().await.unwrap();
    }

    /// A chapter naming another band is refused, and the band it was
    /// aimed at still holds what it held.
    ///
    /// The surviving contents are the half worth asserting: the
    /// membership check runs before the transaction opens precisely so
    /// that a refusal cannot leave the band emptied.
    #[tokio::test]
    async fn replace_layer_content_refuses_a_chapter_from_another_band() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let (_asset, layer) = seed_layer(&isle).await;

        let held = chapter_with(0x11, layer, 0, Some(10), "held", 0);
        repo.save(&held).await.unwrap();

        let stray = chapter_with(0x22, MaterialLayerId::new(), 0, None, "elsewhere", 0);
        let err = repo
            .replace_layer_content(&layer, &[stray])
            .await
            .expect_err("a chapter that names another band is not this band's content");
        assert!(
            matches!(err, DomainError::Validation(_)),
            "a caller handing over rows for two bands is a caller error, got {err:?}"
        );
        assert_eq!(
            repo.list_by_layer(&layer).await.unwrap(),
            vec![held],
            "the refused replacement must not have emptied the band"
        );

        driver.shutdown().await.unwrap();
    }

    /// Deleting the band takes its chapters with it.
    #[tokio::test]
    async fn deleting_the_band_sweeps_its_chapters() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let layers = SqliteMaterialLayerRepository::new(isle.clone());
        let (_asset, layer) = seed_layer(&isle).await;

        repo.save(&chapter_with(0x11, layer, 0, Some(10), "one", 0))
            .await
            .unwrap();
        layers.delete(&layer).await.unwrap();

        assert!(repo.list_by_layer(&layer).await.unwrap().is_empty());

        driver.shutdown().await.unwrap();
    }

    /// A stored span the domain refuses reads back as `Infra`.
    ///
    /// `end_ms == start_ms` is the case: the schema's
    /// `CHECK (end_ms IS NULL OR end_ms > start_ms)` refuses it, so the
    /// row is inserted with the checks suspended — what is under test
    /// is the decoder's behaviour on a row that arrived some other way,
    /// which is the only reason `into_domain` goes through
    /// `TimelineSpan::new` at all.
    #[tokio::test]
    async fn read_back_refuses_an_interval_covering_nothing() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteChapterMarkRepository::new(isle.clone());
        let (_asset, layer) = seed_layer(&isle).await;

        let lid = *layer.as_uuid();
        isle.call(move |conn| {
            conn.pragma_update(None, "ignore_check_constraints", true)?;
            conn.execute(
                "INSERT INTO chapter_mark (id, layer_id, start_ms, end_ms, label, ord)
                 VALUES (?1, ?2, 500, 500, 'nothing', 0)",
                params![Uuid::now_v7(), lid],
            )?;
            conn.pragma_update(None, "ignore_check_constraints", false)?;
            Ok(())
        })
        .await
        .unwrap();

        let err = repo
            .list_by_layer(&layer)
            .await
            .expect_err("a section covering nothing is not a section");
        assert!(
            matches!(err, DomainError::Infra(_)),
            "a row the constructor refuses is an infrastructure fact, got {err:?}"
        );

        driver.shutdown().await.unwrap();
    }
}
