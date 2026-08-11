//! `SeriesStrategyService` — registering, editing and removing the rules
//! the series axis derives keys under.
//!
//! The axis has worked since S3 and had no door: the seeded `VDSL recipe`
//! was the only rule any library could hold. This is the door, and it is
//! an HTTP-shaped one for a structural reason — an importer runs in its
//! own process and talks to the server over the wire, so a rule it (or
//! the agent driving it) wants to state has to cross as **data**. What
//! comes in is a caller's declaration, on the same terms `album_meta`
//! arrives: believed, labelled as somebody's statement, and never a
//! reading the server made of anybody's bytes.
//!
//! Every write here takes an [`AttributionContext`] it does not persist:
//! `series_strategy` carries no attribution column and is not getting
//! one, since a rule states how some *generator* writes metadata rather
//! than anything a person authored. Taking the argument is still the
//! point — see the [`application`](crate::application) module doc for
//! why, and `ModalityService`, which stands in the same position over
//! the same kind of master table.
//!
//! # Refusing a rule is the point of this file
//!
//! One `series_strategy` row this build cannot read makes **every** page
//! of the derivation walk fail — the walk promotes every rule on the page
//! and one `Err` is the page — so no material gets a key under *any*
//! rule (`SqliteSeriesRepository::scan_underived` states the blast
//! radius). The column can only hold what a writer put there, and this is
//! the writer. So the checks below are not input hygiene; they are the
//! thing standing between a typo and a library with no series keys at
//! all, and each of them refuses something the schema would happily
//! store:
//!
//! - an unknown `decode` token — the `CHECK` would refuse it, so the
//!   write fails loudly, but only for tokens the schema knows to name;
//! - an `applies_to` that is not a `type/subtype` pair — [`MimeType::parse`]
//!   is total, so `""`, `"png"` and `"image/*"` all store fine and claim
//!   nothing forever;
//! - an **empty path** — a rule that means nothing and says nothing about
//!   it: an empty `include` path selects nothing and an empty `exclude`
//!   path drops nothing (see [`Path`]), so the author who wrote `[[]]`
//!   gets a rule that silently is not the one they wrote.
//!
//! What the type system already refuses arrives as a deserialisation
//! failure at the transport, before any of this runs.
//!
//! # An edit invalidates, and invalidation is a delete
//!
//! Four of the five fields are inputs to a key. Changing one makes every
//! key derived under the id a key nothing would derive again, and the
//! whole of the repair is: delete that rule's rows, then let the walk —
//! whose population is "a pair with no row" — answer them under the new
//! rule. `name` is not one of the four, so a rename costs nothing.
//!
//! # KNOWN LIMITATION — nothing here bounds how big a rule is
//!
//! `name`, the path count and the segment lengths are unbounded; the only
//! ceiling is axum's default 2 MB request body. That is not merely a
//! large row, because of where the rule travels:
//! `underived_page_sql` selects `s.include, s.exclude` **per pair**, and a
//! page is `SERIES_DERIVE_PAGE = 200` pairs, so one rule carrying a 2 MB
//! `include` makes every page of the walk materialise on the order of
//! 400 MB. `RegisteredRow`'s doc declines to carry *three `i64` columns*
//! down that same path, which is the measure of how much this one is not
//! covered by that care.
//!
//! No number is picked here because there is no measurement to pick it
//! from, and this codebase's other ceilings are all measured (the PNG
//! probe's metadata cap is 1 MiB against a weighed 40 KB card).
//! What is owed is a reading of what a real rule weighs — the seeded one
//! is 21 bytes of `include` — and a limit an order or two above it,
//! refused here beside the other three. Recorded rather than guessed
//! (see `drafting-discipline`'s rule against writing a bug into a spec:
//! this is a bug carried, not a design).

use std::sync::Arc;

use asterism_contract::command::{
    CreateSeriesStrategyCommand, DeleteSeriesStrategyCommand, UpdateSeriesStrategyCommand,
};
use asterism_contract::dto::SeriesStrategyDto;
use chrono::Utc;

use crate::application::mapping::registered_strategy_to_dto;
use crate::domain::attribution::AttributionContext;
use crate::domain::job::JobKind;
use crate::domain::repository::{JobQueue, SeriesRepository};
use crate::domain::series::{Decode, Path, Strategy};
use crate::domain::value::{MimeType, StrategyId};
use crate::error::DomainError;

