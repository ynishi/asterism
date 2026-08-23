# asterism-infra::fault

What storage did, in storage's own words.

A repository knows what the database answered: a unique index
rejected the row, a predicate matched nothing, a row would not
decode. It does not know whether the caller should try again, which
is what [`DomainError::Conflict`]'s kind promises — and that promise
is the reason this module exists.

# What went wrong without it

`DomainError`'s four shared variants had no written rule for which
one a refusal belonged to, so the choice was made at every call site
that raised one: fifty-eight of them, thirty-nine inside this crate.
A SQLite repository was answering an API question. Several answers
were wrong, and the wrongness only became visible when `ConflictKind`
turned "some vague 409" into advice a client acts on.

# The shape

```text
  repository ──► StoreFault ──► DomainError ──► 400 / 404 / 409 / 500
                      │              │
        what storage ─┘              └─ what it means, decided once,
        did. Seven cases,               in the `From` impl below and
        no judgement.                   nowhere else.
```

Each case has exactly one destination, so the mapping is a table
rather than an argument. That is deliberate: the repository picks by
what happened, which it can see, and the meaning is written down in
one place a reviewer can read in full before adding the next
refusal. `asterism-core`'s `error` module doc holds the four
definitions this table implements; `domain::forge::model::error` is
the same structure one layer up, where the forge's own vocabulary
meets the shared one at a single hand-written edge.

# Why the conversion is written out

`thiserror`'s `#[from]` derives a conversion that carries a value
across unchanged. This one reads which case it has and picks a
different destination for each, which no derive expresses — and
that is the point rather than a limitation. The mapping *is* the
specification, so it wants to be read, not generated.

## Types

- `StoreFault` — A refusal in storage's vocabulary.

