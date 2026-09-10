# asterism-core::domain::release

`Release` — the record that a change point's state was written out.

```text
  Line ── History ── ChangePoint          Snapshot ── DispatchJob
                         ▲                    ▲            ▲
                         └──── Release ───────┴────────────┘
                                 act
                                 file stamps*
```

# A release is not a change point

Nothing the line carries changes when its contents go out. Putting a
release on the chain would make "the line moved" and "what the line
holds was sent somewhere" the same event — the argument
[`line`](crate::domain::forge::model::line) already gives for keeping
a rename off the history. So a release is a record beside the chain
that names one change point, carries its own
[`Act`](crate::domain::forge::model::act::Act), and leaves the head
where it was.

Two releases of one change point are two records. Nothing here keys
on the change point, and nothing refuses a second one: a set going
out twice is two things that happened, and a store that collapsed
them would answer "when did this leave" with one of the two dates.

# Why this is not in the forge

It is the forge's kind of statement — an operator's account, with an
actor and a time — and it names a [`SnapshotId`] and a
[`DispatchId`], which the forge may not. `tests/forge_boundary.rs`
holds the list of words a contract across that boundary may be
written in, and adding two core ids to it would be the reversal
#254's "boundary that moves" section describes, paid for a record
that does not have to live inside the forge to be written.

So it sits here, where both vocabularies are already legal, and the
direction the forge's own module doc states is preserved: the
outside may name the forge, and the forge may not name the outside.
Nothing the forge holds moves, and no core row learns a forge word —
the release is a row of its own that names both by id.

# What the stamps are, and why they are per file

[`Stamped`] is what applying a disclosure to one file achieved, and
its two halves fail independently. A release writes out as many
files as the change point had live entries, so the honest record is
one outcome per file: a build with no certificate configured reports
the manifest half [`Skipped`](crate::domain::disclosure::Skipped) on
every one of them, and a container that could not take a packet
reports it on one. A summary would have to pick which of those to
report, and the caller reading it could not get back to the file.

## Types

- `FileStamp` — What became of the disclosure on one written-out file.
- `Release` — One change point's state, written out.

