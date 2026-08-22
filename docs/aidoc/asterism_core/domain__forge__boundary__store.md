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

Whose asset it is stays a real question for a layer that has one to
ask on behalf of — a surface that has authenticated somebody, which
this one has not. When there is such a surface, the check belongs
there, on an identity the caller did not choose.

[`Lines::list`]: crate::domain::forge::lines::Lines::list

[`Content`]: crate::domain::forge::model::value::Content

## Types

- `StoreClient` — The forge's side of [`Store`].

## Traits

- `Store` — What the layer below answers.