/// Series Strategy lifecycle: list / create / update / delete, plus the
/// invalidation an edit owes.
///
/// Holds the queue as well as the repository because a rule change is
/// only half of what the caller asked for — the other half is the walk
/// that turns the change into keys, and a service that wrote the row and
/// left the derivation to the next launch would be answering `200` to a
/// request nothing acted on.
pub struct SeriesStrategyService {
    repo: Arc<dyn SeriesRepository>,
    jobs: Arc<dyn JobQueue>,
}

impl SeriesStrategyService {
    /// Constructs the service around the series port and the job queue.
    pub fn new(repo: Arc<dyn SeriesRepository>, jobs: Arc<dyn JobQueue>) -> Self {
        Self { repo, jobs }
    }

    /// Every registered rule, oldest first, seeded and user-written
    /// alike.
    ///
    /// Rules, not groups. "Which materials did this rule put on which
    /// key" is a different question whose shape follows from the reader
    /// asking it, and the reader this axis is ultimately for — a person
    /// promoting one series into a real Group, which is why a key is not
    /// one already ([`series`](crate::domain::series), "A key on the
    /// material, and not a Group") — does not exist yet; the
    /// `SeriesRepository` port doc records what that statement will owe
    /// when it does.
    pub async fn list(&self) -> Result<Vec<SeriesStrategyDto>, DomainError> {
        Ok(self
            .repo
            .list_strategies()
            .await?
            .iter()
            .map(registered_strategy_to_dto)
            .collect())
    }

    /// Registers a rule and asks for the keys it implies.
    ///
    /// The id is minted here rather than taken from the caller: it is
    /// what derived rows are filed under from the first derivation
    /// onward, and the response carries it so the caller holds the value
    /// its own `PATCH` and `DELETE` will name.
    pub async fn create(
        &self,
        command: CreateSeriesStrategyCommand,
        _attribution: &AttributionContext,
    ) -> Result<SeriesStrategyDto, DomainError> {
        let strategy = Strategy {
            id: StrategyId::new(),
            name: command.name,
            applies_to: parse_applies_to(&command.applies_to)?,
            decode: Decode::parse(&command.decode)?,
            include: parse_paths(command.include, "include")?,
            exclude: parse_paths(command.exclude, "exclude")?,
        };
        self.repo.create_strategy(&strategy, Utc::now()).await?;
        // A new rule has no rows at all, so every pair it names is
        // already in the walk's population — what is missing is somebody
        // asking for a pass. Without this the rule derives nothing until
        // the next launch, which is why a failure here is logged rather
        // than returned: see `enqueue_walk`, and note that a retry of
        // this call would register a second rule rather than repair the
        // first.
        self.enqueue_walk("create").await;
        self.find_dto(&strategy.id).await
    }

    /// Partially updates a rule, invalidating the keys it derived when —
    /// and only when — the change is one the derivation would read.
    ///
    /// # Why the clear runs before the write
    ///
    /// The two statements are not one transaction, so one of the two
    /// orders has to be picked for its failure, and they are not
    /// symmetric: **the walk repairs absence and never staleness.** Its
    /// population is pairs with no row, so a missing key is a key the
    /// next pass derives, while a key left behind under a rule that has
    /// changed is one no pass will ever look at again. Clearing first
    /// means a failed write leaves a library with no keys for a rule
    /// that did not change — which the walk fixes on its own. The
    /// reverse leaves keys nothing can notice.
    ///
    /// A pass already running is a separate matter and is not closed
    /// here: it holds a page of rules read before this edit, so rows it
    /// files afterwards are derived under the rule as it was. That is
    /// the window `series_derive_batch` records for its own read of
    /// `meta_kv`, one table over.
    ///
    /// **The enqueue at the end cannot be allowed to fail this call**,
    /// and the reason is specific to a partial update: a retry re-reads
    /// the rule, finds it already equal to what was asked for, and skips
    /// both the clear and the enqueue — so an error returned from here
    /// would hand the caller a repair that is a no-op by construction.
    /// [`enqueue_walk`](Self::enqueue_walk) holds the whole argument.
    pub async fn update(
        &self,
        command: UpdateSeriesStrategyCommand,
        _attribution: &AttributionContext,
    ) -> Result<SeriesStrategyDto, DomainError> {
        let id = parse_id(&command.id)?;
        let current = self
            .repo
            .find_strategy(&id)
            .await?
            .ok_or_else(|| DomainError::not_found("series strategy", id))?;

        let mut resolved = current.strategy.clone();
        if let Some(name) = command.name {
            resolved.name = name;
        }
        if let Some(applies_to) = command.applies_to {
            resolved.applies_to = parse_applies_to(&applies_to)?;
        }
        if let Some(decode) = command.decode {
            resolved.decode = Decode::parse(&decode)?;
        }
        if let Some(include) = command.include {
            resolved.include = parse_paths(include, "include")?;
        }
        if let Some(exclude) = command.exclude {
            resolved.exclude = parse_paths(exclude, "exclude")?;
        }

        // Asked once and reused, because a second call would be a second
        // chance for the two branches to disagree about whether this edit
        // invalidates — and clearing without enqueueing is the one
        // combination that leaves a library keyless.
        let invalidates = derives_differently(&current.strategy, &resolved);
        if invalidates {
            self.repo.clear_derived(&id).await?;
        }
        self.repo.update_strategy(&resolved, Utc::now()).await?;
        if invalidates {
            self.enqueue_walk("update").await;
        }
        self.find_dto(&id).await
    }

