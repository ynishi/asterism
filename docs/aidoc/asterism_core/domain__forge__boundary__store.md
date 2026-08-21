# asterism-core::domain::forge::boundary::store

The face that asks downward, and the client that speaks it.

[`Store`] is stated in the shared vocabulary: ids the two sides
already agree on, and the shared error. [`StoreClient`] is the
forge's side of it, and the only thing in the forge that turns a
[`Content`] into the id a contract can carry.

# Ownership, not existence

The question is `owns`, not `exists`, and the difference is the
persona. A reference to something real but belonging to somebody
else is exactly as unusable as a reference to nothing, and a
crossing that forgets whose data it is asking about is the kind of
mistake that does not surface until two tenants are in the same
database. Carrying the persona in the signature means forgetting it
does not compile.

The forge does not decide what ownership means — it asks, and the
side that holds the content answers.

[`Content`]: crate::domain::forge::model::value::Content

## Types

- `StoreClient` — The forge's side of [`Store`].

## Traits

- `Store` — What the layer below answers.

