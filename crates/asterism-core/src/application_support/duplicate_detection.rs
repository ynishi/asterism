//! Duplicate detection — what happens the moment a fingerprint lands on
//! bytes somebody else already holds.
//!
//! The `material_hash` job writes a digest; this decides what that
//! digest *means* for the corpus. Three outcomes, chosen by the
//! [`OnDuplicate`] strategy the registering caller declared: park the
//! question for a person, fold without asking, or record the coincidence
//! and move on. All three record the match itself as an
//! [`identical_to`](EdgeKind::IdenticalTo) edge — a pair ruled apart is
//! still a pair that hashed the same, and that fact is what stops the
//! same conflict from being rediscovered as news.
//!
//! # A function, not a service
//!
//! Everything else in this module's neighbours is a struct wired at the
//! composition root, because it holds policy (a retention period) or
//! collaborators a handler cannot reach. This holds neither: the job
//! handler already carries every port named here, and the only decision
//! that is not derived from the row is the fallback in
//! [`resolve_strategy`]. A struct would add a `JobDeps` field and a
//! constructor call to hand a handler things it is already holding.
//!
//! # Detection never fails the hash
//!
//! Every error out of here is the caller's to log and drop. The digest
//! is a fact about bytes and it has already been written; a conflict is
//! a derivation from that fact, and a derivation that fell over must not
//! take the observation with it. The concrete cost of getting this wrong
//! is permanent: the backfill walk finds work by asking whether the
//! fingerprint columns hold an answer
//! ([`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint)),
//! so a hash rolled back for a failed lookup would be re-read on every
//! future pass, while a conflict that was not raised is re-raised the
//! next time the pair is fingerprinted.

use std::collections::{HashSet, VecDeque};

use chrono::{DateTime, Utc};

use crate::domain::asset::Asset;
use crate::domain::content_hash::is_duplicate_key;
use crate::domain::duplicate_conflict::{DuplicateAxis, DuplicateConflict, FoldExclusion};
use crate::domain::edge::{ConstellationEdge, EdgeDirection, EdgeKind};
use crate::domain::job::JobKind;
use crate::domain::repository::{AssetRepository, EdgeRepository, JobQueue, MaterialFingerprint};
use crate::domain::value::{AssetId, FoldPolicy, OnDuplicate, SourceKind};
use crate::error::DomainError;

/// The strategy an asset that declared none is handled under.
///
/// One value, and it is the one that asks: a library where the same
/// bytes legitimately arrive twice cannot silently pick either "these
/// are one thing" or "these are two" on the user's behalf.
pub const UNDECLARED_STRATEGY: OnDuplicate = OnDuplicate::Ask;

/// Turns an undeclared strategy into the one that will be applied —
/// **the only place absence becomes an answer.**
///
/// `asset.on_duplicate` is `NULL` when nobody declared anything, which
/// is deliberately not the same as declaring `ask`
/// ([`OnDuplicate`]). The resolution ladder
/// puts an importer / lane setting and a
/// persona default between that absence and the built-in fallback, and
/// neither layer exists yet — so today this is one `unwrap_or`. It is a
/// named function anyway, because the day those layers land, this is the
/// body that grows and nothing else has to: a `.unwrap_or(Ask)` inlined
/// at the detection site would be a second, invisible place where a lane
/// setting would have to be remembered.
///
/// The strategy read is the **newcomer's** — the younger of the two
/// rows ([`orient`]). That is the registration whose duplicate-ness is
/// in question; the incumbent declared how *its own* arrival should be
/// handled, and that question was answered when it arrived. (What the
/// incumbent still gets a say in is [`FoldPolicy::Keep`] — a ruling made
/// after the fact, which [`detect_duplicate`] honours from either side.)
pub fn resolve_strategy(declared: Option<OnDuplicate>) -> OnDuplicate {
    declared.unwrap_or(UNDECLARED_STRATEGY)
}

