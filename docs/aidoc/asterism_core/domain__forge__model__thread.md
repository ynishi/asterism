# asterism-core::domain::forge::model::thread

Saying something about work.

```text
  Thread ── anchor : Pursuit | Round | (Round, Entry) | ChangePoint
   │         title?
   └ Message ── body / act
        │        parent? : another message of this thread
        └ Revision ── body / act        the body now is the last one
```

A node carries a note, and a note is one line written once by
whoever wrote the node. This is the rest: a remark on somebody
else's round, a question about one entry it touched, a review of
what landed.

# Four anchors, because four things are worth remarking on

A pursuit as a whole — what this work is for. A round — the judgement
somebody made in it. One entry a round touched — this particular
thing, in this particular attempt. And a change point — a review
after the fact.

**The entry anchor is deliberately not an entry.** Hanging a remark
on an entry alone would make it follow that entry into every other
pursuit it is ever carried into, which is how a note about one
attempt becomes a note about the thing itself. Anchoring at
`(round, entry)` says *this entry, as this round had it*, and that
does not travel. A change point can be anchored to on its own,
because there is nowhere for a remark on it to travel to.

# It is the forge's own, and not the annotation surface downstairs

There is a thread primitive in the layer below, and it is not this
one. It anchors to snapshots, cards and query groups — the things
that layer has — and the four things worth remarking on here are
not among them, nor could they be without that layer learning what
a pursuit is. Sharing the type would mean one of the two sides
carrying anchors it can never use, and every reader of either
side asking which half applies.

So they are separate, and they say who did something in different
words: that one records the write-side attribution triple, this one
records an [`Act`], because the forge's actors include a line's own
rule and a rule is not somebody with an attribution.

# Nothing is overwritten and nothing is resolved

Editing a message appends a [`Revision`]; the body now is the last
one, and every earlier one stays readable. That is the same reason
the two logs work the way they do — what was said is a fact about
what happened, and a correction is another one.

There is no resolved flag, no closing a thread, no marking a remark
as handled. Whether something is dealt with is a word people use
about their work, not a shape the record has; if it matters, a
later message says so and that is a better record than a boolean
nobody has to explain.

# Order here is the clock, and that is a real difference

Everywhere else in this model the chain orders things and a
timestamp is evidence. A discussion has no chain to read an order
out of — a reply names its parent, but two replies to one message
are ordered by nothing else — so messages are read in the order
they were written, and a conversation reads as a transcript rather
than as a tree.

It is affordable because nothing derives from it. No fold reads a
thread, no rule consults one, and no refusal depends on the order
of two remarks. A clock that steps backwards makes a conversation
read oddly; it cannot make the line wrong.

The relation is asked one question and no more. An answer cannot be
read back before its question — [`Thread::say`] refuses a reply to
something the thread does not hold yet — so [`restore`] moves a
reply the stamps put before its parent to just after it, and leaves
everything else where the clock had it. That is the difference
between a conversation that reads oddly and one that cannot be read
at all.

[`restore`]: super::restore

## Types

- `Anchor` — What a thread hangs off.
- `Body` — What somebody said.
- `Message` — One thing said, and every correction to it.
- `Revision` — A correction to something already said.
- `Thread` — A run of messages about one thing.

