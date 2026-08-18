//! `Cull` — the record of one close's narrowing (#22, model on #63):
//! who decided to keep or drop what, out of which frozen candidate
//! set, in which line of work.
//!
//! The cull is a **close-time record**, not a mid-work gate. Mid-work
//! tidying moves through the ledger
//! ([`tx`](super::tx)); the cull converts the final state into
//! verdicts at the one moment they become statements — just before
//! the close that lands them. One cull per close event; a repeat
//! close is a new event and may carry a new cull.
//!
//! # Verdict rules (resolved by [`resolve_verdicts`])
//!
//! - A verdict names a **candidate** — an asset the ledger admitted.
//!   Judging what never entered is refused.
//! - A **newly entered** member (`generated` / `imported`) takes
//!   `keep` or `reject`.
//! - An **existing** member takes `reject` only: keeping what the
//!   library already holds is the untouched default, not a statement.
//!   The one exception is salvage — a `keep` on a *removed* existing
//!   member cancels the removal's default and is recorded.
//! - A member **removed** in the ledger and not spoken for culls as
//!   `reject` — the default is materialised as a row, because the
//!   cull is the record and a reader must not have to re-derive it
//!   from the ledger.
//! - An **untouched** member without a verdict gets no row: "this act
//!   said nothing about it" is the absence, deliberately (#63 — no
//!   forced verdict; the unprocessed remainder just stays).

use chrono::{DateTime, Utc};

use crate::domain::attribution::{AttributionContext, PersistedAttribution};
use crate::domain::forge::tx::{Ledger, TxOrigin};
use crate::domain::forge::value::{CullId, PursuitEventId, PursuitId};
use crate::domain::value::{AssetId, PersonaId, SnapshotId};
use crate::error::DomainError;

/// The closed set of member verdicts. Two values, no third —
/// "unjudged" is the absence of a row (the `ConflictResolution`
/// precedent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CullVerdict {
    /// Chosen out of the candidate set.
    Keep,
    /// Dropped out of the candidate set. The asset row stays live —
    /// trash is orthogonal; the record is what makes discarding safe.
    Reject,
}

impl CullVerdict {
    /// Storage slug.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Reject => "reject",
        }
    }

    /// Parses a storage slug (closed set).
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "keep" => Ok(Self::Keep),
            "reject" => Ok(Self::Reject),
            other => Err(DomainError::Validation(format!(
                "unknown cull verdict: {other:?}"
            ))),
        }
    }
}

/// One act of narrowing, bound to the close event it happened at.
#[derive(Debug, Clone, PartialEq)]
pub struct Cull {
    /// Surrogate id (UUID v7).
    pub id: CullId,
    /// Pursuit whose close this records.
    pub pursuit_id: PursuitId,
    /// Redundant persona copy (the `pursuit_event.persona_id`
    /// precedent).
    pub persona_id: PersonaId,
    /// The close event this cull belongs to — one cull per event
    /// (UNIQUE in storage).
    pub pursuit_event_id: PursuitEventId,
    /// The candidate set, derived from the ledger and frozen at
    /// close. What every verdict is "out of".
    pub candidate_snapshot_id: SnapshotId,
    /// One short free-text slot for the act.
    pub note: Option<String>,
    /// Creation time.
    pub created_at: DateTime<Utc>,
    operator_ai: Option<crate::domain::attribution::OperatorRef>,
    author: Option<crate::domain::attribution::Author>,
    attributed_via: Option<crate::domain::attribution::AttributionChannel>,
}

