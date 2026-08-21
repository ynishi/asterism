# asterism-core::domain::forge::model::error

What the model refuses.

The model has its own error type, and every refusal in this module
is one of its variants. Reaching for the shared error instead would
mean the forge states a rule in one place and names it in another's
vocabulary — and a rule named `Validation` is a rule nobody can
match on, so callers end up matching on message text.

# It is folded once, at the edge

[`DomainError`] is the shared vocabulary, and the conversion below
is the only place the model meets it. Adding a refusal means adding
a variant here and deciding, once, which shared kind it reads as —
not repeating that decision at every call site.

# It only grows

Every refusal the forge learns is added here. That is what makes
the set of ways this model can say no readable in one place, which
is the same reason a caller wants a typed error at all.

## Types

- `ForgeError` — A refusal the model makes.

