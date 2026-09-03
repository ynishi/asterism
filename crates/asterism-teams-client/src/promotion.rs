//! Handing an Asset over (#148 decisions 3, 4, 5, 7 and 8).
//!
//! ## One act, five steps, in this order
//!
//! 1. **Gather what cannot be re-derived** — the material's bytes, and
//!    the marks whose layer origin is `User`. That filter is decision
//!    4's and it lives in
//!    [`PromotedMark::gather`](crate::mapper::PromotedMark::gather);
//!    thumbnails, indexed bodies and `Imported`/`Machine` marks stay
//!    home because the receiving side can make them again.
//! 2. **Ask what the team already has**, so a send can be skipped when
//!    the answer allows it — see the note on the have-check below.
//! 3. **Enter the content against open work.** The team mints a
//!    `TeamAsset` for it, one per promotion (decision 7), so two
//!    members bringing identical bytes get one each and "who brought
//!    what" survives the second contributor. The work is the caller's
//!    to have opened: none is opened here, for the reason
//!    [`Promotion::pursuit_id`] gives.
//! 4. **Push a round that names it**, with the entry id this client
//!    minted (decision 8) and the projection riding along (decision
//!    12). The content is there before the round names it, which is
//!    decision 5's ordering.
//! 5. **Record the relation at home** — and only at home. The server
//!    holds no reference to the local Asset, in either direction.
//!
//! The order is not an implementation detail. Content before round is
//! decision 5; link after both is what keeps a link row from ever
//! naming an entry that was never written.
//!
//! ## What v0 will promote
//!
//! **One Asset, one local material.** Decision 3 says the team holds a
//! conversion and deliberately does not fix its composition — an Asset
//! may be several materials, or a Collection whose content is the
//! assets pointing at it. The teams plane's schema already admits
//! those: `team_asset.digest` is nullable precisely so a conversion
//! composed some other way can leave it empty.
//!
//! What is missing is the composition itself — which parts land in the
//! CAS and which in rows — and that is a design question decision 3
//! leaves open rather than a gap in this function. So a Collection and
//! a multi-material Asset are **refused with a message that says
//! which**, rather than promoted as one of their parts. Promoting one
//! part of an Asset and calling it the Asset would be worse than
//! refusing: the receiving side could not reproduce it, and decision 4
//! is precisely the rule that what travels must be enough to.
//!
//! ## The have-check, honestly
//!
//! Decision 19 adds it "to avoid re-sending". With the transport as
//! #151 built it, that saving is only partly available, and it is
//! worth stating rather than papering over.
//!
//! The content verb is the only thing that mints a `TeamAsset`, and it
//! mints one from bytes — there is no verb that mints an asset over a
//! digest the team already holds. Decision 7 requires the mint on
//! every promotion, so a second member promoting identical bytes still
//! calls the verb and still sends the body.
//!
//! What is available on the other axis is the one that matters day to
//! day: **a repeat of the same promotion sends nothing at all.** Before
//! anything is uploaded, [`promote`] asks the relation whether this
//! client already promoted this Asset onto this line, and a client
//! that did is answered from its own machine — without the have-check,
//! which is why [`PromotionOutcome::bytes_already_held`] is `None`
//! there. On the path that does send, the digest answer is reported so
//! a caller can show it, and the day a mint-over-held-digest verb
//! exists this function skips the body on it too.

use std::path::Path;

use asterism_contract::digest::ContentHasher;
use asterism_contract::forge::{ForgeOpDto, ForgePursuitDto};
use asterism_core::domain::asset::Asset;
use asterism_core::domain::material::Material;
use asterism_core::domain::repository::AssetLinkRepository;
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};
use asterism_core::domain::value::AssetRole;
use asterism_core::error::DomainError;
use asterism_teams_wire::projection::{EntryProjectionEnvelope, PROJECTION_VERSION};
use tokio::io::AsyncReadExt as _;

use crate::client::{TeamsClient, TeamsClientError};
use crate::mapper::{LocalSubject, projection_body};

/// One promotion, described.
#[derive(Debug, Clone, Copy)]
pub struct Promotion<'a> {
    /// The team hosting the line.
    pub team_id: TeamScopedId,
    /// The line the entry lands on.
    pub line_id: TeamScopedId,
    /// The open work the content enters against (#148 decision 5).
    ///
    /// Always somebody's: a pursuit is the record that a person chose
    /// to start work, and this function does not open one on a
    /// caller's behalf. It was asked to (#219) and refused, because a
    /// pursuit opened as a step of a promotion the team then refuses
    /// is a record of a decision nobody made — the orphan of a
    /// transaction that did not complete — and the forge has no verb
    /// that takes such a record back without recording a second
    /// decision. Opening work for an entry from the asset's pane is an
    /// act of its own there, pressed by the person.
    pub pursuit_id: TeamScopedId,
    /// The Asset and the marks a person wrote on it.
    pub subject: LocalSubject<'a>,
    /// What the entry answers to on the line.
    ///
    /// Stated by the caller rather than taken from the Asset's title:
    /// a line's names are the team's namespace, and what a person
    /// called something in their own library is not automatically what
    /// it should be called on somebody else's line. The title travels
    /// too, in the projection, where it reads as what the promoter
    /// said rather than as the line's own name for the entry.
    pub named: &'a str,
}

