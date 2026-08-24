# asterism-core::domain::forge::threads

Keeping what was said about work.

One face, and a narrow one. A thread is read by whoever is looking
at the thing it hangs off, so the question this answers is always
"what was said about this", never "what was said lately".

# Reading gives back the whole thread

Messages and every correction to them. A conversation is read in
full or it is read misleadingly: a correction the reader does not
see is a sentence attributed to somebody who withdrew it.

# Nothing here removes and nothing overwrites

No delete, and no method that replaces a message. Correcting one
appends, which is what [`Thread::amend`] does in memory and what
this keeps. The absence is the same one the two logs have and it is
there for the same reason.

One thing does take a conversation, and it is not on this face:
[`Lines::discard`] takes every thread anchored to the line it
drops — to a pursuit, a round, an entry as a round had it, or a
change point, which is every anchor [`Anchor`] has. The alternative
is a remark about something that no longer exists. A conversation
ends when the line it is about is thrown away, and nowhere else.

[`Thread::amend`]: crate::domain::forge::model::thread::Thread::amend
[`Anchor`]: crate::domain::forge::model::thread::Anchor
[`Lines::discard`]: super::lines::Lines::discard

## Traits

- `Threads` — Keeps what was said.