    /// Removes a rule. The keys derived under it go with it, by the
    /// schema's cascade.
    ///
    /// No guard, unlike `ModalityService::delete`. That one refuses
    /// while assets still carry the slug because nothing can recompute
    /// an asset's modality; a series key is recomputed from rows already
    /// in hand, and refusing here would refuse what the
    /// [`series`](crate::domain::series) module doc sells the axis on —
    /// changing a rule and watching the groups move.
    ///
    /// **A seeded rule is deletable too.** `system` records that a
    /// migration wrote the row and grants it nothing (V73's doc holds
    /// the argument); a later corrective migration is written to no-op
    /// on a row that is gone.
    ///
    /// Nothing is enqueued afterwards: the pairs the cascade freed
    /// belong to a rule that no longer exists, so there is nothing left
    /// to derive them under.
    pub async fn delete(
        &self,
        command: DeleteSeriesStrategyCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let id = parse_id(&command.id)?;
        self.repo.delete_strategy(&id).await
    }

    /// Asks for a derivation pass over every unanswered pair.
    ///
    /// **Not deduped against a queued walk**, unlike the one at startup.
    /// A pass survives a restart as a chained page carrying a cursor, and
    /// a cursor already past the pairs this call just freed is a pass
    /// that will never reach them — so "one is already queued" does not
    /// mean "the rows this edit is waiting on will be answered". A
    /// redundant pass costs a page of rows that all turn out to be
    /// answered.
    ///
    /// # A failed enqueue is logged, not returned
    ///
    /// Both neighbours on this axis already do that — the startup walk in
    /// `core_init` and `derive_series_after_hash` in the job handlers —
    /// and here the alternative is worse than merely inconsistent,
    /// because **the retry that a returned error invites cannot repair
    /// what the error left behind.**
    ///
    /// Follow it through on `update`: the rows are cleared, the rule is
    /// written, the enqueue fails (a busy queue, a shutdown, a client
    /// disconnecting so axum drops this future), and the caller sees
    /// `500`. It sends the identical `PATCH` again. The rule it now reads
    /// back *is* the edited one, so nothing differs, so nothing is
    /// cleared and nothing is enqueued — `200 OK`, over a library holding
    /// no keys under that rule. Every retry answers the same way, and the
    /// only recovery left is a restart.
    ///
    /// Swallowed, the mutation the caller asked for has happened, the
    /// `200` says so honestly, and what is outstanding is a derivation —
    /// which is **absence**, the one thing the walk repairs on its own
    /// (`update`'s doc holds that argument). The startup walk is the
    /// stated recovery and it needs no second restart, since the rows are
    /// missing rather than stale.
    ///
    /// The same reasoning covers `create`, where a returned error is
    /// worse still: the id is minted here, so a retry registers a
    /// **second** rule rather than colliding with the first, and both
    /// derive identical keys for every material forever.
    ///
    /// What this costs is that "the keys are coming" is not something the
    /// response can promise. It never was — the walk is asynchronous —
    /// and the log line is where the difference between "queued" and "not
    /// queued" is now recorded.
    async fn enqueue_walk(&self, occasion: &'static str) {
        if let Err(err) = self
            .jobs
            .enqueue(JobKind::SeriesDerive, serde_json::json!({ "batch": true }))
            .await
        {
            tracing::warn!(
                event = "diag.series_derive.enqueue_failed",
                occasion,
                error = %err,
                "the rule was written but nothing was asked to derive its \
                 series keys; the next startup walk will"
            );
        }
    }

