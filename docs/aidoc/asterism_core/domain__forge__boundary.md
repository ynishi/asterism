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
           │  Correlation  the forge answers       │
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

**[`Store`] is the face that asks downward**, and it was the only
one built for a while. The model refers to content it does not own, and before
that content is put on a line the forge has to know it is real —
that question, and no other, is what crosses today. The rest of
what the forge will eventually ask for (freezing a set, reading a
round) belongs to work that is not modelled yet, and declaring it
now would fix a shape nothing has tested.

**[`Actors`] is the face that asks sideways.** The forge records
who did a thing as a handle, and what that handle stands for — an
authenticated user, an instance — belongs to the side that knows
what a person is. That question crosses here.

**The face that answers upward is not here yet.** What the layer
below asks the forge is which work a finished round files under,
and that arrives with the transport that needs it, in this module.

# What crosses without a contract

Identity and failure are **shared, not twinned**, and naming them
directly anywhere in the forge is correct rather than a leak:
`AssetId`, `PersonaId`, `DomainError`. There is no client for them
and there should not be one.

The attribution triple used to be on that list and is not any
more. It came off for a reason the others do not share: the forge's
actors are a **wider** set than the triple can spell — a line's own
rule can act, and no person did that — so it is not a word the
forge borrows but a word that cannot say what the forge means. It
crosses through [`Actors`] like any other capability, and the
forge's own word for who did something is
[`Actor`](crate::domain::forge::model::act::Actor).

The reason is that they are not the other side's words that the
forge borrows — they are words neither side owns. A persona is the
tenancy axis both sides carry on every row; an asset id is what a
reference *is* on both sides of the line; a failure has one shape
or callers cannot handle it. Giving each of them a forge-side twin
would put a conversion in every file of this layer, and buy a
boundary that nothing is crossing.

So the split is: **identity is shared, capability is contracted.**
What the other side can *do* — answer whether something is held,
freeze a set, file a round — goes through a face and a client, and
every one of those is in this module. What something *is* does not.

# Translation happens once, in a client

A contract is stated in the shared vocabulary — the ids, the
attribution, the error that both sides already use. The forge's
[`Content`] is not in it, and must not be: putting it there would
hand the forge's word for a reference to the side that has its own.

So each face gets a client on this side, and the client is the
translation. [`StoreClient`] takes the forge's [`Content`] and asks
the contract in the contract's terms. Callers in the forge see only
the forge's words; the contract sees only its own. Neither of them
has to know the other exists.

# What holds this

`tests/forge_boundary.rs` reads every line of forge code that is
not a test and refuses anything named outside the forge that is not
on a list with a reason beside it. Until the forge is a crate, that
test is the arrow: a module boundary stops nothing, and
`use crate::domain::asset::Asset` in a model file compiles today.

It also means the shared vocabulary has a written surface: five
names, each with the reason it belongs to neither side. That is
what the split lifts when it happens.

# What lives here later

The contracts are declared in this module because there is nowhere
better yet. They describe something neither side owns, so their
home is a place neither side owns — a crate both depend on and
neither can reach around. Until that crate exists, a trait declared
here is a promise made in the right words at the wrong address, and
moving it is a move rather than a redesign.

[`Content`]: crate::domain::forge::model::value::Content