/// Which pass fingerprinted the material.
///
/// The distinction exists for exactly one rule: **a conflict found by
/// the backfill is never
/// folded automatically**, because both rows have been in the library
/// long enough to be used, and folding two rows somebody has been
/// working with is always a confirmed act.
///
/// It is passed by the caller rather than carried in the job payload
/// because the payload already says it — the backfill's is
/// `{ "batch": true, … }` and the per-asset fan-out's is
/// `{ "asset_id": … }`, and the two are dispatched by different
/// branches of the handler. A dedicated field would be a second source
/// for the same fact, set by whoever enqueued the job, and the two could
/// disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionOrigin {
    /// The per-asset run that follows a registration.
    Ingest,
    /// The walk over materials that were imported before they could be
    /// fingerprinted, or whose earlier attempt failed.
    Backfill,
    /// **Nothing was measured.** The digest is what the registering
    /// caller said the bytes hash to
    /// (`AddAssetCommand::declared_content_hash`), read at ingest so a
    /// match can be proposed without the server opening the file.
    ///
    /// It is a third value rather than a flag beside the other two
    /// because it answers the same question they do — which pass
    /// produced the digest — and because the rule it carries is the
    /// same shape as the backfill's: **this pass never folds**. The
    /// reason is different, though, and worth keeping distinct from
    /// "both rows have been in the library". A claim is unverified
    /// until the hashing job confirms it, there is no unfold verb, and
    /// a fold driven by an assertion nobody checked is not reversible.
    /// A proposal is.
    Declared,
}

impl DetectionOrigin {
    /// The strategy actually applied, after the rules that belong to
    /// the pass rather than to the pair.
    ///
    /// Only [`Fold`](OnDuplicate::Fold) moves, and it moves to
    /// [`Ask`](OnDuplicate::Ask) — the conflict still goes on the queue,
    /// it is simply not acted on without a person. `Separate` is left
    /// alone: a lane that deliberately produces identical material has
    /// already answered the question, and queueing its pairs during a
    /// backfill would fill the panel with questions whose answer is on
    /// record.
    ///
    /// Two passes move it, for the two reasons their own docs give:
    /// [`Backfill`](Self::Backfill) because both rows have been in the
    /// library long enough to be used, and [`Declared`](Self::Declared)
    /// because nothing has hashed the bytes yet.
    fn applies(self, declared: OnDuplicate) -> OnDuplicate {
        match (self, declared) {
            (Self::Backfill | Self::Declared, OnDuplicate::Fold) => OnDuplicate::Ask,
            (_, other) => other,
        }
    }
}

/// The three ports one detection needs, as one handle.
///
/// Bundled rather than passed one by one because they always travel
/// together — the job handler holds all three and hands them straight
/// on — and because the alternative is a seven-argument call whose
/// order is the only thing distinguishing three `&dyn` references.
pub struct DetectionPorts<'a> {
    /// Read: who else holds these bytes, and what do their rows say.
    /// Write: the conflict queue.
    pub assets: &'a dyn AssetRepository,
    /// Write: the `identical_to` record of the match.
    pub edges: &'a dyn EdgeRepository,
    /// Write: the fold, when a lane asked for one without confirmation.
    pub queue: &'a dyn JobQueue,
}

/// What one detection did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// Nothing was compared: the value is not a duplicate key, the
    /// material is not the asset's primary one, or the row has since
    /// been deleted or folded away.
    NotApplicable,
    /// Nobody else in this persona holds these bytes.
    Unique,
    /// The match was recorded and the question put on the queue.
    Queued(AssetId),
    /// The match was recorded; the question was already on the queue.
    AlreadyQueued(AssetId),
    /// The match was recorded and an [`AssetFold`](JobKind::AssetFold)
    /// enqueued against the incumbent.
    Folding(AssetId),
    /// The match was recorded and nothing else follows — the strategy is
    /// `separate`, or one of the two rows has been ruled a thing of its
    /// own.
    Recorded(AssetId),
}

impl Detection {
    /// One-line form for the job's return message.
    pub fn describe(&self) -> String {
        match self {
            Self::NotApplicable => "conflict: not applicable".into(),
            Self::Unique => "conflict: none".into(),
            Self::Queued(id) => format!("conflict queued against {id}"),
            Self::AlreadyQueued(id) => format!("conflict already queued against {id}"),
            Self::Folding(id) => format!("conflict folding into {id}"),
            Self::Recorded(id) => format!("conflict recorded against {id}"),
        }
    }
}

