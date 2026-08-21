//! Work use cases — opening a line of work, writing passes, looking at
//! what the line did, and ending it.
//!
//! ```text
//!   open      reads the line's head, writes the work log
//!   push      writes the work log. does not read the line
//!   resolve   reads the line, writes the work log
//!   close     reads both, writes both — the only one
//!
//!   collisions / behind    read both, write nothing
//! ```
//!
//! Four verbs and two questions, and only one of them touches a line's
//! history. That asymmetry is the point of the design rather than an
//! accident of it: the operation that happens most often, writing a
//! pass, never reads the line at all, so two people working against
//! one line do not contend until one of them finishes.
//!
//! # What this service is allowed to decide
//!
//! Nothing. It loads what the model needs, calls it, writes back what
//! came out, and — for the one operation that can lose a race — reads
//! again and asks again. Every refusal in here comes from the model or
//! from a port.
//!
//! # Losing the race is not an error to report
//!
//! Two pieces of work can finish against one line at the same moment,
//! and only one of them lands on the head. The other is told, and this
//! service reads the line again and decides again rather than handing
//! that back to a caller.
//!
//! It matters that this is a fresh decision and not a retry of the old
//! one. Reading again means normalising against a line that has moved,
//! so what the work still changes may be less than it was, and what it
//! now collides with may be more. Handing back the same answer would
//! be writing a decision made against a line that no longer exists.

use std::sync::Arc;

use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::{Actors, StoreClient};
use crate::domain::forge::clock::Clock;
use crate::domain::forge::closings::Closings;
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::change::{Collision, collisions, since};
use crate::domain::forge::model::closing::close;
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::op::{Op, OpKind};
use crate::domain::forge::model::pursuit::{Intent, Outcome, Pursuit, Round};
use crate::domain::forge::model::react::react;
use crate::domain::forge::model::value::{ChangePointId, LineId, PursuitId};
use crate::domain::forge::pursuits::Pursuits;
use crate::domain::forge::strategies::Strategies;
use crate::domain::value::PersonaId;
use crate::error::DomainError;

/// How many times a close is decided, counting the first.
///
/// So five is one attempt and four re-decisions. Bounded because a
/// line under constant traffic would otherwise keep a caller here
/// indefinitely; reaching the bound is not a failure of the work but
/// the line being busier than this caller is patient, so it comes back
/// as a conflict and asking again is reasonable.
const ATTEMPTS: usize = 5;

/// Work use-case service.
pub struct PursuitService {
    pursuits: Arc<dyn Pursuits>,
    lines: Arc<dyn Lines>,
    closings: Arc<dyn Closings>,
    strategies: Arc<dyn Strategies>,
    store: StoreClient,
    actors: Arc<dyn Actors>,
    clock: Arc<dyn Clock>,
}

