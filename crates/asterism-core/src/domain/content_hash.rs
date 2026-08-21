//! `content_hash` — the fingerprint of an original artefact's bytes.
//!
//! Asterism's identity for an asset is its `locator`
//! (`UNIQUE(source_kind, source_locator)`), which answers "have I seen
//! this *path* before" and nothing else. The same photograph copied
//! into two folders is two assets, rated twice, tagged twice, and
//! shown twice in the grid — the duplicate problem every library tool
//! eventually grows a answer for.
//!
//! This module is that answer's first stage: a digest of the file's
//! bytes, stored on the material, so "the same picture" becomes a
//! question SQL can group by.
//!
//! # Not the snapshot hash
//!
//! [`snapshot_hash`](crate::domain::snapshot_hash) also produces a
//! SHA-256 hex string and means something entirely different: it
//! fingerprints an *ordered list of asset ids*, never a byte on disk.
//! The two must not be confused when reading a schema — a
//! `content_hash` column on `snapshot` is a member set, the one on
//! `material` is a file.
//!
//! # Why the algorithm is in the value
//!
//! Stored values carry a `sha256:` prefix. Exact-byte matching is the
//! cheap half of duplicate detection; the useful half is perceptual
//! (a re-encoded or resized copy of the same photograph), and that
//! wants a different algorithm rather than a different column. A
//! prefixed value lets a later pHash / embedding land beside this one
//! and lets a reader tell at a glance which kind of "same" a row is
//! claiming.
//!
//! # Select or re-render: a new digest has to say which
//!
//! Every digest in this workspace has to be a function of content
//! rather than of the way somebody happened to write that content
//! down, and there are only two routes to it.
//!
//! A digest that **selects** feeds the artefact's own bytes to the
//! hash — a byte range, a chunk, a string exactly as the container
//! stated it — and inherits its stability from the file. A digest that
//! **re-renders** parses the artefact and serialises the result. That
//! buys insensitivity to formatting, and pays for it by taking the
//! serialiser's habits into the definition: key order, number spelling,
//! string escaping, and what to do about a duplicate key all become
//! part of what the value means.
//!
//! Neither route is wrong, and neither is free. What is wrong is
//! leaving the choice unsaid, because the two fail in opposite
//! directions and the directions do not cost the same. A re-rendering
//! digest that widens an equivalence too far reports two different
//! artefacts as one, and duplicate resolution acts on that by folding
//! them — a wrong answer that destroys. A selecting digest that is too
//! narrow only fails to notice a match, which costs a row.
//!
//! So a digest that lands here owes three things: which of the two it
//! is; the canonical form written out in full if it re-renders — naming
//! a published scheme is not enough on its own, because the rule for
//! numbers and the rule for duplicate keys are the parts that decide
//! the answers; and a versioned tag, because a definition that has been
//! stored cannot be edited afterwards without changing what every value
//! written under it meant. [`META_DIGEST_PREFIX`] is that trade being
//! made deliberately for the meta axis: it selects, because
//! re-rendering a ComfyUI `prompt` graph would put a serialiser's
//! number formatting between two files the container itself calls
//! identical.

use crate::domain::duplicate_conflict::DuplicateAxis;
use crate::domain::measurement::MeasurementStatus;
use crate::error::DomainError;

/// The notation itself — the `sha256:` tag, the incremental hasher, and
/// the whole-slice convenience form — re-exported from
/// [`asterism_contract::digest`], which is where it now lives.
///
/// It moved because a caller that may *state* a digest has to be able
/// to spell one, and the caller that does is an importer:
/// `asterism-importer-sdk` depends on `asterism-contract` and on no
/// other Asterism crate, deliberately. Pointing it at this crate to
/// borrow seven lines would put the whole domain behind a plugin that
/// reads files, and copying the seven lines into the SDK would be one
/// grammar with two spellings.
///
/// Re-exported rather than left for each caller to import from the
/// contract crate, so that `content_hash::of_bytes` goes on meaning
/// what it meant at every site inside this workspace, and so that the
/// notation and the rules that read it are still found together. What
/// those rules are — the markers, the reserved values, the axes, the
/// versioned container tags — is domain, and none of it moved.
pub use asterism_contract::digest::{ContentHasher, DIGEST_PREFIX, of_bytes};

/// The **legacy stored spelling** of "this material can never have a
/// digest" — a record inside a container file, or a locator that is not
/// on this disk (every shape
/// [`SourceLocator::local_path`](crate::domain::source_locator::SourceLocator::local_path)
/// answers `None` for).
///
/// Until V92 this string sat in the digest columns themselves, so that
/// "we looked and there is nothing to read" stayed distinguishable from
/// "we have not looked yet". The distinction now lives in the status
/// column beside each digest
/// ([`MeasurementStatus::NoBytes`](crate::domain::measurement::MeasurementStatus::NoBytes)),
/// and no runtime writer produces this spelling any more. It is kept
/// because it is written into live databases: the V92 conversion maps
/// it, and a reader of a pre-V92 dump still meets it.
pub const UNHASHABLE: &str = "unhashable:no-bytes";

/// The digest of zero bytes — the one fingerprint every empty file
/// shares.
///
/// A real digest (SHA-256 of empty input is a well-known constant),
/// but a lie when read as "the same picture": empty files are almost
/// always failed-download debris, and a duplicate group built from
/// them invites a bulk "keep one" over files that have nothing in
/// common but their emptiness. Duplicate grouping excludes it the same
/// way it excludes [`UNHASHABLE`] — the hash stays on the material
/// (the fact is true), only the grouping declines to read it as
/// sameness.
pub const EMPTY: &str = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// The values a hash column holds that stand for something other than
/// "this artefact's bytes hash to that": the [`UNHASHABLE`] marker, and
/// the one digest ([`EMPTY`]) that is real and still says nothing about
/// sameness.
///
/// A list rather than two loose constants because an adapter has to
/// reproduce the exclusion in its own language, and the only safe way
/// to ask "which of these must the query name explicitly" is to walk
/// the same list this module keeps. A later marker added here reaches
/// those queries without an edit on their side.
pub const RESERVED_VALUES: &[&str] = &[UNHASHABLE, EMPTY];

/// Whether a stored value may stand for "the same picture" — the rule
/// duplicate matching runs on, and the only place it is decided.
///
/// A value fails it two ways. It may not be a digest at all: every
/// unhashable material carries the one [`UNHASHABLE`] marker, and a
/// future algorithm's output (`phash:…`) answers a different question,
/// so grouping across either would report the whole conversation
/// corpus — or two unrelated pictures — as one duplicate set. Or it may
/// be a real digest that means nothing as sameness, which is [`EMPTY`]:
/// every 0-byte file shares it, and those are usually failed-download
/// debris rather than copies of anything.
///
/// The duplicate report evaluates this rule inside SQLite rather than
/// here — the grouping and the row limit belong in the query — so its
/// adapter builds the equivalent `WHERE` clause from
/// [`digest_prefix`] and [`reserved_values`]. Constants shared that way
/// still leave the rule stated twice, so a test runs one vector of
/// stored-value shapes through both evaluations, on both axes, and
/// requires the verdicts to match.
///
/// # Why the axis is an argument rather than a second function
///
/// There are two columns now and they hold different vocabularies, but
/// they are read by one question. A `is_content_duplicate_key` beside
/// this one would be the same three lines with two constants swapped,
/// and the day a third exclusion is added it would be added to one of
/// them. The axis is already a type the schema stores
/// ([`DuplicateAxis`], in `duplicate_conflict.axis`) and the edge label
/// spells, so taking it here means the query, the queue row and the
/// predicate all name the axis with the same word.
pub fn is_duplicate_key(axis: DuplicateAxis, value: &str) -> bool {
    value.starts_with(digest_prefix(axis)) && !reserved_values(axis).contains(&value)
}

