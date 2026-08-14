# asterism-core::application::material_layer_service

`MaterialLayerService` — the bands of marks over an Asset's
material, and the one rule that makes them worth having: **a person
edits their own band and nothing else**.

Verbs:
- [`list_by_asset`](MaterialLayerService::list_by_asset) — every
  band over an asset, in display order.
- [`create_user_layer`](MaterialLayerService::create_user_layer) —
  opens an empty band the person owns.
- [`set_default`](MaterialLayerService::set_default) — chooses which
  band a surface shows, and which one a new note lands in.
- [`delete_user_layer`](MaterialLayerService::delete_user_layer) —
  removes a band the person owns, with its contents.
- [`list_chapters`](MaterialLayerService::list_chapters) /
  [`post_chapter`](MaterialLayerService::post_chapter) /
  [`edit_chapter`](MaterialLayerService::edit_chapter) /
  [`delete_chapter`](MaterialLayerService::delete_chapter) — the
  sections inside one band.

# Where the immutability rule lives, and why here

An `Imported` band is the file's own statement about itself, and a
`Machine` band is a job's. Both are reproduced by running their
producer again, and neither has an author to ask about a conflict —
so a hand edit into one is either lost at the next re-read or
silently promoted into a claim the file never made. The guard is
therefore: **the four writing verbs above accept `LayerOrigin::User`
and refuse the rest.**

It is not in the entity, because the entity cannot see who is
calling: the re-probe path writes into an imported band *by design*
(that is what re-reading a file means), and a rule that refused
every write to one would refuse the only legitimate writer along
with the illegitimate ones. It is not in the schema either, for the
same reason — a `CHECK` cannot read the caller. What the schema does
hold is the half that is about rows rather than callers: one default
per `(asset, material, role)`, and a default annotation band that
belongs to the user.

The machine-side counterpart is
[`chapter_intake`](crate::application_support::chapter_intake),
in the support layer because a job drives it and no transport does.
Both routes reach the same two ports, and this is the only door with
a person on the other side of it.

# Attribution

Every write takes an [`AttributionContext`] it does not persist, for
the reason
[`MaterialMarkService`](crate::application::MaterialMarkService)
gives: a layer has no author column at all — it is a container, and
what it contains is what carries a voice. Receiving the argument is
still the point, since it is what makes a new caller of one of these
verbs name the channel it arrived through before it compiles.

# Two faces, and which one to call

The verbs above take and return **domain types**, unlike every
sibling in this module. Below them is a second `impl` block spelling
the same eight acts in **contract types** — commands in, DTOs out —
and that is the one the three adapters (HTTP, MCP, Tauri IPC) call.

The split is not a preference. Each adapter would otherwise parse the
wire ids and shape the DTOs itself, including the per-band assembly
[`MaterialLayerService::list_views`] does, and three copies of that
is how a surface ends up answering a question differently depending
on which door it was asked through. Keeping it here also keeps the
domain face, which is what the storage-level tests drive and what an
in-process caller holding a [`MaterialLayerId`] already has in hand:
neither of those should have to spell an id back into a string to ask
this service something.

When a verb is added, it belongs on both faces or on neither — a
domain verb no adapter can reach is the state this module was in
before the adapters landed, and it is not a state anything checks.

## Functions

- `default_annotation_layer` — The band a note lands in when the caller names none: the default
- `imported_structure_layer` — The band a re-read of the material writes into: the imported

## Types

- `MaterialLayerService` — Application-layer surface for [`MaterialLayer`] and the chapters

## Constants

- `MARKED_MATERIAL_ORD` — The material a timeline mark addresses.

