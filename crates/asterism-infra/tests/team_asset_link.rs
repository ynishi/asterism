//! The relation a promotion leaves at home, over a real database
//! (#148 decisions 8 and 9).
//!
//! What is under test here is the half that only a database can show:
//! that the table has **no foreign key**, that a local delete is
//! therefore not refused and not cascaded, and that the pair of verbs
//! which make the relation attended rather than unattended behave —
//! `dangling_locally` finds what a delete left behind, and `reap`
//! removes link rows and nothing else.
//!
//! The other half — a promotion driving these verbs against a live
//! server — is in `teams-server/tests/member_client_e2e.rs`, and it
//! has to be there: #83 §4 puts `asterism-infra` first on the list of
//! crates the teams plane never names.

use asterism_core::domain::asset::Asset;
use asterism_core::domain::attribution::AttributionContext;
use asterism_core::domain::persona::Persona;
use asterism_core::domain::repository::{AssetLinkRepository, AssetRepository, PersonaRepository};
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind, SourceRef};
use asterism_infra::sqlite::open_and_migrate_in_memory;
use asterism_infra::sqlite::repo::{
    SqliteAssetLinkRepository, SqliteAssetRepository, SqlitePersonaRepository,
};
use chrono::Utc;
use rusqlite_isle::{AsyncIsle, AsyncIsleDriver};

struct Fixture {
    links: SqliteAssetLinkRepository,
    assets: SqliteAssetRepository,
    personas: SqlitePersonaRepository,
    isle: AsyncIsle,
    driver: AsyncIsleDriver,
}

impl Fixture {
    async fn open() -> Self {
        let (isle, driver) = open_and_migrate_in_memory().await.unwrap();
        Self {
            links: SqliteAssetLinkRepository::new(isle.clone()),
            assets: SqliteAssetRepository::new(isle.clone()),
            personas: SqlitePersonaRepository::new(isle.clone()),
            isle,
            driver,
        }
    }

    async fn persona(&self) -> PersonaId {
        let persona = Persona::new("P", None).unwrap();
        self.personas.save(&persona).await.unwrap();
        persona.id
    }

    async fn asset(&self, persona: PersonaId) -> AssetId {
        let locator = format!("/library/{}.png", uuid::Uuid::now_v7());
        let asset = Asset::new(
            persona,
            SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
            None,
            Utc::now(),
            &AttributionContext::asserted(None, None).unwrap(),
        );
        self.assets.save(&asset).await.unwrap();
        asset.id
    }