impl Cull {
    /// Builds a fresh cull record.
    pub fn new(
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        pursuit_event_id: PursuitEventId,
        candidate_snapshot_id: SnapshotId,
        note: Option<String>,
        now: DateTime<Utc>,
        attribution: &AttributionContext,
    ) -> Self {
        Self {
            id: CullId::new(),
            pursuit_id,
            persona_id,
            pursuit_event_id,
            candidate_snapshot_id,
            note: note.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()),
            created_at: now,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Read-path twin of [`new`](Self::new).
    #[allow(clippy::too_many_arguments)]
    pub fn from_persisted(
        id: CullId,
        pursuit_id: PursuitId,
        persona_id: PersonaId,
        pursuit_event_id: PursuitEventId,
        candidate_snapshot_id: SnapshotId,
        note: Option<String>,
        created_at: DateTime<Utc>,
        attribution: PersistedAttribution,
    ) -> Self {
        Self {
            id,
            pursuit_id,
            persona_id,
            pursuit_event_id,
            candidate_snapshot_id,
            note,
            created_at,
            operator_ai: attribution.operator_ai().cloned(),
            author: attribution.author().cloned(),
            attributed_via: attribution.attributed_via(),
        }
    }

    /// Subject that made this act (`None` = unrecorded).
    pub fn author(&self) -> Option<&crate::domain::attribution::Author> {
        self.author.as_ref()
    }

    /// Agent that made this act (`None` = unrecorded).
    pub fn operator_ai(&self) -> Option<&crate::domain::attribution::OperatorRef> {
        self.operator_ai.as_ref()
    }

    /// Channel the pair above arrived through (`None` = unrecorded).
    pub fn attributed_via(&self) -> Option<crate::domain::attribution::AttributionChannel> {
        self.attributed_via
    }

    /// Hands the triple back out whole (see
    /// [`Pursuit::persisted_attribution`](super::pursuit::Pursuit::persisted_attribution)).
    pub fn persisted_attribution(&self) -> PersistedAttribution {
        PersistedAttribution::recorded(
            self.author.clone(),
            self.operator_ai.clone(),
            self.attributed_via,
        )
    }
}

/// One member's verdict within a cull.
#[derive(Debug, Clone, PartialEq)]
pub struct CullMember {
    /// The cull this verdict belongs to.
    pub cull_id: CullId,
    /// The judged asset — an id, never a foreign key (the ledger's
    /// stance: the record outlives the asset).
    pub asset_id: AssetId,
    /// The verdict.
    pub verdict: CullVerdict,
    /// One short free-text slot — the grounds, when someone states
    /// them.
    pub note: Option<String>,
}

/// A caller's requested verdict, before resolution against the
/// ledger.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestedVerdict {
    /// The asset spoken for.
    pub asset_id: AssetId,
    /// What the caller says about it.
    pub verdict: CullVerdict,
    /// Optional grounds.
    pub note: Option<String>,
}

/// A resolved verdict, ready to become a `cull_member` row.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedVerdict {
    /// The judged asset.
    pub asset_id: AssetId,
    /// The verdict after defaults were applied.
    pub verdict: CullVerdict,
    /// Grounds carried over from the request (defaults carry none).
    pub note: Option<String>,
}

