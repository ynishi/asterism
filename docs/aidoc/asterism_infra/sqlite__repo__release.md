# asterism-infra::sqlite::repo::release

SQLite adapter for the `ReleaseRepository` port.

`forge_release` holds what was released, by whom and when;
`forge_release_file` holds what the disclosure writer reported for
each copy that left, in the order the run wrote them. Reads join the
two, on the same terms the snapshot adapter joins its membership: a
release read without its stamps is a release that cannot answer the
question it exists to answer.

# Why this is not on `SqliteForge`

That adapter satisfies the forge's four ports and is stated in the
forge's own words. This port is not one of them — a release names a
snapshot and a dispatch, which the forge may not
([`asterism_core::domain::release`]) — so putting it there would
give one type two vocabularies and no reason to have both.

What it does borrow is [`ActRow`], because an act is stored the same
way wherever it appears: three columns, and a kind this build refuses
to widen by guessing.

# The two halves of a stamp, as columns

[`Half`] is three states, and the third carries words. So each half
is a state column and a detail column, and the table's own `CHECK`
says a state that is not `written` has to carry one — a skipped half
with no reason and a failed half with no cause are the values the
type exists to refuse, and a row is where they would come back.

## Types

- `SqliteReleaseRepository` — SQLite adapter for `ReleaseRepository`.

