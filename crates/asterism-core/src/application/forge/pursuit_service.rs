//! Work use cases — opening a line of work, writing passes, looking at
//! what the line did, and ending it.
//!
//! ```text
//!   open      reads the line's head, writes the pursuit
//!   push      writes the pursuit. does not read the line
//!   resolve   reads the line, writes the pursuit
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
//! Nothing. It loads what the model needs, calls it, and writes back
//! what came out. Every refusal in here comes from the model or from a
//! port.
//!
//! # Losing the race is not an error to report, and not answered here
//!
//! Two pieces of work can finish against one line at the same moment,
//! and only one of them lands on the head. What the loser needs is a
//! fresh decision against the line that won — not the same answer
//! written again, because normalising against a line that has moved
//! may leave less to record than there was, or more to collide with.
//!
//! Deciding that again is the model's, and the moment to do it belongs
//! to whoever is holding the write. So this service hands the store
//! [`Deciding`] along with what it decided, and the store asks for a
//! second answer under its own lock if the first is refused. Reading
//! again from here and trying again would be deciding against a line
//! that can move between the read and the write, once per attempt, for
//! as many attempts as anybody has patience for.

use std::sync::Arc;

use crate::domain::attribution::AttributionContext;
use crate::domain::forge::boundary::{Actors, StoreClient};
use crate::domain::forge::clock::Clock;
use crate::domain::forge::closings::{Closings, Deciding};
use crate::domain::forge::lines::Lines;
use crate::domain::forge::model::act::{Act, Actor};
use crate::domain::forge::model::change::{Collision, collisions, since};
use crate::domain::forge::model::closing::{Closing, close};
use crate::domain::forge::model::line::Line;
use crate::domain::forge::model::op::{Op, OpKind};
use crate::domain::forge::model::pursuit::{Intent, Outcome, Pursuit, Round};
use crate::domain::forge::model::react::react;
use crate::domain::forge::model::value::{ActorId, ChangePointId, LineId, PursuitId};
use crate::domain::forge::pursuits::Pursuits;
use crate::domain::forge::strategies::Strategies;
use crate::domain::value::PersonaId;
use crate::error::DomainError;

/// One person's answer to "what does ending this work put on the
/// line" — asked once out here, and at most once more inside the
/// write.
///
/// What it holds is everything the caller said and nothing the logs
/// say: the outcome asked for, the note written, and whose the write
/// is. The line and the pursuit come from whoever is asking, which is
/// what makes the second answer a fresh one — same question, logs that
/// have moved.
///
/// The time is not held either. Each answer is stamped when it is
/// given, so the one that lands says when it was decided rather than
/// when somebody first asked.
struct Ending {
    outcome: Outcome,
    note: Option<String>,
    by: ActorId,
    clock: Arc<dyn Clock>,
}