/// Runs every axis over a freshly written fingerprint, **strongest
/// first, and stops at the first agreement**.
///
/// # Why it stops
///
/// A byte-identical pair agrees on every axis, and `duplicate_conflict`
/// is unique on `(pair_lo, pair_hi, axis)` — so nothing in the schema
/// would stop one pair being queued three times for a person to answer
/// three times. `Artefact` implies both of the others, so those rows
/// would be the same finding stated more weakly.
///
/// Between `Content` and `Meta` the argument is different, because
/// neither implies the other: the stop is about the queue rather than
/// about the axes — a conflict row is a question put to a person about
/// a **pair**, and one pair is one question. What that discards, and
/// why nothing is lost that a reader cannot recover from the stored
/// columns, is set out on [`DuplicateAxis::STRONGEST_FIRST`], which
/// holds the order because it is a property of what the axes mean
/// rather than of this walk.
///
/// # The stop is right and the premise under it is wrong
///
/// Everything above about *why the walk stops* still holds. What does
/// not is the sentence it leans on — that `Content` and `Meta` are two
/// independent claims about identity, so a pair reaching either one is
/// a pair that might be one thing.
///
/// **The algebra is `Artefact = Content + Meta`**: the whole bytes are
/// the picture plus the metadata written about it, so there are **two**
/// independent axes and `Artefact` is the name for both agreeing rather
/// than a third thing to compare. Read that way, this loop says
/// something it was not written to say. It stops at the first
/// agreement, so **a pair reaches `Meta` only when `Artefact` and
/// `Content` both found nothing** — which is to say the pictures
/// differ. A meta-alone agreement is therefore not a weaker identity
/// claim; it is not an identity claim at all. It is "made the same
/// way", which is [`series`](crate::domain::series)'s sentence, and
/// that module is explicit that the claim never folds anything.
///
/// So today's loop queues those pairs as questions with a fold button
/// on them, and the pair a person is being asked to fold is two
/// different pictures. Nothing here is changed for it: the restructure
/// — `Content` first, `Meta` consulted only to word the question —
/// is its own slice, and is not built here. This paragraph is here so
/// that whoever next reads
/// the walk does not re-derive the old premise from the argument above
/// it, which is otherwise the natural reading.
///
/// # What "agreement" means here
///
/// Any outcome naming another row: queued, already queued, folding, or
/// recorded. [`Unique`](Detection::Unique) and
/// [`NotApplicable`](Detection::NotApplicable) both mean "this axis
/// found nothing", and the next axis is tried. When no axis finds
/// anything the answer is `Unique` if any of them was in a position to
/// look, and `NotApplicable` when none was — a row whose material is
/// not primary, or whose digests are all markers, has not been compared
/// on anything.
///
/// An error from one axis ends the walk rather than falling through to
/// the next: the caller swallows detection failures (see
/// `detect_after_hash`), and a partial answer reported as a whole one
/// would say "nothing agreed" about an axis that never ran.
pub async fn detect_duplicate(
    ports: DetectionPorts<'_>,
    asset_id: &AssetId,
    ord: u32,
    fingerprint: &MaterialFingerprint,
    origin: DetectionOrigin,
    now: DateTime<Utc>,
) -> Result<Detection, DomainError> {
    let mut looked = false;
    for axis in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
        let digest = match axis {
            DuplicateAxis::Artefact => fingerprint.file.as_str(),
            DuplicateAxis::Content => fingerprint.content.as_str(),
            DuplicateAxis::Meta => fingerprint.meta.as_str(),
        };
        // The ports are `&dyn` behind a struct that is not `Copy`, so
        // one is built per axis from the same three references.
        let per_axis = DetectionPorts {
            assets: ports.assets,
            edges: ports.edges,
            queue: ports.queue,
        };
        match detect_duplicate_on_axis(per_axis, asset_id, ord, axis, digest, origin, now).await? {
            Detection::NotApplicable => {}
            Detection::Unique => looked = true,
            agreed => return Ok(agreed),
        }
    }
    if looked {
        Ok(Detection::Unique)
    } else {
        Ok(Detection::NotApplicable)
    }
}

