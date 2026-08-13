//! SQLite adapter for the `MaterialLayerRepository` port.

use asterism_core::domain::material_layer::{LayerOrigin, LayerRole, MaterialLayer};
use asterism_core::domain::repository::MaterialLayerRepository;
use asterism_core::domain::value::{AssetId, MaterialLayerId};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::{OptionalExtension, params};
use rusqlite_isle::AsyncIsle;
use uuid::Uuid;

use crate::sqlite::map::infra_err;

/// SQLite adapter for `MaterialLayerRepository`.
#[derive(Clone)]
pub struct SqliteMaterialLayerRepository {
    isle: AsyncIsle,
}

impl SqliteMaterialLayerRepository {
    /// Wraps a writer `AsyncIsle`.
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

/// Narrows an ordinal to the signed range the column stores.
///
/// `u32` always fits an `i64`, so this is a conversion for the reader
/// rather than a check with teeth — written out so that widening either
/// ordinal to `u64` later is a compile error here instead of a silent
/// `as` truncation.
fn stored_ord(value: u32) -> i64 {
    i64::from(value)
}

/// Lifts a stored ordinal back onto the domain's `u32`.
///
/// The column is `INTEGER` with `CHECK (… >= 0)`, so a negative value
/// means a row written around the constraint; either way it is an
/// infrastructure fact, not a caller error.
fn domain_ord(value: i64, column: &str) -> Result<u32, DomainError> {
    u32::try_from(value).map_err(|_| {
        DomainError::Infra(anyhow::anyhow!(
            "{column} = {value} is outside the range an ordinal takes"
        ))
    })
}

struct LayerRow {
    id: Uuid,
    asset_id: Uuid,
    material_ord: i64,
    origin: String,
    role: String,
    is_default: i64,
    ord: i64,
}

impl LayerRow {
    const COLUMNS: &'static str = "id, asset_id, material_ord, origin, role, is_default, ord";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get(0)?,
            asset_id: row.get(1)?,
            material_ord: row.get(2)?,
            origin: row.get(3)?,
            role: row.get(4)?,
            is_default: row.get(5)?,
            ord: row.get(6)?,
        })
    }

    /// Promotes a row into the domain **through the constructor**.
    ///
    /// Not a struct literal: the rule that the default annotation band
    /// belongs to the user is in both the schema and
    /// `MaterialLayer::validate`, and this call is what keeps a row
    /// written around the `CHECK` — by hand, or by a build whose
    /// vocabulary differs — from arriving as a valid layer.
    ///
    /// An unrecognised `origin` or `role` is `Infra` rather than a
    /// fallback to some nearest variant: falling back would be this
    /// adapter deciding who wrote a band it cannot read the provenance
    /// of, and every guard above it answers on exactly that field.
    fn into_domain(self) -> Result<MaterialLayer, DomainError> {
        let id = self.id;
        let origin = LayerOrigin::from_slug(&self.origin).ok_or_else(|| {
            DomainError::Infra(anyhow::anyhow!(
                "material_layer {id} holds an origin this build cannot read: {:?}",
                self.origin
            ))
        })?;
        let role = LayerRole::from_slug(&self.role).ok_or_else(|| {
            DomainError::Infra(anyhow::anyhow!(
                "material_layer {id} holds a role this build cannot read: {:?}",
                self.role
            ))
        })?;
        MaterialLayer::rehydrate(
            MaterialLayerId::from_uuid(id),
            AssetId::from_uuid(self.asset_id),
            domain_ord(self.material_ord, "material_ord")?,
            origin,
            role,
            self.is_default != 0,
            domain_ord(self.ord, "ord")?,
        )
        .map_err(|e| {
            DomainError::Infra(anyhow::anyhow!(
                "material_layer {id} holds a row the domain refuses: {e}"
            ))
        })
    }
}

