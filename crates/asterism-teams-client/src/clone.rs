//! Taking a copy of what a team holds (#148 decision 10).
//!
//! Working on a shared line needs no copy — open a pursuit against it,
//! and decision 16 serves the reads through. Cloning is for when the
//! copy *is* the point: leaving a team, or starting from what a team
//! has and going a different way, which the model will not let you do
//! in place because it refuses a fork outright.
//!
//! ## A clone is an import, not a forge concept
//!
//! Three things follow from that word, and each is visible in the code
//! below rather than left to a caller's discipline.
//!
//! It **mints new ids**, because the local plane mints its own
//! ([`Imports::record`] hands back a local [`AssetId`], and nothing
//! here reads a [`TeamScopedId`] as one).
//!
//! It **writes no relation row**. There is no [`AssetLinkRepository`]
//! in this module's signature at all, which is the strongest way to say
//! it: a link row means "I put this there", and a copy did not put
//! anything anywhere.
//!
//! It **says where it came from through `source_kind` and
//! `source_locator`**, the way every other import does — and that is
//! the part with a design decision in it, so [`cloned_locator`] carries
//! the argument.
//!
//! ## What is not taken
//!
//! The projection is read and handed back whole, but only its title
//! lands on the copy, through the one field an ingest has for saying
//! what something is called. Marks do not: the ingest command has no
//! slot for one, and [`ReadMark`](crate::ReadMark) is deliberately not
//! [`PromotedMark`](crate::PromotedMark) — a mark read off the wire has
//! no layer under it and no verified origin, so it is not a thing this
//! machine may go on to promote as its own. A caller wanting more than
//! the title has [`Cloned::projection`] and the whole view in it.
//!
//! [`AssetLinkRepository`]: asterism_core::domain::repository::AssetLinkRepository

use std::path::{Path, PathBuf};

use asterism_core::domain::team_link::TeamScopedId;
use asterism_core::domain::value::{AssetId, PersonaId, SourceKind};
use asterism_core::error::DomainError;
use chrono::{DateTime, Utc};

use crate::client::{TeamsClient, TeamsClientError};
use crate::mapper::{ProjectionView, read_projection_body};

/// What a clone hands the local plane, once the bytes are on disk.
///
/// Every field is what an ingest already asks for. There is no team
/// vocabulary in it on purpose: by this point the copy is an artefact
/// that arrived from somewhere, the somewhere is spelled in
/// `source_kind` and `locator`, and the side that records it has no
/// business knowing what a team is.
#[derive(Debug, Clone, Copy)]
pub struct Arrival<'a> {
    /// The bucket the copy is filed under. A clone lands in a persona
    /// the way an import does, because that is what it is.
    pub persona_id: &'a PersonaId,
    /// [`SourceKind::TEAM_LINE`], as a slug.
    pub source_kind: &'a str,
    /// Where the bytes are, which is also where they came from — see
    /// [`cloned_locator`].
    pub locator: &'a str,
    /// How many bytes arrived.
    pub bytes: u64,
    /// When the copy was taken. Not when the team's entry was written:
    /// that is the team's record and this is a new artefact on this
    /// machine.
    pub occurred_at: DateTime<Utc>,
    /// What the promoter called it, when the projection said so.
    pub cover_hint: Option<&'a str>,
}

/// Where a clone puts what it took.
///
/// Two questions rather than one, because the answer to the first
/// decides whether the download happens at all — the same ordering
/// [`promote`](crate::promotion::promote) uses when it asks whether an
/// asset is already on a line before it sends a byte.
#[async_trait::async_trait]
pub trait Imports: Send + Sync {
    /// The local asset already recorded under this
    /// `(source_kind, locator)` pair, if there is one.
    ///
    /// This is the existing duplicate machinery, asked early. A `Some`
    /// means the caller already has this, so nothing is fetched and
    /// nothing is written.
    ///
    /// The implementation scopes the question further than the pair —
    /// the real lookup is per persona — and this signature does not
    /// carry that, because the persona is the implementation's own
    /// (it is the one it will file the arrival under) rather than
    /// something a clone gets to vary between the two calls.
    async fn held(&self, source_kind: &str, locator: &str) -> Result<Option<AssetId>, DomainError>;