/// What a promotion left behind, on both sides.
#[derive(Debug, Clone)]
pub struct PromotionOutcome {
    /// The relation row's key — the three ids this client minted or
    /// took from the team (#148 decision 8).
    pub key: AssetLinkKey,
    /// The `TeamAsset` the team minted, held as an opaque handle and
    /// never read as a local `AssetId` (#148 decision 6).
    ///
    /// Absent when the promotion was a repeat and nothing was sent.
    pub team_asset_id: Option<TeamScopedId>,
    /// The digest the material hashed to at promote time.
    pub digest: String,
    /// Whether the team already held those bytes when asked, or
    /// nothing when the question was not put.
    ///
    /// `None` on a repeat, where the promotion was answered from this
    /// machine and no send was ever going to happen. The distinction
    /// is worth a type because this is a value a caller shows a
    /// person: "the team already has these" and "nobody checked" are
    /// different statements, and a `bool` can only make the first.
    /// Reported rather than acted on — see the module doc.
    pub bytes_already_held: Option<bool>,
    /// Whether this client had already promoted this Asset onto this
    /// line, in which case nothing was sent and nothing was written.
    ///
    /// **A repeat does not update anything.** If the description
    /// changed since, that change does not reach the team through this
    /// function: replacing a projection is a forge op (decision 12),
    /// so it needs a round, and pushing one that carries no operation
    /// is not something the forge accepts. Bringing an edited
    /// description across is an act #152 does not build.
    pub already_promoted: bool,
    /// The work as it now reads, or nothing on a repeat.
    pub pursuit: Option<ForgePursuitDto>,
}

/// Runs a promotion.
///
/// `now_ms` is the caller's clock, so that a caller batching several
/// promotions can stamp them alike and a test can stamp them at all.
pub async fn promote(
    client: &TeamsClient,
    links: &dyn AssetLinkRepository,
    promotion: Promotion<'_>,
    now_ms: i64,
) -> Result<PromotionOutcome, TeamsClientError> {
    let asset = promotion.subject.asset;
    let material = sole_material(asset)?;
    let path = material.locator.local_path().ok_or_else(|| {
        DomainError::Validation(format!(
            "promoting reads the material's bytes, and asset {}'s material is not a local \
             file: {}",
            asset.id,
            material.locator.to_display()
        ))
    })?;

    // Re-read the file rather than trusting the stored hash. The
    // declared digest is what the server verifies while it writes, and
    // a stored hash that has gone stale would be refused as a mismatch
    // after the whole body had been sent.
    let digest = hash_at_promote_time(path).await?;

    // A repeat of the same promotion, answered from this machine and
    // before an entry id is minted — one handed out here and then
    // discarded would be a promotion this machine half-began.
    let already = links
        .for_asset(promotion.team_id, &asset.id)
        .await?
        .into_iter()
        .find(|link| link.key.line_id == promotion.line_id);
    if let Some(link) = already {
        return Ok(PromotionOutcome {
            key: link.key,
            team_asset_id: None,
            digest,
            // Not asked, and it must not claim otherwise. Nothing was
            // going to be sent, so nothing needed to know — and a
            // purge may have taken those bytes since the promotion
            // this row records.
            bytes_already_held: None,
            already_promoted: true,
            pursuit: None,
        });
    }

    let entry_id = TeamScopedId::new();
    let held = client
        .have_content(promotion.team_id, vec![digest.clone()])
        .await?;
    let bytes_already_held = Some(held.held.iter().any(|one| one == &digest));

    let entered = client
        .enter_content(promotion.team_id, promotion.pursuit_id, &digest, path)
        .await?;
    let team_asset = TeamScopedId::parse(&entered.asset_id, "team asset id")?;

    let projections = match projection_body(&promotion.subject)? {
        Some(body) => vec![EntryProjectionEnvelope {
            entry_id: entry_id.to_string(),
            version: PROJECTION_VERSION,
            body,
        }],
        None => Vec::new(),
    };

    let pursuit = client
        .push_round(
            promotion.team_id,
            promotion.pursuit_id,
            vec![ForgeOpDto {
                entry_id: entry_id.to_string(),
                kind: "add".to_string(),
                content_asset_id: Some(team_asset.to_string()),
                name: Some(promotion.named.to_string()),
            }],
            None,
            projections,
        )
        .await?;

    // Last, and only here. A link row that named an entry no round had
    // written would be a promotion this machine believes in and the
    // team has never heard of.
    let key = AssetLinkKey {
        team_id: promotion.team_id,
        line_id: promotion.line_id,
        entry_id,
    };
    links.record(&AssetLink::new(key, asset.id, now_ms)).await?;

    Ok(PromotionOutcome {
        key,
        team_asset_id: Some(team_asset),
        digest,
        bytes_already_held,
        already_promoted: false,
        pursuit: Some(pursuit),
    })
}

