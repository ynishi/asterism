# asterism-core::domain::series

`series` — "made the same way": a rule for reading a material's
metadata, and the key that rule derives.

[`material_meta`](crate::domain::material_meta) answers one question
about a container — *it carried this text* — over everything the
container carried. That is the right sentence for the meta axis and
the wrong one for a run. Measured on eleven images out of two VDSL
runs: a digest over the whole metadata set separates all eleven, and
so does dropping the run's `timestamp`, and so does dropping the
generator's chunk entirely. What splits them is the `prompt` chunk —
a compiled graph that differs per image — and **no exclusion reaches
it**. The only reading that recovers the two runs selects the recipe
and nothing else (`["vdsl","script"]` → two keys, five images and
six). The tests at the bottom of this file are that measurement,
frozen.

# A key is a second sentence, not a better digest

Nothing here touches `m1-`. `meta_kv` states what the container
carried; a [`SeriesKey`] states what one [`Strategy`] made of it.
Two statements about one material, so neither has to be weakened to
accommodate the other — the digest keeps saying *this text*, and the
key gets to say *this recipe* without a `m2-` and without a claim
that two files are the same thing.

It is also why a Strategy can be rewritten cheaply. [`derive`] reads
`meta_kv` and nothing else: no bytes, no locator, no disk. Changing a
rule re-derives a whole library out of rows that are already loaded,
which is what makes a Strategy something a person can iterate on
rather than a decision to get right the first time. A `cr2-` costs
somebody's disk; a `sk1-` selection change costs a scan.

# A key on the material, and not a Group

The obvious shape for "these belong together" in this codebase is a
Group, and it is the wrong one — not because the grouping is wrong
but because of where a Group keeps its membership. `asset_bucket` is
the table a person's own curation lives in: it carries the
hand-placed order, and a card's *primary* group is whichever of its
`group_ids` sorts first, which the repository fixes as the lowest
`bucket_id` so that the answer is at least stable
(`fetch_group_ids_map`). Let a system rule mint Groups and a card's
primary group changes the moment one of those minted ids happens to
sort low: the Group axis quietly stops showing the arrangement
somebody made, and no screen can explain it, because the card
belongs to both legitimately. `list_duplicate_groups` runs into none
of this by writing no `asset_bucket` row at all. **The harm is the
write into the curation table, not the splitting.**

Structure says the same thing four more times. A Group's rule is a
column on the group row (`bucket.query_json`), so one rule is one
group by construction; `UNIQUE (persona_id, name)` means N groups
need N names somebody has to invent; the refresh stamps are per
group, with nowhere to record a per-rule result; and every service
signature takes a single `GroupId`. A Session is worse rather than
better — membership there is one `container_id` on the member, so an
asset sits in exactly one, while "made the same way", "shot in one
burst" and "made on one day" overlap by nature.

So the key stays on the material and a group is computed when
somebody asks for one. When a series turns out to deserve a name, a
hand-placed order or a dispatch target, **a person promotes that one
into a real Group** through the path that already exists — which is
also the only thing `DispatchService::run` will accept, since it
wants one id and a frozen membership.

# Include is sharp and goes stale, exclude is blunt and safe

[`probe`](crate::domain::probe)'s denylist obligation reappears here
one layer up, and with the asymmetry pointing the other way, so both
rules are offered rather than one.

| | a field nobody named arrives | result |
|---|---|---|
| [`include`](Strategy::include) | it is not selected | fewer distinctions — separate things share a key |
| [`exclude`](Strategy::exclude) | it is not dropped | more distinctions — one run splits |

Include's error is the unrecoverable one, and it is the rule VDSL
needs; exclude's error is merely a lost improvement, and it is the
only rule available where the vocabulary is open (EXIF vendor
MakerNotes). A Strategy states which of the two it is making, per
field, and the author owns that choice — which is the reason a
Strategy is data rather than code.

