//! Seeding a team's line from a private one (#148 decision 11).
//!
//! The other direction from a clone, and it transfers the current state
//! for the same reason: what a team is being given is something to work
//! on, not somebody's record of how they got there.
//!
//! ## Two seedings, and only one of them is free
//!
//! [`Seeding::CurrentState`] is the default and the cheap one. The
//! team's line gets a genesis and one change point, holding what the
//! private line holds now.
//!
//! [`Seeding::Reenactment`] replays the chain — one pursuit and one
//! close per change point, so the team's line ends with as many change
//! points as the private one had. **It is chosen at init and nowhere
//! else**: a line that was seeded with its current state cannot be
//! given its history afterwards, because the history would have to
//! arrive underneath change points that already exist.
//!
//! What it costs is worth saying rather than leaving to be discovered.
//! A re-enactment sends **every content the line ever named** — the set
//! [`Line::holds`] describes, which only ever grows and includes
//! everything an entry was replaced with and everything taken back off
//! the line. Seeding the current state sends what is on the line now,
//! which is usually far less.
//!
//! The cheaper seeding is also the narrower one, in a way that is
//! nobody's guess: it cannot take a line whose live entries share
//! content, because a promotion's repeat check is keyed on the asset
//! and the line, so the second entry would be answered from the first
//! and the team would receive one where the private line has two.
//! `seeds` refuses that outright rather than narrowing it silently, and
//! a re-enactment takes it, because a chain names each entry in its own
//! right. Both refusals happen before the team's line is opened — see
//! `seeds` for the ordering and for the one failure it cannot cover.
//!
//! ## Why it is a re-enactment and not a history
//!
//! The acts are restamped to whoever published. The original actors are
//! not necessarily members of this team, so there is nobody on the team
//! plane for the old stamps to name — and inventing one would be the
//! team's record claiming knowledge it does not have. So the team's
//! line does not record who did the work upstream, and that is not a
//! hole: at this boundary the question is who brought this here, and
//! the restamped act answers exactly that. Who made it before is the
//! private side's record, which #66 decision 2 says the team never had
//! a claim on.
//!
//! The word for that is **re-enactment**, and it is in the type, in
//! [`Published::reenacted`], and in what the UI says.
//!
//! ## What does not travel at all
//!
//! Work logs and conversations, and they are not offered at init
//! either. A pursuit that was abandoned, a round that was pushed and
//! thought better of, a thread arguing about it — those are the private
//! deliberation #66 decision 2 protects. Nothing in this module reads
//! the private line's pursuits: the seeding walks the *line's* chain,
//! which is what was landed, and the work that produced it is not
//! reachable from here.

use std::collections::BTreeMap;

use asterism_contract::forge::ForgeOpDto;
use asterism_core::domain::asset::Asset;
use asterism_core::domain::forge::model::line::Line;
use asterism_core::domain::forge::model::table::Row;
use asterism_core::domain::forge::model::value::{Content, EntryId, Existence};
use asterism_core::domain::repository::AssetLinkRepository;
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};
use asterism_core::domain::value::AssetId;
use asterism_core::error::DomainError;

use crate::client::{TeamsClient, TeamsClientError};
use crate::mapper::{LocalSubject, PromotedMark, projection_body};
use crate::promotion::{Promotion, hash_at_promote_time, promote, sole_material};
use asterism_teams_wire::projection::{EntryProjectionEnvelope, PROJECTION_VERSION};

/// How much of a private line the team's copy is given.
///
/// Chosen when the team's line is opened and never afterwards — see the
/// module doc for why there is no later verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Seeding {
    /// What the line holds now, as a genesis and one change point.
    #[default]
    CurrentState,
    /// Every change point replayed as its own pursuit and close: a
    /// **re-enactment**, with the acts restamped to the publisher and
    /// every content the line ever named sent.
    Reenactment,
}

impl Seeding {
    /// Whether this seeding is the re-enactment.
    pub const fn is_reenactment(self) -> bool {
        matches!(self, Self::Reenactment)
    }
}

/// A private line, and the team to seed from it.
#[derive(Debug, Clone, Copy)]
pub struct Publication<'a> {
    /// The team that will host the new line.
    pub team_id: TeamScopedId,
    /// The private line to seed from. Read whole, and never written.
    pub line: &'a Line,
    /// What to call the team's line. Not taken from the private line's
    /// name: where a name has to be unique is the hosting layer's
    /// question, and the answer there is not the answer here.
    pub named: &'a str,
    /// The rule the team's line answers a collision with, by slug.
    pub strategy_id: &'a str,
    /// How much of the chain to give it.
    pub seeding: Seeding,
}

