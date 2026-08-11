//! SQLite adapter for the `ModalityRepository` port — the Modality
//! master (`modality` table), backed by rusqlite-isle.
//!
//! Follows the crate-wide adapter convention: only `rusqlite`
//! primitives inside the isle closure; promotion into domain types
//! (parsing `kind` → `ContentKind`, `cover_template` → `CoverTemplate`)
//! happens outside via [`ModalityRow::into_domain`].

use asterism_core::domain::modality::{ModalityDef, ModalityView};
use asterism_core::domain::repository::ModalityRepository;
use asterism_core::domain::value::{CoverTemplate, Modality};
use asterism_core::error::DomainError;
use async_trait::async_trait;
use rusqlite::params;
use rusqlite_isle::AsyncIsle;

use crate::sqlite::map::infra_err;

/// Primitive row built inside the isle closure; promotion to the domain
/// type (including `kind` / `cover_template` parsing) happens outside.
struct ModalityRow {
    slug: String,
    label: String,
    terminal: bool,
    sort_order: i64,
    hidden: bool,
    cover_template: Option<String>,
}

impl ModalityRow {
    const COLUMNS: &'static str = "slug, label, terminal, sort_order, hidden, cover_template";

    fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            slug: row.get(0)?,
            label: row.get(1)?,
            terminal: row.get::<_, i64>(2)? != 0,
            sort_order: row.get(3)?,
            hidden: row.get::<_, i64>(4)? != 0,
            cover_template: row.get(5)?,
        })
    }

    fn into_domain(self) -> Result<ModalityDef, DomainError> {
        Ok(ModalityDef {
            slug: Modality::new(self.slug)?,
            label: self.label,
            terminal: self.terminal,
            sort_order: self.sort_order,
            hidden: self.hidden,
            cover_template: self
                .cover_template
                .as_deref()
                .map(CoverTemplate::parse)
                .transpose()?,
        })
    }
}

/// SQLite adapter for `ModalityRepository` (uses a writer isle).
#[derive(Clone)]
pub struct SqliteModalityRepository {
    isle: AsyncIsle,
}

impl SqliteModalityRepository {
    /// Wraps a writer `AsyncIsle` (pragma / schema initialisation is done
    /// by `sqlite::open`).
    pub fn new(isle: AsyncIsle) -> Self {
        Self { isle }
    }
}