**So the instruction the table implies runs against the grain: a
field an author is unsure about belongs *in* an include list.** Left
out, it cannot separate anything, and two materials it would have
told apart land on one key that reads exactly like a correct
grouping. Named, the worst it does is split a run — which is visible
in the result and repaired by editing the rule, at the cost of a scan
(see this module's opening on why re-deriving is cheap). Writing it
down because the wrong reading is the one that sounds careful: the
design memo this argument was drafted in stated the sentence inverted
in its first draft, and it reached three other files — including the
MCP schema resource an agent reads before writing a rule — before
anybody looked at the table beside it.

# `decode` absorbs the container's shape, and never drops

A path can only walk a structure, and containers hand over their
metadata as text: raw JSON inside a `tEXt` chunk, base64 of JSON in a
character card, a typed EXIF field written as `type:rendering`.
[`Decode`] is a small closed set for the reason the design gives: the
author of a Strategy is on the far side of a process boundary, so a
rule they can register has to be chosen from tools that already
shipped, not spelled in a language.

**A value the decoder cannot read stays the string the container
stated.** The alternative — treat it as absent — quietly removes a
distinction, and removing distinctions is how unrelated materials end
up under one key. Keeping it costs nothing: a path cannot descend
into a string, so a deep path finds nothing there and says so, while
a whole-map selection still carries the text.

Keeping the text is not on its own enough to keep the two apart,
which is what [`Selected`] is for: `hello` and `"hello"` are
different text in the container, and both reach the rendering as one
[`Value::String`]. So each selected sub-tree is rendered with the
kind of thing it is, and the two land on different keys.

# What is not decided here

Where derived keys are stored, when they are recomputed, how a
Strategy is registered over HTTP, and whether a format's meta axis is
claimed at all — a Strategy over a format whose probe declares
`meta: false` reads an empty `meta_kv` and answers
[`NotApplicable`](SeriesKey::NotApplicable) for every row, correctly
and uselessly, until that probe claims the axis.

One of those was owed and named here so S2 would not have to find
it: `content_hash` gives each versioned column a **reserved value**
for the digest of its own empty rendering
([`CONTENT_REGION_EMPTY`](crate::domain::content_hash::CONTENT_REGION_EMPTY),
[`META_EMPTY`](crate::domain::content_hash::META_EMPTY)), listed so
that a value which reached the column by some other route is not read
as sameness. `material_series.key` is that column here (V73), so the
constant is [`SERIES_KEY_EMPTY`].

**Half of that debt is paid.** The sibling constants are consulted on
the *matching* side — the thing that decides whether two rows group —
and this one is so far consulted only on the writing side, where
`SqliteSeriesRepository::record` refuses it. That covers the writer
and by construction cannot cover the case the argument is actually
about: a row that arrived some other way, which no write path ever
sees. The reader that closes it is S3's, and the shape of the column
says where it will go wrong — a hand-edited row carrying this value
sits *inside* `idx_material_series_strategy_key` and satisfies the
natural grouping statement
(`WHERE strategy_id = ? AND key IS NOT NULL GROUP BY key`), which is
precisely one group holding every material whose rule selected
nothing. **That query has to name [`SERIES_RESERVED_VALUES`]**, the
way the duplicate report's adapter names
[`reserved_values`](crate::domain::content_hash::reserved_values).

## Functions

- `derive` — Applies a [`Strategy`] to one material's metadata.
- `is_series_key` — Whether a stored value may stand for "made the same way" — the rule

## Types

- `Decode` — How the text a container carried becomes a structure a [`Path`] can
- `Path` — Where in the metadata a rule is pointing.
- `SeriesKey` — What applying a [`Strategy`] to a material concluded.
- `Strategy` — One rule for reading "made the same way" out of a material's

## Constants

- `SERIES_KEY_EMPTY` — The `sk1-` key over an empty selection — the reserved value of this
- `SERIES_KEY_PREFIX` — Algorithm tag of a derived series key — `sk1-sha256:<64 lowercase
- `SERIES_RESERVED_VALUES` — The values carrying [`SERIES_KEY_PREFIX`] that still do not stand

