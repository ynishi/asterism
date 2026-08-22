# asterism-core::domain::forge::boundary::store

The face that asks downward, and the client that speaks it.

[`Store`] is stated in the shared vocabulary: ids the two sides
already agree on, and the shared error. [`StoreClient`] is the
forge's side of it, and the only thing in the forge that turns a
[`Content`] into the id a contract can carry.

# Existence, not ownership

The question is `exists`, and it used to be `owns(persona, asset)`.
That was wrong twice over, and the second HTTP surface built on it
is what made both visible.

**A line carries no owner.** [`Lines::list`] says so: grouping and
access are outside the forge, and an instance has the lines
somebody made on purpose rather than one per person. So "real but
belonging to somebody else" is not a reason a reference is
unusable *here* — putting one person's asset on a shared line is
the thing a shared line is for.

**And the check could not refuse a caller who wanted to pass.** A
persona is a column on the asset row and the caller chose both
halves of the pair, so naming the asset's own persona always
succeeded — and nothing here knew whether the caller was that
persona. What it caught was a client that paired the two wrongly.
It read as a guard on whose asset this is, and it was a consistency
check on two values one caller supplied.

What the forge actually needs to know is whether the reference is
real, because an operation naming content that is not there is a
line lying about the present. That is one id and no persona.

Nothing is deferred by this. "Who" is a question the forge already
asks, once, through [`Actors`](super::actors::Actors): a write
carries an [`Actor`](crate::domain::forge::model::act::Actor), the
handle is resolved by the side that knows what a user is, and it is
a handle precisely so that it exists before authentication binds it
and keeps pointing at the same actor afterwards. A persona was
never the forge's word for who, and an asset's owner was never the
forge's question.

Access is per line and outside the forge, so what governs putting
content on one is who may write to that line. If the forge ever had
to record an owner rather than an author, it would be an `Actor` on
the entry — a fourth axis beside existence, content and name,
resolved through the same contract as every other handle. Nothing
asks for that today.

[`Lines::list`]: crate::domain::forge::lines::Lines::list

[`Content`]: crate::domain::forge::model::value::Content

## Types

- `StoreClient` — The forge's side of [`Store`].

## Traits

- `Store` — What the layer below answers.