impl PursuitService {
    /// Wires the service around its ports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pursuits: Arc<dyn Pursuits>,
        lines: Arc<dyn Lines>,
        closings: Arc<dyn Closings>,
        strategies: Arc<dyn Strategies>,
        store: StoreClient,
        actors: Arc<dyn Actors>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            pursuits,
            lines,
            closings,
            strategies,
            store,
            actors,
            clock,
        }
    }

    /// Opens work against a line, cut from wherever the line is now.
    pub async fn open(
        &self,
        of: &LineId,
        parent: Option<PursuitId>,
        intent: Intent,
        by: &AttributionContext,
    ) -> Result<Pursuit, DomainError> {
        let line = self.line(of).await?;
        let pursuit = Pursuit::open(line.id(), parent, line.head(), intent, self.act(by).await?);
        self.pursuits.open(&pursuit).await?;
        Ok(pursuit)
    }

    /// Reads work back whole, every pass included.
    pub async fn get(&self, id: &PursuitId) -> Result<Pursuit, DomainError> {
        self.pursuits
            .get(id)
            .await?
            .ok_or_else(|| DomainError::not_found("pursuit", id))
    }

    /// Writes a pass.
    ///
    /// The line is not read. What the work says means nothing on its
    /// own — it is only measured against a line when somebody takes a
    /// change in or ends the work — so this is the operation that can
    /// run all day without touching anything anybody else is using.
    ///
    /// What it does check is that the content each operation points at
    /// is real and this persona's, which is a question for the layer
    /// that holds the bytes. The persona comes from the caller: the
    /// forge does not know whose a line is, and asking about content
    /// without saying whose it is would be asking a question with two
    /// right answers.
    pub async fn push(
        &self,
        id: &PursuitId,
        persona: &PersonaId,
        ops: Vec<Op>,
        note: Option<String>,
        by: &AttributionContext,
    ) -> Result<Round, DomainError> {
        for op in &ops {
            // Only the two verbs that put content on a line name any:
            // a rename moves a name and a removal moves nothing, so
            // there is nothing to ask about either.
            let content = match op.kind() {
                OpKind::Add { content, .. } | OpKind::Replace { content } => *content,
                OpKind::Rename { .. } | OpKind::Remove => continue,
            };
            if !self.store.holds(persona, &content).await? {
                return Err(DomainError::Validation(
                    "an operation points at content this persona does not hold".into(),
                ));
            }
        }

        let pursuit = self.get(id).await?;
        let round = Round::new(pursuit.log().head(), ops, note, self.act(by).await?)?;
        self.pursuits.push(id, round.parent(), &round).await?;
        Ok(round)
    }

    /// Lets the line's rule answer whatever this work collides with.
    ///
    /// Writes at most one pass, as the server. A rule that leaves
    /// collisions to a person writes nothing and this reports that
    /// nothing was written — the collision is still there to be read.
    ///
    /// Nothing about the line is written. Resolving is work deciding
    /// something, and what it decides goes in the work log like any
    /// other decision.
    pub async fn resolve(
        &self,
        id: &PursuitId,
        by: &AttributionContext,
    ) -> Result<Option<Round>, DomainError> {
        let pursuit = self.get(id).await?;
        let line = self.line(&pursuit.of()).await?;
        let rule = self.rule(&line)?;
        let server = self.actors.server().await?;

        let Some(round) = react(&line, &pursuit, rule, server, self.act(by).await?)? else {
            return Ok(None);
        };

        self.pursuits.push(id, round.parent(), &round).await?;
        Ok(Some(round))
    }

    /// Ends the work, and puts what it says on the line if it says
    /// anything.
    ///
    /// The one call that writes to both logs, and it writes them as one
    /// — see [`Closings`].
    pub async fn close(
        &self,
        id: &PursuitId,
        outcome: Outcome,
        note: Option<String>,
        by: &AttributionContext,
    ) -> Result<(), DomainError> {
        for _ in 0..ATTEMPTS {
            let pursuit = self.get(id).await?;
            let line = self.line(&pursuit.of()).await?;
            let on = line.head();

            // Stamped per attempt rather than once. Each of these is a
            // fresh decision against a line that has moved, and the
            // time on the one that lands is when it was decided — not
            // when somebody first asked.
            let act = self.act(by).await?;
            let closing = close(&line, &pursuit, outcome, note.clone(), act)?;

            return match self.closings.commit(&line.id(), id, on, &closing).await {
                // Somebody landed first. Read again and decide again.
                Err(DomainError::Conflict(_)) => continue,
                other => other,
            };
        }

        Err(DomainError::Conflict(
            "the line moved every time this work tried to land on it".into(),
        ))
    }

    /// What this work would write that the line has already moved, and
    /// that this work has not looked at.
    ///
    /// Derived from the two logs on every call, so it cannot go stale
    /// and there is no flag anybody has to clear. What clears a
    /// collision is [`resolve`](Self::resolve) — looking, and then
    /// writing the axis anyway.
    pub async fn collisions(&self, id: &PursuitId) -> Result<Vec<Collision>, DomainError> {
        let pursuit = self.get(id).await?;
        let line = self.line(&pursuit.of()).await?;

        Ok(collisions(&line, &pursuit)?)
    }

    /// Every piece of work against a line, open or ended.
    ///
    /// What was abandoned is in it. A listing that showed only live
    /// work would hide what this layer is for.
    pub async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
        self.line(line).await?;
        self.pursuits.of_line(line).await
    }

    /// The work filed under a larger piece of work.
    pub async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError> {
        self.get(parent).await?;
        self.pursuits.children(parent).await
    }

    /// Everything the line has recorded since this work was cut from
    /// it.
    ///
    /// Not a list of problems: most of it will not touch anything this
    /// work is doing. What it answers is how far the line has moved
    /// underneath, which is worth showing somebody next to what did
    /// collide.
    pub async fn behind(&self, id: &PursuitId) -> Result<Vec<ChangePointId>, DomainError> {
        let pursuit = self.get(id).await?;
        let line = self.line(&pursuit.of()).await?;

        Ok(since(line.history(), pursuit.base())?
            .iter()
            .map(|change| change.id())
            .collect())
    }

    async fn line(&self, id: &LineId) -> Result<Line, DomainError> {
        self.lines
            .get(id)
            .await?
            .ok_or_else(|| DomainError::not_found("line", id))
    }

    /// The rule this line settles by.
    ///
    /// Refuses rather than falling back: a line pointing at a rule this
    /// instance does not carry has to say so, because settling it by
    /// some other rule would settle it by one nobody chose, and no
    /// record would say that had happened.
    fn rule(
        &self,
        line: &Line,
    ) -> Result<&dyn crate::domain::forge::model::strategy::Strategy, DomainError> {
        self.strategies.get(line.strategy()).ok_or_else(|| {
            DomainError::Validation(format!(
                "this line settles by {:?}, which this instance does not carry",
                line.strategy()
            ))
        })
    }

    /// Stamps an act: now, by whoever this write is from.
    async fn act(&self, by: &AttributionContext) -> Result<Act, DomainError> {
        Ok(Act::new(
            self.clock.now(),
            Actor::User(self.actors.resolve(by).await?),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::{DateTime, TimeZone, Utc};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    use crate::application::forge::LineService;
    use crate::domain::forge::boundary::Store;
    use crate::domain::forge::model::closing::Closing;
    use crate::domain::forge::model::strategy::Strategy;
    use crate::domain::forge::model::value::{ActorId, Content, EntryId, Name, NodeId, StrategyId};
    use crate::domain::forge::strategies::{Builtin, MainlineFirst};
    use crate::domain::value::AssetId;

    /// Everything the services keep, held in memory.
    ///
    /// One fake behind every port rather than six, because each test
    /// needs the same wiring and what differs between them is the
    /// state, not the shape.
    #[derive(Default)]
    struct World {
        lines: Mutex<Vec<Line>>,
        pursuits: Mutex<Vec<Pursuit>>,
        /// How many closes have to lose the head before one lands.
        losses: Mutex<usize>,
        /// How many closes were attempted, won or lost.
        attempts: Mutex<usize>,
        /// Whether the layer below claims to hold anything.
        holds: bool,
        /// Handles already minted, keyed by who asked.
        actors: Mutex<BTreeMap<String, ActorId>>,
        /// This server's handle, once anybody has asked for it.
        server: Mutex<Option<ActorId>>,
    }

    impl World {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                holds: true,
                ..Self::default()
            })
        }

        fn line(&self, id: &LineId) -> Option<Line> {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .find(|line| line.id() == *id)
                .cloned()
        }

        fn pursuit(&self, id: &PursuitId) -> Option<Pursuit> {
            self.pursuits
                .lock()
                .unwrap()
                .iter()
                .find(|work| work.id() == *id)
                .cloned()
        }

        fn put_line(&self, line: Line) {
            let mut held = self.lines.lock().unwrap();
            match held.iter().position(|kept| kept.id() == line.id()) {
                Some(at) => held[at] = line,
                None => held.push(line),
            }
        }

        fn put(&self, pursuit: Pursuit) {
            let mut held = self.pursuits.lock().unwrap();
            match held.iter().position(|work| work.id() == pursuit.id()) {
                Some(at) => held[at] = pursuit,
                None => held.push(pursuit),
            }
        }
    }

    #[async_trait]
    impl Lines for Arc<World> {
        async fn open(&self, line: &Line) -> Result<(), DomainError> {
            self.put_line(line.clone());
            Ok(())
        }

        async fn get(&self, id: &LineId) -> Result<Option<Line>, DomainError> {
            Ok(self.line(id))
        }

        async fn list(&self) -> Result<Vec<Line>, DomainError> {
            Ok(self.lines.lock().unwrap().clone())
        }

        async fn rename(&self, _: &LineId, _: &Name, _: &Act) -> Result<(), DomainError> {
            Ok(())
        }

        async fn set_strategy(
            &self,
            _: &LineId,
            _: &StrategyId,
            _: &Act,
        ) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[async_trait]
    impl Pursuits for Arc<World> {
        async fn open(&self, pursuit: &Pursuit) -> Result<(), DomainError> {
            self.put(pursuit.clone());
            Ok(())
        }

        async fn get(&self, id: &PursuitId) -> Result<Option<Pursuit>, DomainError> {
            Ok(self.pursuit(id))
        }

        async fn of_line(&self, line: &LineId) -> Result<Vec<Pursuit>, DomainError> {
            Ok(self
                .pursuits
                .lock()
                .unwrap()
                .iter()
                .filter(|work| work.of() == *line)
                .cloned()
                .collect())
        }

        async fn children(&self, parent: &PursuitId) -> Result<Vec<Pursuit>, DomainError> {
            Ok(self
                .pursuits
                .lock()
                .unwrap()
                .iter()
                .filter(|work| work.parent() == Some(*parent))
                .cloned()
                .collect())
        }

        async fn push(&self, id: &PursuitId, on: NodeId, round: &Round) -> Result<(), DomainError> {
            let mut work = self
                .pursuit(id)
                .ok_or_else(|| DomainError::not_found("pursuit", id))?;
            if work.log().head() != on {
                return Err(DomainError::Conflict("the work log moved".into()));
            }
            work.push(round.clone())?;
            self.put(work);
            Ok(())
        }
    }

    #[async_trait]
    impl Closings for Arc<World> {
        async fn commit(
            &self,
            line: &LineId,
            pursuit: &PursuitId,
            on: ChangePointId,
            closing: &Closing,
        ) -> Result<(), DomainError> {
            *self.attempts.lock().unwrap() += 1;
            {
                let mut losses = self.losses.lock().unwrap();
                if *losses > 0 {
                    *losses -= 1;
                    return Err(DomainError::Conflict("somebody landed first".into()));
                }
            }

            let mut held = self
                .line(line)
                .ok_or_else(|| DomainError::not_found("line", line))?;
            if held.head() != on {
                return Err(DomainError::Conflict("the line moved".into()));
            }
            let mut work = self
                .pursuit(pursuit)
                .ok_or_else(|| DomainError::not_found("pursuit", pursuit))?;

            closing.clone().apply(&mut held, &mut work)?;

            self.put_line(held);
            self.put(work);
            Ok(())
        }
    }

    #[async_trait]
    impl Actors for Arc<World> {
        /// The one property this port exists for: the same writer
        /// resolves to the same handle every time. A fake that minted
        /// a fresh id per call would let a service depend on that
        /// being false without anything noticing.
        async fn resolve(&self, by: &AttributionContext) -> Result<ActorId, DomainError> {
            let mut known = self.actors.lock().unwrap();
            let key = format!("{by:?}");
            Ok(*known.entry(key).or_default())
        }

        async fn server(&self) -> Result<ActorId, DomainError> {
            Ok(*self.server.lock().unwrap().get_or_insert_with(ActorId::new))
        }
    }

    #[async_trait]
    impl Store for Arc<World> {
        async fn owns(&self, _: &PersonaId, _: &AssetId) -> Result<bool, DomainError> {
            Ok(self.holds)
        }
    }

    struct Fixed(DateTime<Utc>);

    impl Clock for Fixed {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 21, 12, minute, 0).unwrap()
    }

    fn services(world: &Arc<World>) -> (LineService, PursuitService) {
        let clock: Arc<dyn Clock> = Arc::new(Fixed(at(0)));
        let rules = Arc::new(Builtin::default());
        (
            LineService::new(
                Arc::new(world.clone()),
                rules.clone(),
                Arc::new(world.clone()),
                clock.clone(),
            ),
            PursuitService::new(
                Arc::new(world.clone()),
                Arc::new(world.clone()),
                Arc::new(world.clone()),
                rules,
                StoreClient::new(Arc::new(world.clone())),
                Arc::new(world.clone()),
                clock,
            ),
        )
    }

    fn by() -> AttributionContext {
        AttributionContext::owner_surface()
    }

    fn name(text: &str) -> Name {
        Name::new(text).unwrap()
    }

    fn content() -> Content {
        Content::from_uuid(Uuid::now_v7())
    }

    fn persona() -> PersonaId {
        PersonaId::new()
    }

    async fn opened(world: &Arc<World>) -> (LineService, PursuitService, Line) {
        let (lines, work) = services(world);
        let line = lines
            .open(name("ROOT"), MainlineFirst.id(), &by())
            .await
            .unwrap();
        (lines, work, line)
    }

    #[tokio::test]
    async fn a_line_opens_with_a_head_and_nothing_on_it() {
        let world = World::new();
        let (lines, _, line) = opened(&world).await;

        let read = lines.get(&line.id()).await.unwrap();

        assert_eq!(read.head(), line.head());
        assert!(lines.states(&line.id()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_rule_this_instance_does_not_carry_cannot_be_chosen() {
        let world = World::new();
        let (lines, _) = services(&world);

        let refused = lines
            .open(
                name("ROOT"),
                StrategyId::new("from-elsewhere").unwrap(),
                &by(),
            )
            .await;

        assert!(matches!(refused, Err(DomainError::Validation(_))));
        assert!(
            world.lines.lock().unwrap().is_empty(),
            "nothing was written"
        );
    }

    /// Every rule offered says what it is, so the choice above can be
    /// made by somebody rather than guessed at.
    #[tokio::test]
    async fn the_rules_on_offer_describe_themselves() {
        let world = World::new();
        let (lines, _) = services(&world);

        let offered = lines.strategies().await;

        assert!(offered.len() >= 2);
        assert!(offered.iter().any(|(id, _)| *id == MainlineFirst.id()));
        assert!(
            offered
                .iter()
                .all(|(_, about)| !about.name.is_empty() && !about.summary.is_empty())
        );
    }

    #[tokio::test]
    async fn work_writes_passes_without_the_line_being_read() {
        let world = World::new();
        let (_, work, line) = opened(&world).await;
        let pursuit = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();

        work.push(
            &pursuit.id(),
            &persona(),
            vec![Op::add(content(), name("cut-01"))],
            None,
            &by(),
        )
        .await
        .unwrap();

        let read = work.get(&pursuit.id()).await.unwrap();
        assert_eq!(read.log().rounds().len(), 1);
        // The line is where it was: a pass is not a change to it.
        assert_eq!(world.line(&line.id()).unwrap().head(), line.head());
    }

    #[tokio::test]
    async fn a_pass_naming_content_the_persona_does_not_hold_is_refused() {
        let world = Arc::new(World {
            holds: false,
            ..World::default()
        });
        let (_, work, line) = opened(&world).await;
        let pursuit = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();

        let refused = work
            .push(
                &pursuit.id(),
                &persona(),
                vec![Op::add(content(), name("cut-01"))],
                None,
                &by(),
            )
            .await;

        assert!(matches!(refused, Err(DomainError::Validation(_))));
        assert!(
            work.get(&pursuit.id())
                .await
                .unwrap()
                .log()
                .rounds()
                .is_empty(),
            "the pass was refused before it was written"
        );
    }

    #[tokio::test]
    async fn closing_satisfied_work_moves_the_line() {
        let world = World::new();
        let (lines, work, line) = opened(&world).await;
        let pursuit = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        work.push(
            &pursuit.id(),
            &persona(),
            vec![Op::add(content(), name("cut-01"))],
            None,
            &by(),
        )
        .await
        .unwrap();

        work.close(&pursuit.id(), Outcome::Satisfied, None, &by())
            .await
            .unwrap();

        assert_ne!(world.line(&line.id()).unwrap().head(), line.head());
        assert_eq!(lines.states(&line.id()).await.unwrap().len(), 1);
        assert_eq!(
            work.get(&pursuit.id()).await.unwrap().outcome(),
            Some(Outcome::Satisfied)
        );
    }

    /// Losing the head is not reported. The service reads the line
    /// again and decides again, which is a fresh decision rather than
    /// the same one retried.
    #[tokio::test]
    async fn a_close_that_loses_the_head_is_decided_again() {
        let world = World::new();
        *world.losses.lock().unwrap() = 2;
        let (_, work, line) = opened(&world).await;
        let pursuit = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        work.push(
            &pursuit.id(),
            &persona(),
            vec![Op::add(content(), name("cut-01"))],
            None,
            &by(),
        )
        .await
        .unwrap();

        work.close(&pursuit.id(), Outcome::Satisfied, None, &by())
            .await
            .unwrap();

        assert_eq!(
            *world.attempts.lock().unwrap(),
            3,
            "twice lost, once landed"
        );
        assert_ne!(world.line(&line.id()).unwrap().head(), line.head());
    }

    /// A line busier than this caller is patient comes back as a
    /// conflict rather than as an unbounded wait.
    #[tokio::test]
    async fn a_line_that_never_stops_moving_gives_up_as_a_conflict() {
        let world = World::new();
        *world.losses.lock().unwrap() = ATTEMPTS + 1;
        let (_, work, line) = opened(&world).await;
        let pursuit = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        work.push(
            &pursuit.id(),
            &persona(),
            vec![Op::add(content(), name("cut-01"))],
            None,
            &by(),
        )
        .await
        .unwrap();

        let refused = work
            .close(&pursuit.id(), Outcome::Satisfied, None, &by())
            .await;

        assert!(matches!(refused, Err(DomainError::Conflict(_))));
        assert_eq!(*world.attempts.lock().unwrap(), ATTEMPTS);
        assert!(work.get(&pursuit.id()).await.unwrap().outcome().is_none());
    }

    /// Work is found by the line it is against and by the work it is
    /// filed under, and what was abandoned is in both — a listing that
    /// hid it would hide what the record is for.
    #[tokio::test]
    async fn work_is_listed_by_its_line_and_by_its_parent() {
        let world = World::new();
        let (lines, work, line) = opened(&world).await;
        let epic = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        let under = work
            .open(&line.id(), Some(epic.id()), Intent::default(), &by())
            .await
            .unwrap();

        // One of them is abandoned, and stays in the listing.
        work.push(
            &under.id(),
            &persona(),
            vec![Op::add(content(), name("tried"))],
            None,
            &by(),
        )
        .await
        .unwrap();
        work.close(&under.id(), Outcome::Abandoned, None, &by())
            .await
            .unwrap();

        let against = work.of_line(&line.id()).await.unwrap();
        assert_eq!(against.len(), 2);
        assert!(
            against
                .iter()
                .any(|w| w.outcome() == Some(Outcome::Abandoned))
        );

        let children = work.children(&epic.id()).await.unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id(), under.id());
        assert!(work.children(&under.id()).await.unwrap().is_empty());

        // And the line it is all against is one of the lines there are.
        let every = lines.list().await.unwrap();
        assert_eq!(every.len(), 1);
        assert_eq!(every[0].id(), line.id());
    }

    /// The round trip the whole design exists for: two pieces of work
    /// against one line, the second held up by a collision, looking at
    /// what happened, and landing.
    #[tokio::test]
    async fn a_collision_is_reported_then_settled_then_landed() {
        let world = World::new();
        let (_, work, line) = opened(&world).await;
        let entry = EntryId::new();

        let mine = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        work.push(
            &mine.id(),
            &persona(),
            vec![Op::add_to(entry, content(), name("cut-01"))],
            None,
            &by(),
        )
        .await
        .unwrap();

        // Somebody else lands the same entry first.
        let theirs = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        work.push(
            &theirs.id(),
            &persona(),
            vec![Op::add_to(entry, content(), name("cut-01"))],
            None,
            &by(),
        )
        .await
        .unwrap();
        work.close(&theirs.id(), Outcome::Satisfied, None, &by())
            .await
            .unwrap();
        let theirs_content = world.line(&line.id()).unwrap().states()[&entry]
            .content
            .unwrap();

        // Mine is now behind, and colliding.
        assert_eq!(work.behind(&mine.id()).await.unwrap().len(), 1);
        assert!(!work.collisions(&mine.id()).await.unwrap().is_empty());
        assert!(matches!(
            work.close(&mine.id(), Outcome::Satisfied, None, &by())
                .await,
            Err(DomainError::Validation(_)) | Err(DomainError::Conflict(_))
        ));

        // The line's rule writes what a person would have written:
        // the entry stays as the line has it, and my version is
        // carried onto one of its own.
        work.resolve(&mine.id(), &by())
            .await
            .unwrap()
            .expect("the rule answered");

        assert!(work.collisions(&mine.id()).await.unwrap().is_empty());
        let settled = work.get(&mine.id()).await.unwrap();
        assert!(
            settled
                .log()
                .rounds()
                .iter()
                .any(|round| round.act().by().is_system()),
            "the divergence is the server's pass"
        );

        work.close(&mine.id(), Outcome::Satisfied, None, &by())
            .await
            .unwrap();

        // Both candidates are on the line under different names, and
        // the entry that was there holds what the line said.
        let states = world.line(&line.id()).unwrap().states();
        assert_eq!(states.len(), 2);
        assert_eq!(states[&entry].content, Some(theirs_content));
    }
}