#[async_trait]
impl ModalityRepository for SqliteModalityRepository {
    async fn list(&self) -> Result<Vec<ModalityView>, DomainError> {
        // Correlated subquery for the live asset count — `asset.modality`
        // is an open slug with no FK to this table, so a plain LEFT JOIN
        // + GROUP BY would over-count nothing but reads cleaner as a
        // scalar subquery here.
        //
        // "Live" is literal: trashed assets and headstones (rows folded
        // into a keeper, V49) are both excluded, because this count
        // drives the user-facing modality chips and has to agree with
        // what clicking one shows — the same contract
        // `asset::GRID_POPULATION` states for the other facets. The
        // sibling `asset_count` below answers the opposite question
        // (the delete guard) and deliberately keeps counting both.
        let rows: Vec<(ModalityRow, i64)> = self
            .isle
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {}, \
                        (SELECT COUNT(*) FROM asset a \
                          WHERE a.modality = m.slug AND a.trashed_at IS NULL \
                            AND a.folded_into IS NULL) AS asset_count \
                     FROM modality m \
                     ORDER BY m.sort_order, m.slug",
                    ModalityRow::COLUMNS
                        .split(", ")
                        .map(|c| format!("m.{c}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?;
                let rows = stmt
                    .query_map([], |r| Ok((ModalityRow::from_row(r)?, r.get::<_, i64>(6)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(infra_err)?;
        rows.into_iter()
            .map(|(row, count)| {
                Ok(ModalityView {
                    def: row.into_domain()?,
                    asset_count: count.max(0) as u64,
                })
            })
            .collect()
    }

    async fn find(&self, slug: &Modality) -> Result<Option<ModalityDef>, DomainError> {
        let slug_owned = slug.as_str().to_string();
        let row = self
            .isle
            .call(move |conn| {
                conn.query_row(
                    &format!(
                        "SELECT {} FROM modality WHERE slug = ?1",
                        ModalityRow::COLUMNS
                    ),
                    params![slug_owned],
                    ModalityRow::from_row,
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
            })
            .await
            .map_err(infra_err)?;
        row.map(ModalityRow::into_domain).transpose()
    }

    async fn create(&self, def: &ModalityDef) -> Result<(), DomainError> {
        let slug = def.slug.as_str().to_string();
        let label = def.label.clone();
        let terminal = def.terminal as i64;
        let sort_order = def.sort_order;
        let hidden = def.hidden as i64;
        let cover_template = def.cover_template.map(|t| t.as_str().to_string());
        let slug_for_err = slug.clone();
        self.isle
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO modality \
                        (slug, label, terminal, sort_order, hidden, cover_template) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![slug, label, terminal, sort_order, hidden, cover_template],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| {
                // Primary-key violation → domain-level Conflict so the
                // HTTP boundary returns 409 (mirrors the group adapter).
                let msg = err.to_string();
                if msg.contains("UNIQUE") || msg.contains("unique") {
                    DomainError::Conflict(format!("modality {slug_for_err:?} already exists"))
                } else {
                    infra_err(err)
                }
            })
    }

    async fn update(&self, def: &ModalityDef) -> Result<(), DomainError> {
        let slug = def.slug.as_str().to_string();
        let label = def.label.clone();
        let terminal = def.terminal as i64;
        let sort_order = def.sort_order;
        let hidden = def.hidden as i64;
        let cover_template = def.cover_template.map(|t| t.as_str().to_string());
        let slug_for_err = slug.clone();
        let updated = self
            .isle
            .call(move |conn| {
                conn.execute(
                    "UPDATE modality SET \
                        label = ?2, terminal = ?3, sort_order = ?4, hidden = ?5, \
                        cover_template = ?6 \
                     WHERE slug = ?1",
                    params![slug, label, terminal, sort_order, hidden, cover_template],
                )
            })
            .await
            .map_err(infra_err)?;
        if updated == 0 {
            return Err(DomainError::not_found("modality", &slug_for_err));
        }
        Ok(())
    }

    async fn delete(&self, slug: &Modality) -> Result<(), DomainError> {
        let slug_owned = slug.as_str().to_string();
        let slug_for_err = slug_owned.clone();
        let deleted = self
            .isle
            .call(move |conn| {
                conn.execute("DELETE FROM modality WHERE slug = ?1", params![slug_owned])
            })
            .await
            .map_err(infra_err)?;
        if deleted == 0 {
            return Err(DomainError::not_found("modality", &slug_for_err));
        }
        Ok(())
    }

    async fn asset_count(&self, slug: &Modality) -> Result<u64, DomainError> {
        let slug_owned = slug.as_str().to_string();
        let count: i64 = self
            .isle
            .call(move |conn| {
                // No `trashed_at` filter, deliberately: this is the
                // delete guard, so a trashed asset still counts as
                // "someone is using this slug". Dropping the master row
                // out from under it would leave the asset pointing at a
                // missing modality the moment it is restored. The
                // display count in `list` is the filtered one.
                //
                // No `folded_into` filter either, and for the stronger
                // version of the same reason: a headstone is permanent
                // (nothing deletes it), so a slug it references is in
                // use forever.
                conn.query_row(
                    "SELECT COUNT(*) FROM asset WHERE modality = ?1",
                    params![slug_owned],
                    |r| r.get(0),
                )
            })
            .await
            .map_err(infra_err)?;
        Ok(count.max(0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open_and_migrate_in_memory;
    use asterism_contract::command::{
        CreateModalityCommand, DeleteModalityCommand, UpdateModalityCommand,
    };
    use asterism_core::application::ModalityService;
    use asterism_core::domain::attribution::AttributionContext;
    use std::sync::Arc;
    use uuid::Uuid;

    /// The unrecorded context, spelled the way a crate outside
    /// `asterism-core` has to spell it (`unrecorded()` is crate-private
    /// there): an empty assertion is defined to be the same value. These
    /// fixtures are about the master rows, not about who edited them.
    fn nobody() -> AttributionContext {
        AttributionContext::asserted(None, None).unwrap()
    }

    /// Inserts a persona + one asset carrying `modality`, so the delete
    /// guard has something to trip on.
    async fn seed_asset(isle: &AsyncIsle, modality: &str) {
        let persona = Uuid::now_v7();
        let asset = Uuid::now_v7();
        let modality = modality.to_string();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, 'P', 0, 0, 0, 0)",
                params![persona],
            )?;
            conn.execute(
                "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                    modality, occurred_at, created_at, updated_at) \
                 VALUES (?1, ?2, 'fs', ?3, ?4, 0, 0, 0)",
                params![asset, persona, format!("a-{asset}.md"), modality],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    }

    /// The modality chips are a facet like any other, so they count
    /// the same population the grid does — a headstone (V49) is under
    /// its keeper now. The delete guard below asks the opposite
    /// question and is asserted here too, because the two counts
    /// diverging on purpose is the thing worth pinning.
    #[tokio::test]
    async fn a_folded_row_leaves_the_chip_count_but_still_holds_the_slug() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteModalityRepository::new(isle.clone());

        let persona = Uuid::now_v7();
        let keeper = Uuid::now_v7();
        let headstone = Uuid::now_v7();
        isle.call(move |conn| {
            conn.execute(
                "INSERT INTO persona (id, name, display_order, archived, created_at, updated_at) \
                 VALUES (?1, 'P', 0, 0, 0, 0)",
                params![persona],
            )?;
            for id in [keeper, headstone] {
                conn.execute(
                    "INSERT INTO asset (id, persona_id, source_kind, source_locator, \
                                        modality, occurred_at, created_at, updated_at) \
                     VALUES (?1, ?2, 'fs', ?3, 'tape', 0, 0, 0)",
                    params![id, persona, format!("a-{id}.md")],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

        let count_of = |views: Vec<ModalityView>| -> u64 {
            views
                .into_iter()
                .find(|v| v.def.slug.as_str() == "tape")
                .expect("the tape chip exists")
                .asset_count
        };
        assert_eq!(
            count_of(repo.list().await.unwrap()),
            2,
            "both rows count before the fold"
        );

        isle.call(move |conn| {
            let marked = conn.execute(
                "UPDATE asset SET folded_into = ?2 WHERE id = ?1",
                params![headstone, keeper],
            )?;
            assert_eq!(marked, 1, "the fixture must actually stand a headstone");
            Ok(())
        })
        .await
        .unwrap();

        assert_eq!(
            count_of(repo.list().await.unwrap()),
            1,
            "the chip counts what clicking it would show"
        );
        assert_eq!(
            repo.asset_count(&Modality::new("tape").unwrap())
                .await
                .unwrap(),
            2,
            "but the delete guard still sees the headstone holding the slug"
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn seed_rows_are_listed_with_counts_and_kinds() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteModalityRepository::new(isle.clone());

        let views = repo.list().await.unwrap();
        // Asset-model v4 (V38) left seven semantic rows — the format
        // rows (image / video / audio) and the structure rows
        // (dialogue / session) went. V42 / V43 then put the
        // conversation pair back on the semantic axis, so nine.
        assert_eq!(views.len(), 9, "seven v4 semantic rows + session + message");
        // Ordered by sort_order, coarse before fine: a session, then
        // the messages inside one.
        assert_eq!(views[0].def.slug.as_str(), "session");
        assert_eq!(views[1].def.slug.as_str(), "message");
        assert_eq!(views[2].def.slug.as_str(), "work_product");
        // tape → term with the tape cover template.
        let tape = views
            .iter()
            .find(|v| v.def.slug.as_str() == "tape")
            .unwrap();
        assert!(tape.def.terminal, "a tape reads as a terminal transcript");
        assert_eq!(tape.def.cover_template, Some(CoverTemplate::Tape));
        // `dialogue` stays gone — it was the *structural* naming, and
        // its job belongs to `asset.role` now.
        assert!(views.iter().all(|v| v.def.slug.as_str() != "dialogue"));
        // No assets yet → every count is zero.
        assert!(views.iter().all(|v| v.asset_count == 0));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn create_find_update_delete_round_trip() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = SqliteModalityRepository::new(isle.clone());

        let def = ModalityDef {
            slug: Modality::new("sketch").unwrap(),
            label: "Sketch".into(),
            terminal: false,
            sort_order: 42,
            hidden: false,
            cover_template: None,
        };
        repo.create(&def).await.unwrap();

        // Duplicate create → Conflict (409).
        let dup = repo.create(&def).await;
        assert!(matches!(dup, Err(DomainError::Conflict(_))));

        let found = repo.find(&def.slug).await.unwrap().unwrap();
        assert_eq!(found, def);

        // Partial-style full-row update: relabel + flip hidden + set
        // the terminal bit.
        let updated = ModalityDef {
            label: "Sketchpad".into(),
            terminal: true,
            hidden: true,
            cover_template: Some(CoverTemplate::WorkProduct),
            ..def.clone()
        };
        repo.update(&updated).await.unwrap();
        assert_eq!(repo.find(&def.slug).await.unwrap().unwrap(), updated);

        // Update a missing slug → NotFound.
        let missing = ModalityDef {
            slug: Modality::new("ghost").unwrap(),
            ..def.clone()
        };
        assert!(matches!(
            repo.update(&missing).await,
            Err(DomainError::NotFound { .. })
        ));

        // Delete + re-delete (the second one is a NotFound).
        repo.delete(&def.slug).await.unwrap();
        assert!(repo.find(&def.slug).await.unwrap().is_none());
        assert!(matches!(
            repo.delete(&def.slug).await,
            Err(DomainError::NotFound { .. })
        ));

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_delete_rejects_when_assets_reference_the_slug() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        // tape is a surviving semantic seed row; give it one asset.
        seed_asset(&isle, "tape").await;
        let repo = Arc::new(SqliteModalityRepository::new(isle.clone()));
        assert_eq!(
            repo.asset_count(&Modality::new("tape").unwrap())
                .await
                .unwrap(),
            1
        );

        let service = ModalityService::new(repo.clone());
        let err = service
            .delete(
                DeleteModalityCommand {
                    slug: "tape".into(),
                },
                &nobody(),
            )
            .await;
        assert!(
            matches!(err, Err(DomainError::Conflict(_))),
            "delete must be refused while an asset carries the slug"
        );
        // Still present.
        assert!(
            repo.find(&Modality::new("tape").unwrap())
                .await
                .unwrap()
                .is_some()
        );

        // An empty seed slug (tick_log, no assets) deletes cleanly.
        service
            .delete(
                DeleteModalityCommand {
                    slug: "tick_log".into(),
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert!(
            repo.find(&Modality::new("tick_log").unwrap())
                .await
                .unwrap()
                .is_none()
        );

        driver.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn service_create_and_update_surface_dtos() {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        let repo = Arc::new(SqliteModalityRepository::new(isle.clone()));
        let service = ModalityService::new(repo.clone());

        let created = service
            .create(
                CreateModalityCommand {
                    slug: "clip".into(),
                    label: "Clip".into(),
                    terminal: true,
                    sort_order: 20,
                    hidden: false,
                    cover_template: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(created.slug, "clip");
        assert!(created.terminal);
        assert_eq!(created.asset_count, 0);

        // Bad slug grammar → Validation (400). The classification set
        // is open, so the slug is the only thing left to reject here —
        // the display bit is a bool and cannot be malformed.
        let bad = service
            .create(
                CreateModalityCommand {
                    slug: "Not A Slug".into(),
                    label: "Weird".into(),
                    terminal: false,
                    sort_order: 21,
                    hidden: false,
                    cover_template: None,
                },
                &nobody(),
            )
            .await;
        assert!(matches!(bad, Err(DomainError::Validation(_))));

        // Partial update: only the label changes.
        let updated = service
            .update(
                UpdateModalityCommand {
                    slug: "clip".into(),
                    label: Some("Clips".into()),
                    terminal: None,
                    sort_order: None,
                    hidden: None,
                    cover_template: None,
                },
                &nobody(),
            )
            .await
            .unwrap();
        assert_eq!(updated.label, "Clips");
        assert!(updated.terminal, "terminal untouched by None");
        assert_eq!(updated.sort_order, 20, "sort_order untouched by None");

        driver.shutdown().await.unwrap();
    }
}
