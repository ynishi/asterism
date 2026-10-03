//! The timer #299 said belonged on top of the supervisor.
//!
//! A definition carries minutes between starts; something has to ask,
//! now and then, which of them are due. That is the whole of this
//! module. Every hard question is somewhere else on purpose — when a
//! definition is due is
//! [`ImportDefinitionRepository::due`](crate::domain::repository::ImportDefinitionRepository::due)'s,
//! whether one is already going is
//! [`ImportRunService::run`](super::ImportRunService::run)'s, and what
//! a run did is the record's.
//!
//! ## It holds nothing
//!
//! No next-fire time, no list of what it has started, no memory of a
//! tick it missed. Due-ness is derived from rows, so this can be killed
//! and started again with no state to carry across and nothing to
//! reconcile. A process that dies mid-run leaves what #299 already
//! handles: a `running` row a startup sweep closes.
//!
//! That is also why there is no catch-up. A machine asleep for a day
//! owes no forty-eight runs — it performs one when it is next due,
//! because an import resumes (#293, #295), so a missed window costs
//! latency and never costs records.
//!
//! **That property is `due`'s and not this loop's**, which is worth
//! being exact about because the code below looks as though it shares
//! the credit. [`MissedTickBehavior::Delay`] stops a suspended process
//! waking up owing a tick for every minute it slept, and a tick is a
//! *question*: asking it six hundred times in a burst would answer the
//! same thing six hundred times and start the same one run. So that
//! line saves queries and guards nothing.
//!
//! ## What it does not limit
//!
//! How many start at once. Everything `due` answers with is started on
//! the same tick, so a machine whose definitions all come due together
//! — every one of them, the first time the process runs — spawns an
//! importer each. No limit is invented here for the reason no backoff
//! is: a number capping them is a policy with a knob, and whoever wants
//! one will want to say what it is and what becomes of the rest.
//!
//! ## Where it is started, and why not in `init_core`
//!
//! By the process that binds the port, beside the listener itself, and
//! **not** by `init_core`.
//!
//! `init_core` assembles the service graph. What starts a loop over
//! that graph is the same kind of decision as what binds a port, and
//! the process that serves makes that one itself. A timer started
//! inside `init_core` would be a collaborator every end-to-end test
//! acquired without asking — importers spawning behind a test that
//! wanted a queue and nothing else — and answering that with a mode
//! argument would rebuild, in one step, the bundling #300 took apart.
//!
//! The risk in that placement is #299's own, and it is worth naming
//! rather than dressing up: a step in the serving process that can be
//! forgotten is one that will be, and #299's first shape was forgotten
//! at exactly that seam. What is done about it is that there is one
//! function to call and the end-to-end test calls it too. The type
//! system does not prevent the omission.
//!
//! [`MissedTickBehavior::Delay`]: tokio::time::MissedTickBehavior::Delay

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use super::ImportRunService;
use crate::domain::attribution::AttributionContext;

/// How often the timer asks, when nobody says otherwise.
///
/// A minute, and it is not a schedule — it is the resolution of one. An
/// import set to every thirty minutes starts within a minute of being
/// due, which is the accuracy a thing that fills an archive needs and
/// far short of what a person would notice.
///
/// It is deliberately unrelated to any definition's interval. Tying the
/// two would make the shortest schedule on the machine decide how often
/// every other one is examined.
pub const DEFAULT_TICK: Duration = Duration::from_secs(60);

/// A running timer. Dropping it stops the timer.
///
/// A guard rather than a `JoinHandle` to `await`, because nothing ever
/// waits for this loop to finish — it has no end — and what a caller
/// actually wants is for it to stop when whatever owns it goes away.
/// In the product that is the process; in a test it is the test.
pub struct ImportSchedule {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ImportSchedule {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ImportSchedule {
    /// Starts asking every `tick` which imports are due, and starting
    /// them, on the runtime the caller is already inside.
    ///
    /// The form for a caller already inside a runtime. It is
    /// [`spawn_on`](Self::spawn_on) with the
    /// current runtime's handle, and like `tokio::spawn` it panics when
    /// there is none — a caller on a thread no runtime owns reaches for
    /// `spawn_on` instead.
    pub fn spawn(service: Arc<ImportRunService>, tick: Duration) -> Self {
        Self::spawn_on(&tokio::runtime::Handle::current(), service, tick)
    }