    /// Re-reads one rule through the listing shape so a write path
    /// returns what is stored — including the `updated_at` it just
    /// moved — rather than what it sent.
    async fn find_dto(&self, id: &StrategyId) -> Result<SeriesStrategyDto, DomainError> {
        self.repo
            .find_strategy(id)
            .await?
            .as_ref()
            .map(registered_strategy_to_dto)
            .ok_or_else(|| DomainError::not_found("series strategy", id))
    }
}

/// Whether two rules would derive different keys — the test that decides
/// whether an edit invalidates.
///
/// Every field [`derive`](crate::domain::series::derive) reads is
/// compared and `name` is not, which is the whole content of "a rename
/// must not move a single key": a rule renamed and a rule rewritten
/// differ in what they cost, and treating them alike would either throw
/// away a library's keys over a spelling or leave them standing under a
/// rule nobody wrote.
///
/// Spelled as a comparison of the four rather than as a set of `if`s at
/// the call site so that a fifth deriving field added to
/// [`Strategy`] fails to compile here — the destructuring below is what
/// makes that true. It is not the whole of that guard, because the
/// minimal repair for the resulting `E0027` is `new_field: _`, which the
/// two lines above establish as the house form; the test
/// `the_invalidation_test_names_every_field_a_strategy_declares` is what
/// closes it, on the same terms `Decode::ALL`'s doc argues for reading
/// the enum's own source.
///
/// # The path lists are compared as sets
///
/// The question this function asks is "would these two rules derive
/// different keys", and order and repetition inside `include` /
/// `exclude` are things the derivation cannot see:
/// [`select`](crate::domain::series) files each selected sub-tree into a
/// `BTreeMap` keyed by its path, so two identical paths make one entry
/// and the entries come out in path order however they were written; and
/// exclusion is a set of deletions, which commute and are idempotent (an
/// excluded path that covers another leaves the same selection either way
/// round).
///
/// Storage keeps the order and the repetition on purpose — V73's doc
/// refuses a side table precisely so a person reads back the rule they
/// wrote — so the two spellings are a real difference in the row and no
/// difference at all in the key. Compared as written, reordering two
/// paths would clear a library's keys and re-derive them byte for byte,
/// which is exactly the cost `UpdateSeriesStrategyCommand`'s doc claims
/// to have avoided for `name`.
fn derives_differently(before: &Strategy, after: &Strategy) -> bool {
    let Strategy {
        id: _,
        name: _,
        applies_to,
        decode,
        include,
        exclude,
    } = before;
    *applies_to != after.applies_to
        || *decode != after.decode
        || as_set(include) != as_set(&after.include)
        || as_set(exclude) != as_set(&after.exclude)
}

/// A path list as the derivation sees it: deduplicated and ordered.
fn as_set(paths: &[Path]) -> std::collections::BTreeSet<&Path> {
    paths.iter().collect()
}

/// Reads the id a path segment named.
fn parse_id(raw: &str) -> Result<StrategyId, DomainError> {
    uuid::Uuid::parse_str(raw.trim())
        .map(StrategyId::from_uuid)
        .map_err(|_| DomainError::Validation(format!("{raw:?} is not a strategy id")))
}

/// Reads the media type a rule claims, refusing one that could never
/// name a material.
///
/// [`MimeType::parse`] is total — it answers for every string, including
/// `""`, `"png"` and `"image/*"`, all of which become an ordinary value
/// that stores without complaint. [`Strategy::claims`] then compares
/// parsed equality against `material.mime`, so each of those matches
/// nothing, ever, and the rule reads from the outside exactly like a
/// broken one: it derives no key, and this axis publishes no surface on
/// which the author could see why (the schema resource says so in its
/// own last section). A blank value is the least likely of the three to
/// be typed; `"png"` is the most.
///
/// So the test is **shape, not vocabulary**: one `/`, both halves
/// non-empty, no `*`. Checked against the parsed token rather than the
/// raw string, so the normalisation `MimeType::parse` performs —
/// trimming, lowercasing, dropping `;` parameters — is what is judged,
/// and `" IMAGE/PNG; charset=binary "` passes as the `image/png` it is.
///
/// **A subtype this build has never heard of passes**, and that is not
/// an oversight. `material.mime` holds whatever an importer declared,
/// and [`MimeType::parse`]'s own doc keeps the family readable for
/// exactly that case (`Image(Other(..))`); refusing what this build
/// cannot name would refuse rules against formats a library already
/// holds. The wildcard is refused instead, because it is the one shape
/// that *looks* like it widens a rule and in fact narrows it to nothing.
fn parse_applies_to(raw: &str) -> Result<MimeType, DomainError> {
    let mime = MimeType::parse(raw);
    let token = mime.as_str();
    let shaped = match token.split_once('/') {
        Some((kind, subtype)) => !kind.is_empty() && !subtype.is_empty() && !subtype.contains('/'),
        None => false,
    };
    if !shaped || token.contains('*') {
        return Err(DomainError::Validation(format!(
            "`applies_to` is the one media type a rule is written against \
             and is compared for equality against a material's own, so it \
             has to be a `type/subtype` pair with no wildcard; {raw:?} \
             would match nothing"
        )));
    }
    Ok(mime)
}