/// Resolves a caller's verdicts against the ledger into the rows the
/// cull records, applying the module-doc rules. The result is ordered
/// by asset id. Errors are refusals, not repairs: a verdict on a
/// non-candidate, a duplicate verdict, or a `keep` of a present
/// existing member each name a request that misunderstands the state,
/// and guessing over it would record the misunderstanding.
pub fn resolve_verdicts(
    ledger: &Ledger,
    requested: &[RequestedVerdict],
) -> Result<Vec<ResolvedVerdict>, DomainError> {
    let mut resolved: std::collections::BTreeMap<AssetId, ResolvedVerdict> =
        std::collections::BTreeMap::new();
    for request in requested {
        let state = ledger.get(&request.asset_id).ok_or_else(|| {
            DomainError::Validation(format!(
                "verdict on {}: not a candidate of this pursuit",
                request.asset_id
            ))
        })?;
        if state.origin == TxOrigin::Existing
            && request.verdict == CullVerdict::Keep
            && !state.removed
        {
            return Err(DomainError::Validation(format!(
                "keep of existing asset {}: keeping what the library already \
                 holds is the untouched default, not a statement (salvage — \
                 keep of a removed member — is the one exception)",
                request.asset_id
            )));
        }
        if resolved
            .insert(
                request.asset_id,
                ResolvedVerdict {
                    asset_id: request.asset_id,
                    verdict: request.verdict,
                    note: request.note.clone(),
                },
            )
            .is_some()
        {
            return Err(DomainError::Validation(format!(
                "two verdicts on {}: one act says one thing per member",
                request.asset_id
            )));
        }
    }
    // The removal default, materialised: removed and not spoken for
    // culls as reject.
    for (asset_id, state) in ledger {
        if state.removed && !resolved.contains_key(asset_id) {
            resolved.insert(
                *asset_id,
                ResolvedVerdict {
                    asset_id: *asset_id,
                    verdict: CullVerdict::Reject,
                    note: None,
                },
            );
        }
    }
    Ok(resolved.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::forge::tx::MemberState;

    fn entry(origin: TxOrigin, removed: bool) -> MemberState {
        MemberState { origin, removed }
    }

    fn keep(asset: AssetId) -> RequestedVerdict {
        RequestedVerdict {
            asset_id: asset,
            verdict: CullVerdict::Keep,
            note: None,
        }
    }

    fn reject(asset: AssetId) -> RequestedVerdict {
        RequestedVerdict {
            asset_id: asset,
            verdict: CullVerdict::Reject,
            note: None,
        }
    }

    #[test]
    fn verdicts_resolve_and_untouched_members_stay_silent() {
        let kept = AssetId::new();
        let dropped = AssetId::new();
        let untouched = AssetId::new();
        let ledger: Ledger = [
            (kept, entry(TxOrigin::Generated, false)),
            (dropped, entry(TxOrigin::Imported, false)),
            (untouched, entry(TxOrigin::Generated, false)),
        ]
        .into_iter()
        .collect();
        let rows = resolve_verdicts(&ledger, &[keep(kept), reject(dropped)]).unwrap();
        assert_eq!(rows.len(), 2, "the untouched member gets no row");
        assert!(
            rows.iter()
                .any(|r| r.asset_id == kept && r.verdict == CullVerdict::Keep)
        );
        assert!(
            rows.iter()
                .any(|r| r.asset_id == dropped && r.verdict == CullVerdict::Reject)
        );
    }

    #[test]
    fn a_verdict_on_a_non_candidate_is_refused() {
        let ledger: Ledger = Ledger::new();
        assert!(resolve_verdicts(&ledger, &[keep(AssetId::new())]).is_err());
    }

    #[test]
    fn a_removed_member_defaults_to_reject_and_salvage_overrides() {
        let doomed = AssetId::new();
        let saved = AssetId::new();
        let ledger: Ledger = [
            (doomed, entry(TxOrigin::Generated, true)),
            (saved, entry(TxOrigin::Generated, true)),
        ]
        .into_iter()
        .collect();
        let rows = resolve_verdicts(&ledger, &[keep(saved)]).unwrap();
        assert!(
            rows.iter()
                .any(|r| r.asset_id == doomed && r.verdict == CullVerdict::Reject),
            "unspoken removal materialises as reject"
        );
        assert!(
            rows.iter()
                .any(|r| r.asset_id == saved && r.verdict == CullVerdict::Keep),
            "salvage is a keep on a removed member"
        );
    }

    #[test]
    fn keeping_a_present_existing_member_is_not_a_statement() {
        let held = AssetId::new();
        let ledger: Ledger = [(held, entry(TxOrigin::Existing, false))]
            .into_iter()
            .collect();
        assert!(resolve_verdicts(&ledger, &[keep(held)]).is_err());
        assert!(resolve_verdicts(&ledger, &[reject(held)]).is_ok());
    }

    #[test]
    fn salvaging_a_removed_existing_member_is_the_exception() {
        let held = AssetId::new();
        let ledger: Ledger = [(held, entry(TxOrigin::Existing, true))]
            .into_iter()
            .collect();
        let rows = resolve_verdicts(&ledger, &[keep(held)]).unwrap();
        assert_eq!(rows[0].verdict, CullVerdict::Keep);
    }

    #[test]
    fn two_verdicts_on_one_member_are_refused() {
        let asset = AssetId::new();
        let ledger: Ledger = [(asset, entry(TxOrigin::Generated, false))]
            .into_iter()
            .collect();
        assert!(resolve_verdicts(&ledger, &[keep(asset), reject(asset)]).is_err());
    }

    #[test]
    fn verdict_slugs_round_trip() {
        for verdict in [CullVerdict::Keep, CullVerdict::Reject] {
            assert_eq!(CullVerdict::parse(verdict.slug()).unwrap(), verdict);
        }
        assert!(CullVerdict::parse("salvage").is_err());
    }
}