/// The algorithm tag values on `axis` carry — the half of
/// [`is_duplicate_key`] that SQL can express as a prefix test.
pub const fn digest_prefix(axis: DuplicateAxis) -> &'static str {
    match axis {
        DuplicateAxis::Artefact => DIGEST_PREFIX,
        DuplicateAxis::Content => CONTENT_DIGEST_PREFIX,
        DuplicateAxis::Meta => META_DIGEST_PREFIX,
    }
}

/// The values on `axis` that carry its prefix and still do not stand
/// for sameness — what an adapter has to name one by one, because a
/// prefix test cannot exclude them.
///
/// The `unsupported:` markers are **not** in either list, and that was
/// the shape difference worth noticing while they were stored values:
/// they do not carry the digest prefix, so the prefix test refused
/// them without a list entry. Since V92 they are not stored at all —
/// the status column carries the distinction — and the lists hold only
/// what they always genuinely needed to: the real digest every
/// artefact with nothing to hash would share, [`EMPTY`] on the file
/// axis and its siblings on the other two.
pub const fn reserved_values(axis: DuplicateAxis) -> &'static [&'static str] {
    match axis {
        DuplicateAxis::Artefact => RESERVED_VALUES,
        DuplicateAxis::Content => CONTENT_RESERVED_VALUES,
        DuplicateAxis::Meta => META_RESERVED_VALUES,
    }
}

/// Which question a stored value answers, read off its tag — `None`
/// when it carries none of them (a marker, a future algorithm, a
/// blank).
///
/// The tags cannot be confused for one another: neither `cr1-sha256:`
/// nor `m1-sha256:` begins with `sha256:`, which is what the version
/// prefix buys besides versioning.
///
/// # Why this walks the axes instead of testing them one by one
///
/// It was three `if`s, one tag each, and that shape is the one place in
/// this contract a fourth axis could be added without the compiler
/// objecting: [`digest_prefix`] and [`reserved_values`] are exhaustive
/// `match`es and would stop the build, while an `if` chain would simply
/// keep answering `None` for the new vocabulary — a value that is a
/// perfectly good digest read as though it carried no tag at all.
/// Walking [`STRONGEST_FIRST`](DuplicateAxis::STRONGEST_FIRST) and
/// asking [`digest_prefix`] means the question is answered from the
/// same list detection walks, so an axis is either on both or on
/// neither rather than half-known.
///
/// The order of the walk does not decide the answer, and that is a
/// property rather than an accident: it holds only while no axis's tag
/// begins with another's. Every tag today ends with its one and only
/// colon, which is what makes that true — a tag can only contain
/// another whole tag by swallowing its colon. A fourth one written as
/// a sub-namespace (`sha256:p1-`) would swallow the artefact tag and
/// make the answer depend on strength order;
/// `no_axis_tag_begins_with_another` fails on it here rather than
/// letting perceptual values group with files they share no bytes
/// with.
pub fn axis_of(value: &str) -> Option<DuplicateAxis> {
    DuplicateAxis::STRONGEST_FIRST
        .iter()
        .copied()
        .find(|axis| value.starts_with(digest_prefix(*axis)))
}

/// Algorithm tag of the *content* axis — the digest of only those bytes
/// that decide what the artefact decodes to, so that two files
/// differing solely in metadata share it.
///
/// Named here, next to the file axis, because the two prefixes are one
/// grammar: a value declares which question it answers, and a reader
/// that knows one tag has to know the other exists or it will read a
/// content digest as a file one. What computes it is whichever
/// [`ArtefactProbe`](crate::domain::probe::ArtefactProbe) claims the
/// container, the job that asks is `material_hash`, and the column that
/// holds its output is `material.content_region_hash` — which is what
/// makes a claim carrying this tag answerable, and so acceptable to
/// [`parse_declaration`].
///
/// [`content_region`](crate::domain::content_region) is the vocabulary
/// the answer is phrased in — the three outcomes and the markers — and
/// not the reading: it holds no format knowledge and opens nothing.
///
/// Versioned (`cr1-`) rather than bare `content-sha256:`, because the
/// region definition is the algorithm: widen or narrow which chunks are
/// fed to the hash and every previously computed value means something
/// else. A version in the tag lets the two generations coexist in one
/// column instead of silently comparing across a redefinition.
pub const CONTENT_DIGEST_PREFIX: &str = "cr1-sha256:";

/// The content-axis digest of a region with no bytes in it — the
/// [`EMPTY`] of the second axis.
///
/// No probe produces it — a container one of them walked to no region is
/// [`EMPTY_SPAN`](crate::domain::content_region::EMPTY_SPAN) instead,
/// which is the failure that measurement caught: every truncated PNG and
/// every fragmented mp4 walks to zero bytes, and a digest over zero bytes
/// is a real one, so they would all have landed in a single duplicate
/// group. It is reserved anyway, on the same terms as its file-axis
/// sibling: the exclusion is what the *grouping* trusts, and a value that
/// reached the column another way — a hand-edited row, a probe written
/// later that forgets — must not be read as sameness because the writer
/// of the day happened to be careful.
pub const CONTENT_REGION_EMPTY: &str =
    "cr1-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// [`RESERVED_VALUES`] for the content axis.
pub const CONTENT_RESERVED_VALUES: &[&str] = &[CONTENT_REGION_EMPTY];

/// Algorithm tag of the **meta** axis — the digest over the metadata a
/// container carries about the artefact, rendered canonically
/// ([`material_meta`](crate::domain::material_meta)).
///
/// Named here beside the other two because the three prefixes are one
/// grammar: a value declares which question it answers, and a reader
/// that knows two tags has to know the third exists or it will read a
/// meta digest as one of the others. What computes it is whichever
/// [`ArtefactProbe`](crate::domain::probe::ArtefactProbe) claims the
/// container, the job that asks is `material_hash`, and the column that
/// holds its output is `material.meta_hash`.
///
/// # `m1-`, and what a `m2-` would be for
///
/// Versioned for the reason [`CONTENT_DIGEST_PREFIX`] is: the
/// definition *is* the algorithm, and this one has two rules inside it
/// that a later reading could reasonably want to change.
///
/// - **Values stay as the container stated them — strings, unparsed.**
///   A ComfyUI `prompt` chunk happens to hold JSON, and parsing it in
///   order to re-render it would put number formatting and nested key
///   order into the digest's definition, so two files the container
///   calls identical could stop matching on a serialiser's habits. If
///   that proves too strict — the same workflow re-saved by a tool that
///   reformats — the answer is **`m2-sha256:`**, not an edit to the
///   walker. A new tag lets the two generations sit in one column
///   instead of silently comparing across a redefinition; editing the
///   rule in place would make every stored `m1-` value mean something
///   it was not computed to mean.
/// - **Album's own fields never enter it.** Title, labels,
///   `register_note`, ratings: those are what a person wrote here, and
///   a digest that moved when somebody renamed a picture would be
///   measuring the library rather than the artefact. Widening the form
///   to include *more of the container* (`zTXt`, `iTXt`, `eXIf`) is
///   also an `m2-`; widening it to include anything of Album's is not a
///   version bump but a different axis.
///
/// Whoever bumps it owes what a bump to `cr2-` would owe — see
/// [`needs_content_walk`] for why re-reading a whole library is a
/// decision about somebody's disk rather than a consequence of
/// shipping.
pub const META_DIGEST_PREFIX: &str = "m1-sha256:";