    /// Takes an Asset out of the library the way a hard delete would.
    ///
    /// Raw SQL rather than a repository verb, because what is being
    /// exercised is the *absence* of a constraint: this statement
    /// succeeding while a link row points at the deleted id is the
    /// property decision 9 asks the schema for, and a helper that went
    /// through a port would be testing the port instead.
    async fn hard_delete(&self, asset: AssetId) {
        let id = *asset.as_uuid();
        self.isle
            .call(move |conn| {
                conn.execute("DELETE FROM asset WHERE id = ?1", rusqlite::params![id])?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn rows(&self) -> i64 {
        self.isle
            .call(|conn| conn.query_row("SELECT count(*) FROM team_asset_link", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn assets_left(&self) -> i64 {
        self.isle
            .call(|conn| conn.query_row("SELECT count(*) FROM asset", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    async fn close(self) {
        self.driver.shutdown().await.unwrap();
    }
}

fn key(team: TeamScopedId, line: TeamScopedId) -> AssetLinkKey {
    AssetLinkKey {
        team_id: team,
        line_id: line,
        entry_id: TeamScopedId::new(),
    }
}

#[tokio::test]
async fn rows_are_listed_per_team_and_nothing_else_leaks_in() {
    let f = Fixture::open().await;
    let persona = f.persona().await;
    let asset = f.asset(persona).await;
    let (ours, theirs) = (TeamScopedId::new(), TeamScopedId::new());
    let line = TeamScopedId::new();

    f.links
        .record(&AssetLink::new(key(ours, line), asset, 1_000))
        .await
        .unwrap();
    f.links
        .record(&AssetLink::new(key(theirs, line), asset, 2_000))
        .await
        .unwrap();

    // One Asset, two teams, two rows — which is decision 8's "across
    // teams one Asset has as many rows as teams", and each is a weak
    // reference to its own team.
    let mine = f.links.list_for_team(ours).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].key.team_id, ours);
    assert_eq!(mine[0].local_asset_id, asset);
    assert_eq!(f.links.list_for_team(theirs).await.unwrap().len(), 1);

    f.close().await;
}

#[tokio::test]
async fn one_asset_in_one_team_answers_with_every_line_it_reached() {
    // The read `idx_team_asset_link_on_asset` exists for, asked from
    // the direction the primary key cannot serve.
    let f = Fixture::open().await;
    let persona = f.persona().await;
    let promoted = f.asset(persona).await;
    let other = f.asset(persona).await;
    let team = TeamScopedId::new();
    let elsewhere = TeamScopedId::new();
    let (first, second) = (TeamScopedId::new(), TeamScopedId::new());

    f.links
        .record(&AssetLink::new(key(team, first), promoted, 1_000))
        .await
        .unwrap();
    f.links
        .record(&AssetLink::new(key(team, second), promoted, 2_000))
        .await
        .unwrap();
    // Same Asset, another team — must not appear.
    f.links
        .record(&AssetLink::new(key(elsewhere, first), promoted, 3_000))
        .await
        .unwrap();
    // Same team, another Asset — must not appear either.
    f.links
        .record(&AssetLink::new(
            key(team, TeamScopedId::new()),
            other,
            4_000,
        ))
        .await
        .unwrap();

    let found = f.links.for_asset(team, &promoted).await.unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    let lines: Vec<TeamScopedId> = found.iter().map(|link| link.key.line_id).collect();
    assert!(lines.contains(&first) && lines.contains(&second));

    // An Asset nobody promoted here is an empty answer, not an error.
    let untouched = f.asset(persona).await;
    assert!(
        f.links
            .for_asset(team, &untouched)
            .await
            .unwrap()
            .is_empty()
    );

    f.close().await;
}

#[tokio::test]
async fn recording_the_same_promotion_twice_keeps_the_stamp_it_had() {
    // A retry of one promotion is the same fact, and the first write
    // is the one that says when it happened.
    let f = Fixture::open().await;
    let persona = f.persona().await;
    let asset = f.asset(persona).await;
    let team = TeamScopedId::new();
    let key = key(team, TeamScopedId::new());

    f.links
        .record(&AssetLink::new(key, asset, 1_000))
        .await
        .unwrap();
    f.links
        .record(&AssetLink::new(key, asset, 9_999))
        .await
        .unwrap();

    let rows = f.links.list_for_team(team).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pushed_at_ms, 1_000, "the retry did not move it");

    f.close().await;
}

#[tokio::test]
async fn deleting_a_promoted_asset_is_not_refused_and_leaves_the_row() {
    // The whole point of the missing foreign key. `RESTRICT` would
    // make this delete fail; `CASCADE` would take the row with it.
    // Neither is what decision 9 asks for — either end can vanish
    // independently, and neither may break the other.
    let f = Fixture::open().await;
    let persona = f.persona().await;
    let asset = f.asset(persona).await;
    let team = TeamScopedId::new();
    let key = key(team, TeamScopedId::new());
    f.links
        .record(&AssetLink::new(key, asset, 1_000))
        .await
        .unwrap();

    assert!(f.links.dangling_locally(team).await.unwrap().is_empty());

    f.hard_delete(asset).await;

    assert_eq!(f.rows().await, 1, "the row survived the delete");
    let dangling = f.links.dangling_locally(team).await.unwrap();
    assert_eq!(dangling.len(), 1);
    assert_eq!(dangling[0].key, key);

    f.close().await;
}

#[tokio::test]
async fn a_trashed_asset_is_not_dangling() {
    // Trash is a state the local plane can restore from, so the row
    // still corresponds to something. Gone means gone from the table.
    let f = Fixture::open().await;
    let persona = f.persona().await;
    let locator = format!("/library/{}.png", uuid::Uuid::now_v7());
    let mut asset = Asset::new(
        persona,
        SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), locator).unwrap(),
        None,
        Utc::now(),
        &AttributionContext::asserted(None, None).unwrap(),
    );
    asset.trashed_at = Some(Utc::now());
    f.assets.save(&asset).await.unwrap();

    let team = TeamScopedId::new();
    f.links
        .record(&AssetLink::new(
            key(team, TeamScopedId::new()),
            asset.id,
            1_000,
        ))
        .await
        .unwrap();

    assert!(f.links.dangling_locally(team).await.unwrap().is_empty());

    f.close().await;
}

#[tokio::test]
async fn reap_removes_link_rows_and_touches_nothing_else() {
    let f = Fixture::open().await;
    let persona = f.persona().await;
    let doomed = f.asset(persona).await;
    let kept = f.asset(persona).await;
    let team = TeamScopedId::new();
    let doomed_key = key(team, TeamScopedId::new());
    let kept_key = key(team, TeamScopedId::new());

    f.links
        .record(&AssetLink::new(doomed_key, doomed, 1_000))
        .await
        .unwrap();
    f.links
        .record(&AssetLink::new(kept_key, kept, 2_000))
        .await
        .unwrap();

    assert_eq!(f.links.reap(&[doomed_key]).await.unwrap(), 1);

    // The named row, and only it.
    let left = f.links.list_for_team(team).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].key, kept_key);

    // And nothing either row pointed at. A relation tidying itself up
    // must not be a path by which either end loses something.
    assert_eq!(f.assets_left().await, 2);
    assert!(f.assets.find(&doomed).await.unwrap().is_some());

    f.close().await;
}

#[tokio::test]
async fn reaping_a_row_that_is_not_there_removes_nothing_and_says_so() {
    let f = Fixture::open().await;
    let team = TeamScopedId::new();
    assert_eq!(f.links.reap(&[]).await.unwrap(), 0);
    assert_eq!(
        f.links
            .reap(&[key(team, TeamScopedId::new())])
            .await
            .unwrap(),
        0
    );
    f.close().await;
}