impl Deciding for Ending {
    fn close(&self, line: &Line, pursuit: &Pursuit) -> Result<Closing, DomainError> {
        Ok(close(
            line,
            pursuit,
            self.outcome,
            self.note.clone(),
            Act::new(self.clock.now(), Actor::User(self.by)),
        )?)
    }
}

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
        let round = Round::new(pursuit.head(), ops, note, self.act(by).await?)?;
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
    /// something, and what it decides goes in the pursuit like any
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
    ///
    /// Decided here, against the line as this read finds it, and the
    /// same decision goes to the store as something it can ask again.
    /// Which of the two answers lands is the store's to say and not
    /// this service's to watch: there is no loop here, and nothing to
    /// count.
    pub async fn close(
        &self,
        id: &PursuitId,
        outcome: Outcome,
        note: Option<String>,
        by: &AttributionContext,
    ) -> Result<(), DomainError> {
        let pursuit = self.get(id).await?;
        let line = self.line(&pursuit.of()).await?;

        // Resolved once, because who the write is by does not change
        // when the line does — and resolving it is the one part of an
        // act that asks a port, which the store cannot do while it is
        // holding a write open.
        let ending: Arc<dyn Deciding> = Arc::new(Ending {
            outcome,
            note,
            by: self.actors.resolve(by).await?,
            clock: self.clock.clone(),
        });

        let closing = ending.close(&line, &pursuit)?;
        self.closings.commit(&line.id(), id, &closing, ending).await
    }

    /// What this work still asks to write that the line moved after
    /// the work was cut from it.
    ///
    /// Derived from the two logs on every call, so it cannot go stale
    /// and there is no flag anybody has to clear. What clears a
    /// collision is the work asking for something else — writing a
    /// pass that stops requesting the axis, which
    /// [`resolve`](Self::resolve) does under the line's own rule.
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
        /// An ending somebody else decided, to be landed at the top of
        /// the next close — so that close finds its parent taken.
        ///
        /// The store is where a race is answered now, so the race has
        /// to happen inside the store: arranging it out here would
        /// only test a line that had already moved before anybody read
        /// it, which is the case that never needed answering.
        first: Mutex<Option<(PursuitId, Closing)>>,
        /// Whether this store refuses to keep even what was decided
        /// against the line as it is.
        adamant: bool,
        /// How many times the port was called, won or lost.
        calls: Mutex<usize>,
        /// How many endings it was handed, counting the ones it asked
        /// for itself.
        decisions: Mutex<usize>,
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

        /// The same store, keeping nothing at all.
        fn adamant() -> Arc<Self> {
            Arc::new(Self {
                holds: true,
                adamant: true,
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
            if work.head() != on {
                return Err(DomainError::Conflict("the pursuit moved".into()));
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
            closing: &Closing,
            again: Arc<dyn Deciding>,
        ) -> Result<(), DomainError> {
            *self.calls.lock().unwrap() += 1;
            *self.decisions.lock().unwrap() += 1;

            let mut held = self
                .line(line)
                .ok_or_else(|| DomainError::not_found("line", line))?;

            // Somebody armed an ending of their own. It lands here,
            // between the caller's decision and this write — which is
            // the only place a race can be arranged now that the
            // service does not read the line twice.
            if let Some((theirs, ending)) = self.first.lock().unwrap().take() {
                let mut work = self
                    .pursuit(&theirs)
                    .ok_or_else(|| DomainError::not_found("pursuit", theirs))?;
                ending.apply(&mut held, &mut work).expect("theirs lands");
                self.put_line(held.clone());
                self.put(work);
            }

            let mut work = self
                .pursuit(pursuit)
                .ok_or_else(|| DomainError::not_found("pursuit", pursuit))?;

            // The parent as the constraint sees it: taken, or free.
            let stale = closing
                .point()
                .is_some_and(|point| point.parent() != held.head());
            let closing = if stale {
                *self.decisions.lock().unwrap() += 1;
                again.close(&held, &work)?
            } else {
                closing.clone()
            };

            if self.adamant {
                return Err(DomainError::Conflict("this store keeps nothing".into()));
            }

            closing.apply(&mut held, &mut work)?;

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

    /// Work cut from the line, asking for one entry of its own — so
    /// two of these collide with nothing and both may land.
    async fn cut(work: &PursuitService, line: &Line, label: &str) -> Pursuit {
        let pursuit = work
            .open(&line.id(), None, Intent::default(), &by())
            .await
            .unwrap();
        work.push(
            &pursuit.id(),
            &persona(),
            vec![Op::add(content(), name(label))],
            None,
            &by(),
        )
        .await
        .unwrap();
        pursuit
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
        assert_eq!(read.rounds().len(), 1);
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
            work.get(&pursuit.id()).await.unwrap().rounds().is_empty(),
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

    /// Losing the head is not reported and is not read again from
    /// here. The service hands the store a way to decide again, the
    /// store asks it once, and the caller is told nothing happened out
    /// of the ordinary.
    #[tokio::test]
    async fn a_close_that_loses_the_head_is_decided_again_by_the_store() {
        let world = World::new();
        let (_, work, line) = opened(&world).await;
        let mine = cut(&work, &line, "mine").await;
        let theirs = cut(&work, &line, "theirs").await;

        // Theirs is decided but not landed. The store lands it at the
        // top of the next close, so mine is decided against a line
        // that moves before it is written.
        let ending = close(
            &world.line(&line.id()).unwrap(),
            &world.pursuit(&theirs.id()).unwrap(),
            Outcome::Satisfied,
            None,
            Act::new(at(9), Actor::User(ActorId::new())),
        )
        .unwrap();
        *world.first.lock().unwrap() = Some((theirs.id(), ending));

        work.close(&mine.id(), Outcome::Satisfied, None, &by())
            .await
            .expect("what the store could not keep, it decided again");

        assert_eq!(
            *world.calls.lock().unwrap(),
            1,
            "one call to the port: losing costs a decision, not a round trip"
        );
        assert_eq!(
            *world.decisions.lock().unwrap(),
            2,
            "one decision outside the write, one inside it"
        );

        // Both are on, and mine sits on theirs rather than beside it.
        let held = world.line(&line.id()).unwrap();
        let chain = held.history().changes();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].from(), theirs.id());
        assert_eq!(chain[1].from(), mine.id());
        assert_eq!(chain[1].parent(), chain[0].id());
    }

    /// A store that cannot keep even what was decided against the line
    /// as it is says so, and the work stays open.
    ///
    /// There is nothing for this service to do about that: the second
    /// decision was made where the line could not move, so a refusal
    /// after it is not a race anybody can win by asking again.
    #[tokio::test]
    async fn a_close_the_store_will_not_keep_comes_back_as_a_conflict() {
        let world = World::adamant();
        let (_, work, line) = opened(&world).await;
        let pursuit = cut(&work, &line, "cut-01").await;

        let refused = work
            .close(&pursuit.id(), Outcome::Satisfied, None, &by())
            .await;

        assert!(matches!(refused, Err(DomainError::Conflict(_))));
        assert_eq!(
            *world.calls.lock().unwrap(),
            1,
            "and it is not asked a second time"
        );
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