/// The meta-axis digest of the empty rendering (`{}`) — the [`EMPTY`]
/// of the third axis.
///
/// No probe produces it: a container one of them walked and found no
/// metadata in is
/// [`EMPTY_SPAN`](crate::domain::content_region::EMPTY_SPAN), because a
/// digest over `{}` is a perfectly real digest and every metadata-less
/// PNG in a library would share it — one duplicate group whose members
/// have nothing in common but their silence. It is reserved anyway, on
/// the same terms as its two siblings: the exclusion is what the
/// *grouping* trusts, and a value that reached the column another way
/// must not be read as sameness because the writer of the day happened
/// to be careful.
pub const META_EMPTY: &str =
    "m1-sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

/// [`RESERVED_VALUES`] for the meta axis.
pub const META_RESERVED_VALUES: &[&str] = &[META_EMPTY];

/// Whether one axis holds a **final answer** — the rule that decides
/// whether the fingerprint walk still has work to do on it.
///
/// This is not [`is_duplicate_key`] with the sign flipped. That rule
/// asks whether a digest may stand for "the same thing"; this one asks
/// whether anybody has looked and settled the question. The two
/// disagree on purpose: every non-`computed` answer status is a final
/// answer and none of them is a digest.
///
/// Read off the status column first, and off the digest only where the
/// status says there is one:
///
/// - [`Pending`](MeasurementStatus::Pending) — nobody has looked. Work.
/// - [`Failed`](MeasurementStatus::Failed) — somebody looked and the bytes
///   could not be read, which is an answer that can change (the disk
///   comes back). Work for the walk, so the retry happens; **not** open
///   work for the progress count, which is [`axis_open_work`]'s side of
///   the split.
/// - [`Computed`](MeasurementStatus::Computed) — an answer if the digest is of
///   the **current** generation. On the two versioned axes a value
///   written under an earlier definition (`cr0-sha256:…`) reads as no
///   answer and the row returns to the walk; that is the whole point of
///   versioning the tag — but it also means bumping the version puts
///   the entire library back in front of the walk, which is a decision
///   about somebody's disk rather than a consequence of shipping.
///   Whoever bumps it owes the same answer the column's arrival owed,
///   and [`MeasurementStatus::NotWalked`] records how that one was given. The
///   **artefact** axis's vocabulary is not versioned, so there holding
///   a value at all is holding an answer — the same asymmetry the old
///   `IS NULL` test spelled.
/// - Everything else — a statement about the artefact that no re-read
///   improves on: no probe reads the format, the walk found nothing,
///   the policy declined the bytes, the deferred migration has not
///   reached it, there are no bytes at all.
///
/// # Why the axis is an argument
///
/// The same reason [`is_duplicate_key`] takes one. Three columns are
/// read by one question, and a per-axis function would be the same
/// `match` with a constant swapped — the day a rule changes it would
/// change in one of them.
pub fn is_axis_answer(
    axis: DuplicateAxis,
    status: MeasurementStatus,
    digest: Option<&str>,
) -> bool {
    match status {
        MeasurementStatus::Pending | MeasurementStatus::Failed => false,
        MeasurementStatus::Computed => match axis {
            DuplicateAxis::Artefact => true,
            DuplicateAxis::Content | DuplicateAxis::Meta => {
                digest.is_some_and(|value| value.starts_with(digest_prefix(axis)))
            }
        },
        MeasurementStatus::Unsupported
        | MeasurementStatus::EmptySpan
        | MeasurementStatus::TooLarge
        | MeasurementStatus::NotWalked
        | MeasurementStatus::NoBytes => true,
    }
}

/// Whether one axis is **open work** — the progress side of the split
/// [`is_axis_answer`] describes under `Failed`.
///
/// `Pending` and a stale-generation digest are work that one successful
/// pass finishes, so they belong in the number that is supposed to
/// reach zero. `Failed` does not: the pass already ran and the file was
/// not where the library says it is, which no amount of walking fixes.
/// Counting those rows in the same denominator is what kept the "still
/// fingerprinting" notice from ever clearing — a permanent warning is a
/// warning nobody reads — so they are excluded here and surfaced as
/// their own count, with the reason on the row (issue #17's second
/// half).
pub fn axis_open_work(
    axis: DuplicateAxis,
    status: MeasurementStatus,
    digest: Option<&str>,
) -> bool {
    match status {
        MeasurementStatus::Pending => true,
        MeasurementStatus::Computed => !is_axis_answer(axis, status, digest),
        _ => false,
    }
}

/// Whether one material still owes a fingerprint pass — **the** rule,
/// evaluated in three places and defined here.
///
/// The three are the backfill's page query, the count behind the "still
/// fingerprinting" notice, and the per-asset job's skip test. They have
/// to answer identically or the product lies about itself: a count
/// stricter than the scan leaves a notice that never clears, a scan
/// stricter than the count hands the job rows it will skip forever, and
/// a skip test that disagrees with either re-reads files that were
/// already read. Two of the three are SQL, so the rule is stated twice
/// in two languages, and a differential test pins the two evaluations to
/// the same verdicts over a vector of column shapes.
///
/// The three columns are asked different questions on purpose. The file
/// column's vocabulary is not versioned — a digest or the one marker —
/// so holding *anything* is holding an answer, and `IS NULL` is the
/// whole test. The content and meta columns are versioned, so what they
/// hold has to be read ([`is_axis_answer`]).
///
/// A row where only some axes are answered is work, not a partial
/// result: the pass computes all of them from one read and writes them
/// in one statement, so a half-answered row can only come from a build
/// that predates the newest column, and re-reading is how it gets
/// finished.
///
/// # This is the walk's rule, not the progress count's
///
/// The two used to be one rule, and the split is deliberate (issue
/// #17): a `Failed` axis is work *here* — the walk retries it, because
/// an unreadable file can come back — and not work for
/// [`awaits_fingerprint`], which drives the number a person watches
/// reach zero. A count that included the permanent failures never
/// cleared, and a walk that excluded them never noticed the disk was
/// plugged back in; each rule keeps the half it can be right about.
///
/// # `material.meta_raw` is written by this pass and is not asked here
///
/// The fourth column the pass fills
/// ([`material_meta_raw`](crate::domain::material_meta_raw)) is
/// deliberately absent from the rule, and the paragraph above is exactly
/// why it had to be a decision rather than an omission: by that reading
/// every row in an existing library is half-filled the moment the column
/// arrives, so adding it here would put the whole library in front of
/// the walk and the next launch would read every file on somebody's
/// disk. That is the act
/// [`needs_content_walk`](crate::domain::content_hash::needs_content_walk)
/// was invented to refuse, and the reasoning for the refusal is
/// recorded there.
///
/// Leaving it out costs nothing this build has: a row with no raw holds
/// a correct `meta_kv` already, and the raw is only wanted on the day
/// somebody changes how metadata is expanded — which is a pass over the
/// rows that have one, not a reason to re-read the rows that do not. The
/// migration that adds the column therefore writes an answer
/// ([`NOT_CAPTURED`](crate::domain::material_meta_raw::NOT_CAPTURED))
/// rather than NULL, so that the deferred set stays selectable by
/// whoever writes that pass.
pub fn needs_fingerprint(
    file: (MeasurementStatus, Option<&str>),
    content: (MeasurementStatus, Option<&str>),
    meta: (MeasurementStatus, Option<&str>),
) -> bool {
    !is_axis_answer(DuplicateAxis::Artefact, file.0, file.1)
        || !is_axis_answer(DuplicateAxis::Content, content.0, content.1)
        || !is_axis_answer(DuplicateAxis::Meta, meta.0, meta.1)
}

