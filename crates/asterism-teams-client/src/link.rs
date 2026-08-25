//! Verify and reap — what makes the relation *attended* rather than
//! merely advisory (#148 decision 9).
//!
//! Either end of a promotion can vanish independently and neither may
//! break the other, so nothing in the schema stops a link row from
//! outliving what it points at. Decision 9 names the two systems that
//! live with exactly that and go looking anyway: GitLab's loose
//! foreign keys, which are a missing constraint plus a worker that
//! cleans up after one, and `git annex fsck`, which tolerates a
//! dangling location log and checks for it.
//!
//! ## Two ends, two questions, one of which needs the network
//!
//! - **The Asset was deleted.** Answerable here, with no session and
//!   no server: `AssetLinkRepository::dangling_locally` is an
//!   anti-join against the local `asset` table.
//! - **The entry is gone from the team.** Not answerable here at all.
//!   [`verify`] asks the team — two reads per line the relation names,
//!   for the reason its own doc gives — and a line that no longer
//!   exists answers `404`, which is what a discard leaves behind.
//!
//! ## What is *not* dangling
//!
//! A **trashed** Asset. The local plane can still restore it, so the
//! row still corresponds to something.
//!
//! An entry that was **removed from the line**. Taking an entry off a
//! line is a change point saying so, and the fold still lists it —
//! nothing in a forge truly disappears except through a discard. A row
//! pointing at a removed entry points at something the team can still
//! account for, and reaping it would throw away the record of a
//! promotion that did happen.
//!
//! An entry named by a round of **work that is still open**. It has
//! not landed on the line yet and it is not lost either; see
//! [`verify`] for why that costs a second read.
//!
//! ## Reap
//!
//! [`reap`] is a thin pass to `AssetLinkRepository::reap`, whose doc
//! states what a reap may touch. Nothing is added on the way through,
//! and that is the whole of this module's part in it.

use std::collections::{BTreeMap, BTreeSet};

use asterism_core::domain::repository::AssetLinkRepository;
use asterism_core::domain::team_link::{AssetLink, AssetLinkKey, TeamScopedId};

use crate::client::{TeamsClient, TeamsClientError};

/// Which end of a link went missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// The promoted Asset is no longer in the local library.
    LocalAsset,
    /// The team no longer has the entry — because the line was
    /// discarded, or because the entry was never there.
    TeamEntry,
}

/// One row that points at something that is not there.
#[derive(Debug, Clone)]
pub struct DanglingLink {
    /// The row.
    pub link: AssetLink,
    /// Which end.
    pub why: Missing,
}

/// What a verify found.
#[derive(Debug, Clone, Default)]
pub struct LinkVerification {
    /// The rows that dangle, both ends together.
    pub dangling: Vec<DanglingLink>,
}

impl LinkVerification {
    /// Whether every row still points at both its ends.
    pub fn is_clean(&self) -> bool {
        self.dangling.is_empty()
    }

    /// The keys, for handing to [`reap`].
    ///
    /// A separate step on purpose: verify reports and reap removes,
    /// and a caller that wants to look before anything is deleted —
    /// which is most of them, since a row is the only record that a
    /// promotion happened — can.
    pub fn keys(&self) -> Vec<AssetLinkKey> {
        self.dangling.iter().map(|found| found.link.key).collect()
    }
}

/// Checks every row this machine holds for one team, both ends.
///
/// Two reads per line rather than one per row: the relation names far
/// more entries than lines, and each read answers for all of a line's
/// entries at once.
///
/// **Both reads are needed, and the reason is what an entry id means
/// at each stage.** A promotion names its entry in a round, and a
/// round is a candidate — the entry reaches the line's fold only when
/// the work closes and lands. So a link written a minute ago, against
/// work still open, is *not* in `states` and is not dangling either.
/// Asking `states` alone would report every promotion into open work
/// as broken, which is the loudest possible false positive: it would
/// invite a reap that deletes the record of promotions that are
/// entirely intact. The pursuits against the line are where those
/// entries are, and a discard takes both.
pub async fn verify(
    client: &TeamsClient,
    links: &dyn AssetLinkRepository,
    team: TeamScopedId,
) -> Result<LinkVerification, TeamsClientError> {
    let mut dangling: Vec<DanglingLink> = links
        .dangling_locally(team)
        .await?
        .into_iter()
        .map(|link| DanglingLink {
            link,
            why: Missing::LocalAsset,
        })
        .collect();

    let mut by_line: BTreeMap<TeamScopedId, Vec<AssetLink>> = BTreeMap::new();
    for link in links.list_for_team(team).await? {
        by_line.entry(link.key.line_id).or_default().push(link);
    }

    for (line, rows) in by_line {
        // A discarded line takes its whole log with it, so its `states`
        // read is a 404 and every row on it dangles. Any other refusal
        // is a real failure and is not quietly turned into "gone" —
        // reporting a row as dangling because the network was down
        // would invite a reap that deletes records of promotions that
        // are perfectly intact.
        let known: Option<BTreeSet<String>> = match client.line_states(team, line).await {
            Ok(states) => {
                let mut entries: BTreeSet<String> =
                    states.into_iter().map(|state| state.entry_id).collect();
                // The entries still in flight: named by a round, not
                // yet landed. A discard takes these too, so a line
                // that answered above answers here as well.
                for pursuit in client.pursuits_of_line(team, line).await? {
                    for round in pursuit.rounds {
                        for op in round.ops {
                            entries.insert(op.entry_id);
                        }
                    }
                }
                Some(entries)
            }
            Err(TeamsClientError::Refused { status: 404, .. }) => None,
            Err(other) => return Err(other),
        };
        for link in rows {
            let entry = link.key.entry_id.to_string();
            let gone = match &known {
                None => true,
                Some(entries) => !entries.contains(&entry),
            };
            if gone && !dangling.iter().any(|found| found.link.key == link.key) {
                dangling.push(DanglingLink {
                    link,
                    why: Missing::TeamEntry,
                });
            }
        }
    }

    Ok(LinkVerification { dangling })
}

/// Removes the named link rows, and nothing else. Answers how many
/// there were to remove.
pub async fn reap(
    links: &dyn AssetLinkRepository,
    keys: &[AssetLinkKey],
) -> Result<u64, TeamsClientError> {
    Ok(links.reap(keys).await?)
}