/// One local asset a line names, with what may be said about it.
///
/// Owned rather than borrowed because a publication resolves these one
/// at a time as it walks, and a borrow would ask the caller to hold
/// every asset a line ever named in memory at once — which for a
/// re-enactment is the whole of [`Line::holds`].
#[derive(Debug, Clone)]
pub struct HeldSubject {
    /// The asset itself.
    pub asset: Asset,
    /// The marks a person wrote on it. Gathered by the caller, because
    /// [`PromotedMark`] can only be built by gathering — which is what
    /// makes "a person wrote this" a property of the type rather than a
    /// claim.
    pub user_marks: Vec<PromotedMark>,
}

/// What a publication reads out of the local plane.
///
/// One question, asked per content. The forge holds asset ids and
/// nothing else; turning one into something with bytes and marks is the
/// layer below's business, and this is the seam.
#[async_trait::async_trait]
pub trait Holdings: Send + Sync {
    /// The local asset this content is, with the marks a person wrote.
    async fn subject(&self, content: AssetId) -> Result<HeldSubject, DomainError>;
}

/// What a publication left on the team.
#[derive(Debug, Clone)]
pub struct Published {
    /// The team's new line.
    pub line_id: TeamScopedId,
    /// Whether the chain was re-enacted. `false` is the current-state
    /// seeding, whose line has one change point whatever the private
    /// line's history was.
    pub reenacted: bool,
    /// How many change points the team's line now has, not counting its
    /// genesis.
    pub change_points: usize,
    /// How many times content was sent — once per row that named some.
    ///
    /// Not the size of [`Line::holds`], which is a set: a line that
    /// replaced an entry back to something it already held sends twice
    /// and holds once, so this counts sends rather than distinct
    /// contents. For a re-enactment it is at least that set's size, and
    /// that is the cost the module doc is about.
    pub contents_sent: usize,
    /// The team entry each local entry became, for the entries the
    /// team's line ends up holding.
    pub entries: BTreeMap<EntryId, TeamScopedId>,
}

/// One entry the current-state seeding will put on the team's line.
struct Seed {
    entry: EntryId,
    named: String,
    content: Content,
}

/// What the current state will cost, worked out before anything exists.
///
/// The order matters and it is the whole of this function's reason. A
/// publication that opened the team's line first and then met a refusal
/// would leave a line on somebody's team that nothing had been put on —
/// and unlike a local line, that is not the publisher's to tidy away.
/// So everything that can be known from the private line alone is asked
/// here, before the first write.
///
/// **What this cannot cover**, and what is therefore a stated limitation
/// rather than a hidden one: a refusal from the far side part-way
/// through — a material that has moved, a connection that drops — still
/// leaves a team line holding some of what was meant for it. There is no
/// transaction across the two planes and this issue does not invent one.
/// The line is a line like any other and a member can discard it.
fn seeds(publication: &Publication<'_>) -> Result<Vec<Seed>, TeamsClientError> {
    let mut seeds: Vec<Seed> = Vec::new();
    for (entry, state) in publication.line.states() {
        if !state.alive {
            continue;
        }
        let Some(content) = state.content else {
            continue;
        };
        let named = state
            .name
            .as_ref()
            .map(|name| name.as_str())
            .ok_or_else(|| {
                local(format!(
                    "entry {entry} is on the line with content and no name, which the team's \
                 line has no way to say"
                ))
            })?;
        // Two live entries on one content. A promotion's repeat check
        // is keyed on the asset and the line (#148 decision 8), so the
        // second would be answered from the row the first wrote and the
        // team would quietly get one entry where the private line has
        // two. Refused rather than silently narrowed — and refused
        // here, where nothing has been created yet.
        if let Some(first) = seeds.iter().find(|seed| seed.content == content) {
            return Err(local(format!(
                "entries {} and {entry} are both on the line holding the same content, \
                 and a promotion answers the second from the first — so the team would \
                 receive one of them. Publishing a line whose live entries share content \
                 is not supported; re-enacting it is, because the chain names each entry \
                 in its own right",
                first.entry
            )));
        }
        seeds.push(Seed {
            entry,
            named: named.to_string(),
            content,
        });
    }
    if seeds.is_empty() {
        return Err(local(
            "this line holds nothing, and a change point that says nothing cannot be \
             written; there is nothing to seed a team's line with"
                .to_string(),
        ));
    }
    Ok(seeds)
}

