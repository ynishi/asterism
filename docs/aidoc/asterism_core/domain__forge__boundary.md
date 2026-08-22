# asterism-core::domain::forge::boundary

The only place another layer's words appear.

Everything else under [`forge`](crate::domain::forge) is written in
the forge's own vocabulary. This module is where that stops: it
holds the contracts that cross between the forge and the layer
below, and the clients that translate across them. If a type from
the other side is named anywhere else in the forge, that is the
defect this module exists to make visible.

# Two faces, and no edge between the sides

```text
           ┌──────────── the contracts ────────────┐
           │  Store        the layer below answers │
           │  Actors       the side that knows who │
           └───────┬───────────────────┬───────────┘
                   │                   │
        ┌──────────┴───────┐   ┌───────┴──────────┐
        │   layer below    │   │      forge       │
        └──────────────────┘   └──────────────────┘
            no edge between these two, either way
```

Dependency runs in both directions in truth — the forge asks what a
piece of content is, and the layer below asks which work a result
belongs to — and wiring that directly means two crates that each
need the other to compile. Naming a contract instead leaves each
side depending on something neither owns, which is the only
arrangement where both questions can be asked.

**[`Store`] is the face that asks downward.** The model refers to
content it does not own, and before that content is put on a line
the forge has to know it is real. The rest of what the forge will
eventually ask for (freezing a set, reading a round) waits for the
work that needs it: declaring a question now would fix a shape
nothing has tested.

**[`Actors`] is the face that asks sideways.** The forge records
who did a thing as a handle, and what that handle stands for — an
authenticated user, an instance — belongs to the side that knows
what a person is. That question crosses here.

**The face that answers upward belongs in this module too.** What
the layer below asks the forge is which work a finished round files
under, and that question is declared here, with the transport that
needs it.

# What crosses without a contract

Identity and failure are **shared, not twinned**, and naming them
directly anywhere in the forge is correct rather than a leak:
`AssetId` and `DomainError`. There is no client for them and there
should not be one.

`PersonaId` was one of these while [`store::Store`] asked whose an
asset is. It is not any more — a line carries no owner, so the
forge has nothing to measure an answer about ownership against —
and the word left the forge with the question. It also left the
boundary list that `tests/forge_boundary.rs` enforces, which is a
wider list than the two words above: the enforced list is every
name the forge may reach for, and these are the ones a contract may
be *stated* in.

The attribution triple is on the enforced list for a reason the
others do not share: the forge's actors are a **wider** set than the triple
can spell — a line's own rule can act, and no person did that — so
it is not a word the forge records but a word it asks a question
in. It crosses at [`Actors`] and is turned into a handle there, and
the forge's own word for who did something is
[`Actor`](crate::domain::forge::model::act::Actor).

The reason is that they are not the other side's words that the
forge borrows — they are words neither side owns. An asset id is
what a reference *is* on both sides of the line; a failure has one
shape or callers cannot handle it. Giving each of them a forge-side
twin would put a conversion in every file of this layer, and buy a
boundary that nothing is crossing.

So the split is: **identity is shared, capability is contracted.**
What the other side can *do* — answer whether something exists,
and answer who somebody is — goes through a face, and every face
there is lives in this module. What something *is* does not.

# Translation happens once, in a client

A contract is stated in the shared vocabulary — the ids, the
attribution, the error that both sides already use. The forge's
[`Content`] is not in it, and must not be: putting it there would
hand the forge's word for a reference to the side that has its own.

So a face gets a client on this side when there is something to
translate, and the client is where the translation happens.
[`StoreClient`] takes the forge's [`Content`] and asks the contract
in the contract's terms. Callers in the forge see only the forge's
words; the contract sees only its own. Neither of them has to know
the other exists.

[`Actors`] has no client. What it takes is the attribution triple
and what it hands back is a handle the forge uses as it stands, so
there is nothing on either end for a client to convert, and callers
hold the contract directly.

# What holds this

`tests/forge_boundary.rs` reads every line of forge code that is
not a test and refuses anything named outside the forge that is not
on a list with a reason beside it. Until the forge is a crate, that
test is the arrow: a module boundary stops nothing, and
`use crate::domain::asset::Asset` in a model file compiles today.

It also means the shared vocabulary has a written surface: the
names on that list, each with the reason it belongs to neither
side. That is what the split lifts when it happens. How many there
are is not written here — a count in one file and its list in
another is a thing that goes stale silently, and this pair already
did once.

# What lives here later

The contracts are declared in this module because there is nowhere
better yet. They describe something neither side owns, so their
home is a place neither side owns — a crate both depend on and
neither can reach around. Until that crate exists, a trait declared
here is a promise made in the right words at the wrong address, and
moving it is a move rather than a redesign.

[`Content`]: crate::domain::forge::model::value::Content