/// Looks for other holders of a freshly written digest **on one axis**
/// and applies the newcomer's strategy to what it finds.
///
/// Most callers want [`detect_duplicate`], which walks the axes in
/// order. This is the single-axis entry, for the one caller that holds
/// a value on exactly one axis: the ingest-time check of a digest the
/// registering caller *declared* rather than measured.
///
/// `asset_id` / `ord` name the material that was just fingerprinted,
/// `axis` says which question is being asked, and `digest` is the value
/// on that axis. The asset is re-read here rather
/// than passed in because the two callers hold different things (an
/// entity on the ingest path, a scan row on the backfill one) and
/// because the columns that decide the outcome — `on_duplicate`,
/// `fold_policy`, `folded_into` — must be read *after* the hash landed,
/// not from an entity loaded before it.
///
/// # What is not compared
///
/// - **Anything but the primary material.** The lookup's key is
///   `(persona, digest, ord = 0)`: a secondary original holding the same
///   bytes as somebody's primary is not two of the same asset.
/// - **Values that are not digests.** The `unhashable:` marker every
///   fragment and remote locator shares would match the entire
///   conversation corpus, and the empty-file digest every failed
///   download shares would match all of those; the rule is
///   [`is_duplicate_key`], and the lookup refuses to answer without it.
/// - **A row that is already a headstone.** It has been folded; asking
///   what to do about its bytes is asking about a row that is gone.
///
/// # Which of the two is the incumbent
///
/// The **oldest** holder, which is the front of the lookup's result —
/// the same row a fold would keep, and the same order the duplicate
/// report lists members in. That is usually the other row, but not
/// always: the backfill can reach the older half of a pair second, and
/// then the row being fingerprinted is the incumbent and its partner is
/// the newcomer. [`orient`] decides, and says why the axis is age
/// rather than "who is being hashed".
///
/// Trashed rows are holders too (the lookup says why), so a re-import of
/// something the user threw away raises a question rather than passing
/// as unique.
///
/// # What stops a fold a lane asked for
///
/// Three rules, and all three stop the **automatic** fold only — the
/// pair still reaches a person, and the manual merge verb is
/// deliberately not bound by any of them.
///
/// - [`FoldPolicy::Keep`] on either row ([`ruled_apart`]) — somebody
///   already answered, so nothing is queued either.
/// - The pair is one lineage ([`lineage_connects`]).
/// - Either row is the output of an export run ([`born_of_dispatch`]).
///
/// The last two turn the fold into a queued question carrying the rule
/// that declined it
/// ([`FoldExclusion`]), so the panel can say what the rule was
/// protecting instead of showing a question indistinguishable from an
/// ordinary `ask`. That is the whole reason the reason is stored: the
/// rules do not forbid the fold, they take it away from the machine, and
/// a person handed the decision without being told why is a person who
/// will make it the other way.
pub async fn detect_duplicate_on_axis(
    ports: DetectionPorts<'_>,
    asset_id: &AssetId,
    ord: u32,
    axis: DuplicateAxis,
    digest: &str,
    origin: DetectionOrigin,
    now: DateTime<Utc>,
) -> Result<Detection, DomainError> {
    let DetectionPorts {
        assets,
        edges,
        queue,
    } = ports;
    // The axis is the caller's, and the value is validated against it:
    // a `cr1-sha256:` digest is not a duplicate key on the artefact
    // axis and vice versa, so a caller that crossed the two is refused
    // here rather than answered against the wrong column.
    if ord != 0 || !is_duplicate_key(axis, digest) {
        return Ok(Detection::NotApplicable);
    }
    let Some(fingerprinted) = assets.find(asset_id).await? else {
        return Ok(Detection::NotApplicable);
    };
    if fingerprinted.folded_into.is_some() {
        return Ok(Detection::NotApplicable);
    }

    let holders = assets
        .find_by_content_hash(&fingerprinted.persona_id, axis, digest)
        .await?;
    // The lookup answers "who holds these bytes", oldest first, and the
    // row that was just fingerprinted is one of them.
    let Some((newcomer, incumbent)) = orient(&fingerprinted, &holders) else {
        return Ok(Detection::Unique);
    };

    // Recorded before the branch, and on every branch. The bytes agreed
    // — that is true whatever anyone decides about it, and it is what a
    // later reader traces the decision back through.
    let mut edge = ConstellationEdge::new(newcomer.id, incumbent.id, EdgeKind::IdenticalTo)?;
    edge.label = Some(axis.as_str().to_string());
    edges.add_edges(vec![edge]).await?;

    if ruled_apart(newcomer, incumbent) {
        return Ok(Detection::Recorded(incumbent.id));
    }

    let mut declined: Option<FoldExclusion> = None;
    let applied = match origin.applies(resolve_strategy(newcomer.on_duplicate)) {
        OnDuplicate::Fold => {
            // The exclusions bear on the *automatic* fold and on
            // nothing else, so they are evaluated here and not a line
            // earlier. Under `ask` the pair was already going to a
            // person, and a lineage walk on every fingerprint — `ask`
            // being the default — would be a graph traversal per
            // imported file to annotate a question nobody asked
            // differently. The pair a person eventually proposes to
            // merge gets its warning from the merge verb, computed
            // then, against the
            // graph as it is then.
            declined = fold_excluded_by(edges, newcomer, incumbent).await?;
            match declined {
                None => OnDuplicate::Fold,
                Some(_) => OnDuplicate::Ask,
            }
        }
        other => other,
    };

    match applied {
        OnDuplicate::Separate => Ok(Detection::Recorded(incumbent.id)),
        OnDuplicate::Fold => {
            queue
                .enqueue(
                    JobKind::AssetFold,
                    serde_json::json!({
                        "asset_id": newcomer.id.to_string(),
                        "keeper_id": incumbent.id.to_string(),
                    }),
                )
                .await?;
            Ok(Detection::Folding(incumbent.id))
        }
        OnDuplicate::Ask => {
            let conflict = DuplicateConflict::raise(
                fingerprinted.persona_id,
                newcomer.id,
                incumbent.id,
                axis,
                digest,
                declined,
                now,
            )?;
            if assets.record_duplicate_conflict(&conflict).await? {
                Ok(Detection::Queued(incumbent.id))
            } else {
                Ok(Detection::AlreadyQueued(incumbent.id))
            }
        }
    }
}

