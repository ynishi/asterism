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
    /// them.
    ///
    /// The product and the test that proves the product start the same
    /// loop through here. `tick` is the resolution, not a schedule:
    /// [`DEFAULT_TICK`] is what the serving process passes, and a test
    /// passes something short because it cannot wait a minute to learn
    /// anything.
    pub fn spawn(service: Arc<ImportRunService>, tick: Duration) -> Self {
        let task = tokio::spawn(async move {
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