    /// Records an arrival, or hands back the asset already recorded
    /// under its pair.
    ///
    /// **Must be idempotent on `(source_kind, locator)`.** The check
    /// above is an optimisation — it saves a download — and this is
    /// where being right about a repeat is actually settled, because
    /// two clones can race between the two calls.
    async fn record(&self, arrival: Arrival<'_>) -> Result<AssetId, DomainError>;
}

/// One entry of one line of one team, to be copied.
#[derive(Debug, Clone, Copy)]
pub struct CloneRequest<'a> {
    /// The team hosting the line.
    pub team_id: TeamScopedId,
    /// The line the entry is on.
    pub line_id: TeamScopedId,
    /// The entry to copy.
    pub entry_id: TeamScopedId,
    /// The persona the copy is filed under.
    pub persona_id: &'a PersonaId,
    /// The directory clones are kept under. The rest of the path is
    /// [`cloned_locator`]'s and is not the caller's to choose.
    pub root: &'a Path,
}

/// What a clone left behind, on this machine.
#[derive(Debug, Clone)]
pub struct Cloned {
    /// The local asset the copy is, freshly minted or the one that was
    /// already there.
    pub asset_id: AssetId,
    /// Whether the duplicate machinery answered — `true` means this
    /// entry had been cloned before and nothing was fetched.
    pub already_held: bool,
    /// The team's own id for what was copied. Kept for the caller to
    /// report with, never to read as a local id (#148 decision 6).
    pub team_asset_id: TeamScopedId,
    /// What the bytes hashed to, as the team holds them.
    pub digest: String,
    /// Where the copy is, which is also the locator it was recorded
    /// under.
    pub locator: PathBuf,
    /// How many bytes arrived, or `None` when nothing was fetched
    /// because the copy was already here.
    pub bytes: Option<u64>,
    /// What the promoter said about the entry.
    ///
    /// `None` three ways, and a caller cannot tell them apart: no
    /// projection was captured, one was captured and this build could
    /// not read it, or nothing was fetched at all because the copy was
    /// already here. The last is the commonest — a repeat answers from
    /// `already_held` before any read — and none of the three is a
    /// failure: the bytes are the copy, and a description is what it
    /// can also have (#148 decision 12).
    pub projection: Option<ProjectionView>,
}

