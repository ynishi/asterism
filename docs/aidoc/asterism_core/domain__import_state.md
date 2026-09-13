# asterism-core::domain::import_state

Where an importer got to, kept so the next run does not start over.

An importer scans a source and hands records to the server. #293 gave
it a way to say where it stopped — a checkpoint on the scan's own
stream, carried out of a run that earned one — and no way to keep
that anywhere. This is the place it is kept, and the whole of what
this layer does with it is store it and hand it back.

## The one rule

**The offset is opaque here.** It belongs to the adapter that wrote
it, and nothing on this side parses, validates, migrates or compares
it. That rule is stated in the inbound port's own vocabulary
(`asterism-importer-sdk`'s `SyncState`), and this is the layer where
breaking it would be easiest and least visible: a column tempting
somebody to index, a service tempted to "fix up" a shape it
recognises. It is carried as text for that reason as much as for the
bindings — text has nothing to be clever about.

The **key** is not opaque, because a store that could not compare
keys could not find anything. It is a persona and a partition, and
both halves are load-bearing. The persona, because the same source
imported into two personas has two independent positions and one
being ahead says nothing about the other. The partition, because it
is the adapter's own statement of what it is scanning — and the
adapter names it fully, its own kind included, so two adapters cannot
collide inside one persona by both calling something `root=/photos`.

## Types

- `ImportState` — A resumption point as this side holds it.
- `ImportStateKey` — What a stored position is a position *in*.