    /// Starts the same loop on `runtime`, from any thread.
    ///
    /// The product and the test that proves the product start the same
    /// loop through here. `tick` is the resolution, not a schedule:
    /// [`DEFAULT_TICK`] is what the serving process passes, and a test
    /// passes something short because it cannot wait a minute to learn
    /// anything.
    ///
    /// The runtime is a parameter because the serving process starts
    /// the timer from a thread that has none. Tauri's `setup` hook runs
    /// on the main thread, outside every runtime, and a bare
    /// `tokio::spawn` there panicked inside the OS's launch callback,
    /// where a panic cannot unwind — the app aborted before a window
    /// appeared (#311). Entering the runtime at the call site would have
    /// fixed that one call too, but as a guard the next edit can move
    /// the call out of, at a site that reads as plain synchronous code;
    /// a parameter puts the requirement in the signature, where a caller
    /// with no runtime has to answer it.
    pub fn spawn_on(
        runtime: &tokio::runtime::Handle,
        service: Arc<ImportRunService>,
        tick: Duration,
    ) -> Self {
        let task = runtime.spawn(async move {
            let mut ticker = tokio::time::interval(tick);
            // Not `Burst`, the default: it saves the repeated
            // question after a suspend, and guards nothing. Module doc.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first tick of a tokio interval completes immediately,
            // and that is wanted: a definition that came due while the
            // process was down should not wait out a whole tick for
            // somebody to ask.
            loop {
                ticker.tick().await;
                // A timer states no author, and nothing here records
                // one — the argument exists so that adding a verb that
                // writes is a decision rather than an omission.
                let by = AttributionContext::asserted(None, None)
                    .expect("stating no author and no operator is always valid");
                match service.start_due(Utc::now(), &by).await {
                    Ok(0) => {}
                    Ok(started) => tracing::info!(
                        event = "diag.import_schedule.started",
                        imports = started,
                        "started imports that had come due"
                    ),
                    // `start_due` already logs and keeps going for
                    // anything one definition did wrong; reaching here
                    // means the read of what is due failed, which is
                    // the whole tick lost. Said once and the loop
                    // continues: the next tick asks again, and a timer
                    // that stopped on a transient read would need a
                    // restart to come back.
                    Err(err) => tracing::warn!(
                        event = "diag.import_schedule.due_failed",
                        error = %err,
                        "could not read which imports are due"
                    ),
                }
            }
        });
        Self { task }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use async_trait::async_trait;
    use chrono::DateTime;

    use super::*;
    use crate::application::{ImportLauncher, LaunchOutcome, LaunchSpec};
    use crate::domain::import_definition::{ImportDefinition, ImportRun};
    use crate::domain::repository::ImportDefinitionRepository;
    use crate::error::DomainError;

    /// Answers `due` with nothing and says each time it was asked;
    /// nothing else is reached, because nothing is ever due.
    struct Asked(std::sync::Mutex<mpsc::Sender<()>>);

    #[async_trait]
    impl ImportDefinitionRepository for Asked {
        async fn upsert(&self, _: &ImportDefinition) -> Result<(), DomainError> {
            unreachable!("nothing is due, so nothing is written")
        }
        async fn find(&self, _: &str) -> Result<Option<ImportDefinition>, DomainError> {
            unreachable!("nothing is due, so nothing is looked up")
        }
        async fn list(&self) -> Result<Vec<ImportDefinition>, DomainError> {
            unreachable!("the timer never lists")
        }
        async fn abandon_running(&self) -> Result<u64, DomainError> {
            unreachable!("the timer never sweeps")
        }
        async fn record_run(&self, _: &ImportRun) -> Result<(), DomainError> {
            unreachable!("nothing is due, so nothing runs")
        }
        async fn runs_for(&self, _: &str, _: u32) -> Result<Vec<ImportRun>, DomainError> {
            unreachable!("the timer never reads runs")
        }
        async fn due(&self, _: DateTime<Utc>) -> Result<Vec<ImportDefinition>, DomainError> {
            // The receiver is gone once the test has heard enough.
            let _ = self.0.lock().expect("the sender").send(());
            Ok(Vec::new())
        }
    }

    struct NeverLaunched;

    #[async_trait]
    impl ImportLauncher for NeverLaunched {
        async fn launch(&self, _: LaunchSpec) -> Result<LaunchOutcome, DomainError> {
            unreachable!("nothing is due, so nothing is launched")
        }
    }

    /// The shape of Tauri's `setup`: a plain thread no runtime owns,
    /// holding a handle to one that lives elsewhere. `#[test]` and not
    /// `#[tokio::test]` on purpose — inside a runtime a bare
    /// `tokio::spawn` works, and this is the case where it did not
    /// (#311).
    #[test]
    fn spawn_on_starts_the_timer_from_a_thread_with_no_runtime() {
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "the test thread must not be inside a runtime, or it proves nothing"
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime");
        let (tx, rx) = mpsc::channel();
        let service = Arc::new(ImportRunService::new(
            Arc::new(Asked(std::sync::Mutex::new(tx))),
            Arc::new(NeverLaunched),
        ));

        let schedule =
            ImportSchedule::spawn_on(runtime.handle(), service, Duration::from_millis(10));

        // Two asks, not one: the first tick of an interval is immediate,
        // so only the second says the timer is ticking.
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the timer asks at once");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the timer asks again a tick later");

        // Dropping the handle aborts the task, which drops the service
        // and with it the only sender, so the channel closes. Asks
        // already queued drain first; a timer still running would keep
        // the channel open and keep adding to it.
        drop(schedule);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while rx.recv_timeout(Duration::from_secs(5)) != Err(mpsc::RecvTimeoutError::Disconnected) {
            assert!(
                std::time::Instant::now() < deadline,
                "the timer kept asking after its handle was dropped"
            );
        }
    }
}