/// Whether one material still counts toward the "still fingerprinting"
/// notice — the progress half of the split [`needs_fingerprint`]
/// describes.
///
/// Any axis that is [`open work`](axis_open_work) keeps the row in the
/// denominator: `pending`, or a digest of a superseded generation. A
/// row whose only remaining work is retrying a `failed` read does not —
/// it is not going to move on its own, and the number this drives is
/// supposed to reach zero. Those rows are
/// [`fingerprint_unreadable`]'s to count.
pub fn awaits_fingerprint(
    file: (MeasurementStatus, Option<&str>),
    content: (MeasurementStatus, Option<&str>),
    meta: (MeasurementStatus, Option<&str>),
) -> bool {
    axis_open_work(DuplicateAxis::Artefact, file.0, file.1)
        || axis_open_work(DuplicateAxis::Content, content.0, content.1)
        || axis_open_work(DuplicateAxis::Meta, meta.0, meta.1)
}

/// Whether one material is stuck on an unreadable original: the walk
/// still owes it a pass ([`needs_fingerprint`]) and nothing about it is
/// open work ([`awaits_fingerprint`]) — every unanswered axis is
/// `failed`.
///
/// The set behind `unreadable_material_count`: rows whose originals are
/// not where the library says they are. Surfaced as its own number,
/// with the I/O error in the reason column, rather than folded into the
/// progress count it would keep from ever reaching zero.
pub fn fingerprint_unreadable(
    file: (MeasurementStatus, Option<&str>),
    content: (MeasurementStatus, Option<&str>),
    meta: (MeasurementStatus, Option<&str>),
) -> bool {
    needs_fingerprint(file, content, meta) && !awaits_fingerprint(file, content, meta)
}

/// Whether one material is still owed the **data migration** that
/// finishes what the content column's schema migration started — the
/// rows it answered with
/// [`NOT_WALKED`](crate::domain::content_region::NOT_WALKED).
///
/// Adding the column was half a migration: the statement that creates it
/// cannot also compute its values, because computing them means opening
/// files. So the marker was written instead, as the record of *which
/// rows the values were deferred for*, and this predicate is how the
/// step that follows finds them again. Both steps belong to one chain
/// and run before the application serves anything, so a row selected
/// here is never a row somebody is looking at.
///
/// # Why this is a second predicate rather than a mode on the first
///
/// [`needs_fingerprint`] and this one disagree about the same value on
/// purpose, and the disagreement is the design. `NOT_WALKED` is an
/// *answer* to "has anybody looked", which is what keeps a pre-existing
/// library out of the ordinary walk; it is *work* for the migration that
/// exists to look. One function returning both verdicts would need an
/// argument saying which question is being asked, and the ordinary walk
/// evaluates its rule in three places — a page query, a progress count,
/// a per-asset skip test, two of them in SQL. A boolean threaded through
/// those three and defaulted wrong at one of them compiles, reads
/// correctly, and quietly merges the two passes: the ordinary backfill
/// starts handing out pre-existing rows, so the same file is read by two
/// walks at once and the "still fingerprinting" notice counts a
/// migration it does not describe.
///
/// Separate predicates cannot make that mistake, because widening this
/// one does not widen that one. What this drives is a different SQL
/// fragment reached through a different port method, so the ordinary
/// walk's query is not in the blast radius of a change here — the schema
/// migration's own teeth
/// (`v55_adds_the_content_axis_without_handing_the_walk_the_whole_library`)
/// go on proving that unchanged while this exists beside it.
///
/// # Equality against one status, not the family
///
/// The other answer statuses are answers no later pass improves on: a
/// format with no walker, a file past the size gate, a walk that found
/// no region. Selecting them would re-read every video in the library
/// in order to write back the status it already carries.
/// [`NotWalked`](MeasurementStatus::NotWalked) is the only one whose cause is
/// "the bytes were never spent", which is the only thing spending them
/// now can change.
///
/// # The next region version selects the same way and must not run the
/// same way
///
/// Bumping the tag (`cr2-`) redefines what a stored value means, and
/// [`is_axis_answer`] would read every `cr1-` value as no answer —
/// putting the whole library in front of the *ordinary* walk, which is
/// the act this marker was invented to refuse. The bump therefore owes
/// the V55 half: one `UPDATE` stamping `NOT_WALKED` over the superseded
/// generation, after which this predicate selects exactly those rows
/// with no second detector and no edit here.
///
/// The **other** half does not carry over. Computing today's values runs
/// inside the migration chain because the set is what one small library
/// held when the column arrived, and only its walkable part is read at
/// all. A version bump makes the set *every row ever walked*, on
/// libraries of any size, and a released application may not answer an
/// update by reading somebody's whole disk before it will open. That
/// bump owes an explicitly managed upgrade moment — announced, and
/// timed by the person whose disk it is — rather than a second copy of
/// the step that fills the column in today.
///
/// `Pending` (nothing written at all) is **not** work here: that row
/// belongs to [`needs_fingerprint`], and claiming it in both places
/// would hand one file to two passes that each read it.
pub fn needs_content_walk(content_status: MeasurementStatus) -> bool {
    content_status == MeasurementStatus::NotWalked
}

/// Hex characters in a SHA-256 digest.
const DIGEST_HEX_LEN: usize = 64;