/// Node budget for one side of the lineage probe — and, reused as the
/// per-node edge limit, the width it will look at before giving up.
///
/// Deliberately below
/// [`LINEAGE_MAX_NODES`](crate::application::AssetService) territory
/// (200, the read-side picture). That walk runs when a person opens a
/// panel; this one runs **once per fingerprint**, which is once per
/// imported file, on the job worker. Sixty-four ancestors above one row
/// is already a composite assembled from a whole shoot; past that the
/// question "are these two the same lineage" is not being answered
/// cheaply enough to answer on the ingest path.
///
/// One number rather than a depth and a fan-out: every hop and every
/// branch costs the same thing here (one `edges_incident` query), so
/// the honest budget is the count of rows looked at — the row the walk
/// starts from included, since it is one of the rows read.
pub const LINEAGE_PROBE_BUDGET: u32 = 64;

/// Which rule, if any, stands between this pair and an automatic fold.
///
/// Cheapest first, and the first answer is the answer. Both rules are
/// reasons not to fold, and a pair that trips both is not folded twice
/// — the row records why the fold did not happen, not an inventory of
/// everything true about the two rows. Ordering by cost puts the column
/// read ahead of the graph walk, which means the case this pairing was
/// shipped for (an exporter's copy-mode output beside its own input)
/// answers without touching the edge table at all.
///
/// # Where the manual merge verb reads it
///
/// The rules stop the *automatic* fold and are deliberately not binding
/// on a person's ruling ([`merge_into`](crate::domain::repository::AssetRepository::merge_into)
/// port doc). The manual merge verb still asks the question — a panel
/// showing "these are the same thing" wants to say what a rule was
/// protecting before the person overrode it — so this function is `pub`
/// to let the verb read it, one pair at a time, on the dry-run path.
/// The helpers it dispatches to
/// ([`born_of_dispatch`], [`lineage_connects`], [`ancestry_of`]) stay
/// private: what a caller here can see is the answer, not the two
/// separate readings it was assembled from, because the ordering
/// between them is part of the answer.
pub async fn fold_excluded_by(
    edges: &dyn EdgeRepository,
    newcomer: &Asset,
    incumbent: &Asset,
) -> Result<Option<FoldExclusion>, DomainError> {
    if born_of_dispatch(newcomer) || born_of_dispatch(incumbent) {
        return Ok(Some(FoldExclusion::Dispatch));
    }
    if lineage_connects(edges, newcomer, incumbent).await? {
        return Ok(Some(FoldExclusion::Lineage));
    }
    Ok(None)
}