/// Opens a line on the team and seeds it from a private one.
pub async fn publish(
    client: &TeamsClient,
    links: &dyn AssetLinkRepository,
    holdings: &dyn Holdings,
    publication: Publication<'_>,
    now_ms: i64,
) -> Result<Published, TeamsClientError> {
    // Before the team has a line: see `seeds` for why this is not
    // inside the seeding it belongs to.
    let planned = if publication.seeding.is_reenactment() {
        None
    } else {
        Some(seeds(&publication)?)
    };

    let opened = client
        .open_line(
            publication.team_id,
            publication.named,
            publication.strategy_id,
        )
        .await?;
    let team_line = TeamScopedId::parse(&opened.id, "line id")?;

    match planned {
        Some(planned) => {
            seed_current_state(
                client,
                links,
                holdings,
                &publication,
                team_line,
                planned,
                now_ms,
            )
            .await
        }
        None => reenact(client, links, holdings, &publication, team_line, now_ms).await,
    }
}

/// The cheap seeding: what the line holds now, as one change point.
///
/// Each entry goes over as a promotion, which is the same act with the
/// same ordering — content, then the round that names it, then the row
/// at home saying this machine put it there. A second spelling of that
/// would be a second answer to when a link row may exist.
#[allow(clippy::too_many_arguments)]
async fn seed_current_state(
    client: &TeamsClient,
    links: &dyn AssetLinkRepository,
    holdings: &dyn Holdings,
    publication: &Publication<'_>,
    team_line: TeamScopedId,
    planned: Vec<Seed>,
    now_ms: i64,
) -> Result<Published, TeamsClientError> {
    let pursuit = TeamScopedId::parse(
        &client
            .open_pursuit(
                publication.team_id,
                team_line,
                Some("seeding the line"),
                None,
            )
            .await?
            .id,
        "pursuit id",
    )?;

    let mut entries = BTreeMap::new();
    let mut contents_sent = 0usize;
    for seed in planned {
        let held = holdings
            .subject(asset_of(seed.content))
            .await
            .map_err(TeamsClientError::Local)?;
        let outcome = promote(
            client,
            links,
            Promotion {
                team_id: publication.team_id,
                line_id: team_line,
                pursuit_id: pursuit,
                subject: LocalSubject {
                    asset: &held.asset,
                    user_marks: &held.user_marks,
                },
                named: &seed.named,
            },
            now_ms,
        )
        .await?;
        // A promotion that answered from an existing row sent nothing,
        // and this counts sends. On a line this call just opened there
        // is no such row to answer from — `seeds` refused the one shape
        // that could produce one — so this is a guard rather than a
        // case, and it is here because the count is reported.
        if !outcome.already_promoted {
            contents_sent += 1;
        }
        entries.insert(seed.entry, outcome.key.entry_id);
    }

    client
        .close_pursuit(publication.team_id, pursuit, "satisfied", None)
        .await?;

    Ok(Published {
        line_id: team_line,
        reenacted: false,
        change_points: 1,
        contents_sent,
        entries,
    })
}

