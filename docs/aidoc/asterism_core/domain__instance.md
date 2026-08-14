# asterism-core::domain::instance

Instance identity — the referent behind
[`Author::Owner`](crate::domain::attribution::Author::Owner).

One profile database is one Asterism instance, and one instance has
exactly one owner. Before this record existed, `Owner` was a variant
with nothing behind it: the write path could stamp it, but nothing
could answer "which owner". A single row fixes that — `Owner` is an
indirect reference to it, the same way a foreign key refers rather
than copies.

**Co-ownership is not modelled.** Sharing adds subjects
([`Visibility::Restricted`](crate::domain::value::Visibility::Restricted)),
it does not add owners; an instance with two owners would make
"whose instance is this" unanswerable at exactly the moment the
answer starts to matter.

`owner_subject` is `None` while Asterism runs locally: there is no
authentication, so no token names the person at the keyboard, and
inventing one would be a value where there is a question. It is
bound once, when authentication arrives, and from then on `Owner`
resolves to a name that lives in the same namespace as sharing
subjects (see the [`attribution`](crate::domain::attribution) module
docs).

## Types

- `InstanceIdentity` — The identity record of this Asterism instance (the `instance`
- `OwnerResolution` — What `Author::Owner` resolves to on this instance.