/// Whether a row is the output of an export run.
///
/// **Either side disqualifies the pair**, which is the only reading
/// that covers the case the rule exists for: an exporter in copy mode
/// writes its input's bytes verbatim, so the pair is one dispatch
/// product and one ordinary import — never two dispatch products. A
/// both-sides rule would not fire on it at all.
///
/// It is also the right reading on its own terms. A dispatch row is the
/// record that a run produced something; fold it away and the run's
/// output becomes a run with no output. Which of the two rows would
/// have become the headstone is decided by age
/// ([`orient`]), which has nothing to do with which one is a run
/// product — so a rule that only looked at one side would protect the
/// run's record or not depending on when the export happened to
/// occur.
///
/// The test is [`SourceKind::DISPATCH_PREFIX`] rather than a
/// `starts_with("dispatch")` spelled out here, for the reason
/// [`SourceKind::for_dispatch`] gives about its own side of the
/// convention: one construction site, one recognition site, and the
/// 2026-07-19 `dispatch:file` regression is what a hand-written second
/// copy of the grammar looks like.
///
/// A copy that left the library and came back through the filesystem
/// importer carries `fs` and is not caught by this — which is
/// intended, since what the exclusion protects is a
/// lineage, and that path has none of its own.
fn born_of_dispatch(asset: &Asset) -> bool {
    asset
        .source
        .kind
        .as_str()
        .starts_with(SourceKind::DISPATCH_PREFIX)
}

/// Whether the two rows sit in one `derived_from` lineage — **or the
/// graph was too big to say they do not.**
///
/// # What counts as connected
///
/// One is an ancestor of the other, *or* they share an ancestor. The
/// second half is the deliberate one: two exports of the same input are
/// siblings, and that shape — a deliberate variant — is exactly why
/// this library cannot fold on byte equality
/// the way a photo manager can. Two renders of one prompt that came out
/// byte-identical are two runs, and folding one away silently deletes
/// the record that the second run happened. Ancestry is also what makes
/// the relation answerable from either end: "is there a common
/// ancestor" gives the same verdict whichever row is asked first, while
/// "walk out from here" does not.
///
/// A shared *descendant* is not connected. A composite assembled from A
/// and B says the two were used together, not that they are one
/// another's lineage — and if A and B hold the same bytes, they are two
/// copies of one input that a fold would tidy correctly. Leaving it out
/// also keeps the walk one-directional.
///
/// # The bound, and what happens at it
///
/// [`LINEAGE_PROBE_BUDGET`] rows per side. Hitting it returns
/// **`true`** — not "unrelated". The rule exists to protect lineage, so
/// the failure mode worth having is a pair that stays unfolded and
/// reaches a person, not a pair folded because the walk ran out of
/// budget before it found what connects them. The queue row that
/// results says `lineage`, which is what was undetermined; the panel
/// shows the pair, and a person looking at two rows can see what the
/// graph could not be walked far enough to say.
///
/// The width is bounded the same way and read the same way: a node
/// whose `derived_from` edges fill the per-node limit may have more
/// beyond it, so that too is "could not finish looking".
async fn lineage_connects(
    edges: &dyn EdgeRepository,
    newcomer: &Asset,
    incumbent: &Asset,
) -> Result<bool, DomainError> {
    let (above_newcomer, newcomer_truncated) = ancestry_of(edges, &newcomer.id).await?;
    let (above_incumbent, incumbent_truncated) = ancestry_of(edges, &incumbent.id).await?;
    if above_newcomer
        .intersection(&above_incumbent)
        .next()
        .is_some()
    {
        return Ok(true);
    }
    Ok(newcomer_truncated || incumbent_truncated)
}