/// The one material a v0 promotion carries, or a refusal that says why
/// this Asset is not one (#148 decision 3 — see the module doc).
///
/// Crate-visible because publishing sends the same materials under the
/// same rule: a private line seeding a team one hands over each content
/// it names, and "what a team holds for a Collection" is the same
/// unanswered question there as here. Two spellings of it would be two
/// answers.
pub(crate) fn sole_material(asset: &Asset) -> Result<&Material, DomainError> {
    if asset.role == AssetRole::Collection {
        return Err(DomainError::Validation(format!(
            "asset {} is a Collection, whose content is the assets pointing at it rather \
             than a material of its own; what a team holds for one is a conversion whose \
             composition #148 decision 3 leaves open, and promoting one part of it would \
             hand the team something it cannot reproduce the Asset from",
            asset.id
        )));
    }
    match asset.materials.as_slice() {
        [one] => Ok(one),
        [] => Err(DomainError::Validation(format!(
            "asset {} has no material, and what must travel is what cannot be re-derived \
             (#148 decision 4)",
            asset.id
        ))),
        many => Err(DomainError::Validation(format!(
            "asset {} has {} materials, and a promotion carries one; a conversion composed \
             of several is admitted by the team's schema and not yet composed by anything \
             (#148 decision 3)",
            asset.id,
            many.len()
        ))),
    }
}

/// Hashes a file the way the wire spells digests, in chunks.
///
/// Crate-visible for the reason [`sole_material`] is, and carrying the
/// same rule with it: the file is read now rather than trusted from a
/// stored hash, because the declared digest is what the server verifies
/// while it writes and a stale one is refused after the whole body has
/// gone.
pub(crate) async fn hash_at_promote_time(path: &Path) -> Result<String, DomainError> {
    let mut file = tokio::fs::File::open(path).await.map_err(|err| {
        DomainError::Infra(anyhow::anyhow!(
            "reading {} to compute its digest: {err}",
            path.display()
        ))
    })?;
    let mut hasher = ContentHasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await.map_err(|err| {
            DomainError::Infra(anyhow::anyhow!("reading {}: {err}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::attribution::AttributionContext;
    use asterism_core::domain::value::{PersonaId, SourceKind, SourceRef};
    use chrono::Utc;

    /// One ordinary asset: one material, one local file.
    ///
    /// The material is added rather than assumed — `Asset::new` records
    /// the source and leaves the materials to the ingest path, so an
    /// asset fresh from the constructor is the *no material* case.
    fn an_asset() -> Asset {
        let source = SourceRef::new(SourceKind::new(SourceKind::FS).unwrap(), "/tmp/x.png")
            .expect("a source");
        let mut asset = Asset::new(
            PersonaId::new(),
            source.clone(),
            None,
            Utc::now(),
            &AttributionContext::asserted(None, None).expect("stating nobody is always valid"),
        );
        asset
            .materials
            .push(Material::primary(source.locator, Some(1), Utc::now()));
        asset
    }

    /// What a v0 promotion carries, over the asset it is meant for.
    #[test]
    fn one_material_is_what_a_promotion_carries() {
        let asset = an_asset();
        assert!(sole_material(&asset).is_ok(), "an ordinary asset promotes");
    }

    // The three refusals below are pinned on their messages because the
    // surfaces above this crate deliberately do not pre-empt them: a
    // pane hands the client an asset and shows whatever comes back, so
    // what these say *is* what a person reads. A refusal that stopped
    // naming which case it was would leave them with "no" and nowhere
    // to go.

    #[test]
    fn a_collection_is_refused_as_one() {
        let mut asset = an_asset();
        asset.role = AssetRole::Collection;

        let said = sole_material(&asset)
            .expect_err("a Collection is refused")
            .to_string();

        assert!(
            said.contains("Collection"),
            "the refusal says which case this is: {said}"
        );
        // And why it is undecided rather than unsupported, which is
        // what makes it an answer instead of a wall.
        assert!(
            said.contains("decision 3"),
            "the refusal points at the decision that leaves the composition open: {said}"
        );
    }

    #[test]
    fn several_materials_are_refused_with_their_count() {
        let mut asset = an_asset();
        let one = asset.materials[0].clone();
        asset.materials.push(one);

        let said = sole_material(&asset)
            .expect_err("a multi-material asset is refused")
            .to_string();

        assert!(
            said.contains("2 materials"),
            "the refusal says how many it found: {said}"
        );
        assert!(
            said.contains("decision 3"),
            "and why one of them is not the answer: {said}"
        );
    }

    #[test]
    fn nothing_to_send_is_refused_as_that() {
        let mut asset = an_asset();
        asset.materials.clear();

        let said = sole_material(&asset)
            .expect_err("an asset with no material is refused")
            .to_string();

        assert!(
            said.contains("no material"),
            "the refusal says what is missing: {said}"
        );
        // Decision 4 rather than 3: this one is not about composition,
        // it is about there being nothing that cannot be re-derived.
        assert!(
            said.contains("decision 4"),
            "and which rule makes that fatal: {said}"
        );
    }
}
