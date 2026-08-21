# asterism-core::domain::forge::threads

Keeping what was said about work.

One face, and a narrow one. A thread is read by whoever is looking
at the thing it hangs off, so the question this answers is always
"what was said about this", never "what was said lately".

# Reading gives back the whole thread

Messages and every correction to them. A conversation is read in
full or it is read misleadingly: a correction the reader does not
see is a sentence attributed to somebody who withdrew it.

# Nothing removes and nothing overwrites

No delete, and no method that replaces a message. Correcting one
appends, which is what [`Thread::amend`] does in memory and what
this keeps. The absence is the same one the two logs have and it is
there for the same reason.

[`Thread::amend`]: crate::domain::forge::model::thread::Thread::amend

## Traits

- `Threads` — Keeps what was said.

