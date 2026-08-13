//! SQLite adapter for the `MaterialMarkRepository` port.

use asterism_core::domain::asset_comment::CommentAuthor;
use asterism_core::domain::material_mark::{MaterialAnchor, MaterialMark, TimelineSpan};
use asterism_core::domain::repository::MaterialMarkRepository;
use asterism_core::domain::value::{AssetId, MaterialLayerId, MaterialMarkId, PersonaId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::{datetime_to_ms, infra_err, ms_to_datetime, opt_u64};

/// SQLite adapter for `MaterialMarkRepository`.
#[derive(Clone)]
pub struct SqliteMaterialMarkRepository {
    isle: AsyncIsle,
}

impl SqliteMaterialMarkRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Narrows a domain millisecond value to the signed range the column
/// stores.
///
/// `TimelineSpan::new` has already refused anything out of range, so
/// this cannot fire — which is the point of writing it as a conversion
/// rather than `as i64`. A silent wrap would land as a `CHECK
/// (start_ms >= 0)` abort or an `end_ms > start_ms` abort, reporting an
/// out-of-range value under the name of a different rule.
fn to_stored_ms(value: u64, column: &str) -> Result<i64, DomainError> {
    i64::try_from(value).map_err(|_| {
        DomainError::Infra(anyhow::anyhow!(
            "{column} = {value} is past the signed 64-bit range the column stores"
        ))
    })
}

struct MarkRow {
    id: Uuid,
    asset_id: Uuid,
    layer_id: Uuid,
    anchor_kind: String,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    body: String,
    author_kind: String,
    author_persona_id: Option<Uuid>,
    created_at: i64,
    edited_at: Option<i64>,
}

impl MarkRow {
    const COLUMNS: &'static str = "id, asset_id, layer_id, anchor_kind, start_ms, end_ms, body, \
         author_kind, author_persona_id, created_at, edited_at";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            asset_id: row.get(1)?,
            layer_id: row.get(2)?,
            anchor_kind: row.get(3)?,
            start_ms: row.get(4)?,
            end_ms: row.get(5)?,
            body: row.get(6)?,
            author_kind: row.get(7)?,
            author_persona_id: row.get(8)?,
            created_at: row.get(9)?,
            edited_at: row.get(10)?,
        })
    }

    /// Promotes a row into the domain **through the constructor**.
    ///
    /// Not a struct literal (which is what the sibling
    /// `asset_comment` adapter uses): two of this aggregate's rules —
    /// a non-empty body under a Unicode trim, and `edited_at` no
    /// earlier than `created_at` — are deliberately absent from the
    /// schema, so this call is the only thing standing between a row
    /// that the domain would refuse and a caller holding it as a valid
    /// `MaterialMark`.
    fn into_domain(self) -> Result<MaterialMark, DomainError> {
        let id = self.id;
        let author = match self.author_kind.as_str() {
            "user" => CommentAuthor::User,
            "persona" => {
                let pid = self.author_persona_id.ok_or_else(|| {
                    DomainError::Infra(anyhow::anyhow!(
                        "author_kind = persona but author_persona_id is NULL"
                    ))
                })?;
                CommentAuthor::Persona {
                    persona_id: PersonaId::from_uuid(pid),
                }
            }
            other => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "unknown author_kind: {other:?}"
                )));
            }
        };
        let corrupt = |e: DomainError| {
            DomainError::Infra(anyhow::anyhow!(
                "material_mark {id} holds a row the domain refuses: {e}"
            ))
        };
        // `anchor_kind` decides which coordinate columns carry the
        // position, so the read side is a match on it and not a set of
        // independent column reads. An unknown value is an
        // infrastructure fact — a row written by a newer schema, or by
        // hand — and not something to fall back from: falling back to
        // `'temporal'` would be inventing a position for a mark that
        // does not have one.
        let anchor = match self.anchor_kind.as_str() {
            "temporal" => {
                // `CHECK (anchor_kind <> 'temporal' OR start_ms IS NOT
                // NULL)` makes the NULL unreachable from a database
                // that has run V66; a row that predates it, or one
                // written around the CHECK, is what this arm names.
                let start_ms = opt_u64(self.start_ms, "start_ms")?.ok_or_else(|| {
                    DomainError::Infra(anyhow::anyhow!(
                        "material_mark {id} is anchored temporally but start_ms is NULL"
                    ))
                })?;
                let end_ms = opt_u64(self.end_ms, "end_ms")?;
                MaterialAnchor::Temporal(TimelineSpan::new(start_ms, end_ms).map_err(corrupt)?)
            }
            other => {
                return Err(DomainError::Infra(anyhow::anyhow!(
                    "unknown anchor_kind: {other:?}"
                )));
            }
        };
        MaterialMark::rehydrate(
            MaterialMarkId::from_uuid(id),
            AssetId::from_uuid(self.asset_id),
            MaterialLayerId::from_uuid(self.layer_id),
            anchor,
            author,
            self.body,
            ms_to_datetime(self.created_at)?,
            self.edited_at.map(ms_to_datetime).transpose()?,
        )
        .map_err(corrupt)
    }
}