/// Reads a caller's **declaration** about the bytes it is registering
/// — `AddAssetCommand::declared_content_hash` — and hands back the
/// digest it claims, on whichever axis it named.
///
/// # This is an assertion, not a fingerprint
///
/// The returned string is what the caller *said*, and the whole point
/// of routing it through a named parser is that it never reaches
/// `material.content_hash`: that column holds what the hash job read
/// off the disk, and a claim written into it would be authority granted
/// before any verification happened. The claim is kept on the row's
/// `_trace` bag and confirmed later, when the bytes are actually read.
///
/// # What is accepted
///
/// | form | verdict |
/// |---|---|
/// | `sha256:<64 lowercase hex>` | accepted — the file axis |
/// | `cr1-sha256:<64 lowercase hex>` | accepted — the content axis, now that a column holds it and a walker computes it |
/// | `m1-sha256:<64 lowercase hex>` | accepted on the same terms — the meta axis |
/// | anything else (`phash:…`, bare hex, a storage marker, blank) | refused |
///
/// The two walker axes are accepted rather than special-cased even
/// though nothing outside is expected to declare one: computing either
/// means running the container walker, which is what makes a caller
/// that *has* done so worth believing enough to check. The rule is the
/// same on all three — a claim is worth keeping only if something later
/// arrives to check it against — so the acceptance follows the tag
/// rather than a list this function keeps.
///
/// The content axis was refused until this wave, and the reason it was
/// refused is the reason it is accepted now: a claim is worth keeping
/// only if something later arrives to check it against. The walker
/// exists, the column holds its output, and the hash job compares the
/// claim with the value it computed **on the axis the claim named** —
/// so a `cr1-sha256:` declaration is now answerable, and the answer is
/// the useful one for this corpus (a caller that knows its pixels are
/// unchanged can say so, and be told when they are not).
///
/// A claim on the content axis can land where no digest was computed:
/// the file may be a format with no walker, or too big to walk, and the
/// column then holds a marker. The claim is **not** checked against it
/// and keeps no verdict — the state [`declaration_claim`] spells by
/// having no `verified` field. A marker is not a smaller digest, it is
/// the record of a measurement that did not happen, and "the bytes
/// disagree with you" is a false thing to say about bytes nobody
/// hashed. The check arrives if the axis becomes measurable (the gate
/// is raised, a walker lands) and the row is fingerprinted again.
///
/// The empty-file digest ([`EMPTY`]) is accepted like any other: it is
/// a true statement about a 0-byte file and the job can confirm it.
/// [`is_duplicate_key`] excludes that value from *grouping*, which is a
/// different question — "does this stand for sameness" rather than "is
/// this a digest of bytes".
///
/// # Why the malformed shapes are refused instead of recorded
///
/// A claim that cannot equal any digest the hasher produces is a
/// guaranteed future mismatch. Recording one would put an alarm on the
/// row that says the file's bytes disagree with the caller, when what
/// disagrees is the caller with the notation — and the person reading
/// the alarm goes looking at the file. Refusing says the true thing, at
/// the moment the caller can still fix it, and costs nothing: the field
/// is optional, so the retry is the same request without it.
///
/// A bare hex string is refused for the reason
/// [`provenance::parse`](crate::domain::provenance::parse) refuses a
/// bare uuid — guessing the algorithm makes today's `sha256` and
/// tomorrow's second one the same spelling, and the callers written
/// against the guess resolve against the wrong one from then on.
/// Uppercase hex is refused on the same terms rather than lowercased
/// on the caller's behalf: the storage form is lowercase, and quietly
/// repairing a value means the declaration that gets checked is not the
/// one that was made.
///
/// The refusal is here, in the domain, rather than at deserialisation
/// the way `on_duplicate` refuses an unknown token. That field is a
/// closed three-value set, which serde can police; this one is an open
/// notation whose accepted set grows when a walker lands, and the rule
/// belongs beside the prefixes it is written from.
pub fn parse_declaration(raw: &str) -> Result<String, DomainError> {
    let declared = raw.trim();
    if declared.is_empty() {
        return Err(DomainError::Validation(
            "declared_content_hash is blank; leave the field out rather than \
             declaring nothing"
                .into(),
        ));
    }
    if let Some(axis) = axis_of(declared) {
        let prefix = digest_prefix(axis);
        let hex = declared.strip_prefix(prefix).unwrap_or_default();
        if hex.len() != DIGEST_HEX_LEN
            || !hex
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(DomainError::Validation(format!(
                "declared_content_hash {declared:?} is not a {prefix}digest \
                 ({DIGEST_HEX_LEN} lowercase hex characters)"
            )));
        }
        return Ok(declared.to_string());
    }
    Err(DomainError::Validation(format!(
        "declared_content_hash {declared:?} has no known algorithm tag \
         (expected \"{DIGEST_PREFIX}<hex>\", \"{CONTENT_DIGEST_PREFIX}<hex>\" \
         or \"{META_DIGEST_PREFIX}<hex>\")"
    )))
}

/// Key under which a declaration and its verdict live on the asset's
/// `extra.`[`_trace`](crate::domain::provenance::TRACE_KEY) bag.
///
/// `_trace` is where this library keeps "where did this statement come
/// from and what happened to it" — `source` and `operator` for a
/// provenance claim, `fold` and `absorbed` for a resolved duplicate. A
/// hash somebody asserted is the same kind of thing, and the bag is the
/// one place a reader already looks for assertions rather than for
/// facts.
///
/// The alternative considered and rejected was a second column on
/// `material`, beside `content_hash`. It would have been cheaper for
/// the backfill (the scan row could carry it), and that is exactly the
/// shape of the hazard: two hash-looking columns on the row that owns
/// the digest, so every future query that means "the fingerprint" has
/// to pick the right one, and a wrong pick reads a caller's assertion
/// as a measurement. It would also have cost a migration to hold a
/// value that is read once, and left the claim in one place while its
/// verdict lived in another.
pub const DECLARED_HASH_NOTE_KEY: &str = "declared_hash";

/// The note recorded at registration: the claim, and nothing else.
///
/// No verdict field, deliberately — absent means "not checked yet",
/// which is the true state between the ingest returning and the hash
/// job reading the file. A `verified: false` written here would be
/// indistinguishable from a failed check.
///
/// The axis label is read off the value's own tag rather than assumed.
/// It was a constant (`"file"`, this axis's spelling before V64) while
/// that was the only acceptable claim, and the note carried it anyway so
/// a note written then would still say what it meant once there were two
/// — which is now. Notes written under the old spelling were rewritten
/// by V64 along with every other stored axis slug. A value with
/// no tag cannot reach here through [`parse_declaration`]; if one does,
/// the label is `null`, because naming an axis for it would be this
/// function inventing the fact the field exists to record.
pub fn declaration_claim(declared: &str) -> serde_json::Value {
    let axis = axis_of(declared).map(|axis| axis.as_str());
    serde_json::json!({ "value": declared, "axis": axis })
}

/// The note the hash job writes once it has read the bytes: the same
/// claim, plus what the file actually hashed to.
///
/// `got` appears **only on a mismatch**. On agreement the recomputed
/// digest is already on the material, and copying it here would put the
/// same fingerprint in two places for a reader to reconcile. On
/// disagreement it is the whole point: `value` is what was specified,
/// `got` is what the bytes say, and a reader who sees only one of them
/// cannot tell which side to go and look at.
pub fn declaration_verdict(declared: &str, actual: &str, at_ms: i64) -> serde_json::Value {
    let mut note = declaration_claim(declared);
    note["verified"] = serde_json::json!(declared == actual);
    note["checked_at_ms"] = serde_json::json!(at_ms);
    if declared != actual {
        note["got"] = serde_json::json!(actual);
    }
    note
}

// `ContentHasher` and `of_bytes` used to be defined here. They are the
// same code in `asterism_contract::digest` now and are re-exported at
// the top of this module, so every call site inside the workspace still
// reads `content_hash::of_bytes` — see the `pub use` for why they went.

