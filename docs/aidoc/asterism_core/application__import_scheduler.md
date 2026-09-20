# asterism-core::application::import_scheduler

The timer #299 said belonged on top of the supervisor.

A definition carries minutes between starts; something has to ask,
now and then, which of them are due. That is the whole of this
module. Every hard question is somewhere else on purpose — when a
definition is due is
[`ImportDefinitionRepository::due`](crate::domain::repository::ImportDefinitionRepository::due)'s,
whether one is already going is
[`ImportRunService::run`](super::ImportRunService::run)'s, and what
a run did is the record's.

## It holds nothing

No next-fire time, no list of what it has started, no memory of a
tick it missed. Due-ness is derived from rows, so this can be killed
and started again with no state to carry across and nothing to
reconcile. A process that dies mid-run leaves what #299 already
handles: a `running` row a startup sweep closes.

That is also why there is no catch-up. A machine asleep for a day
owes no forty-eight runs — it performs one when it is next due,
because an import resumes (#293, #295), so a missed window costs
latency and never costs records.

**That property is `due`'s and not this loop's**, which is worth
being exact about because the code below looks as though it shares
the credit. [`MissedTickBehavior::Delay`] stops a suspended process
waking up owing a tick for every minute it slept, and a tick is a
*question*: asking it six hundred times in a burst would answer the
same thing six hundred times and start the same one run. So that
line saves queries and guards nothing.

## What it does not limit

How many start at once. Everything `due` answers with is started on
the same tick, so a machine whose definitions all come due together
— every one of them, the first time the process runs — spawns an
importer each. No limit is invented here for the reason no backoff
is: a number capping them is a policy with a knob, and whoever wants
one will want to say what it is and what becomes of the rest.

## Where it is started, and why not in `init_core`

By the process that binds the port, beside the listener itself, and
**not** by `init_core`.

`init_core` assembles the service graph. What starts a loop over
that graph is the same kind of decision as what binds a port, and
the process that serves makes that one itself. A timer started
inside `init_core` would be a collaborator every end-to-end test
acquired without asking — importers spawning behind a test that
wanted a queue and nothing else — and answering that with a mode
argument would rebuild, in one step, the bundling #300 took apart.

The risk in that placement is #299's own, and it is worth naming
rather than dressing up: a step in the serving process that can be
forgotten is one that will be, and #299's first shape was forgotten
at exactly that seam. What is done about it is that there is one
function to call and the end-to-end test calls it too. The type
system does not prevent the omission.

[`MissedTickBehavior::Delay`]: tokio::time::MissedTickBehavior::Delay

## Types

- `ImportSchedule` — A running timer. Dropping it stops the timer.

## Constants

- `DEFAULT_TICK` — How often the timer asks, when nobody says otherwise.

