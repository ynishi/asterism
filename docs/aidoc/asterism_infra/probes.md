# asterism-infra::probes

The probes this build has, and the one question a caller asks of all
of them at once.

Each probe is one format's reading of the two walking fingerprint
axes ([`ArtefactProbe`]); this module is the list of probes, and
which formats each one answers for is that probe's own statement
([`ArtefactProbe::declares`]) rather than a second list kept here. So
a new format is one implementation and one line below, and nothing in
`asterism-core` is edited when it arrives.

That is what the format costs for the next file read, not for the
library already on disk. Rows imported before the probe landed hold
`unsupported:image/jpeg`, and every `unsupported:` value is a final
answer to "has anybody looked"
([`is_axis_answer`](asterism_core::domain::content_hash::is_axis_answer)),
so the ordinary fingerprint pass never offers them again: new imports
would take digests while the rows that were there first keep the
marker, and one column would hold two meanings with nothing to tell
them apart. A format therefore also arrives owing a way back to the
rows it was refused on, **per axis and per column**, since a probe
may claim one of them a slice before the other — which is what JPEG
did.

Two shapes have been used for that, and which one a case wants is
decided by how much reading it costs. A numbered `UPDATE` writing
NULL over the one stale marker is the cheap one, and it is what both
of JPEG's axes took (V72 for content, V76 for meta): the rows rejoin
the ordinary walk, which is already built for reads that must not
happen inside a transaction.
[`needs_content_walk`](asterism_core::domain::content_hash::needs_content_walk)
is the other — a second predicate over one column, driven by a
migration-chain read — and it stays reserved for the case it was
written for, a region definition that is versioned up. It still reads
the content column and only that; the meta axis has no equivalent
predicate and has not needed one.

# Answering for a set of probes

The gates are **OR** — a file is read if any probe claims it — and
the readings are **first claim wins**, in registration order. That
pairing is deliberate: the gate has to be optimistic because it runs
before the file is open and its only input is a guess from a
filename, while the reading happens with the bytes in hand and can
afford to be decided. Two probes claiming one mime **on one axis** is
a mistake at registration rather than a case to arbitrate at runtime,
and `tests::every_probe_here_is_reachable_through_the_registry` is
where it shows up.

Per axis, because the axes are counted separately everywhere else
([`ArtefactProbe`]): one probe reading a container's pixels and
another reading the metadata alongside it is a legitimate
arrangement, and a rule that counted claimants across both axes at
once would call it a collision.

Nothing claiming the format is not a failure: the row falls back to
the file axis, which groups byte-identical copies perfectly well, and
the columns say which format was declined rather than pretending the
bytes were read.

## Functions

- `content` — The content-axis reading, from the first probe that claims the
- `meta` — The meta-axis reading, from the first probe that claims the format.
- `meta_raw` — The metadata bytes the same probe keeps — the meta axis's other
- `walks_content` — Whether **any** probe reads the content axis for this declared
- `walks_meta` — Whether **any** probe reads the meta axis for this declared format.