#[async_trait]
impl MaterialLayerRepository for SqliteMaterialLayerRepository {
    /// Writes a layer, refusing one the read path would refuse.
    ///
    /// The check is the domain's own, for the reason the mark adapter
    /// gives beside it: the fields are public, so a record update
    /// reaches them without passing a constructor, and a listing
    /// collects into one `Result` — a single row the read door refuses
    /// makes the whole asset's set of bands `Err`.
    ///
    /// A second `is_default` row on the same
    /// `(asset_id, material_ord, role)` is refused by the partial
    /// unique index, and surfaces here as `Infra` (a
    /// `UNIQUE constraint failed` from SQLite). That is the intended
    /// door: moving the flag is [`Self::set_default`]'s job, and a
    /// caller reaching for `save` to do it is asking for the state the
    /// index exists to forbid.
    ///
    /// `asset_id` is **not** in the `DO UPDATE SET` list, matching the
    /// mark adapter, which leaves out `asset_id` and the author columns
    /// for the same reason: a band is over one original for its whole
    /// life. Moving one to another asset is not an edit of the band, it
    /// is a different band — its chapters and notes are about *that*
    /// material, and they carry their own `asset_id`, so an upsert that
    /// honoured a changed one here would leave the children pointing at
    /// the old asset and the parent at the new one, with no verb having
    /// been called that says so. Omitting the column makes such a save
    /// a silent no-op on that field rather than a split row; a caller
    /// that genuinely wants the band elsewhere writes a new one.
    async fn save(&self, layer: &MaterialLayer) -> Result<(), DomainError> {
        layer.validate()?;
        let id = *layer.id.as_uuid();
        let asset_id = *layer.asset_id.as_uuid();
        let material_ord = stored_ord(layer.material_ord);
        let origin = layer.origin.slug().to_string();
        let role = layer.role.slug().to_string();
        let is_default = i64::from(layer.is_default);
        let ord = stored_ord(layer.ord);
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO material_layer
                         (id, asset_id, material_ord, origin, role, is_default, ord)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                         material_ord = excluded.material_ord,
                         origin = excluded.origin,
                         role = excluded.role,
                         is_default = excluded.is_default,
                         ord = excluded.ord",
                    params![id, asset_id, material_ord, origin, role, is_default, ord],
                )?;
                Ok(())
            })
            .await
            .map_err(infra_err)
    }

    async fn find(&self, id: &MaterialLayerId) -> Result<Option<MaterialLayer>, DomainError> {
        let uuid = *id.as_uuid();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM material_layer WHERE id = ?1",
                        LayerRow::COLUMNS
                    ),
                    params![uuid],
                    LayerRow::from_row,
                )
                .optional()
            })
            .await
            .map_err(infra_err)?;
        row.map(LayerRow::into_domain).transpose()
    }

    async fn list_by_asset(&self, asset_id: &AssetId) -> Result<Vec<MaterialLayer>, DomainError> {
        let aid = *asset_id.as_uuid();
        let rows = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {} FROM material_layer
                        WHERE asset_id = ?1
                        ORDER BY material_ord, role, ord, id",
                    LayerRow::COLUMNS
                ))?;
                let rows: Vec<LayerRow> = stmt
                    .query_map(params![aid], LayerRow::from_row)?
                    .collect::<Result<_, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter().map(LayerRow::into_domain).collect()
    }

    /// Moves the default flag onto `id` inside one transaction.
    ///
    /// Two statements, and they have to be one unit: the partial unique
    /// index refuses a moment in which both rows carry the flag, so the
    /// clear has to precede the set — and a failure between them would
    /// leave the triple with *no* default, which the lazy-creation path
    /// reads as "this asset has no band" and answers by making a second
    /// one.
    ///
    /// The `WHERE` on the clear names the triple by reading it off the
    /// target row rather than taking it from the caller: the caller
    /// holds an id, and asking it to also state the scope would let the
    /// two disagree.
    async fn set_default(&self, id: &MaterialLayerId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        let moved = self
            .isle
            .call(move |conn| {
                let tx = conn.transaction()?;
                let scope: Option<(Uuid, i64, String)> = tx
                    .query_row(
                        "SELECT asset_id, material_ord, role FROM material_layer WHERE id = ?1",
                        params![uuid],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((asset_id, material_ord, role)) = scope else {
                    return Ok(false);
                };
                tx.execute(
                    "UPDATE material_layer SET is_default = 0
                      WHERE asset_id = ?1 AND material_ord = ?2 AND role = ?3
                        AND id <> ?4 AND is_default = 1",
                    params![asset_id, material_ord, role, uuid],
                )?;
                tx.execute(
                    "UPDATE material_layer SET is_default = 1 WHERE id = ?1",
                    params![uuid],
                )?;
                tx.commit()?;
                Ok(true)
            })
            .await
            .map_err(infra_err)?;
        if moved {
            Ok(())
        } else {
            Err(DomainError::not_found("material layer", id))
        }
    }

    async fn delete(&self, id: &MaterialLayerId) -> Result<(), DomainError> {
        let uuid = *id.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM material_layer WHERE id = ?1", params![uuid])?;
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
    use crate::sqlite::repo::SqliteMaterialMarkRepository;
    use asterism_core::domain::asset_comment::CommentAuthor;
    use asterism_core::domain::material_mark::{MaterialAnchor, MaterialMark, TimelineSpan};
    use asterism_core::domain::repository::MaterialMarkRepository;
    use asterism_core::domain::value::PersonaId;
    use chrono::Utc;

    /// Seeds one persona. `pack_id` is UNIQUE, so it is derived from
    /// the id.
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

    /// Seeds one video asset under `persona`.
    async fn seed_asset(isle: &AsyncIsle, persona: PersonaId) -> AssetId {
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
        AssetId::from_uuid(aid)
    }

    /// A band with an id chosen by hand.
    ///
    /// The id matters to the ordering test, and neither way of leaving
    /// it to chance works: `Uuid::now_v7` leaves the order of ids
    /// minted inside one millisecond to the implementation (RFC 9562),
    /// and `ORDER BY id` compares BLOBs with memcmp, which need not
    /// agree with Rust's `Ord`. A first-byte-distinct pattern makes
    /// both comparisons give the same answer.
    fn layer_with(
        first_byte: u8,
        asset: AssetId,
        material_ord: u32,
        origin: LayerOrigin,
        role: LayerRole,
        is_default: bool,
        ord: u32,
    ) -> MaterialLayer {
        let mut layer =
            MaterialLayer::new(asset, material_ord, origin, role, is_default, ord).unwrap();
        layer.id = MaterialLayerId::from_uuid(Uuid::from_bytes([first_byte; 16]));
        layer
    }

    /// A layer round-trips through `save` / `find` / `list_by_asset`,
    /// and `delete` removes exactly one.
    #[tokio::test]
    async fn save_finds_lists_and_deletes() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialLayerRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_asset(&isle, persona).await;

        let imported = layer_with(
            0x11,
            asset,
            0,
            LayerOrigin::Imported,
            LayerRole::Structure,
            true,
            0,
        );
        let mine = layer_with(
            0x22,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Structure,
            false,
            1,
        );
        repo.save(&imported).await.unwrap();
        repo.save(&mine).await.unwrap();

        assert_eq!(
            repo.find(&imported.id).await.unwrap(),
            Some(imported.clone())
        );
        assert_eq!(
            repo.list_by_asset(&asset).await.unwrap(),
            vec![imported.clone(), mine.clone()]
        );

        // Same id, edited ord: the upsert path, not a second row.
        let mut moved = mine.clone();
        moved.ord = 7;
        repo.save(&moved).await.unwrap();
        assert_eq!(repo.list_by_asset(&asset).await.unwrap().len(), 2);
        assert_eq!(repo.find(&mine.id).await.unwrap().unwrap().ord, 7);

        repo.delete(&imported.id).await.unwrap();
        assert_eq!(repo.find(&imported.id).await.unwrap(), None);
        assert_eq!(repo.list_by_asset(&asset).await.unwrap(), vec![moved]);
        // Idempotent.
        repo.delete(&imported.id).await.unwrap();

        driver.shutdown().await.unwrap();
    }

    /// `list_by_asset` orders by `(material_ord, role, ord, id)`, and
    /// the fixture puts arrival order in disagreement with every term.
    ///
    /// Rows go in exactly reversed, so a listing that returned arrival
    /// order — which is what a rowid scan gives — would fail on the
    /// first assertion rather than pass by coincidence.
    #[tokio::test]
    async fn list_by_asset_orders_by_material_role_and_ord() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialLayerRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_asset(&isle, persona).await;

        // Expected order: annotation before structure (slug order),
        // then ord, then id; material 0 before material 1.
        let first = layer_with(
            0x11,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Annotation,
            true,
            0,
        );
        let second = layer_with(
            0x22,
            asset,
            0,
            LayerOrigin::Imported,
            LayerRole::Structure,
            true,
            0,
        );
        let third = layer_with(
            0x33,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Structure,
            false,
            5,
        );
        let fourth = layer_with(
            0x44,
            asset,
            1,
            LayerOrigin::Machine,
            LayerRole::Structure,
            false,
            0,
        );
        for layer in [&fourth, &third, &second, &first] {
            repo.save(layer).await.unwrap();
        }

        assert_eq!(
            repo.list_by_asset(&asset).await.unwrap(),
            vec![first, second, third, fourth]
        );

        driver.shutdown().await.unwrap();
    }

    /// Only one band per `(asset, material, role)` may be the default,
    /// and `set_default` is the verb that moves the flag.
    ///
    /// The `save` refusal is asserted first, because it is what makes
    /// the dedicated verb necessary rather than convenient. The second
    /// half then shows the flag actually moving — and that a band of a
    /// *different* role keeps its own, which is the part a `WHERE
    /// asset_id = ?` clear would break.
    #[tokio::test]
    async fn the_default_is_unique_per_role_and_moves_only_through_set_default() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialLayerRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_asset(&isle, persona).await;

        let imported = layer_with(
            0x11,
            asset,
            0,
            LayerOrigin::Imported,
            LayerRole::Structure,
            true,
            0,
        );
        let mine = layer_with(
            0x22,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Structure,
            false,
            1,
        );
        let notes = layer_with(
            0x33,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Annotation,
            true,
            0,
        );
        repo.save(&imported).await.unwrap();
        repo.save(&mine).await.unwrap();
        repo.save(&notes)
            .await
            .expect("a default annotation band is a different triple");

        let mut second_default = mine.clone();
        second_default.is_default = true;
        let err = repo.save(&second_default).await.expect_err(
            "two default structure bands on one material is the state the index forbids",
        );
        assert!(
            matches!(err, DomainError::Infra(_)),
            "a constraint the schema holds is an infrastructure fact, got {err:?}"
        );

        repo.set_default(&mine.id).await.unwrap();
        let listed = repo.list_by_asset(&asset).await.unwrap();
        let flagged: Vec<(MaterialLayerId, bool)> =
            listed.iter().map(|l| (l.id, l.is_default)).collect();
        assert_eq!(
            flagged,
            vec![(notes.id, true), (imported.id, false), (mine.id, true)],
            "the flag moved within the structure triple and left the annotation one alone"
        );

        // Idempotent: naming the holder leaves it holding.
        repo.set_default(&mine.id).await.unwrap();
        assert!(repo.find(&mine.id).await.unwrap().unwrap().is_default);

        let err = repo
            .set_default(&MaterialLayerId::new())
            .await
            .expect_err("an id no band carries cannot become the default");
        assert!(
            matches!(err, DomainError::NotFound { .. }),
            "expected NotFound, got {err:?}"
        );

        driver.shutdown().await.unwrap();
    }

    /// A vocabulary this build cannot read comes back as `Infra`, not
    /// as a band with a guessed origin.
    ///
    /// Inserted over raw SQL with the `CHECK` sidestepped — the value
    /// is one the schema refuses, so what is under test is the
    /// decoder's behaviour on a database written by a build whose
    /// vocabulary is wider than this one's. `PRAGMA ignore_check_
    /// constraints` is what makes that row reachable at all.
    #[tokio::test]
    async fn read_back_refuses_an_origin_this_build_cannot_read() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialLayerRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_asset(&isle, persona).await;

        let aid = *asset.as_uuid();
        isle.call(move |conn| {
            conn.pragma_update(None, "ignore_check_constraints", true)?;
            conn.execute(
                "INSERT INTO material_layer
                     (id, asset_id, material_ord, origin, role, is_default, ord)
                 VALUES (?1, ?2, 0, 'crowdsourced', 'structure', 0, 0)",
                params![Uuid::now_v7(), aid],
            )?;
            conn.pragma_update(None, "ignore_check_constraints", false)?;
            Ok(())
        })
        .await
        .unwrap();

        let err = repo
            .list_by_asset(&asset)
            .await
            .expect_err("a band whose provenance this build cannot read is not a band");
        assert!(
            matches!(err, DomainError::Infra(_)),
            "a row this build cannot read is an infrastructure fact, got {err:?}"
        );

        driver.shutdown().await.unwrap();
    }

    /// Deleting the asset takes its bands with it.
    ///
    /// The cascade is what keeps a purge from having to name this table
    /// (`repo/persona.rs::purge` clears RESTRICT children by hand, in
    /// an order it spells out; a RESTRICT table here would be another
    /// line owed to that list).
    #[tokio::test]
    async fn deleting_the_asset_sweeps_its_bands() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialLayerRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_asset(&isle, persona).await;
        let kept = seed_asset(&isle, persona).await;

        repo.save(&layer_with(
            0x11,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Structure,
            false,
            0,
        ))
        .await
        .unwrap();
        let survivor = layer_with(
            0x22,
            kept,
            0,
            LayerOrigin::User,
            LayerRole::Structure,
            false,
            0,
        );
        repo.save(&survivor).await.unwrap();

        let aid = *asset.as_uuid();
        isle.call(move |conn| {
            conn.execute("DELETE FROM asset WHERE id = ?1", params![aid])?;
            Ok(())
        })
        .await
        .unwrap();

        assert!(repo.list_by_asset(&asset).await.unwrap().is_empty());
        assert_eq!(repo.list_by_asset(&kept).await.unwrap(), vec![survivor]);

        driver.shutdown().await.unwrap();
    }

    /// Deleting the band takes its notes with it.
    ///
    /// The port doc promises the cascade reaches *both* child tables,
    /// and the chapter half is covered beside the chapter adapter
    /// (`repo/chapter_mark.rs::deleting_the_band_sweeps_its_chapters`).
    /// This is the other half, and it is the one worth asserting from
    /// here: `material_mark` also references `asset`, so a mark is
    /// reachable by two cascades and the marks read back by asset
    /// rather than by band — a `layer_id` FK declared `RESTRICT`, or a
    /// `SET NULL` against the `NOT NULL` column, would leave the delete
    /// failing or the note stranded, and every listing this repository
    /// has would still look right.
    ///
    /// The second band is what separates the cascade from a delete that
    /// simply empties the asset: its note has to survive.
    #[tokio::test]
    async fn deleting_the_band_sweeps_its_notes() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteMaterialLayerRepository::new(isle.clone());
        let marks = SqliteMaterialMarkRepository::new(isle.clone());
        let persona = seed_persona(&isle).await;
        let asset = seed_asset(&isle, persona).await;

        let doomed = layer_with(
            0x11,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Annotation,
            true,
            0,
        );
        let kept = layer_with(
            0x22,
            asset,
            0,
            LayerOrigin::User,
            LayerRole::Annotation,
            false,
            1,
        );
        repo.save(&doomed).await.unwrap();
        repo.save(&kept).await.unwrap();

        let note = |layer: MaterialLayerId, start: u64, body: &str| {
            MaterialMark::new(
                asset,
                layer,
                MaterialAnchor::Temporal(TimelineSpan::new(start, None).unwrap()),
                CommentAuthor::User,
                body.to_string(),
                Utc::now(),
            )
            .unwrap()
        };
        marks
            .save(&note(doomed.id, 1_000, "in the doomed band"))
            .await
            .unwrap();
        let survivor = note(kept.id, 2_000, "in the other band");
        marks.save(&survivor).await.unwrap();

        repo.delete(&doomed.id).await.unwrap();

        let left: Vec<(MaterialLayerId, String)> = marks
            .list_by_asset(&asset)
            .await
            .unwrap()
            .into_iter()
            .map(|m| (m.layer_id, m.body))
            .collect();
        assert_eq!(
            left,
            vec![(kept.id, "in the other band".to_string())],
            "the deleted band's note went with it, and only that one"
        );

        driver.shutdown().await.unwrap();
    }
}