#[async_trait]
impl MaterialMarkRepository for SqliteMaterialMarkRepository {
    /// Writes a mark, refusing one the read path would refuse.
    ///
    /// The check is the domain's own ([`MaterialMark::validate`]),
    /// not a second copy of the rules: `body` and `edited_at` are public
    /// fields, so a record update reaches them without passing a
    /// constructor, and neither rule is in the schema (see the V66 doc
    /// comment in `migrations.rs`). Without this call the write path
    /// would be the more permissive of the two, and `list_by_asset` —
    /// the only read verb this port has — collects into a single
    /// `Result`, so one such row would make the whole asset's timeline
    /// `Err` with no way left to name the row that did it.
    ///
    /// `Validation`, not `Infra`: nothing infrastructural failed, the
    /// caller handed over a value the domain forbids.
    async fn save(&self, mark: &MaterialMark) -> Result<(), DomainError> {
        mark.validate()?;
        let id = *mark.id.as_uuid();
        let asset_id = *mark.asset_id.as_uuid();
        let layer_id = *mark.layer_id.as_uuid();
        let anchor_kind = mark.anchor.kind_slug().to_string();
        // One arm per anchor kind, so a variant added to
        // `MaterialAnchor` fails to compile here rather than storing a
        // row with the wrong columns populated.
        let (start_ms, end_ms) = match &mark.anchor {
            MaterialAnchor::Temporal(span) => (
                Some(to_stored_ms(span.start_ms(), "start_ms")?),
                span.end_ms()
                    .map(|v| to_stored_ms(v, "end_ms"))
                    .transpose()?,
            ),
        };
        let body = mark.body.clone();
        let author_kind = mark.author.kind_slug().to_string();
        let author_persona_id = mark.author.persona_id().map(|p| *p.as_uuid());
        let created = datetime_to_ms(&mark.created_at);
        let edited = mark.edited_at.as_ref().map(datetime_to_ms);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO material_mark
                         (id, asset_id, layer_id, anchor_kind, start_ms, end_ms, body,
                          author_kind, author_persona_id, created_at, edited_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(id) DO UPDATE SET
                         layer_id = excluded.layer_id,
                         anchor_kind = excluded.anchor_kind,
                         start_ms = excluded.start_ms,
                         end_ms = excluded.end_ms,
                         body = excluded.body,
                         edited_at = excluded.edited_at",
                    params![
                        id,
                        asset_id,
                        layer_id,
                        anchor_kind,
                        start_ms,
                        end_ms,
                        body,
                        author_kind,
                        author_persona_id,
                        created,
                        edited
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn list_by_asset(&self, asset_id: &AssetId) -> Result<Vec<MaterialMark>, DomainError> {
        let aid = *asset_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM material_mark
                        WHERE asset_id = ?1
                        ORDER BY start_ms, id",
                    MarkRow::COLUMNS
                ))?;
                let rows: Vec<MarkRow> = stmt
                    .query_map(params![aid], MarkRow::from_row)?
                    .collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(MarkRow::into_domain).collect()
    }

    async fn delete(&self, id: &MaterialMarkId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM material_mark WHERE id = ?1", params![uuid])?;
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
    use asterism_core::domain::material_layer::{LayerOrigin, LayerRole, MaterialLayer};
    use asterism_core::domain::repository::{MaterialLayerRepository, PersonaRepository};
    use chrono::{DateTime, Utc};

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

    /// Seeds one video asset under `persona`, with the default
    /// annotation band its marks belong to.
    ///
    /// The two are returned together because a mark cannot exist
    /// without a band: `layer_id` is `NOT NULL`, so an asset with no
    /// layer is an asset nothing can be marked on.
    async fn seed_asset(isle: &AsyncIsle, persona: PersonaId) -> (AssetId, MaterialLayerId) {
        let aid = Uuid::now_v7();
        let owner = *persona.as_uuid();
        let locator = format!("v-{aid}.mp4");
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator,
                                    modality, occurred_at, created_at, updated_at)
                 VALUES (?1, ?2, 'fs', ?3, 'video', 0, 0, 0)",
                params![aid, owner, locator],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let asset = AssetId::from_uuid(aid);
        (asset, seed_layer(isle, asset).await)
    }

    /// The band every mark in these fixtures belongs to: the asset's
    /// default annotation layer, which is what the service would have
    /// created on the first post.
    ///
    /// Written over the layer adapter rather than over raw SQL so that
    /// the shape these fixtures assume is the shape the production path
    /// produces — a hand-written `INSERT` here would keep passing after
    /// the two disagreed.
    async fn seed_layer(isle: &AsyncIsle, asset: AssetId) -> MaterialLayerId {
        let layers = crate::sqlite::repo::SqliteMaterialLayerRepository::new(isle.clone());
        let layer = MaterialLayer::new(asset, 0, LayerOrigin::User, LayerRole::Annotation, true, 0)
            .unwrap();
        layers.save(&layer).await.unwrap();
        layer.id
    }

    /// A mark with an id chosen by hand.
    ///
    /// The id matters to the ordering tests, and neither of the two
    /// ways of leaving it to chance works: `Uuid::now_v7` leaves the
    /// order of ids minted inside one millisecond to the
    /// implementation (RFC 9562), and `ORDER BY id` compares BLOBs
    /// with memcmp, which need not agree with Rust's `Ord`. A
    /// first-byte-distinct pattern makes both comparisons give the
    /// same answer.
    ///
    /// `created_at` is a fixed millisecond, not `Utc::now()`: the
    /// column stores epoch milliseconds, so a clock read carrying
    /// microseconds does not survive the round trip and an equality
    /// assertion on the whole mark would fail on the truncation rather
    /// than on anything this file is responsible for.
    fn mark_with(
        first_byte: u8,
        asset: AssetId,
        layer: MaterialLayerId,
        start_ms: u64,
        end_ms: Option<u64>,
    ) -> MaterialMark {
        let anchor = MaterialAnchor::Temporal(TimelineSpan::new(start_ms, end_ms).unwrap());
        let placed_at = DateTime::from_timestamp_millis(1_700_000_000_000).unwrap();
        let mut mark = MaterialMark::new(
            asset,
            layer,
            anchor,
            CommentAuthor::User,
            format!("mark {first_byte:#04x} at {start_ms}"),
            placed_at,
        )
        .unwrap();
        mark.id = MaterialMarkId::from_uuid(Uuid::from_bytes([first_byte; 16]));
        mark
    }

    /// The temporal span of a mark, for assertions that only care
    /// about the position.
    fn span_of(mark: &MaterialMark) -> TimelineSpan {
        match mark.anchor {
            MaterialAnchor::Temporal(span) => span,
        }
    }

    /// A mark round-trips, an instant stays an instant, and `delete`
    /// removes exactly one.
    ///
    /// The anchor is the part this pins: it goes into the row as a
    /// `kind` plus a column group and has to come back as the same
    /// variant, not as a bare pair of milliseconds.
    #[tokio::test]
    async fn save_lists_and_deletes() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        let instant = mark_with(0x11, asset, layer, 1_000, None);
        let interval = mark_with(0x22, asset, layer, 2_000, Some(4_500));
        repo.save(&instant).await.unwrap();
        repo.save(&interval).await.unwrap();

        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(listed, vec![instant.clone(), interval.clone()]);
        assert!(span_of(&listed[0]).is_instant());
        assert_eq!(span_of(&listed[1]).end_ms(), Some(4_500));

        // Same id, edited body: the upsert path, not a second row.
        let mut edited = interval.clone();
        edited.body = "moved the note".into();
        edited.edited_at = Some(edited.created_at);
        repo.save(&edited).await.unwrap();
        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].body, "moved the note");

        repo.delete(&instant.id).await.unwrap();
        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(listed, vec![edited]);
        // Idempotent.
        repo.delete(&instant.id).await.unwrap();

        driver.shutdown().await.unwrap();
    }

    /// `save` puts the anchor's kind in the row.
    ///
    /// Asserted through raw SQL because the round trip alone cannot
    /// see it: a `save` writing a constant and an `into_domain`
    /// ignoring the column would agree with each other and produce the
    /// same list. The column is what a second anchor kind will be
    /// dispatched on, so it has to hold the mark's own kind rather than
    /// whatever this build happens to have one of.
    #[tokio::test]
    async fn save_stores_the_anchor_kind() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        repo.save(&mark_with(0x11, asset, layer, 1_000, None))
            .await
            .unwrap();

        let stored: String = isle
            .call(move |conn| {
                conn.query_row(
                    "SELECT anchor_kind FROM material_mark WHERE id = ?1",
                    params![Uuid::from_bytes([0x11; 16])],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(stored, "temporal", "the axis is in the row, not implied");

        driver.shutdown().await.unwrap();
    }

    /// A row shaped the way the decoder must refuse it.
    ///
    /// Built as a `MarkRow` rather than inserted, because the schema
    /// refuses both shapes below — `CHECK (anchor_kind IN ('temporal'))`
    /// and `CHECK (anchor_kind <> 'temporal' OR start_ms IS NOT NULL)`
    /// (both exercised over raw SQL in `migrations.rs`). What is left
    /// to check is the decoder's own behaviour on a row that arrived
    /// some other way: a database written by a build that has the
    /// second anchor kind and then opened by one that does not.
    fn row_with(anchor_kind: &str, start_ms: Option<i64>) -> MarkRow {
        MarkRow {
            id: Uuid::now_v7(),
            asset_id: Uuid::now_v7(),
            layer_id: Uuid::now_v7(),
            anchor_kind: anchor_kind.into(),
            start_ms,
            end_ms: None,
            body: "here".into(),
            author_kind: "user".into(),
            author_persona_id: None,
            created_at: 0,
            edited_at: None,
        }
    }

    /// An anchor kind this build cannot place reads as `Infra`.
    ///
    /// Not a fallback to `'temporal'`: that would invent a position
    /// for a mark whose position this build cannot express, and the
    /// listing would show it somewhere on the timeline as if it
    /// belonged there.
    #[test]
    fn decode_refuses_an_anchor_kind_this_build_cannot_place() {
        assert!(
            row_with("temporal", Some(100)).into_domain().is_ok(),
            "the kind this build does have decodes — otherwise the \
             assertion below would pass for the wrong reason"
        );

        let err = row_with("spatial", Some(100))
            .into_domain()
            .expect_err("a kind with no arm must not decode");
        assert!(
            matches!(err, DomainError::Infra(_)),
            "a row this build cannot place is an infrastructure fact, got {err:?}"
        );
    }

    /// A temporal row with no `start_ms` reads as `Infra`.
    ///
    /// The column is nullable so that a later anchor kind can leave it
    /// empty, and the CHECK is what keeps `'temporal'` from doing the
    /// same. This is the decoder's half of that pair — the half that
    /// still holds if the CHECK is ever widened wrongly.
    #[test]
    fn decode_refuses_a_temporal_row_without_a_position() {
        let err = row_with("temporal", None)
            .into_domain()
            .expect_err("a temporal mark with nowhere to point is not a mark");
        assert!(
            matches!(err, DomainError::Infra(_)),
            "a position-less temporal row is an infrastructure fact, got {err:?}"
        );
    }

    /// A persona's mark on another persona's asset is a mark authored
    /// by a persona, and `save` writes the author through the domain's
    /// own codec.
    #[tokio::test]
    async fn persona_authored_mark_round_trips() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let owner = seed_persona(&isle).await;
        let author = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, owner).await;

        let mut mark = mark_with(0x11, asset, layer, 500, None);
        mark.author = CommentAuthor::Persona { persona_id: author };
        repo.save(&mark).await.unwrap();

        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(listed, vec![mark]);

        driver.shutdown().await.unwrap();
    }

    /// `list_by_asset` returns marks in timeline order — the order
    /// they are read in, which is not the order they were placed in.
    ///
    /// The fixture puts the two axes in disagreement on purpose. Rows
    /// go in at descending `start_ms` while their ids ascend, so the
    /// arrival order (what a rowid scan returns) is the reverse of the
    /// answer. Insertion order agreeing with the expected order is how
    /// an ordering assertion passes without the ordering — an ordering
    /// claim needs a fixture where the axis under test disagrees with
    /// the default.
    ///
    /// Checked by mutation on 2026-08-06: with the clause cut down to
    /// `ORDER BY id`, this test failed — left `[3000, 2000, 1000]`,
    /// right `[1000, 2000, 3000]`. Restored, it passes.
    #[tokio::test]
    async fn list_by_asset_orders_by_start_not_by_arrival() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        // Descending position, ascending id.
        repo.save(&mark_with(0x11, asset, layer, 3_000, None))
            .await
            .unwrap();
        repo.save(&mark_with(0x22, asset, layer, 2_000, Some(2_500)))
            .await
            .unwrap();
        repo.save(&mark_with(0x33, asset, layer, 1_000, None))
            .await
            .unwrap();

        let positions: Vec<u64> = repo
            .list_by_asset(&asset)
            .await
            .unwrap()
            .iter()
            .map(|m| span_of(m).start_ms())
            .collect();
        assert_eq!(positions, vec![1_000, 2_000, 3_000]);

        driver.shutdown().await.unwrap();
    }

    /// Marks sharing a position come back in id order.
    ///
    /// Separate from the test above because that one never reaches the
    /// tie-break: with three distinct `start_ms` values, `, id` can be
    /// deleted and it still passes. Here the two rows arrive in
    /// descending id order at one position, so arrival order and id
    /// order disagree.
    ///
    /// Checked by mutation on 2026-08-06: with the clause cut down to
    /// `ORDER BY start_ms`, this test failed — left
    /// `[22222222-…, 11111111-…]`, right `[11111111-…, 22222222-…]`,
    /// i.e. the rows came back in arrival order. Restored, it passes.
    ///
    /// That negative result is a guarantee, not a coincidence, and the
    /// clause it pins is not decoration. `id` is the PRIMARY KEY (V66,
    /// `migrations.rs`), so `ORDER BY start_ms, id` sorts on a unique
    /// key — there are no equal sort keys, so the "SQL leaves ties
    /// undefined" caveat has nothing to apply to and the result order is
    /// total. Nor does the tie-break come free from the index scan:
    /// measured 2026-08-06 (SQLite 3.43.2, the V60 shape of this table,
    /// this adapter's own statement), `EXPLAIN QUERY PLAN` returns
    ///
    /// ```text
    /// SEARCH material_mark USING INDEX idx_material_mark_asset_start (asset_id=?)
    /// USE TEMP B-TREE FOR RIGHT PART OF ORDER BY
    /// ```
    ///
    /// i.e. the index (`asset_id, start_ms`) serves the filter and the
    /// leading sort term, and SQLite builds a sorter for `, id`
    /// specifically. Removing the clause therefore changes the answer
    /// rather than leaving it to chance, and no future index choice
    /// makes removing it safe.
    #[tokio::test]
    async fn marks_at_the_same_position_are_ordered_by_id() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        let low = mark_with(0x11, asset, layer, 7_000, None);
        let high = mark_with(0x22, asset, layer, 7_000, Some(7_100));
        // Arrival order is the reverse of the expected order.
        repo.save(&high).await.unwrap();
        repo.save(&low).await.unwrap();

        let ids: Vec<MaterialMarkId> = repo
            .list_by_asset(&asset)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, vec![low.id, high.id], "the tie-break decides");

        driver.shutdown().await.unwrap();
    }

    /// `save` refuses a body the domain refuses, by the same rule the
    /// read path applies.
    ///
    /// The route is the edit idiom this file's own round-trip test uses:
    /// clone a saved mark, assign `body`, save again — `body` is a `pub`
    /// field, so a record update reaches it without passing a
    /// constructor. Substituting `'\t'` for the edited text is the whole
    /// difference, and it is the value that makes the point: every CHECK
    /// on the table accepts it (SQL's `trim` strips only U+0020) and
    /// Rust's `str::trim` empties it.
    ///
    /// The listing assertion is the load-bearing half. The upsert
    /// targets the same id, so had the write got through it would have
    /// replaced a mark that lists with one that does not — and
    /// `list_by_asset` collects into one `Result`, so the whole asset's
    /// timeline would read `Err` from then on.
    #[tokio::test]
    async fn save_refuses_a_body_the_domain_refuses() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        let placed = mark_with(0x11, asset, layer, 1_000, None);
        repo.save(&placed).await.unwrap();

        let mut blanked = placed.clone();
        blanked.body = "\t".into();
        blanked.edited_at = Some(placed.created_at);
        let err = repo.save(&blanked).await.unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(_)),
            "a caller handing over a body the domain forbids is a caller \
             error, not an infrastructure failure, got {err:?}"
        );

        assert_eq!(
            repo.list_by_asset(&asset).await.unwrap(),
            vec![placed],
            "the refused write must not have reached the row"
        );

        driver.shutdown().await.unwrap();
    }

    /// `save` refuses `edited_at` before `created_at` — the other rule
    /// the schema does not carry (V15 does not constrain the pair
    /// either, and this table follows it).
    ///
    /// One millisecond early is the smallest gap the column can hold:
    /// `datetime_to_ms` stores epoch milliseconds, so anything finer
    /// would not survive the round trip and the read path would never
    /// see it.
    #[tokio::test]
    async fn save_refuses_edited_before_created() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        let placed = mark_with(0x11, asset, layer, 1_000, None);
        repo.save(&placed).await.unwrap();

        let mut backdated = placed.clone();
        backdated.body = "moved the note".into();
        backdated.edited_at = Some(placed.created_at - chrono::Duration::milliseconds(1));
        let err = repo.save(&backdated).await.unwrap_err();
        assert!(
            matches!(err, DomainError::Validation(_)),
            "an edit stamped before the mark was placed is a caller error, got {err:?}"
        );

        assert_eq!(
            repo.list_by_asset(&asset).await.unwrap(),
            vec![placed],
            "the refused write must not have reached the row"
        );

        driver.shutdown().await.unwrap();
    }

    /// A body that the schema accepts and the domain does not comes
    /// back as `Infra`, not as a valid mark.
    ///
    /// `'\t'` is the case that makes the point: it passes every CHECK
    /// on the table (and would pass a `length(trim(body)) > 0` CHECK
    /// too, since SQL's `trim` strips only U+0020), and Rust's
    /// `str::trim` empties it. If `into_domain` assembled the struct
    /// field by field, this row would list without complaint.
    #[tokio::test]
    async fn read_back_refuses_a_body_the_domain_refuses() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        let aid = *asset.as_uuid();
        let lid = *layer.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO material_mark
                     (id, asset_id, layer_id, anchor_kind, start_ms, end_ms, body, author_kind,
                      author_persona_id, created_at)
                 VALUES (?1, ?2, ?3, 'temporal', 100, NULL, char(9), 'user', NULL, 0)",
                params![Uuid::now_v7(), aid, lid],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let err = repo.list_by_asset(&asset).await.unwrap_err();
        assert!(
            matches!(err, DomainError::Infra(_)),
            "a row the constructor refuses is an infrastructure fact, got {err:?}"
        );

        driver.shutdown().await.unwrap();
    }

    /// `edited_at` before `created_at` is refused on the way out, the
    /// rule having no home in the schema (V15 does not constrain the
    /// pair either, and this table follows it).
    #[tokio::test]
    async fn read_back_refuses_edited_before_created() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, persona).await;

        let aid = *asset.as_uuid();
        let lid = *layer.as_uuid();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO material_mark
                     (id, asset_id, layer_id, anchor_kind, start_ms, end_ms, body, author_kind,
                      author_persona_id, created_at, edited_at)
                 VALUES (?1, ?2, ?3, 'temporal', 100, NULL, 'here', 'user', NULL, 5000, 4000)",
                params![Uuid::now_v7(), aid, lid],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let err = repo.list_by_asset(&asset).await.unwrap_err();
        assert!(
            matches!(err, DomainError::Infra(_)),
            "edited before created must not list as a valid mark, got {err:?}"
        );

        driver.shutdown().await.unwrap();
    }

    /// Purging a persona sweeps the marks it authored, on anyone's
    /// assets, and does not fail on a foreign key.
    ///
    /// This is the reason the author FK is `CASCADE`. `purge`
    /// (`repo/persona.rs`) clears RESTRICT-referencing children by
    /// hand, in an order it spells out, because SQLite does not order
    /// sibling-table cascades — a RESTRICT table added here would be
    /// another line owed to that list, and the test for the omission
    /// is an FK error at purge time. `persona.rs` is untouched by this
    /// unit; that it stays untouched is what is being checked.
    ///
    /// The asset belongs to a second persona so that the author FK is
    /// the only path from the purged persona to the mark.
    #[tokio::test]
    async fn purging_a_persona_sweeps_the_marks_it_authored() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialMarkRepository::new(isle.clone());
        let owner = seed_persona(&isle).await;
        let author = seed_persona(&isle).await;
        let (asset, layer) = seed_asset(&isle, owner).await;

        let mut by_author = mark_with(0x11, asset, layer, 1_000, None);
        by_author.author = CommentAuthor::Persona { persona_id: author };
        let by_user = mark_with(0x22, asset, layer, 2_000, None);
        repo.save(&by_author).await.unwrap();
        repo.save(&by_user).await.unwrap();

        let personas = crate::sqlite::repo::SqlitePersonaRepository::new(isle.clone());
        let now: DateTime<Utc> = Utc::now();
        personas.trash(&author, now).await.unwrap();
        personas
            .purge(&author)
            .await
            .expect("the author's marks must not hold the purge back");

        let listed = repo.list_by_asset(&asset).await.unwrap();
        assert_eq!(
            listed,
            vec![by_user],
            "the purged persona's mark goes with it; the User's stays"
        );

        driver.shutdown().await.unwrap();
    }
}