// `is_hashable_locator(&str)` used to live here. It is gone rather than
// ported: its whole body was the discrimination
// [`SourceLocator`](crate::domain::source_locator::SourceLocator) and
// its four shapes now perform on their own behalf, and the question it
// answered is
// [`local_path()`](crate::domain::source_locator::SourceLocator::local_path)
// — which returns the path instead of a `bool`, so the caller that
// wanted one no longer re-derives it (`original_file` used to ask the
// predicate and then strip `file://` itself, because the predicate's
// answer was not a path).
//
// Keeping a `&str` version would have left a way to ask the question
// again without going through a boundary. The one caller that legally
// works on raw column values — the V56 walk — carries a frozen copy
// beside itself in `asterism-infra`, so that a landed migration keeps
// meaning what it meant when it ran.

#[cfg(test)]
mod tests {
    use super::*;

    // The four tests that stood here — same-bytes-same-value, the
    // algorithm tag and hex shape, streaming against whole-slice, and
    // the empty input — went with the code they assert. They are in
    // `asterism_contract::digest` now, beside the hasher.

    #[test]
    fn only_a_digest_of_real_bytes_is_a_duplicate_key() {
        let file = DuplicateAxis::Artefact;
        assert!(is_duplicate_key(file, &of_bytes(b"star")));
        assert!(!is_duplicate_key(file, UNHASHABLE));
        // A future algorithm's values must not be grouped with sha256
        // ones either — same fingerprint, different question.
        assert!(!is_duplicate_key(file, "phash:0f0f0f0f"));
        // A real digest, and still not sameness.
        assert!(!is_duplicate_key(file, EMPTY));
        assert!(!is_duplicate_key(file, &of_bytes(b"")));
    }

    /// Each axis reads its own column with the same rule, and no two of
    /// them may read each other's values: a content digest grouped as a
    /// file one would report two files that differ in a metadata chunk
    /// as byte-identical, and a meta digest grouped as a content one
    /// would report two frames off one workflow as the same picture.
    #[test]
    fn each_axis_admits_only_its_own_digests() {
        let content = format!("{CONTENT_DIGEST_PREFIX}{}", "a".repeat(64));
        let meta = format!("{META_DIGEST_PREFIX}{}", "a".repeat(64));
        let file = of_bytes(b"star");

        // Every axis against every other axis's digest — the pairwise
        // form rather than a spot check, because the failure is one
        // vocabulary being admitted by one other axis.
        for (owner, value) in [
            (DuplicateAxis::Artefact, &file),
            (DuplicateAxis::Content, &content),
            (DuplicateAxis::Meta, &meta),
        ] {
            for axis in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
                assert_eq!(
                    is_duplicate_key(axis, value),
                    axis == owner,
                    "{value} read on the {} axis",
                    axis.as_str()
                );
            }
            assert_eq!(axis_of(value), Some(owner));
        }