/// The row and everything it descends from, up to
/// [`LINEAGE_PROBE_BUDGET`]; the flag says the walk stopped early.
///
/// Includes the start, so "B is an ancestor of A" and "A and B share an
/// ancestor" are one intersection test rather than three cases.
///
/// A `derived_from` edge points child → parent, so the ancestor
/// direction is the outgoing one — the same reading `provenance_of`
/// and `lineage_of` use. `Both` is folded in with `Outgoing` for the
/// reason `provenance_of` records: the write path stamps one direction,
/// and a symmetric row would mean somebody declared the pair each
/// other's parent, which is best read as "related" rather than
/// silently dropped.
///
/// The `visited` set is what makes a hand-declared cycle (A → B → A,
/// reachable because `derived_from` can be asserted at ingest) end
/// rather than spin.
async fn ancestry_of(
    edges: &dyn EdgeRepository,
    start: &AssetId,
) -> Result<(HashSet<AssetId>, bool), DomainError> {
    let mut seen: HashSet<AssetId> = HashSet::from([*start]);
    let mut queue: VecDeque<AssetId> = VecDeque::from([*start]);
    let mut truncated = false;
    while let Some(current) = queue.pop_front() {
        let incidents = edges
            .edges_incident(&current, Some(EdgeKind::DerivedFrom), LINEAGE_PROBE_BUDGET)
            .await?;
        if incidents.len() as u32 >= LINEAGE_PROBE_BUDGET {
            // A full page is indistinguishable from a page that had
            // more behind it, and the parents this node did not report
            // are the ones that could have connected the pair.
            truncated = true;
            break;
        }
        for incident in &incidents {
            let parent = match incident.direction {
                EdgeDirection::Outgoing | EdgeDirection::Both => incident.edge.to,
                EdgeDirection::Incoming => continue,
            };
            if !seen.insert(parent) {
                continue;
            }
            if seen.len() as u32 >= LINEAGE_PROBE_BUDGET {
                truncated = true;
                break;
            }
            queue.push_back(parent);
        }
        if truncated {
            break;
        }
    }
    Ok((seen, truncated))
}

/// Picks the pair out of the holders and puts it the right way round:
/// `(newcomer, incumbent)` = **(younger, oldest)**, always.
///
/// `None` when nobody else holds the bytes.
///
/// # Why age decides, and not "who was just fingerprinted"
///
/// The obvious reading of
/// [`EdgeKind::IdenticalTo`](crate::domain::edge::EdgeKind::IdenticalTo)'s
/// direction rule — `from` is the row whose arrival raised the conflict
/// — would make the row being fingerprinted the `from` side every time.
/// It is right for the ordinary case and wrong for the backfill: that
/// walk reaches the two halves of a pair in id order, so the *older*
/// row can be the one whose digest lands second. Oriented by "who is
/// being hashed", that pass writes `(older, younger)` while the earlier
/// one wrote `(younger, older)` — and `UNIQUE (from_asset, to_asset,
/// kind)` is happy to hold both. One symmetric fact, stored twice, with
/// no constraint able to catch it: exactly the failure that edge kind's
/// doc says nothing but the writer's discipline prevents.
///
/// Age is the axis that gives the pair one answer from either end, and
/// it agrees with the arrival reading wherever the two are both
/// defined: an import that arrives second is also the younger row.
/// It is the same axis
/// [`find_by_content_hash`](crate::domain::repository::AssetRepository::find_by_content_hash)
/// already sorts on and the same one the fold uses to pick a keeper, so
/// the edge, the queue row and the fold all name the same two sides.
///
/// The consequence worth stating: when the fingerprinted row *is* the
/// oldest, the strategy consulted is the **other** row's, because that
/// is the registration whose duplicate-ness is in question. The older
/// row's own declaration was answered when it arrived.
fn orient<'a>(fingerprinted: &'a Asset, holders: &'a [Asset]) -> Option<(&'a Asset, &'a Asset)> {
    let oldest = holders.first()?;
    if oldest.id == fingerprinted.id {
        // The row just fingerprinted is the oldest holder, so the pair
        // runs the other way: the next holder is younger than it.
        let younger = holders.iter().find(|held| held.id != fingerprinted.id)?;
        Some((younger, oldest))
    } else {
        Some((fingerprinted, oldest))
    }
}

