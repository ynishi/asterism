# asterism-exporter-http::record

What the platform said, and — when it said nothing — why.

A record with no seed in it does not say why on its own, and there
are three different reasons behind that one absence:

- **captured** — the profile said where to look and the value was
  there.
- **not captured** — the profile said where to look and nothing was
  there. This is the only one that is a gap on our side, and the only
  one worth acting on.
- **not returned** / **not supported** — the platform ran with a seed
  and does not tell you which, or the parameter does not exist on
  this model at all. Both are properties of the platform, fixed
  before any call is made, and the profile author is the person who
  read the documentation that says so.

A `null` reports all four states identically, which is why this is a
status beside a nullable value rather than a marker written into the
value's own slot. [#17] argues the same from `getxattr(2)` (which
separates "the filesystem does not support this" from "the attribute
is not there" from a value) against `statx(2)` (which collapses them
and fills in a plausible-looking dummy).

[#17]: https://github.com/ynishi/asterism/issues/17

# Why the profile is the place to say it

Which of the three applies is fixed per platform rather than
discovered per call, and this adapter already treats a platform as a
profile rather than as an adapter of its own — the reason `auth`,
`fetch` and `deadline_seconds` are optional blocks a profile fills
in. A declared absence is one more thing the profile author knows
from reading the platform's documentation, stated once, in the same
place as the rest.

# Declaring one is not the same as failing to capture it

A profile may not both name a path for a field and declare it absent.
That is a profile contradicting itself — it says the platform does
not return a value and then says where the value is — and it is
rejected when the params are parsed rather than resolved by a
precedence rule. A precedence rule would make the contradiction
survive into the record it was supposed to describe.

# Where a path points

Paths are evaluated against a document assembled per artefact:

```text
{ "response": <the harvest response>,
  "item":     <this artefact's element of it>,
  "params":   <the dispatch params> }
```

One document because the fields a record wants do not all come from
one place. A platform may return the seed once for the whole response
and the URL per image; another generating a batch with one seed each
returns it per item. `$.response.seed` and `$.item.seed` are both
sayable, in the grammar the profile already uses for `items_path`,
and neither needs a root that only exists here.

## Types

- `Absence` — Why a value is not in the record, as declared by the profile.
- `FieldRecord` — One field of the record.
- `Record` — One artefact's record, field by field.
- `RecordSchema` — What the profile says the record should contain.