        // Every marker the two versioned columns carry is refused, and
        // the `unsupported:` family is refused by the prefix test rather
        // than by being listed — which is what lets a marker added
        // later reach the queries without an edit on their side.
        for marker in [
            crate::domain::content_region::EMPTY_SPAN,
            crate::domain::content_region::TOO_LARGE,
            crate::domain::content_region::NOT_WALKED,
            "unsupported:video/mp4",
            "unsupported:a-format-invented-tomorrow",
            UNHASHABLE,
            CONTENT_REGION_EMPTY,
            META_EMPTY,
        ] {
            for axis in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
                assert!(
                    !is_duplicate_key(axis, marker),
                    "{marker} groups as a duplicate on {}",
                    axis.as_str()
                );
            }
        }

        assert_eq!(axis_of(UNHASHABLE), None);
        assert_eq!(axis_of("phash:0f0f0f0f"), None);
    }

    /// What lets [`axis_of`] read a tag without caring which order it
    /// walks the axes in.
    ///
    /// A tag that began with another tag would make the answer depend
    /// on strength order: a perceptual axis tagged `sha256:p1-` starts
    /// with `sha256:`, so its values would read as artefact digests and
    /// group with files they share no bytes with. What rules that out
    /// today is that every tag ends with its one and only colon —
    /// `cr1-sha256:` prefixes the version rather than suffixing it, so
    /// no tag contains another whole one. This holds that consequence
    /// rather than the convention, because the convention is a habit
    /// and the consequence is what [`axis_of`] depends on.
    ///
    /// Both directions, because the hazard is not symmetric in the
    /// walk: whichever of an overlapping pair is reached first wins,
    /// and which one that is depends on
    /// [`STRONGEST_FIRST`](DuplicateAxis::STRONGEST_FIRST).
    #[test]
    fn no_axis_tag_begins_with_another() {
        for outer in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
            for inner in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
                if outer == inner {
                    continue;
                }
                assert!(
                    !digest_prefix(outer).starts_with(digest_prefix(inner)),
                    "the {} tag ({}) begins with the {} tag ({}), so axis_of's \
                     answer depends on the order of the walk",
                    outer.as_str(),
                    digest_prefix(outer),
                    inner.as_str(),
                    digest_prefix(inner),
                );
            }
        }
    }

    /// The empty-region constant is pinned against the file axis's own
    /// empty digest rather than copied from a reference: the region
    /// hasher is the same SHA-256, so the two differ only by tag, and a
    /// constant that drifted would stop excluding the one value every
    /// artefact with nothing to hash would share.
    #[test]
    fn the_empty_region_constant_is_the_empty_digest_under_the_region_tag() {
        let hex = EMPTY
            .strip_prefix(DIGEST_PREFIX)
            .expect("the empty digest carries its algorithm");
        assert_eq!(
            CONTENT_REGION_EMPTY,
            format!("{CONTENT_DIGEST_PREFIX}{hex}")
        );
        // The meta axis's reserved value is **not** the same shape: its
        // canonical form is a JSON object, so the value every
        // metadata-less container would share is the digest of `{}`
        // rather than of nothing at all. Pinned against the walker's own
        // renderer in `material_meta`, which is where the two are
        // computed side by side.
        assert!(META_EMPTY.starts_with(META_DIGEST_PREFIX));
        assert_ne!(
            META_EMPTY,
            format!("{META_DIGEST_PREFIX}{hex}"),
            "the empty *object* is not the empty *input*"
        );
    }

    #[test]
    fn every_reserved_value_is_refused_as_a_duplicate_key() {
        // The list is what adapters walk to build their own form of the
        // exclusion. A value that landed in it while still passing the
        // rule would be an exclusion that excludes nothing.
        for axis in DuplicateAxis::STRONGEST_FIRST.iter().copied() {
            assert!(!reserved_values(axis).is_empty());
            for value in reserved_values(axis) {
                assert!(
                    !is_duplicate_key(axis, value),
                    "{value} groups as a duplicate on {}",
                    axis.as_str()
                );
            }
        }
        // Note that one entry earns its place differently from the
        // others. `EMPTY` is a real digest under the file prefix, so the
        // list is the only thing standing between it and a duplicate
        // group; `UNHASHABLE` is not a digest at all and the prefix test
        // already refuses it, which makes its listing a second lock on a
        // door that is shut. Dropping it would reach the SQL dialect of
        // this rule and the differential test that pins the two together,
        // so it stays until something is being changed there anyway.
    }

    /// The rule the fingerprint walk stops on. An answer is not the same
    /// thing as a fingerprint: every non-`computed` answer status is a
    /// final answer and none of them is a digest, and a walk that read
    /// them as unanswered would pick the same rows up on every pass
    /// forever.
    #[test]
    fn a_material_owes_a_pass_until_every_axis_holds_an_answer() {
        use MeasurementStatus::{Computed, Failed, Pending};

        let file = of_bytes(b"star");
        let content = format!("{CONTENT_DIGEST_PREFIX}{}", "b".repeat(64));
        let meta = format!("{META_DIGEST_PREFIX}{}", "d".repeat(64));
        let done_file = (Computed, Some(file.as_str()));
        let done_content = (Computed, Some(content.as_str()));
        let done_meta = (Computed, Some(meta.as_str()));
        let blank = (Pending, None);

        // Nothing looked yet, and the half-answered shapes: any of them
        // is work, because a pass writes every axis together.
        assert!(needs_fingerprint(blank, blank, blank));
        assert!(needs_fingerprint(done_file, blank, blank));
        assert!(needs_fingerprint(blank, done_content, done_meta));
        assert!(needs_fingerprint(done_file, done_content, blank));
        assert!(needs_fingerprint(done_file, blank, done_meta));

        // All answered — a digest, or any status saying why there is
        // no digest.
        assert!(!needs_fingerprint(done_file, done_content, done_meta));
        for answered in [
            MeasurementStatus::Unsupported,
            MeasurementStatus::EmptySpan,
            MeasurementStatus::TooLarge,
            MeasurementStatus::NotWalked,
            MeasurementStatus::NoBytes,
        ] {
            let settled = (answered, None);
            assert!(
                !needs_fingerprint(done_file, settled, settled),
                "{} is an answer, so the row is not work",
                answered.as_str()
            );
            // The statuses are shared across the two versioned axes on
            // purpose: they say something about the artefact, not about
            // which measurement was attempted.
            assert!(is_axis_answer(DuplicateAxis::Content, answered, None));
            assert!(is_axis_answer(DuplicateAxis::Meta, answered, None));
        }

        // A digest from an earlier definition is not an answer to the
        // question being asked now — on either versioned axis.
        let stale_content = format!("cr0-sha256:{}", "c".repeat(64));
        let stale_meta = format!("m0-sha256:{}", "d".repeat(64));
        assert!(needs_fingerprint(
            done_file,
            (Computed, Some(&stale_content)),
            done_meta
        ));
        assert!(needs_fingerprint(
            done_file,
            done_content,
            (Computed, Some(&stale_meta))
        ));
        // Neither is one axis's digest sitting in another's column,
        // which is what a column mix-up looks like from here.
        assert!(needs_fingerprint(
            done_file,
            (Computed, Some(&file)),
            done_meta
        ));
        assert!(needs_fingerprint(
            done_file,
            done_content,
            (Computed, Some(&content))
        ));
        assert!(!is_axis_answer(
            DuplicateAxis::Content,
            Computed,
            Some(&file)
        ));
        assert!(!is_axis_answer(
            DuplicateAxis::Meta,
            Computed,
            Some(&content)
        ));
        // The artefact axis's vocabulary is not versioned, so there
        // holding a value at all is holding an answer — the asymmetry
        // the old `IS NULL` test spelled.
        assert!(is_axis_answer(
            DuplicateAxis::Artefact,
            Computed,
            Some("phash:0f0f0f0f")
        ));

        // A failed read is work for the walk — the retry is the visible
        // exit an unreadable original keeps.
        let failed = (Failed, None);
        assert!(needs_fingerprint(done_file, failed, failed));
        assert!(needs_fingerprint(failed, failed, failed));
    }

    /// The split issue #17 asked for: `failed` rows leave the progress
    /// denominator and become their own count, while staying work for
    /// the walk so the retry survives.
    #[test]
    fn an_unreadable_original_is_counted_apart_from_open_work() {
        use MeasurementStatus::{Computed, Failed, Pending};

        let file = of_bytes(b"star");
        let content = format!("{CONTENT_DIGEST_PREFIX}{}", "b".repeat(64));
        let meta = format!("{META_DIGEST_PREFIX}{}", "d".repeat(64));
        let done_file = (Computed, Some(file.as_str()));
        let done_content = (Computed, Some(content.as_str()));
        let done_meta = (Computed, Some(meta.as_str()));
        let blank = (Pending, None);
        let failed = (Failed, None);

        // Open work counts toward progress and is not "unreadable".
        assert!(awaits_fingerprint(blank, blank, blank));
        assert!(!fingerprint_unreadable(blank, blank, blank));

        // A row whose only remaining work is retrying a failed read:
        // out of the denominator, into the unreadable count.
        assert!(!awaits_fingerprint(failed, failed, failed));
        assert!(fingerprint_unreadable(failed, failed, failed));
        assert!(!awaits_fingerprint(done_file, failed, failed));
        assert!(fingerprint_unreadable(done_file, failed, failed));

        // Mixed: one axis pending, one failed — still in progress (a
        // pass is going to run anyway), so not counted twice.
        assert!(awaits_fingerprint(blank, failed, done_meta));
        assert!(!fingerprint_unreadable(blank, failed, done_meta));

        // A stale generation is open work, not an unreadable original.
        let stale = format!("cr0-sha256:{}", "c".repeat(64));
        assert!(awaits_fingerprint(
            done_file,
            (Computed, Some(&stale)),
            done_meta
        ));

        // Fully answered rows are in neither number.
        assert!(!awaits_fingerprint(done_file, done_content, done_meta));
        assert!(!fingerprint_unreadable(done_file, done_content, done_meta));
        let settled = (MeasurementStatus::NoBytes, None);
        assert!(!awaits_fingerprint(settled, settled, settled));
        assert!(!fingerprint_unreadable(settled, settled, settled));
    }

    /// The two predicates split the same column, and the split is what
    /// keeps the deferred migration out of the ordinary walk.
    ///
    /// The failure being pinned is the one that compiles: widening the
    /// ordinary rule so that `NOT_WALKED` reads as work there too. Every
    /// pre-existing row would then be handed to the startup backfill —
    /// the whole library re-read by the pass that is supposed to be for
    /// material that just arrived, with the migration walking the same
    /// files from the other end.
    #[test]
    fn the_migration_claims_exactly_the_rows_the_ordinary_walk_calls_answered() {
        use MeasurementStatus::{Computed, NotWalked, Pending};

        let file = of_bytes(b"star");
        let meta = format!("{META_DIGEST_PREFIX}{}", "d".repeat(64));
        let done_file = (Computed, Some(file.as_str()));
        let done_meta = (Computed, Some(meta.as_str()));

        // The one status the migration is about, and the two rules
        // disagreeing about it on purpose.
        assert!(needs_content_walk(NotWalked));
        assert!(!needs_fingerprint(done_file, (NotWalked, None), done_meta));

        // Nothing else is the migration's work — including the states
        // that are work for the other pass, which is the direction that
        // would give one file to two walks.
        for other in [
            MeasurementStatus::Pending,
            MeasurementStatus::Computed,
            MeasurementStatus::Unsupported,
            MeasurementStatus::EmptySpan,
            MeasurementStatus::TooLarge,
            MeasurementStatus::NoBytes,
            MeasurementStatus::Failed,
        ] {
            assert!(
                !needs_content_walk(other),
                "{} is not a row the migration was deferred for",
                other.as_str()
            );
        }
        // …and the pending state really is work for the other rule, so
        // the loop above is a split rather than a predicate that refuses
        // everything.
        assert!(needs_fingerprint(done_file, (Pending, None), done_meta));
        let stale = format!("cr0-sha256:{}", "c".repeat(64));
        assert!(needs_fingerprint(
            done_file,
            (Computed, Some(&stale)),
            done_meta
        ));
    }

    // The two tests that stood here — `container_records_and_remote_-
    // locators_are_not_hashable` and `a_logical_name_is_not_a_file_to_-
    // open` — went with the predicate they asserted. What they were
    // about is now asserted against the type that decides it, in
    // `domain::source_locator`: the Windows-drive fixture in
    // `a_one_character_scheme_is_a_windows_drive_not_a_scheme`, the
    // container / rootless-container pair in
    // `a_record_needs_a_container_that_can_be_opened`, and the
    // caller-minted names in `a_caller_minted_name_is_the_sink`. The
    // `file://` case moved *and* changed answer, deliberately — see
    // `the_file_scheme_is_consumed_so_two_spellings_are_one_locator`.

    #[test]
    fn a_declaration_on_the_file_axis_is_taken_verbatim() {
        let digest = of_bytes(b"star");
        assert_eq!(parse_declaration(&digest).unwrap(), digest);
        // The token travels through shells and agent payloads, so a
        // stray space is the most ordinary damage it takes — same
        // allowance `provenance::parse` makes.
        assert_eq!(parse_declaration(&format!("  {digest}\n")).unwrap(), digest);
        // A true statement about a 0-byte file. Grouping declines to
        // read this value as sameness; the integrity check has no
        // reason to decline to confirm it.
        assert_eq!(parse_declaration(EMPTY).unwrap(), EMPTY);
    }

    /// The content axis was refused while nothing computed it. It is
    /// taken now, on the same terms as the file axis and under its own
    /// label — the label is what lets the job pick which recomputed
    /// value to check the claim against.
    #[test]
    fn a_content_axis_declaration_is_taken_under_its_own_axis() {
        let declared = format!("{CONTENT_DIGEST_PREFIX}{}", "a".repeat(64));
        assert_eq!(parse_declaration(&declared).unwrap(), declared);
        assert_eq!(
            parse_declaration(&format!(" {declared}\n")).unwrap(),
            declared
        );

        let note = declaration_claim(&declared);
        assert_eq!(note["axis"], serde_json::json!("content"));
        assert_eq!(note["value"], serde_json::json!(declared));

        // Same shape rules as the file axis: a claim the walker could
        // never produce is a guaranteed future mismatch about a file
        // that is fine.
        for wrong in [
            CONTENT_DIGEST_PREFIX.to_string(),
            format!("{CONTENT_DIGEST_PREFIX}{}", "a".repeat(63)),
            format!("{CONTENT_DIGEST_PREFIX}{}", "A".repeat(64)),
            format!("{CONTENT_DIGEST_PREFIX}{}", "z".repeat(64)),
        ] {
            let err = parse_declaration(&wrong).unwrap_err().to_string();
            assert!(
                err.contains("lowercase hex") && err.contains(CONTENT_DIGEST_PREFIX),
                "{wrong} should be refused as a content digest: {err}"
            );
        }

        // A marker is still not a claim about bytes, on either axis.
        let err = parse_declaration(crate::domain::content_region::EMPTY_SPAN)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no known algorithm tag"), "{err}");
    }

    #[test]
    fn a_declaration_that_could_never_match_is_refused_at_the_boundary() {
        // Right tag, wrong shape — a value the hasher cannot produce,
        // so keeping it would guarantee a mismatch report about a file
        // that is fine.
        for wrong in [
            "sha256:",
            "sha256:abc",
            &format!("sha256:{}", "a".repeat(63)),
            &format!("sha256:{}", "a".repeat(65)),
            &format!("sha256:{}", "A".repeat(64)),
            &format!("sha256:{}", "z".repeat(64)),
        ] {
            let err = parse_declaration(wrong).unwrap_err().to_string();
            assert!(
                err.contains("lowercase hex"),
                "{wrong} should be refused for its shape: {err}"
            );
        }
        // No tag at all: guessing that a bare digest means sha256 is
        // what makes a second algorithm indistinguishable from the
        // first.
        let bare = "a".repeat(64);
        let err = parse_declaration(&bare).unwrap_err().to_string();
        assert!(err.contains("no known algorithm tag"), "{err}");
        // A future algorithm answers a different question.
        let err = parse_declaration("phash:0f0f0f0f").unwrap_err().to_string();
        assert!(err.contains("no known algorithm tag"), "{err}");
        // A storage marker is not a claim about bytes.
        let err = parse_declaration(UNHASHABLE).unwrap_err().to_string();
        assert!(err.contains("no known algorithm tag"), "{err}");
        // Blank is a field that says nothing while looking like it
        // says something.
        for blank in ["", "   ", "\n"] {
            let err = parse_declaration(blank).unwrap_err().to_string();
            assert!(err.contains("blank"), "{blank:?}: {err}");
        }
    }

    #[test]
    fn a_declaration_note_carries_no_verdict_until_there_is_one() {
        let digest = of_bytes(b"star");
        let note = declaration_claim(&digest);
        assert_eq!(note["value"], serde_json::json!(digest));
        assert_eq!(note["axis"], serde_json::json!("artefact"));
        // Unchecked is its own state and has to read as one.
        assert!(note.get("verified").is_none(), "{note}");
        assert!(note.get("got").is_none(), "{note}");
    }

    #[test]
    fn a_declaration_verdict_carries_both_sides_only_when_they_differ() {
        let declared = of_bytes(b"what the caller said");
        let actual = of_bytes(b"what the file holds");

        let disagreed = declaration_verdict(&declared, &actual, 1_785_000_000_000);
        assert_eq!(disagreed["verified"], serde_json::json!(false));
        assert_eq!(disagreed["value"], serde_json::json!(declared));
        assert_eq!(
            disagreed["got"],
            serde_json::json!(actual),
            "specified and got both, or the reader cannot tell which side to look at"
        );

        let agreed = declaration_verdict(&declared, &declared, 1_785_000_000_000);
        assert_eq!(agreed["verified"], serde_json::json!(true));
        assert!(
            agreed.get("got").is_none(),
            "the digest is on the material; a second copy here is one more thing to reconcile"
        );
    }

    #[test]
    fn the_empty_constant_is_what_the_hasher_says_about_zero_bytes() {
        // Pinned against the hasher, not copied from a reference: if
        // the storage form ever changes (prefix, casing), the constant
        // must move with it or the duplicate exclusion silently stops
        // matching anything.
        assert_eq!(of_bytes(b""), EMPTY);
    }
}