/// The re-enactment: one pursuit and one close per change point.
///
/// The local entry ids do not cross — the team's line mints its own,
/// and [`Standing`] is what keeps a replacement on the fifth change
/// point landing on the entry the first one added rather than beside
/// it. That running fold is the whole reason this cannot be a loop over
/// independent promotions.
async fn reenact(
    client: &TeamsClient,
    links: &dyn AssetLinkRepository,
    holdings: &dyn Holdings,
    publication: &Publication<'_>,
    team_line: TeamScopedId,
    now_ms: i64,
) -> Result<Published, TeamsClientError> {
    let mut standings: BTreeMap<EntryId, Standing> = BTreeMap::new();
    let mut contents_sent = 0usize;
    let mut change_points = 0usize;

    for point in publication.line.history().changes() {
        let pursuit = TeamScopedId::parse(
            &client
                .open_pursuit(
                    publication.team_id,
                    team_line,
                    Some("re-enacting a change point"),
                    None,
                )
                .await?
                .id,
            "pursuit id",
        )?;

        let mut ops = Vec::new();
        let mut projections = Vec::new();
        for (entry, row) in point.table().rows() {
            let (mut written, sent) = replay_row(
                client,
                holdings,
                publication,
                pursuit,
                *entry,
                row,
                &mut standings,
                &mut projections,
            )
            .await?;
            if sent {
                contents_sent += 1;
            }
            ops.append(&mut written);
        }

        // A change point whose table said nothing cannot exist — the
        // model refuses an empty round — so there is nothing to guard
        // against here beyond letting the refusal speak if one ever
        // did.
        client
            .push_round(
                publication.team_id,
                pursuit,
                ops,
                Some(REENACTED),
                projections,
            )
            .await?;
        client
            .close_pursuit(publication.team_id, pursuit, "satisfied", None)
            .await?;
        change_points += 1;
    }

    // Last, and once. `record` keeps the first row written for a key,
    // and a re-enacted entry is written several times over — so the
    // correspondence is recorded after the walk, from what the team's
    // line ended up holding, rather than from whatever the entry was
    // first.
    let mut entries = BTreeMap::new();
    for (entry, state) in publication.line.states() {
        if !state.alive {
            continue;
        }
        let Some(standing) = standings.get(&entry) else {
            continue;
        };
        let (Some(team_entry), Some(asset)) = (standing.team_entry, standing.asset) else {
            continue;
        };
        links
            .record(&AssetLink::new(
                AssetLinkKey {
                    team_id: publication.team_id,
                    line_id: team_line,
                    entry_id: team_entry,
                },
                asset,
                now_ms,
            ))
            .await?;
        entries.insert(entry, team_entry);
    }

    Ok(Published {
        line_id: team_line,
        reenacted: true,
        change_points,
        contents_sent,
        entries,
    })
}

/// Where one entry stands, as the re-enactment walks the chain.
///
/// A row states only the axes its change point moved, so a row on its
/// own does not say what the entry is — a revival states existence
/// alone, and a table that both replaced and renamed an entry folds to
/// a row stating neither existence. Re-enacting therefore needs the
/// same running fold a reader of the chain keeps, and this is it.
///
/// The first version of this walked a row's three axes as a fixed set
/// of shapes and refused anything else. That was wrong twice over: it
/// refused a revival, which `Op::add_to` exists to write, and it
/// refused a replace and a rename landing together, which `op::fold`
/// emits from one pursuit. Both are ordinary histories, and both were
/// reported to the publisher as a malformed line.
#[derive(Debug, Default, Clone)]
struct Standing {
    /// The team's id for this entry, minted the first time the entry
    /// goes onto the team's line. The local id never crosses.
    team_entry: Option<TeamScopedId>,
    /// Whether the entry is on the line as of the last row read.
    alive: bool,
    /// The team asset the entry currently holds. Remembered because a
    /// revival puts an entry back without restating its content, and
    /// the team's `add` has to name one.
    team_asset: Option<TeamScopedId>,
    /// What the entry currently answers to, for the same reason.
    name: Option<String>,
    /// The local asset behind the content, for the link row at the end.
    asset: Option<AssetId>,
}