/// Whether either row has been ruled a thing of its own
/// ([`FoldPolicy::Keep`]), which suppresses both the queue and the fold.
///
/// **Either side, not just the newcomer.** `fold_policy` is a statement
/// about the row it sits on — "this is not a copy of anything" — and a
/// person who made it has answered the question for every pair the row
/// takes part in, in whichever direction it is asked. Reading only the
/// newcomer's would re-raise a ruled pair as soon as the third copy of
/// the bytes arrived and made the ruled row the incumbent; reading only
/// the incumbent's would ignore the ruling of whoever looked at the
/// newer row.
///
/// The narrower reading — a per-pair ruling, so that keeping A apart
/// from B says nothing about A and C — is a different column and a
/// different design. It is not what `fold_policy` is: the column is on
/// the row, the rule is stated without naming a
/// side, and this is the reading that cannot re-ask a question somebody
/// has already answered.
///
/// The edge is written before this is consulted, on purpose: a ruled
/// pair still hashed the same, and losing that record is what would make
/// the conflict news again.
fn ruled_apart(newcomer: &Asset, incumbent: &Asset) -> bool {
    newcomer.fold_policy == FoldPolicy::Keep || incumbent.fold_policy == FoldPolicy::Keep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absence_resolves_to_the_one_default_there_is() {
        assert_eq!(resolve_strategy(None), OnDuplicate::Ask);
        // A declared strategy is never overridden by the fallback —
        // including when it happens to equal it, which is the case the
        // column exists to keep distinguishable.
        for declared in [OnDuplicate::Ask, OnDuplicate::Fold, OnDuplicate::Separate] {
            assert_eq!(resolve_strategy(Some(declared)), declared);
        }
    }

    #[test]
    fn the_backfill_asks_where_ingest_would_fold() {
        assert_eq!(
            DetectionOrigin::Backfill.applies(OnDuplicate::Fold),
            OnDuplicate::Ask,
            "two rows that have been in the library are folded only by a person"
        );
        assert_eq!(
            DetectionOrigin::Ingest.applies(OnDuplicate::Fold),
            OnDuplicate::Fold
        );
        // The other two strategies mean the same thing on both passes.
        for declared in [OnDuplicate::Ask, OnDuplicate::Separate] {
            assert_eq!(DetectionOrigin::Backfill.applies(declared), declared);
            assert_eq!(DetectionOrigin::Ingest.applies(declared), declared);
        }
    }

    /// A digest nobody has checked proposes and never folds.
    ///
    /// The fixture has to disagree with the pass that *does* fold, or
    /// it would pass with the arm deleted — so `Ingest` is asserted
    /// beside it on the same input.
    #[test]
    fn a_declared_digest_proposes_and_never_folds() {
        assert_eq!(
            DetectionOrigin::Declared.applies(OnDuplicate::Fold),
            OnDuplicate::Ask,
            "the bytes have not been hashed yet, and there is no unfold verb"
        );
        assert_eq!(
            DetectionOrigin::Ingest.applies(OnDuplicate::Fold),
            OnDuplicate::Fold,
            "the same declaration folds once a pass has measured the bytes"
        );
        for declared in [OnDuplicate::Ask, OnDuplicate::Separate] {
            assert_eq!(DetectionOrigin::Declared.applies(declared), declared);
        }
    }
}