/// Reads one of the two path lists, refusing a path with no segments.
///
/// An empty path is inert in both lists and inert in different ways —
/// it selects nothing in `include`, drops nothing in `exclude` — so the
/// rule stored is not the rule the author wrote and nothing later says
/// so. The domain stays lenient about it on purpose (a row already
/// carrying one has to keep deriving something); the refusal belongs at
/// the door, where there is still somebody to tell.
fn parse_paths(raw: Vec<Vec<String>>, column: &str) -> Result<Vec<Path>, DomainError> {
    for (index, segments) in raw.iter().enumerate() {
        if segments.is_empty() {
            return Err(DomainError::Validation(format!(
                "{column}[{index}] is a path with no segments, which selects \
                 nothing and drops nothing; name a keyword or leave the list out"
            )));
        }
    }
    Ok(raw.into_iter().map(Path::new).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::repository::{RegisteredStrategy, UnderivedSeries};
    use crate::domain::series::SeriesKey;
    use crate::domain::value::AssetId;
    use async_trait::async_trait;
    use chrono::{DateTime, Utc};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A queue that refuses everything — the shutdown, the busy pool,
    /// the dropped future.
    struct ClosedQueue;

    #[async_trait]
    impl JobQueue for ClosedQueue {
        async fn enqueue(
            &self,
            _kind: JobKind,
            _payload: serde_json::Value,
        ) -> Result<String, DomainError> {
            Err(DomainError::Infra(anyhow::anyhow!("the queue is closed")))
        }
    }

    /// One rule in memory, with the two verbs an edit uses and a counter
    /// on the invalidation so a test can say whether it ran.
    struct OneRule {
        stored: Mutex<RegisteredStrategy>,
        cleared: AtomicUsize,
    }

    impl OneRule {
        fn holding(strategy: Strategy) -> Arc<Self> {
            let at = DateTime::from_timestamp_millis(1_786_320_000_000).unwrap();
            Arc::new(Self {
                stored: Mutex::new(RegisteredStrategy {
                    strategy,
                    system: false,
                    created_at: at,
                    updated_at: at,
                }),
                cleared: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl SeriesRepository for OneRule {
        async fn list_strategies(&self) -> Result<Vec<RegisteredStrategy>, DomainError> {
            Ok(vec![self.stored.lock().unwrap().clone()])
        }

        async fn find_strategy(
            &self,
            id: &StrategyId,
        ) -> Result<Option<RegisteredStrategy>, DomainError> {
            let stored = self.stored.lock().unwrap().clone();
            Ok((stored.strategy.id == *id).then_some(stored))
        }

        async fn create_strategy(
            &self,
            _strategy: &Strategy,
            _at: DateTime<Utc>,
        ) -> Result<(), DomainError> {
            unimplemented!("not reached by the update path under test")
        }

        async fn update_strategy(
            &self,
            strategy: &Strategy,
            at: DateTime<Utc>,
        ) -> Result<(), DomainError> {
            let mut stored = self.stored.lock().unwrap();
            stored.strategy = strategy.clone();
            stored.updated_at = at;
            Ok(())
        }

        async fn delete_strategy(&self, _id: &StrategyId) -> Result<(), DomainError> {
            unimplemented!("not reached by the update path under test")
        }

        async fn clear_derived(&self, _id: &StrategyId) -> Result<u64, DomainError> {
            self.cleared.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        }

        async fn record(
            &self,
            _asset_id: &AssetId,
            _ord: u32,
            _strategy_id: &StrategyId,
            _key: &SeriesKey,
            _at: DateTime<Utc>,
        ) -> Result<(), DomainError> {
            unimplemented!("not reached by the update path under test")
        }

        async fn scan_underived(
            &self,
            _after: Option<(&AssetId, u32, &StrategyId)>,
            _limit: u32,
        ) -> Result<Vec<UnderivedSeries>, DomainError> {
            unimplemented!("not reached by the update path under test")
        }
    }

    fn unattributed() -> AttributionContext {
        AttributionContext::asserted(None, None)
            .expect("stating no author and no operator is always valid")
    }

    /// **An edit whose enqueue fails still answers `200`, because the
    /// retry a `500` would invite is a no-op.**
    ///
    /// The sequence is the one that has no other repair: the rows are
    /// cleared, the rule is written, the queue refuses. Returning the
    /// error would tell the caller to try again — and the second attempt
    /// re-reads a rule that already equals what it is asking for, so it
    /// clears nothing and enqueues nothing and answers `200` over a
    /// library with no keys under that rule. The second `update` below is
    /// exactly that attempt, and the assertion on `cleared` is what shows
    /// it doing nothing.
    ///
    /// What is asserted instead is that the mutation the caller asked for
    /// landed and was reported as landed. The derivation is outstanding,
    /// the log line says so, and the startup walk is the recovery — which
    /// works precisely because what is missing is rows rather than wrong
    /// rows (`update`'s doc holds that argument).
    ///
    /// Checked by mutation on 2026-08-11 by restoring the earlier
    /// `self.enqueue_walk().await?` — this failed at the first `update`:
    /// *"an edit whose enqueue failed must still report the edit it
    /// made: Infra(the queue is closed)"*. Restored, it passes.
    #[tokio::test]
    async fn an_edit_whose_enqueue_fails_still_reports_the_edit_it_made() {
        let repo = OneRule::holding(rule(&[&["gen", "recipe"]]));
        let id = repo.stored.lock().unwrap().strategy.id;
        let service = SeriesStrategyService::new(repo.clone(), Arc::new(ClosedQueue));

        let edit = |include: Vec<Vec<String>>| UpdateSeriesStrategyCommand {
            id: id.to_string(),
            name: None,
            applies_to: None,
            decode: None,
            include: Some(include),
            exclude: None,
        };

        let dto = service
            .update(
                edit(vec![vec!["gen".into(), "seed".into()]]),
                &unattributed(),
            )
            .await
            .unwrap_or_else(|err| {
                panic!("an edit whose enqueue failed must still report the edit it made: {err:?}")
            });
        assert_eq!(
            dto.include,
            vec![vec!["gen".to_string(), "seed".to_string()]]
        );
        assert_eq!(
            repo.cleared.load(Ordering::SeqCst),
            1,
            "the keys were invalidated, which is the half that did happen"
        );

        // The retry a returned error would have invited. It changes
        // nothing and invalidates nothing — which is why the error is not
        // returned.
        service
            .update(
                edit(vec![vec!["gen".into(), "seed".into()]]),
                &unattributed(),
            )
            .await
            .expect("re-sending an identical edit is not an error");
        assert_eq!(
            repo.cleared.load(Ordering::SeqCst),
            1,
            "the retry cleared nothing, so it could not have re-asked for a \
             derivation either — there is nothing for a caller to repair by \
             repeating itself"
        );
    }

    fn rule(include: &[&[&str]]) -> Strategy {
        Strategy {
            id: StrategyId::new(),
            name: "under test".to_string(),
            applies_to: MimeType::parse("image/png"),
            decode: Decode::RawJson,
            include: include
                .iter()
                .map(|path| Path::new(path.iter().copied()))
                .collect(),
            exclude: vec![],
        }
    }

    /// A rename does not invalidate; every deriving field does.
    ///
    /// The rename case is the one worth a test rather than a comment:
    /// it is the difference between editing a label and re-deriving a
    /// library, and an implementation that compared the whole rule
    /// (`before != after`) would pass every other assertion here.
    #[test]
    fn renaming_derives_the_same_and_every_other_field_does_not() {
        let before = rule(&[&["vdsl", "script"]]);

        let mut renamed = before.clone();
        renamed.name = "something else".to_string();
        assert!(
            !derives_differently(&before, &renamed),
            "a rename must not cost a single key"
        );

        let mut retyped = before.clone();
        retyped.applies_to = MimeType::parse("image/jpeg");
        let mut redecoded = before.clone();
        redecoded.decode = Decode::Base64Json;
        let mut reselected = before.clone();
        reselected.include = vec![Path::new(["vdsl", "version"])];
        let mut excluded = before.clone();
        excluded.exclude = vec![Path::new(["vdsl", "timestamp"])];

        for (what, after) in [
            ("applies_to", retyped),
            ("decode", redecoded),
            ("include", reselected),
            ("exclude", excluded),
        ] {
            assert!(
                derives_differently(&before, &after),
                "{what} is an input to the key; changing it invalidates"
            );
        }

        // The mime is compared parsed, not as text: `IMAGE/PNG` and
        // `image/png` are the one format `Strategy::claims` treats them
        // as, so re-stating a rule in different case is not an edit.
        let mut respelled = before.clone();
        respelled.applies_to = MimeType::parse(" IMAGE/PNG; charset=binary ");
        assert!(!derives_differently(&before, &respelled));
    }

    /// Reordering or repeating a path is a change to the row and not to
    /// the key, so it does not invalidate.
    ///
    /// Both properties come from the derivation, not from taste:
    /// `select` files each selected sub-tree into a `BTreeMap` keyed by
    /// its path, so order is the map's and a repeat is one entry; and
    /// exclusion is a set of deletions, which commute. Storage keeps the
    /// spelling the author used on purpose (V73), which is exactly why
    /// the two can differ here — and comparing them as written would
    /// clear a library's keys and re-derive them byte for byte.
    ///
    /// The last assertion is the one that keeps this from being a test of
    /// "any two path lists are the same": adding a path is still an edit.
    #[test]
    fn reordering_the_paths_of_a_rule_derives_the_same_keys() {
        let before = rule(&[&["gen", "recipe"], &["gen", "seed"]]);

        let mut reordered = before.clone();
        reordered.include.reverse();
        assert_ne!(
            reordered.include, before.include,
            "the fixture says nothing unless the two lists differ as written"
        );
        assert!(!derives_differently(&before, &reordered));

        let mut repeated = before.clone();
        repeated.include.push(Path::new(["gen", "recipe"]));
        assert!(!derives_differently(&before, &repeated));

        // …and the same for the other list.
        let mut excluding = before.clone();
        excluding.exclude = vec![Path::new(["gen", "a"]), Path::new(["gen", "b"])];
        let mut excluding_reordered = excluding.clone();
        excluding_reordered.exclude.reverse();
        assert!(derives_differently(&before, &excluding));
        assert!(!derives_differently(&excluding, &excluding_reordered));

        // A path nobody named before is an edit, whatever the order.
        let mut widened = before.clone();
        widened.include.push(Path::new(["gen", "version"]));
        assert!(derives_differently(&before, &widened));
    }

    /// The names a `{}`-delimited block declares as fields, read off a
    /// source file.
    ///
    /// A field line is `name:` or `name: _` at the block's own
    /// indentation — doc comments and attributes do not match, and
    /// nothing either block below contains does either. Crude on
    /// purpose, the same trade
    /// `series::tests::declared_variants` makes: what it has to survive
    /// is a field being *added*, and there is no way to add one without a
    /// line of that shape.
    fn field_names(source: &str, opening: &str, closing: &str) -> Vec<(String, bool)> {
        let body = source
            .split_once(opening)
            .unwrap_or_else(|| panic!("`{opening}` appears in this source"))
            .1
            .split_once(closing)
            .unwrap_or_else(|| panic!("`{opening}` is closed by `{closing}`"))
            .0;
        body.lines()
            .filter_map(|line| {
                // A declaration is `pub name: Type,`; a destructured
                // binding is `name,` or, when it is dropped, `name: _,`.
                let trimmed = line.trim().trim_start_matches("pub ").trim_end_matches(',');
                let (name, ignored) = match trimmed.split_once(':') {
                    Some((name, rest)) => (name.trim(), rest.trim().starts_with('_')),
                    None => (trimmed, false),
                };
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
                {
                    return None;
                }
                Some((name.to_string(), ignored))
            })
            .collect()
    }

    /// [`derives_differently`] names every field [`Strategy`] declares,
    /// and ignores exactly the two that cannot move a key.
    ///
    /// The destructure catches an added field as `E0027`, and that is
    /// where the guard would stop being one: the smallest edit that
    /// silences the compiler is `new_field: _`, and the two lines
    /// directly above it show that spelling as normal. A field added to
    /// the rule and then quietly ignored here is an edit that changes
    /// what a rule derives and does not invalidate anything — the keys
    /// stay under a rule nobody wrote, which is the failure the whole
    /// clear-then-write ordering exists to avoid.
    ///
    /// So the *set* is asserted rather than the arity, from the two
    /// sources themselves. This is the guard `Decode::ALL`'s doc spends
    /// four paragraphs arguing for one type over: a `match` forces an
    /// arm, never a list entry, and reading the declaration is what
    /// closes the gap without a build dependency.
    ///
    /// Checked by mutation on 2026-08-11 by adding `applies_to: _` to the
    /// destructure (dropping the mime from the comparison, which the
    /// compiler accepts): *"these fields are declared on `Strategy` and
    /// ignored by the invalidation test: [\"applies_to\"]"*. Restored, it
    /// passes.
    #[test]
    fn the_invalidation_test_names_every_field_a_strategy_declares() {
        let declared: Vec<String> = field_names(
            include_str!("../domain/series.rs"),
            "pub struct Strategy {",
            "\n}",
        )
        .into_iter()
        .map(|(name, _)| name)
        .collect();
        assert!(
            declared.len() >= 4,
            "the parse, not the struct, is wrong: {declared:?}"
        );

        let destructured = field_names(
            include_str!("series_strategy_service.rs"),
            "let Strategy {",
            "\n    } = before;",
        );
        assert_eq!(
            destructured
                .iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
            declared.iter().collect::<Vec<_>>(),
            "the destructure and the declaration name different fields"
        );

        let ignored: Vec<&str> = destructured
            .iter()
            .filter(|(_, is_ignored)| *is_ignored)
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(
            ignored,
            vec!["id", "name"],
            "these fields are declared on `Strategy` and ignored by the \
             invalidation test: {:?}. A field the derivation reads must be \
             compared, or editing it leaves every key derived under the old \
             rule standing",
            ignored
                .iter()
                .filter(|name| !["id", "name"].contains(*name))
                .collect::<Vec<_>>()
        );
    }

    /// The refusals, each against a value the schema would have stored.
    #[test]
    fn a_rule_the_derivation_could_not_carry_out_is_refused_at_the_door() {
        // A token no variant spells. It used to be `"exif"`, which is
        // the shape of the case — a rule registered against a decoder
        // that arrived later — and stopped being an example of it when
        // that decoder arrived.
        let unknown = Decode::parse("prose_pairs").expect_err("not a decoder this build ships");
        assert!(matches!(unknown, DomainError::Validation(_)), "{unknown:?}");

        // Every shape that parses into a value the column would store and
        // `Strategy::claims` could never match. `"png"` is the one a
        // person actually types; `"image/*"` is the one the schema
        // resource promises is refused.
        for unmatchable in [
            "", "   ", "png", "image/", "/png", "image/*", "*/*", "a/b/c",
        ] {
            let refused = parse_applies_to(unmatchable)
                .expect_err("{unmatchable:?} can never name a material");
            match &refused {
                DomainError::Validation(message) => assert!(
                    message.contains("applies_to"),
                    "{unmatchable:?}: the message names the field: {message}"
                ),
                other => panic!("{unmatchable:?}: expected a Validation error, got {other:?}"),
            }
        }
        // …and the near miss, which is the reason the test is on shape
        // rather than on vocabulary: a subtype this build has never heard
        // of is a media type a material can legitimately carry, so a rule
        // against it has to register.
        assert!(parse_applies_to("application/x-nobody-ships-this").is_ok());
        assert!(parse_applies_to("image/jxl").is_ok());
        // Parameters and case are the parser's business, not this
        // function's — the check runs on what it produced.
        assert_eq!(
            parse_applies_to(" IMAGE/PNG; charset=binary ").unwrap(),
            MimeType::parse("image/png")
        );

        for column in ["include", "exclude"] {
            let empty = parse_paths(vec![vec!["vdsl".to_string()], vec![]], column)
                .expect_err("an empty path names nothing");
            match &empty {
                DomainError::Validation(message) => assert!(
                    message.contains(column) && message.contains('1'),
                    "the message names the list and the offending entry: {message}"
                ),
                other => panic!("expected a Validation error, got {other:?}"),
            }
        }
        // An empty *list* is a real rule and stays one: no include means
        // the whole of the container's metadata, which is what makes an
        // exclude-only rule expressible.
        assert_eq!(parse_paths(Vec::new(), "include").unwrap(), Vec::new());
    }
}