/// Turns one row of a private change point into the ops that reproduce
/// it on the team's line.
///
/// A row can need more than one op — replace and rename land together
/// in a single table — and a round takes several ops on one entry and
/// folds them, which is the same fold that produced this row.
#[allow(clippy::too_many_arguments)]
async fn replay_row(
    client: &TeamsClient,
    holdings: &dyn Holdings,
    publication: &Publication<'_>,
    pursuit: TeamScopedId,
    entry: EntryId,
    row: &Row,
    standings: &mut BTreeMap<EntryId, Standing>,
    projections: &mut Vec<EntryProjectionEnvelope>,
) -> Result<(Vec<ForgeOpDto>, bool), TeamsClientError> {
    let standing = standings.entry(entry).or_default();
    let was_alive = standing.alive;
    let team_entry = *standing.team_entry.get_or_insert_with(TeamScopedId::new);
    if let Some(name) = row.name() {
        standing.name = Some(name.as_str().to_string());
    }

    // Content is sent when the row states one, and only then. A
    // revival that restates nothing is putting back what the team
    // already holds.
    let mut sent = false;
    if let Some(content) = row.content() {
        let asset = asset_of(content);
        let team_asset = send_content(
            client,
            holdings,
            publication,
            pursuit,
            asset,
            team_entry,
            projections,
        )
        .await?;
        let standing = standings.get_mut(&entry).expect("just inserted");
        standing.team_asset = Some(team_asset);
        standing.asset = Some(asset);
        sent = true;
    }

    let standing = standings.get_mut(&entry).expect("just inserted");
    let now_alive = match row.existence() {
        Some(Existence::Present) => true,
        Some(Existence::Absent) => false,
        // The axis was left alone, so the entry is where it was. An
        // entry's first row always states existence, because a fold
        // that put one on emits an add.
        None => was_alive,
    };
    standing.alive = now_alive;

    let mut ops = Vec::new();
    match (was_alive, now_alive) {
        // Off it goes. `Row::new` refuses a removal that also names or
        // fills, so there is nothing else in this row to carry.
        (true, false) => ops.push(ForgeOpDto {
            entry_id: team_entry.to_string(),
            kind: "remove".to_string(),
            content_asset_id: None,
            name: None,
        }),
        // On it goes — the entry's first arrival, or a revival. Both
        // are an `add` on the wire, which names content and name
        // whether or not this row restated them.
        (false, true) => {
            let content = standing.team_asset.ok_or_else(|| {
                local(format!(
                    "entry {entry} goes onto the line holding nothing, and the team's \
                     line has no way to say that"
                ))
            })?;
            let name = standing.name.clone().ok_or_else(|| {
                local(format!(
                    "entry {entry} goes onto the line unnamed, and the team's line has \
                     no way to say that"
                ))
            })?;
            ops.push(ForgeOpDto {
                entry_id: team_entry.to_string(),
                kind: "add".to_string(),
                content_asset_id: Some(content.to_string()),
                name: Some(name),
            });
        }
        // Still on it, or still off it: whatever axes this row moved,
        // moved. Both can be in one row, and both can be in one round.
        _ => {
            if row.content().is_some()
                && let Some(content) = standing.team_asset
            {
                ops.push(ForgeOpDto {
                    entry_id: team_entry.to_string(),
                    kind: "replace".to_string(),
                    content_asset_id: Some(content.to_string()),
                    name: None,
                });
            }
            if let Some(name) = row.name() {
                ops.push(ForgeOpDto {
                    entry_id: team_entry.to_string(),
                    kind: "rename".to_string(),
                    content_asset_id: None,
                    name: Some(name.as_str().to_string()),
                });
            }
        }
    }

    Ok((ops, sent))
}

/// Puts one content into the team and answers with the team's asset id.
///
/// The same three steps a promotion takes, minus the link row: the
/// correspondence for a re-enacted entry is settled at the end, from
/// what the line ended up holding.
async fn send_content(
    client: &TeamsClient,
    holdings: &dyn Holdings,
    publication: &Publication<'_>,
    pursuit: TeamScopedId,
    asset: AssetId,
    team_entry: TeamScopedId,
    projections: &mut Vec<EntryProjectionEnvelope>,
) -> Result<TeamScopedId, TeamsClientError> {
    let held = holdings
        .subject(asset)
        .await
        .map_err(TeamsClientError::Local)?;
    let material = sole_material(&held.asset)?;
    let path = material.locator.local_path().ok_or_else(|| {
        DomainError::Validation(format!(
            "re-enacting reads the material's bytes, and asset {}'s material is not a \
             local file: {}",
            held.asset.id,
            material.locator.to_display()
        ))
    })?;
    let digest = hash_at_promote_time(path).await?;
    let entered = client
        .enter_content(publication.team_id, pursuit, &digest, path)
        .await?;

    // What the promoter says about the entry is what it is *now*, which
    // on a re-enactment is stated afresh each time the entry's content
    // moves. The team keeps what was said at the time (#148 decision
    // 12), and on this path the time is the replay.
    if let Some(body) = projection_body(&LocalSubject {
        asset: &held.asset,
        user_marks: &held.user_marks,
    })? {
        projections.push(EntryProjectionEnvelope {
            entry_id: team_entry.to_string(),
            version: PROJECTION_VERSION,
            body,
        });
    }

    TeamScopedId::parse(&entered.asset_id, "team asset id").map_err(TeamsClientError::Local)
}

/// The note a re-enacted round carries.
///
/// It says the word, because a reader of the team's line is looking at
/// a round whose act names the publisher and whose content somebody
/// else may have made. It names no change point: the id it came from is
/// on a line this team cannot read, so it would answer nothing.
const REENACTED: &str = "re-enacted from a private line";

/// The asset id behind a content reference.
///
/// The forge keeps this one-way on purpose — a `Content` is the only
/// reference it holds downward, and reaching the referent is the
/// boundary's business. Publishing is on the boundary: it is the thing
/// that turns what a line names into bytes to send.
fn asset_of(content: Content) -> AssetId {
    AssetId::from_uuid(*content.as_uuid())
}

fn local(message: String) -> TeamsClientError {
    TeamsClientError::Local(DomainError::Validation(message))
}