/// Where a clone of one entry lives, and therefore what it is recorded
/// under.
///
/// The locator is a path, and that is the decision worth stating. The
/// other kinds a locator can be — a name that never had bytes, a remote
/// address — would say where the copy came from more literally, and
/// would cost the copy every reader that needs bytes: no hash, no
/// thumbnail, no cover text, and no promoting it onward to another
/// team, all of them silently rather than as an error. A clone whose
/// bytes cannot be read is not a copy.
///
/// So the path is made to answer both questions at once. It carries no
/// timestamp, no counter and nothing the caller chose, which is what
/// makes cloning the same thing twice ask `find_by_source` the same
/// question twice and get the first copy back. The team, line and entry
/// say where it came from; the team asset id at the leaf is what keeps
/// the answer honest when an entry's content is replaced, since the
/// replacement is a different asset and therefore a different path
/// rather than a stale hit on the old one.
///
/// **The extension is a fifth input, and it is not an id.** It is taken
/// from what the line calls the entry, because an extension is the only
/// thing that classifies a material — a copy landing without one is
/// imported and invisible, with no mime, no thumbnail and no indexed
/// body. The cost of that is a stated limitation: renaming an entry
/// from `a.png` to `a.jpg` on the team's line changes the path without
/// changing a single id, so a re-clone after such a rename lands a
/// second copy of bytes this machine already has. Renames that leave
/// the extension alone — which is most of them — do not.
///
/// The extension is also the only part of this path that comes from
/// text somebody else wrote, so it is taken through
/// `Path::extension` and then held to ASCII alphanumerics: a name is
/// not allowed to steer where a clone is written.
pub fn cloned_locator(
    root: &Path,
    team: TeamScopedId,
    line: TeamScopedId,
    entry: TeamScopedId,
    team_asset: TeamScopedId,
    named: Option<&str>,
) -> PathBuf {
    let ext = named
        .and_then(|name| Path::new(name).extension().and_then(|e| e.to_str()))
        .filter(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_else(|| "bin".to_string());
    root.join(team.to_string())
        .join(line.to_string())
        .join(entry.to_string())
        .join(format!("{team_asset}.{ext}"))
}

/// Copies one entry of a team's line onto this machine.
///
/// The order is the point, and it is the promotion's order read
/// backwards: ask what is already here before fetching anything, then
/// fetch, then record. A clone that recorded first would own a row
/// describing bytes that had not arrived.
pub async fn clone_entry(
    client: &TeamsClient,
    imports: &dyn Imports,
    request: CloneRequest<'_>,
    now: DateTime<Utc>,
) -> Result<Cloned, TeamsClientError> {
    let CloneRequest {
        team_id,
        line_id,
        entry_id,
        persona_id,
        root,
    } = request;

    // What the line holds now, folded from its chain. The entry has to
    // be on it: an entry the line took off is one the line no longer
    // presents, and copying it would be reading a state nobody is
    // looking at.
    let states = client.line_states(team_id, line_id).await?;
    let state = states
        .iter()
        .find(|state| state.entry_id == entry_id.to_string())
        .ok_or_else(|| local(format!("line {line_id} has no entry {entry_id} to clone")))?;
    if !state.alive {
        return Err(local(format!(
            "entry {entry_id} was taken off line {line_id}; there is nothing on the \
             line to copy"
        )));
    }
    let content = state.content_asset_id.as_deref().ok_or_else(|| {
        local(format!(
            "entry {entry_id} names no content, so there are no bytes to copy"
        ))
    })?;
    let team_asset = TeamScopedId::parse(content, "team asset id")?;

    // The id a round names is an asset; the bytes are the digest's,
    // because a team mints an asset per promotion over one stored copy
    // (#148 decision 7).
    let resolved = client.resolve_content(team_id, &[team_asset]).await?;
    let digest = resolved
        .held
        .iter()
        .find(|held| held.asset_id == team_asset.to_string())
        .ok_or_else(|| local(format!("this team does not hold {team_asset}")))?
        .digest
        .clone()
        .ok_or_else(|| {
            local(format!(
                "{team_asset} was converted from something other than one blob, which \
                 this build cannot copy"
            ))
        })?;

    let locator = cloned_locator(
        root,
        team_id,
        line_id,
        entry_id,
        team_asset,
        state.name.as_deref(),
    );
    let spelled = locator.to_string_lossy().into_owned();

    // Before a byte moves.
    if let Some(held) = imports
        .held(SourceKind::TEAM_LINE, &spelled)
        .await
        .map_err(TeamsClientError::Local)?
    {
        return Ok(Cloned {
            asset_id: held,
            already_held: true,
            team_asset_id: team_asset,
            digest,
            locator,
            bytes: None,
            projection: None,
        });
    }

    if let Some(parent) = locator.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            local(format!(
                "making room for the clone at {}: {err}",
                parent.display()
            ))
        })?;
    }
    let bytes = client.fetch_content(team_id, &digest, &locator).await?;

    // What the promoter said, opened by the one thing allowed to open a
    // body. A projection may be absent without the line lying (#148
    // decision 12), and any refusal from the mapper — an unreadable
    // version, a body that is not the shape it claims — is the same
    // kind of absence for a copy's purposes. Deliberately not fatal:
    // the bytes are the copy, and a description this build cannot read
    // is not a reason to refuse somebody their own copy of them.
    let projection = match client.entry_projection(team_id, line_id, entry_id).await? {
        Some(found) => read_projection_body(&found.body).ok(),
        None => None,
    };

    let asset_id = imports
        .record(Arrival {
            persona_id,
            source_kind: SourceKind::TEAM_LINE,
            locator: &spelled,
            bytes,
            occurred_at: now,
            cover_hint: projection.as_ref().and_then(|view| view.title.as_deref()),
        })
        .await
        .map_err(TeamsClientError::Local)?;

    Ok(Cloned {
        asset_id,
        already_held: false,
        team_asset_id: team_asset,
        digest,
        locator,
        bytes: Some(bytes),
        projection,
    })
}

fn local(message: String) -> TeamsClientError {
    TeamsClientError::Local(DomainError::Validation(message))
}
