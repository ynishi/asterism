# asterism-core::domain::disclosure::release

The shape of the work a released file came out of, as the file
itself will state it.

A disclosure says how an artefact was made. This says who chose it:
a person opened work with something in mind, took some number of
rounds at it, ended it, and later released what the line then
carried. Those are the facts a reviewer holding only the file has to
be able to read, and the reason the forge records an
[`Act`](crate::domain::forge::model::act::Act) on every node.

# Why the forge's own words do not travel

This module names nothing in the forge. It carries a pursuit id as
text, an actor as [`Hand`], and two instants — none of which is a
forge type — because a disclosure is rendered by
`asterism-disclosure-format`, and a renderer that had to know what a
`Pursuit` is would put the intentional history into a crate whose
subject is container formats. The translation happens once, where
the release is assembled.

# What is left out, and why each one

**Round notes.** Free text somebody wrote for themselves, in a
signed document nobody can edit afterwards. The team publish path
already decided this for a receiver it does not control
(`asterism-teams-client::publish`), and a marketplace reviewer is a
stranger by a wider margin than a team is.

**The operations.** What a round added, replaced, renamed or removed
is the deliberation, and the released set is the conclusion. A
reader holding the files can see what was chosen; what they cannot
see, and what this supplies, is that somebody chose it.

**The prompt.** Kept out of the manifest wherever it appears, on the
terms `PromptDisclosure` already sets: it is disclosed once, in the
packet, under the IPTC property defined for it.

# A count rather than the acts of every round

[`rounds`](ReleaseDisclosure::rounds) says how many, not who did
each. Which rounds a person did and which a rule did is a question
about the deliberation, and the two acts that are carried whole —
the close and the release — are the two that decide something.

## Types

- `Hand` — Whether a person or a rule did something.
- `ReleaseAct` — One act, reduced to what a file may say about it.
- `ReleaseDisclosure` — What a released file says about the work that reached it.

