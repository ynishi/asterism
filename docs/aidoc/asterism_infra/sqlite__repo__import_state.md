# asterism-infra::sqlite::repo::import_state

SQLite adapter for the `ImportStateRepository` port.

Two statements over one table, and the interest is in what is absent
from them. Nothing reads inside `offset_json` — no `json_extract`, no
comparison, no ordering by anything in it. The column goes in as text
and comes out as text; why that rule exists is
[`import_state`](asterism_core::domain::import_state).

The one row a key can have is the primary key's doing, so `upsert`
is an `ON CONFLICT` over the whole key rather than a read followed by
a write: two runs of the same importer finishing at once would
otherwise both read "nothing", both insert, and one would lose to a
constraint the other had not seen.

A timestamp this build cannot represent is *not* treated the way
`app_setting` treats one. There, an uninterpretable row reads as
"not overridden" and the settings screen still renders; here the
equivalent would be a resumption point silently reading as absent,
and an importer told a source it has read is a source it has not
would re-import the whole thing. So the read fails and says so —
`updated_at` is a stamp for a person, and losing a position over one
is the trade that runs the wrong way.

## Types

- `SqliteImportStateRepository` — SQLite adapter for `ImportStateRepository` (uses a writer isle).

