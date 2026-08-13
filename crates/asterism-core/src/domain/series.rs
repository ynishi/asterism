//! `series` — "made the same way": a rule for reading a material's
//! metadata, and the key that rule derives.
//!
//! [`material_meta`](crate::domain::material_meta) answers one question
//! about a container — *it carried this text* — over everything the
//! container carried. That is the right sentence for the meta axis and
//! the wrong one for a run. Measured on eleven images out of two VDSL
//! runs: a digest over the whole metadata set separates all eleven, and
//! so does dropping the run's `timestamp`, and so does dropping the
//! generator's chunk entirely. What splits them is the `prompt` chunk —
//! a compiled graph that differs per image — and **no exclusion reaches
//! it**. The only reading that recovers the two runs selects the recipe
//! and nothing else (`["vdsl","script"]` → two keys, five images and
//! six). The tests at the bottom of this file are that measurement,
//! frozen.
//!
//! # A key is a second sentence, not a better digest
//!
//! Nothing here touches `m1-`. `meta_kv` states what the container
//! carried; a [`SeriesKey`] states what one [`Strategy`] made of it.
//! Two statements about one material, so neither has to be weakened to
//! accommodate the other — the digest keeps saying *this text*, and the
//! key gets to say *this recipe* without a `m2-` and without a claim
//! that two files are the same thing.
//!
//! It is also why a Strategy can be rewritten cheaply. [`derive`] reads
//! `meta_kv` and nothing else: no bytes, no locator, no disk. Changing a
//! rule re-derives a whole library out of rows that are already loaded,
//! which is what makes a Strategy something a person can iterate on
//! rather than a decision to get right the first time. A `cr2-` costs
//! somebody's disk; a `sk1-` selection change costs a scan.
//!
//! # A key on the material, and not a Group
//!
//! The obvious shape for "these belong together" in this codebase is a
//! Group, and it is the wrong one — not because the grouping is wrong
//! but because of where a Group keeps its membership. `asset_bucket` is
//! the table a person's own curation lives in: it carries the
//! hand-placed order, and a card's *primary* group is whichever of its
//! `group_ids` sorts first, which the repository fixes as the lowest
//! `bucket_id` so that the answer is at least stable
//! (`fetch_group_ids_map`). Let a system rule mint Groups and a card's
//! primary group changes the moment one of those minted ids happens to
//! sort low: the Group axis quietly stops showing the arrangement
//! somebody made, and no screen can explain it, because the card
//! belongs to both legitimately. `list_duplicate_groups` runs into none
//! of this by writing no `asset_bucket` row at all. **The harm is the
//! write into the curation table, not the splitting.**
//!
//! Structure says the same thing four more times. A Group's rule is a
//! column on the group row (`bucket.query_json`), so one rule is one
//! group by construction; `UNIQUE (persona_id, name)` means N groups
//! need N names somebody has to invent; the refresh stamps are per
//! group, with nowhere to record a per-rule result; and every service
//! signature takes a single `GroupId`. A Session is worse rather than
//! better — membership there is one `container_id` on the member, so an
//! asset sits in exactly one, while "made the same way", "shot in one
//! burst" and "made on one day" overlap by nature.
//!
//! So the key stays on the material and a group is computed when
//! somebody asks for one. When a series turns out to deserve a name, a
//! hand-placed order or a dispatch target, **a person promotes that one
//! into a real Group** through the path that already exists — which is
//! also the only thing `DispatchService::run` will accept, since it
//! wants one id and a frozen membership.
//!
//! # Include is sharp and goes stale, exclude is blunt and safe
//!
//! [`probe`](crate::domain::probe)'s denylist obligation reappears here
//! one layer up, and with the asymmetry pointing the other way, so both
//! rules are offered rather than one.
//!
//! | | a field nobody named arrives | result |
//! |---|---|---|
//! | [`include`](Strategy::include) | it is not selected | fewer distinctions — separate things share a key |
//! | [`exclude`](Strategy::exclude) | it is not dropped | more distinctions — one run splits |
//!
//! Include's error is the unrecoverable one, and it is the rule VDSL
//! needs; exclude's error is merely a lost improvement, and it is the
//! only rule available where the vocabulary is open (EXIF vendor
//! MakerNotes). A Strategy states which of the two it is making, per
//! field, and the author owns that choice — which is the reason a
//! Strategy is data rather than code.
//!
//! **So the instruction the table implies runs against the grain: a
//! field an author is unsure about belongs *in* an include list.** Left
//! out, it cannot separate anything, and two materials it would have
//! told apart land on one key that reads exactly like a correct
//! grouping. Named, the worst it does is split a run — which is visible
//! in the result and repaired by editing the rule, at the cost of a scan
//! (see this module's opening on why re-deriving is cheap). Writing it
//! down because the wrong reading is the one that sounds careful: the
//! design memo this argument was drafted in stated the sentence inverted
//! in its first draft, and it reached three other files — including the
//! MCP schema resource an agent reads before writing a rule — before
//! anybody looked at the table beside it.
//!
//! # `decode` absorbs the container's shape, and never drops
//!
//! A path can only walk a structure, and containers hand over their
//! metadata as text: raw JSON inside a `tEXt` chunk, base64 of JSON in a
//! character card, a typed EXIF field written as `type:rendering`.
//! [`Decode`] is a small closed set for the reason the design gives: the
//! author of a Strategy is on the far side of a process boundary, so a
//! rule they can register has to be chosen from tools that already
//! shipped, not spelled in a language.
//!
//! **A value the decoder cannot read stays the string the container
//! stated.** The alternative — treat it as absent — quietly removes a
//! distinction, and removing distinctions is how unrelated materials end
//! up under one key. Keeping it costs nothing: a path cannot descend
//! into a string, so a deep path finds nothing there and says so, while
//! a whole-map selection still carries the text.
//!
//! Keeping the text is not on its own enough to keep the two apart,
//! which is what [`Selected`] is for: `hello` and `"hello"` are
//! different text in the container, and both reach the rendering as one
//! [`Value::String`]. So each selected sub-tree is rendered with the
//! kind of thing it is, and the two land on different keys.
//!
//! # What is not decided here
//!
//! Where derived keys are stored, when they are recomputed, how a
//! Strategy is registered over HTTP, and whether a format's meta axis is
//! claimed at all — a Strategy over a format whose probe declares
//! `meta: false` reads an empty `meta_kv` and answers
//! [`NotApplicable`](SeriesKey::NotApplicable) for every row, correctly
//! and uselessly, until that probe claims the axis.
//!
//! One of those was owed and named here so S2 would not have to find
//! it: `content_hash` gives each versioned column a **reserved value**
//! for the digest of its own empty rendering
//! ([`CONTENT_REGION_EMPTY`](crate::domain::content_hash::CONTENT_REGION_EMPTY),
//! [`META_EMPTY`](crate::domain::content_hash::META_EMPTY)), listed so
//! that a value which reached the column by some other route is not read
//! as sameness. `material_series.key` is that column here (V73), so the
//! constant is [`SERIES_KEY_EMPTY`].
//!
//! **Half of that debt is paid.** The sibling constants are consulted on
//! the *matching* side — the thing that decides whether two rows group —
//! and this one is so far consulted only on the writing side, where
//! `SqliteSeriesRepository::record` refuses it. That covers the writer
//! and by construction cannot cover the case the argument is actually
//! about: a row that arrived some other way, which no write path ever
//! sees. The reader that closes it is S3's, and the shape of the column
//! says where it will go wrong — a hand-edited row carrying this value
//! sits *inside* `idx_material_series_strategy_key` and satisfies the
//! natural grouping statement
//! (`WHERE strategy_id = ? AND key IS NOT NULL GROUP BY key`), which is
//! precisely one group holding every material whose rule selected
//! nothing. **That query has to name [`SERIES_RESERVED_VALUES`]**, the
//! way the duplicate report's adapter names
//! [`reserved_values`](crate::domain::content_hash::reserved_values).

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::value::{MimeType, StrategyId};
use crate::error::DomainError;

/// Algorithm tag of a derived series key — `sk1-sha256:<64 lowercase
/// hex>`.
///
/// Named and versioned for the reason
/// [`META_DIGEST_PREFIX`](crate::domain::content_hash::META_DIGEST_PREFIX)
/// is: the rendering *is* the definition. What is hashed is the
/// selection — the surviving `(path, value)` pairs, ordered by path,
/// serialised compactly — and any change to that shape (array indexing
/// that resolves where nothing resolved before, a different way of
/// carrying the path) makes new keys incomparable with stored ones. A
/// new tag lets both generations sit in one column; editing the rule in
/// place would leave every `sk1-` value meaning something it was not
/// computed to mean.
///
/// It is deliberately **not** one of the three prefixes
/// [`content_hash`](crate::domain::content_hash) declares. Those three
/// are duplicate axes, and
/// [`is_duplicate_key`](crate::domain::content_hash::is_duplicate_key)
/// reads a value carrying one as a claim that two rows are *the same
/// thing* — a claim a fold acts on. A series key says only "made the
/// same way", which is exactly the claim that must not fold anything.
/// Sharing the grammar would invite the mistake; a separate tag makes it
/// unspellable.
pub const SERIES_KEY_PREFIX: &str = "sk1-sha256:";

/// The `sk1-` key over an empty selection — the reserved value of this
/// axis, and the one this module's doc owed S2.
///
/// [`derive`] does not produce it: [`SeriesKey::NothingToSelect`]
/// answers before an empty selection ever reaches the digest, and that
/// ordering exists precisely so every material a rule missed does not
/// land on one key. **Declining to write a value is not the same as
/// reserving it**, which is the argument
/// [`content_hash`](crate::domain::content_hash) makes for its own two
/// ([`CONTENT_REGION_EMPTY`](crate::domain::content_hash::CONTENT_REGION_EMPTY),
/// [`META_EMPTY`](crate::domain::content_hash::META_EMPTY)) and the
/// reason this constant is not left implied by [`derive`]'s care: the
/// refusal is a property of today's writer, and the exclusion has to be
/// a property of every reader. A row that reached `material_series.key`
/// by some other route — edited by hand, written by a later pass that
/// forgets, restored from a database some other build wrote — carries a
/// well-formed `sk1-` value that nothing downstream could tell from a
/// derived one.
///
/// What it costs to miss is one group holding every material whose rule
/// selected nothing, across formats and generators, with nothing in
/// common but a Strategy's silence — the same shape the two sibling
/// constants guard against, one axis up and without the fold: a series
/// key groups, it never claims two rows are the same thing
/// (see [`SERIES_KEY_PREFIX`] for why that claim is unspellable here).
///
/// One caveat on "[`derive`] does not produce it", which is true of
/// every path anybody takes and not quite true of every path there is:
/// [`render`]'s serialisation is infallible in practice but falls back
/// to `[]` if it ever were not, and `[]` is what this constant is the
/// digest of. So that one branch would return
/// `Derived(SERIES_KEY_EMPTY)`. It is unreachable — a [`Value`] parsed
/// from JSON holds no non-finite number, and
/// [`SeriesKey::NothingToSelect`] answers before an empty selection
/// reaches the rendering anyway — and the constant is what makes it
/// harmless if it ever were reached, since the writer refuses the value.
pub const SERIES_KEY_EMPTY: &str =
    "sk1-sha256:4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945";

/// The values carrying [`SERIES_KEY_PREFIX`] that still do not stand
/// for "made the same way".
///
/// A list rather than a loose constant for the reason
/// [`RESERVED_VALUES`](crate::domain::content_hash::RESERVED_VALUES) is
/// one: the exclusion has to be reproduced in whatever language reads
/// the column — SQL, once a grouping query exists — and the only safe
/// way to ask "which values must be named one by one" is to walk the
/// list this module keeps, so a later entry reaches those readers
/// without an edit on their side.
///
/// Deliberately **not** folded into
/// [`reserved_values`](crate::domain::content_hash::reserved_values).
/// That function is keyed by
/// [`DuplicateAxis`](crate::domain::duplicate_conflict::DuplicateAxis),
/// which is a stored value (`duplicate_conflict.axis`) and an edge
/// label, and every axis in it is one a fold acts on. Adding a fourth
/// arm would put "made the same way" in the same enum as "the same
/// thing" — the confusion [`SERIES_KEY_PREFIX`] spends a paragraph
/// making unspellable.
pub const SERIES_RESERVED_VALUES: &[&str] = &[SERIES_KEY_EMPTY];

/// Whether a stored value may stand for "made the same way" — the rule
/// a reader of `material_series.key` runs, stated once.
///
/// Two ways to fail it, the same two [`is_duplicate_key`](crate::domain::content_hash::is_duplicate_key)
/// names: the value may carry no `sk1-` tag at all (a marker, a later
/// generation, a digest off one of the duplicate axes), or it may be a
/// real key that means nothing as a grouping, which is
/// [`SERIES_KEY_EMPTY`].
pub fn is_series_key(value: &str) -> bool {
    value.starts_with(SERIES_KEY_PREFIX) && !SERIES_RESERVED_VALUES.contains(&value)
}

/// One rule for reading "made the same way" out of a material's
/// metadata.
///
/// Data, not code, and the reason is a process boundary: an importer
/// runs in its own process and talks to the server over HTTP
/// (`importer-sdk`'s runner), so whatever it — or the agent driving
/// it — wants to say about how a generator writes its metadata has to
/// cross the wire as a value. It also keeps the authority undivided:
/// the rule comes from outside, the bytes are still read by the server,
/// and no field ends up with two claimants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Strategy {
    /// What derived rows are filed under. See [`StrategyId`].
    pub id: StrategyId,
    /// What a person calls this rule. Carried for display and never
    /// read by [`derive`] — a rename must not move a single key.
    pub name: String,
    /// The one format this rule is written against, compared as parsed
    /// so it is written once and matches every spelling the boundary
    /// normalises (`IMAGE/PNG; charset=binary`, ` image/png `).
    ///
    /// One mime rather than a list, and no wildcard: a rule reads a
    /// specific generator's habits inside a specific container, and a
    /// rule that claimed `image/*` would be asserting that PNG `tEXt`
    /// keywords and EXIF tag numbers are one namespace.
    pub applies_to: MimeType,
    /// How the text in `meta_kv` becomes something a [`Path`] can walk.
    pub decode: Decode,
    /// The sub-trees to keep. Empty means the whole of `meta_kv` —
    /// which is a real choice and not a disabled feature: it is what
    /// makes an exclude-only Strategy expressible, and that is the only
    /// rule available for a format whose field vocabulary is open.
    pub include: Vec<Path>,
    /// The sub-trees to drop, applied **after** [`include`](Self::include)
    /// and rooted the same way — the first segment is a key of
    /// `meta_kv`, not a key inside whatever include selected. One
    /// vocabulary for both lists, so a path means the same thing in
    /// either.
    pub exclude: Vec<Path>,
}

impl Strategy {
    /// Whether this rule answers for a declared format at all.
    ///
    /// `None` — a material whose mime was never resolved — matches
    /// nothing, on the same terms as
    /// [`probe`](crate::domain::probe)'s claim lookup: a rule is
    /// selected by what the row says it holds, and guessing from the
    /// metadata that is about to be read would let one material answer
    /// differently depending on how it arrived.
    pub fn claims(&self, declared_mime: Option<&MimeType>) -> bool {
        declared_mime.is_some_and(|mime| self.applies_to == *mime)
    }
}

/// How the text a container carried becomes a structure a [`Path`] can
/// walk.
///
/// A closed set, and small on purpose — see the module doc. The honest
/// limit of that decision is that a format nothing here reads (A1111's
/// `steps: 30, sampler: euler` prose, EXIF's typed values) needs a
/// shipped variant before any Strategy can address inside it. What a
/// Strategy author gets is not "any parser expressible as data" but
/// "the shipped tools, freely combined".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decode {
    /// The value is the whole of what is addressed: a one-segment path
    /// selects it, a longer path finds nothing.
    ///
    /// The right choice for a container whose values are prose, and the
    /// safe default for one whose values are unknown — it never
    /// misreads, it only declines to go deeper.
    None,
    /// The value is a JSON document. ComfyUI's `workflow` / `prompt`
    /// chunks and VDSL's `vdsl` chunk are this.
    RawJson,
    /// The value is base64 of a JSON document — the character card
    /// convention (`ccv3`).
    Base64Json,
    /// The value is one EXIF field, written as `type:rendering` — what
    /// `asterism-media-probe`'s tag read puts in `meta_kv` for a JPEG
    /// (`rational:1/125`, `ascii:ACME`, `undefined:<base64>`).
    ///
    /// # What it makes addressable, and why it is not a no-op
    ///
    /// The keys of that map are already flat — `exif:0x829a` is a whole
    /// address — so a rule could reach every field with
    /// [`None`](Self::None) and a one-segment path. What this adds is
    /// the **type**, as a second segment: `["exif:0x829a","rational"]`
    /// selects `1/125` from a file where that tag is a rational, and
    /// selects *nothing* from one where it is an ASCII string whose text
    /// happens to be `1/125`. Under `None` those two files are one
    /// string and one key.
    ///
    /// So the shape is a one-key object, `{"rational":"1/125"}`, and
    /// **the marker is not stripped** — it is moved from the front of a
    /// string to a position a path can name. Stripping it would be the
    /// tempting reading and it is the wrong one: the type is what stops
    /// two different fields rendering alike, and a decoder that threw it
    /// away would put them under one key, which is the unrecoverable
    /// direction this module is written around.
    ///
    /// # It goes no deeper, and could not
    ///
    /// Two segments is the whole depth. What is behind the type is text
    /// the container stated, and for the two byte-shaped types
    /// (`undefined`, which is where a maker note lives, and the floats)
    /// it is base64 — a path cannot walk into a base64 string, and
    /// decoding it here would mean this decoder deciding what a vendor's
    /// private block means. That reading belongs to whoever writes it,
    /// working from `material.meta_raw`.
    ///
    /// # An unrecognised prefix is not policed
    ///
    /// The rule is syntactic: the text before the first colon is the
    /// type, whatever it spells. This module holds no list of EXIF types
    /// to check it against, and adding one would put the vocabulary in
    /// two places — it is the file's, transcribed by the probe that read
    /// it. A value with no colon at all did not come from that probe and
    /// stays the string the container stated, like any other value a
    /// decoder cannot read. The failure that leaves is a value filed
    /// under a type name that does not exist, which separates rather
    /// than merges: the safe direction.
    ///
    /// # Which tags to name, and which part of that is somebody's
    /// judgement
    ///
    /// The tags are classified publicly. **Exif 3.0 Annex H** (CIPA
    /// DC-008, guidelines for handling tag information in
    /// post-processing) ranks every standard tag by what a tool may do
    /// to it — `Update 0` (rewritten on every edit), `Update 1` (may be
    /// updated alone), `Freeze 0` (shall not be deleted or modified
    /// under any circumstance), `Freeze 1` (needs no update), `Freeze 2`
    /// (may be corrected where wrong) — and binds two categories to a
    /// rank: every image-structure tag is `Update 0`, and the shooting
    /// settings are `Freeze 1`
    /// (<https://www.cipa.jp/e/std/std-sec.html>;
    /// <https://archive.org/details/exif-specs-3.0-dc-008-translation-2023-e>,
    /// Annex H pp. 233–241).
    ///
    /// **Excluding the `Update 0` tags is a quotation.** `DateTime`
    /// (`ifd0:0x0132`), `SubSecTime`, the image-structure tags: the
    /// specification states an editor rewrites them, so a rule keyed on
    /// them is keyed on whatever last exported the file.
    ///
    /// **Treating `Freeze 1` / `Freeze 2` as steady across a run is a
    /// judgement, and it is this project's rather than the
    /// specification's.** Annex H's axis is *may a tool rewrite this*,
    /// not *does this vary per exposure*, and the two part company in
    /// one place that matters here: exposure time, aperture, ISO and
    /// focal length are `Freeze 1`, and under auto-exposure they change
    /// from frame to frame. Whether to name them is the author's call —
    /// which is the whole reason a Strategy is a registered value and
    /// not a definition compiled in. Nothing published classifies tags
    /// by burst-stability at all; the JPEG probe's module doc carries
    /// that survey (MWG, IPTC PMD, `xmpMM`, C2PA) and the citations.
    ///
    /// **`ImageUniqueID` (`exif:0xa420`) is not a key on its own**,
    /// although the specification is at its most emphatic about it —
    /// `Freeze 0`, the only tag with that rank, *shall not be deleted or
    /// modified under any circumstance*. Two implementations
    /// independently record that it is not written reliably — one
    /// accepts it only where the value is UUID-shaped, having found
    /// cameras writing their model name there; the other declined it as
    /// missing, reused and inconsistent across vendors. A rule that
    /// names it should name something beside it.
    Exif,
}

impl Decode {
    /// Every decoder this build ships, in the order the schema's
    /// `CHECK` lists them.
    ///
    /// This is the list the schema's vocabulary guard compares against,
    /// so what matters about it is not that it is right today but that
    /// it **cannot go stale when a variant is added** — the direction
    /// that ends with `create_strategy` writing a token the column
    /// refuses, on somebody's library, the first time they register a
    /// rule using it. That was not hypothetical: [`Decode::Exif`] was
    /// named as coming — by this module's doc and by the design memo
    /// whose argument now lives in it — and then came, and the guard is
    /// what made the `CHECK` widening part
    /// of the same edit rather than something discovered afterwards.
    ///
    /// # Why it is a literal and not built from a `match`
    ///
    /// A `match` forces an *arm* per variant, which is not the same as
    /// forcing a *list entry*, and every shape that tries to close the
    /// gap leaks in the same place. A successor chain
    /// (`Base64Json => Some(Exif)`) compiles once the new variant has an
    /// arm of its own — `Exif => None` satisfies the compiler and leaves
    /// the walk terminating one step earlier, so the new variant is
    /// simply never visited. A tail-slice arm (`Exif => &[Exif]`) is the
    /// same leak: the arms above it keep the list they had. Making the
    /// walk a cycle moves the leak without closing it. What would close
    /// it is `std::mem::variant_count` (unstable) or a derive macro (a
    /// dependency), and neither is available here.
    ///
    /// So completeness is proved from the enum's own source text
    /// instead — `the_decoder_list_names_every_variant_this_enum_has`
    /// counts the variants declared above and requires this list to be
    /// as long, which together with the entries being distinct means it
    /// holds all of them. That is the trade
    /// `migrations::tests::every_step_is_named_for_the_version_it_produces`
    /// already makes in this codebase for the same reason: names rather
    /// than meanings, no build dependency, and a guard that fails on the
    /// *addition* rather than only on the edit.
    pub const ALL: &'static [Self] = &[Self::None, Self::RawJson, Self::Base64Json, Self::Exif];

    /// The token this variant is stored and registered as.
    ///
    /// One spelling for both directions the value travels: it is what
    /// `series_strategy.decode` holds, what the schema's `CHECK` admits,
    /// and what an author on the far side of the process boundary picks
    /// from. A second vocabulary anywhere along that path would be a
    /// rule registered as one decoder and applied as another.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RawJson => "raw_json",
            Self::Base64Json => "base64_json",
            Self::Exif => "exif",
        }
    }

    /// Reads a stored token back.
    ///
    /// An unknown token is refused rather than resolved to
    /// [`None`](Self::None). The two look alike — neither one reads
    /// inside the value — and they are not: `None` is an author saying
    /// *the value is the whole of what I am addressing*, while an
    /// unrecognised token is a rule this build cannot carry out, most
    /// plausibly one registered against a later build's decoder set.
    /// Running it as `None` would derive keys from a rule nobody wrote
    /// and file them in the same column as the real ones.
    pub fn parse(token: &str) -> Result<Self, DomainError> {
        match token {
            "none" => Ok(Self::None),
            "raw_json" => Ok(Self::RawJson),
            "base64_json" => Ok(Self::Base64Json),
            "exif" => Ok(Self::Exif),
            other => Err(DomainError::Validation(format!(
                "no decoder shipped here is spelled {other:?}"
            ))),
        }
    }
}

/// Where in the metadata a rule is pointing.
///
/// The first segment is a key of `meta_kv` — the keyword the container
/// itself carried — and the rest walk into whatever [`Decode`] made of
/// that keyword's value. So `["vdsl","script"]` is "the `script` field
/// of the decoded `vdsl` chunk", and `["prompt"]` is "the `prompt`
/// chunk, whole".
///
/// # Segments name object keys, and only object keys
///
/// There is no array indexing: `["a","0"]` addresses an object key
/// spelled `0`, never the first element of an array, and a path that
/// lands on an array selects it whole rather than descending. Adding
/// indexing later is safe in the direction that matters — it can move a
/// material from [`NothingToSelect`](SeriesKey::NothingToSelect) to
/// [`Derived`](SeriesKey::Derived), because the path resolved where it
/// used to resolve nowhere, but it cannot change a key that was already
/// derived. It would still be a new [`SERIES_KEY_PREFIX`] if it altered
/// the rendering.
///
/// An empty path names nothing: it selects nothing in
/// [`include`](Strategy::include) and drops nothing in
/// [`exclude`](Strategy::exclude). The second half is the one that
/// matters — the empty sequence is a prefix of every path, so an empty
/// exclude read literally would delete the entire selection, which is
/// not what an author who left a field blank meant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Path(Vec<String>);

impl Path {
    /// Builds a path from its segments, outermost first.
    pub fn new<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(segments.into_iter().map(Into::into).collect())
    }

    /// The segments, outermost first.
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// The `meta_kv` keyword this path starts at, if it starts anywhere.
    pub fn head(&self) -> Option<&str> {
        self.0.first().map(String::as_str)
    }
}

/// What applying a [`Strategy`] to a material concluded.
///
/// Three states rather than `Option<String>`, on the same terms as
/// [`MaterialMeta`](crate::domain::material_meta::MaterialMeta): the two
/// ways of having no key lead somewhere different, and a caller that
/// collapsed them could not tell "this rule is not about this material"
/// from "this rule is about it and found nothing" — the second is a
/// Strategy that needs fixing, the first is a Strategy working as
/// written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeriesKey {
    /// A key: `sk1-sha256:<64 lowercase hex>`.
    ///
    /// It carries no body, and that is the one place this type departs
    /// from [`MaterialMeta::Digest`](crate::domain::material_meta::MaterialMeta::Digest),
    /// which travels with the canonical rendering it hashed. That pair
    /// is inseparable because the rendering is a reading of bytes the
    /// caller no longer holds — throw it away and it costs a disk pass
    /// to recover. A selection is re-derivable from `meta_kv` and the
    /// Strategy, both of which are rows; storing it beside the key
    /// would be a second copy of something never out of reach, and a
    /// second copy is a thing that can disagree.
    Derived(String),
    /// The rule applies and selected nothing: the keyword is there, and
    /// no include path resolved inside it (or exclude removed
    /// everything that did).
    ///
    /// Not a key over an empty selection. That would be a perfectly
    /// real digest, and every material a Strategy misses would share
    /// it — one group whose members have nothing in common but the
    /// rule's silence, which is the failure
    /// [`EmptySpan`](crate::domain::material_meta::MaterialMeta::EmptySpan)
    /// is reserved against on the axis below.
    NothingToSelect,
    /// The rule is not about this material: a different format, or a
    /// material carrying none of the keywords the rule names.
    ///
    /// Decided from the map's keys before anything is decoded, which is
    /// what keeps it distinct from
    /// [`NothingToSelect`](Self::NothingToSelect) — that one is the
    /// answer of a walk that ran.
    NotApplicable,
}

impl SeriesKey {
    /// The key, when there is one — for callers that must not group on
    /// an outcome that isn't a key.
    pub fn key(&self) -> Option<&str> {
        match self {
            Self::Derived(key) => Some(key),
            _ => None,
        }
    }

    /// Every token `material_series.outcome` can hold, in the order the
    /// schema's `CHECK` lists them.
    ///
    /// Tokens rather than values, because [`Derived`](Self::Derived)
    /// carries a `String` and so cannot sit in a `const`. Kept complete
    /// the same way [`Decode::ALL`] is — see that constant for why a
    /// `match` cannot do it and what proves this one instead.
    pub const OUTCOMES: &'static [&'static str] =
        &["derived", "nothing_to_select", "not_applicable"];

    /// The token `material_series.outcome` holds for this answer.
    ///
    /// A `match` rather than a derived name, so a fourth outcome fails
    /// to compile here instead of being stored as whichever token an
    /// `if` chain reached last — the arrangement
    /// [`MaterialAnchor::kind_slug`](crate::domain::material_mark::MaterialAnchor::kind_slug)
    /// uses on its own column.
    ///
    /// The column exists because `key IS NULL` cannot say which of the
    /// two silences a row is: a rule that is not about this material and
    /// a rule that is and found nothing lead somewhere different (see
    /// this type's doc), and the pair is held together by
    /// `CHECK ((outcome = 'derived') = (key IS NOT NULL))` in V73 rather
    /// than by the two writers happening to agree.
    pub fn outcome_slug(&self) -> &'static str {
        match self {
            Self::Derived(_) => "derived",
            Self::NothingToSelect => "nothing_to_select",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// Applies a [`Strategy`] to one material's metadata.
///
/// Pure, and total: every input produces one of the three outcomes.
/// `declared_mime` is what the material says it is, and it is an
/// argument rather than the caller's own gate so that there is one door
/// into this function — the same arrangement
/// [`ProbeGates`](crate::domain::probe::ProbeGates) enforces with a
/// token, for the same reason. A gate a caller is merely asked to
/// consult is one a caller can skip, and the skip is silent: a key
/// derived for a format the rule never claimed is a well-formed key,
/// indistinguishable in the column from one that was claimed.
///
/// The order is [`decode`](Strategy::decode) → `include` → `exclude` →
/// digest, and `exclude` runs second because the two rules are answering
/// different questions: include says which part of the container this
/// rule is about, exclude says which of *that* is noise. Reversing them
/// would make an exclude that names something include never selected
/// look meaningful.
pub fn derive(
    strategy: &Strategy,
    declared_mime: Option<&MimeType>,
    meta_kv: &BTreeMap<String, String>,
) -> SeriesKey {
    if !strategy.claims(declared_mime) {
        return SeriesKey::NotApplicable;
    }
    if !names_a_present_keyword(strategy, meta_kv) {
        return SeriesKey::NotApplicable;
    }

    let mut selection = select(strategy, meta_kv);
    remove_excluded(strategy, &mut selection);

    if selection.is_empty() {
        return SeriesKey::NothingToSelect;
    }
    SeriesKey::Derived(digest_of(&render(&selection)))
}

/// Whether the material carries any keyword this rule points at.
///
/// The whole-map case asks whether there is a keyword at all, because an
/// empty `include` points at all of them — so a material with no
/// metadata is one the rule has nothing to say about, rather than one it
/// looked into and found empty.
fn names_a_present_keyword(strategy: &Strategy, meta_kv: &BTreeMap<String, String>) -> bool {
    if strategy.include.is_empty() {
        return !meta_kv.is_empty();
    }
    strategy.include.iter().any(|path| {
        path.head()
            .is_some_and(|keyword| meta_kv.contains_key(keyword))
    })
}

/// The sub-trees a rule keeps, filed under the path that reached each.
///
/// A [`BTreeMap`] keyed by the path so that the ordering the digest
/// depends on is a fact about the type rather than about a caller
/// remembering to sort — the argument
/// [`material_meta::render`](crate::domain::material_meta::render) makes
/// about its own map. It also means two identical include paths select
/// one entry rather than two.
///
/// Overlapping paths (`["vdsl"]` and `["vdsl","script"]`) are kept as
/// they were written: two entries, one nested inside the other's value.
/// The rendering says which path each value arrived by, so the result is
/// still an unambiguous statement about what was selected.
/// A keyword is decoded **once**, however many paths name it. Three
/// paths into one character card is three paths into one parse, not
/// three base64 decodes and three parses of a payload measured at 40 KB
/// (the character card this workspace ships as a fixture, weighed where
/// the PNG probe picks its metadata ceiling). Re-deriving a library is
/// sold as a scan and a parse (this module's opening), and the parse is
/// the whole of that budget — doing it per path multiplies exactly the
/// term the design is spending.
fn select(
    strategy: &Strategy,
    meta_kv: &BTreeMap<String, String>,
) -> BTreeMap<Vec<String>, Selected> {
    let mut selection = BTreeMap::new();

    if strategy.include.is_empty() {
        for (keyword, raw) in meta_kv {
            selection.insert(vec![keyword.clone()], decoded(strategy.decode, raw));
        }
        return selection;
    }

    let mut readings: BTreeMap<&str, Selected> = BTreeMap::new();
    for path in &strategy.include {
        let Some((keyword, rest)) = path.segments().split_first() else {
            continue;
        };
        let Some(raw) = meta_kv.get(keyword.as_str()) else {
            continue;
        };
        let reading = readings
            .entry(keyword.as_str())
            .or_insert_with(|| decoded(strategy.decode, raw));
        if let Some(value) = walk(&reading.value, rest) {
            // The kind travels with the keyword, not with the depth: a
            // field two levels inside a decoded document is part of a
            // reading, and the only path that can select out of an
            // undecoded value is the one-segment one.
            selection.insert(
                path.segments().to_vec(),
                Selected {
                    kind: reading.kind,
                    value: value.clone(),
                },
            );
        }
    }
    selection
}

/// Drops every excluded sub-tree from an already-narrowed selection.
///
/// An exclude path is rooted at `meta_kv` like an include path, so it
/// meets the selection in one of three ways: it covers a whole entry
/// (the entry goes), it points inside one (that sub-tree goes and the
/// entry stays), or it is unrelated (nothing happens). An entry emptied
/// this way stays as an empty object rather than disappearing — that the
/// keyword was there is itself a distinction, and dropping it would
/// merge a material that had the field with one that never did.
fn remove_excluded(strategy: &Strategy, selection: &mut BTreeMap<Vec<String>, Selected>) {
    for excluded in &strategy.exclude {
        let excluded = excluded.segments();
        if excluded.is_empty() {
            continue;
        }
        selection.retain(|path, selected| {
            if excluded.len() <= path.len() {
                return !path.starts_with(excluded);
            }
            if excluded.starts_with(path.as_slice()) {
                drop_at(&mut selected.value, &excluded[path.len()..]);
            }
            true
        });
    }
}

/// One selected sub-tree, and which of the two things it is.
///
/// The pair is inseparable because the rendering needs both, and it
/// needs both for a reason worth stating: `hello` and `"hello"` are
/// different text in the container and would otherwise become one
/// [`Value::String`] and one key. That is the unrecoverable direction —
/// two materials the container distinguishes, filed as made the same
/// way. The kind keeps them apart without inventing a marker inside the
/// document, which a real document could carry and which would then be
/// indistinguishable from ours.
#[derive(Debug, Clone, PartialEq)]
struct Selected {
    /// [`READING`] or [`VERBATIM`].
    kind: &'static str,
    /// The sub-tree, or the container's text when `kind` is
    /// [`VERBATIM`].
    value: Value,
}

/// [`Selected::kind`] for a value a decoder made a structure of.
const READING: &str = "json";

/// [`Selected::kind`] for the text the container stated, either because
/// [`Decode::None`] asked for nothing or because the decoder refused.
const VERBATIM: &str = "text";

/// The structure a path walks, or the text itself when nothing here can
/// read it.
///
/// Failure is not an error state: an undecodable value is a value, and
/// keeping it as the string the container stated preserves every
/// distinction it carried. See the module doc for why the other choice —
/// treating it as absent — is the one that loses data, and [`Selected`]
/// for why the two cases have to stay distinguishable once kept.
///
/// What counts as base64 is [`base64`]'s standard engine, which is to
/// say RFC 4648 with canonical padding and no allowance for a URL-safe
/// alphabet. A payload outside that is not read as something
/// approximate — it falls to the same branch every other undecodable
/// value falls to, and a Strategy pointing inside it selects nothing.
/// The strict reading is the safe one here: the cost of refusing is a
/// key that is never derived, and the cost of guessing is a key derived
/// over the wrong bytes.
///
/// Surrounding whitespace is the one exception, and it is not a
/// judgement call — it is the same trim
/// `asterism_importer_sdk::card::png_chunk::envelope_from_chunk` does,
/// for the reason recorded there: *some editors round-trip a trailing
/// newline into the chunk*. That newline reaches `meta_kv` verbatim
/// (nothing on the walk path trims), so without this a card **this
/// workspace's own reader accepts** would answer
/// [`SeriesKey::NothingToSelect`] here — which reads, per that type, as
/// a Strategy that needs fixing when the Strategy is fine.
///
/// [`Decode::Exif`]'s arm is the one that does not parse anything: an
/// EXIF value is already text with its type written in front of it, so
/// what it produces is that pair as a one-key object. See the variant
/// for why the type is kept rather than stripped, and why the split is
/// syntactic rather than checked against a vocabulary.
fn decoded(decode: Decode, raw: &str) -> Selected {
    let parsed = match decode {
        Decode::None => None,
        Decode::RawJson => serde_json::from_str(raw).ok(),
        Decode::Base64Json => BASE64
            .decode(raw.trim())
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|json| serde_json::from_str(&json).ok()),
        Decode::Exif => raw.split_once(':').map(|(kind, rendering)| {
            let mut field = serde_json::Map::new();
            field.insert(kind.to_string(), Value::String(rendering.to_string()));
            Value::Object(field)
        }),
    };
    match parsed {
        Some(value) => Selected {
            kind: READING,
            value,
        },
        None => Selected {
            kind: VERBATIM,
            value: Value::String(raw.to_string()),
        },
    }
}

/// Follows a path's remaining segments into a decoded value.
///
/// `None` where the path leaves the structure — a segment no object
/// holds, or any segment at all once the cursor is on a scalar or an
/// array. A path that does not resolve selects nothing, which is the
/// difference between a Strategy that misses and one that guesses.
fn walk<'a>(value: &'a Value, segments: &[String]) -> Option<&'a Value> {
    let mut cursor = value;
    for segment in segments {
        let Value::Object(fields) = cursor else {
            return None;
        };
        cursor = fields.get(segment.as_str())?;
    }
    Some(cursor)
}

/// Removes one sub-tree from a selected value, addressed relative to it.
///
/// Silent where the path does not resolve, because an exclude that names
/// a field this material never had has nothing to do and is not a
/// mistake — the whole point of an exclude list is that it is written
/// against fields that may or may not turn up.
fn drop_at(value: &mut Value, segments: &[String]) {
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut cursor = value;
    for segment in parents {
        let Value::Object(fields) = cursor else {
            return;
        };
        let Some(next) = fields.get_mut(segment.as_str()) else {
            return;
        };
        cursor = next;
    }
    if let Value::Object(fields) = cursor {
        fields.remove(last.as_str());
    }
}

/// Renders a selection into the form the key is taken over — **the only
/// place that form is produced.**
///
/// A JSON array of `[path, kind, value]` triples, ordered by path, with
/// every nested object's keys sorted and no whitespace. Three properties
/// the digest depends on, and none of them a caller's responsibility:
/// the triple order is the [`BTreeMap`]'s, the nested key order is
/// [`canonical_value`]'s, and the compactness is `serde_json`'s default.
/// **Nothing here hand-writes JSON** — the rule
/// [`material_meta`](crate::domain::material_meta) states, and it bites
/// harder here, since the values being rendered are arbitrary documents
/// out of a container rather than strings.
///
/// The path travels beside the value rather than being flattened into a
/// string key: any separator would be a character a container is allowed
/// to use in a keyword, and the day one does, two different selections
/// render identically and two unrelated materials share a key.
///
/// The kind travels with them for the same class of reason, one level
/// down — see [`Selected`]. It is a third element rather than a wrapper
/// object around the value, because a wrapper's key would be a name a
/// document could also use, and the pair `(kind, value)` is not
/// something a caller ever writes.
///
/// Not interchangeable with `material_meta`'s canonical form, and not
/// meant to be — that one is a map of the strings a container carried,
/// this one is a list of what a rule picked out of them. They are the
/// two different sentences the module doc is about.
///
/// Infallible in practice: a [`Value`] parsed from JSON holds no
/// non-finite number, so serialisation has nothing to fail on. The empty
/// rendering is returned rather than a panic if it ever did, and it
/// cannot be mistaken for an ordinary result because [`derive`] answers
/// [`SeriesKey::NothingToSelect`] before an empty selection reaches
/// here.
fn render(selection: &BTreeMap<Vec<String>, Selected>) -> String {
    let triples: Vec<(&Vec<String>, &str, Value)> = selection
        .iter()
        .map(|(path, selected)| (path, selected.kind, canonical_value(&selected.value)))
        .collect();
    serde_json::to_string(&triples).unwrap_or_else(|_| "[]".to_string())
}

/// Sorts every object's keys, recursively, leaving arrays alone.
///
/// # Do not delete this. It is not a no-op, and it is not `serde_json`'s
/// job.
///
/// It reads like dead code: `serde_json::Map` is a `BTreeMap` by
/// default, so a value round-tripped through it comes out sorted anyway
/// and this function looks like it is sorting what is already sorted.
/// **That default is off in this workspace.** `serde_json/preserve_order`
/// is declared in the workspace `Cargo.toml` — where the reasoning is —
/// and Cargo unifies features across a build, so it is on for every
/// crate here. With it on, `serde_json::Map` is an `IndexMap` and a
/// parsed object re-serialises in *the order the container's author
/// wrote it*.
///
/// So without this call, [`render`] would hash key order, and:
///
/// - the same workflow re-saved by a tool that reorders keys would
///   produce a different series key, splitting a batch that belongs
///   together — which is a wrong answer to the only question a series
///   key asks;
/// - every stored key would move on the day a dependency turned the
///   feature on or off, with nothing in the diff saying so.
///
/// The deeper reason is not about the feature at all. A JSON object is
/// an unordered collection of name/value pairs (RFC 8259), so a digest
/// over one has to be a function of its content. Sorting here is this
/// module stating its own canonical form instead of borrowing whichever
/// one a dependency's feature flags happen to select — the same
/// independence `material_meta` gets by holding a
/// `BTreeMap<String, String>` and never parsing at all. This module
/// cannot take that route: a rule selects values by path
/// (`["prompt", "3", "inputs"]`), so parsing is a requirement rather
/// than a convenience, and the canonical form has to be re-established
/// afterwards.
///
/// Arrays keep their order, because a JSON array *is* ordered — two
/// documents differing in element order are two different documents,
/// and collapsing them would merge materials that are not the same.
///
/// If the feature is ever dropped, this function still must not be
/// removed: it is what makes the property true independently, and the
/// property is what
/// `the_key_is_the_same_whatever_order_the_container_wrote_its_keys_in`
/// tests. Deleting it would make that test pass for a reason nobody
/// chose.
fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut sorted: Vec<(&String, &Value)> = fields.iter().collect();
            sorted.sort_by_key(|(key, _)| *key);
            // Inserting in sorted order is what produces sorted output
            // under `preserve_order` (an `IndexMap` keeps insertion
            // order); under the default `BTreeMap` the same insertions
            // sort themselves. One expression, correct either way.
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_value(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_value).collect()),
        // Scalars have no order to establish.
        other => other.clone(),
    }
}

/// The key over an already-rendered selection.
///
/// Split from [`render`] on the same terms
/// [`material_meta::digest_of`](crate::domain::material_meta::digest_of)
/// is split from its own rendering: one form is produced, and it is the
/// one that gets hashed.
///
/// It does not call that function, and the duplication is deliberate.
/// The prefix is part of the definition of what a value means, so a
/// shared helper would have to take one as an argument — handing every
/// caller the ability to tag a digest as an axis it did not measure. The
/// hex loop is four lines; a value that reads as a duplicate key when it
/// is not is a fold.
fn digest_of(canonical: &str) -> String {
    let digest = Sha256::digest(canonical.as_bytes());
    let mut key = String::with_capacity(SERIES_KEY_PREFIX.len() + digest.len() * 2);
    key.push_str(SERIES_KEY_PREFIX);
    for byte in digest {
        use std::fmt::Write;
        // Infallible for String.
        let _ = write!(key, "{byte:02x}");
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mime(raw: &str) -> MimeType {
        MimeType::parse(raw)
    }

    fn png() -> Option<MimeType> {
        Some(mime("image/png"))
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    fn strategy(decode: Decode, include: &[&[&str]], exclude: &[&[&str]]) -> Strategy {
        Strategy {
            id: StrategyId::new(),
            name: "under test".to_string(),
            applies_to: mime("image/png"),
            decode,
            include: include
                .iter()
                .map(|p| Path::new(p.iter().copied()))
                .collect(),
            exclude: exclude
                .iter()
                .map(|p| Path::new(p.iter().copied()))
                .collect(),
        }
    }

    // ---- the VDSL corpus -------------------------------------------
    //
    // The shape is the measured one — the eleven images this module's
    // opening states — and each part of it is load bearing:
    //
    // - the `vdsl` chunk has exactly three keys, and v0.4.0 writes no
    //   others;
    // - `timestamp` is written once per *run*, so it is identical
    //   across the images of one run and cannot be what separates
    //   them;
    // - the `prompt` chunk is a compiled graph and differs per image.
    //   **That** is what makes all eleven distinct today, and a fixture
    //   holding it constant would let an implementation that groups by
    //   accident pass every test below.

    const SCRIPT_HIRES: &str =
        "--- phase8_hires.lua: gravure_2605 Phase 8 — hires pass over the phase 7 latents";
    const SCRIPT_PORTRAIT: &str =
        "--- phase9_portrait.lua: gravure_2605 Phase 9 — portrait crop, no hires";

    fn vdsl_chunk(script: &str, timestamp: &str) -> String {
        serde_json::to_string(&json!({
            "script": script,
            "timestamp": timestamp,
            "version": "0.4.0",
        }))
        .expect("the fixture is built by the serialiser, not written by hand")
    }

    fn prompt_chunk(seed: u64) -> String {
        serde_json::to_string(&json!({
            "3": { "class_type": "KSampler", "inputs": { "seed": seed, "steps": 28 } },
            "9": { "class_type": "SaveImage", "inputs": { "filename_prefix": "gravure" } },
        }))
        .expect("the fixture is built by the serialiser, not written by hand")
    }

    fn run(
        script: &str,
        timestamp: &str,
        seeds: std::ops::Range<u64>,
    ) -> Vec<BTreeMap<String, String>> {
        seeds
            .map(|seed| {
                fields(&[
                    ("Software", "VDSL"),
                    ("prompt", &prompt_chunk(seed)),
                    ("vdsl", &vdsl_chunk(script, timestamp)),
                ])
            })
            .collect()
    }

    /// Eleven images out of two runs: five, then six.
    fn corpus() -> Vec<BTreeMap<String, String>> {
        let mut images = run(
            SCRIPT_HIRES,
            "2026-04-26T15:48:29.514778+09:00",
            1_000..1_005,
        );
        images.extend(run(
            SCRIPT_PORTRAIT,
            "2026-04-26T16:12:03.902114+09:00",
            2_000..2_006,
        ));
        assert_eq!(images.len(), 11);
        images
    }

    fn keys_over(strategy: &Strategy, corpus: &[BTreeMap<String, String>]) -> Vec<String> {
        corpus
            .iter()
            .map(|meta_kv| match derive(strategy, png().as_ref(), meta_kv) {
                SeriesKey::Derived(key) => key,
                other => panic!("every image in the corpus carries the chunk: {other:?}"),
            })
            .collect()
    }

    /// The sizes of the groups the keys fall into, ascending — so an
    /// assertion states the count *and* the split, and an
    /// implementation returning one key for everything cannot satisfy
    /// it by agreeing with itself.
    fn group_sizes(keys: &[String]) -> Vec<usize> {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for key in keys {
            *counts.entry(key.as_str()).or_default() += 1;
        }
        let mut sizes: Vec<usize> = counts.into_values().collect();
        sizes.sort_unstable();
        sizes
    }

    /// The measurement this whole module exists for: selecting the
    /// recipe recovers the two runs.
    ///
    /// Both halves are asserted. "Five agree" is satisfied by an
    /// implementation that returns a constant, so the split (`[5, 6]`,
    /// which carries the count) is the claim, and the two keys are then
    /// checked to fall on the run boundary rather than on some other
    /// five-and-six.
    #[test]
    fn vdsl_script_selection_groups_a_run_together() {
        let images = corpus();
        let keys = keys_over(
            &strategy(Decode::RawJson, &[&["vdsl", "script"]], &[]),
            &images,
        );

        assert_eq!(
            group_sizes(&keys),
            vec![5, 6],
            "two groups, and the runs are five and six: {keys:?}"
        );

        let (hires, portrait) = keys.split_at(5);
        assert!(
            hires.iter().all(|key| key == &hires[0]),
            "the hires run is one key: {hires:?}"
        );
        assert!(
            portrait.iter().all(|key| key == &portrait[0]),
            "the portrait run is one key: {portrait:?}"
        );
        assert_ne!(
            hires[0], portrait[0],
            "and the two runs are not the same key"
        );
        assert!(keys.iter().all(|key| key.starts_with(SERIES_KEY_PREFIX)));
    }

    /// The witness for "no exclusion reaches it" — the finding that
    /// turned this axis from a denylist into a selection.
    ///
    /// Three rules, each of which sounds like it should collapse a run,
    /// and all three leave eleven keys: the `prompt` chunk was never
    /// named by any of them, so it stays in, and it differs per image.
    #[test]
    fn excluding_the_timestamp_alone_does_not_group() {
        let images = corpus();
        let eleven = vec![1; 11];

        for (rule, description) in [
            (strategy(Decode::RawJson, &[], &[]), "every chunk digested"),
            (
                strategy(Decode::RawJson, &[], &[&["vdsl", "timestamp"]]),
                "the run's timestamp dropped",
            ),
            (
                strategy(Decode::RawJson, &[], &[&["vdsl"]]),
                "the generator's whole chunk dropped",
            ),
        ] {
            assert_eq!(
                group_sizes(&keys_over(&rule, &images)),
                eleven,
                "{description}: eleven images, eleven keys"
            );
        }
    }

    /// The nested branch of exclude: reaching *into* a selected value
    /// rather than dropping the whole entry.
    ///
    /// Two materials off one run whose metadata differs in exactly the
    /// excluded field, so removing it is the only thing standing
    /// between two keys and one. No corpus row can witness this — every
    /// one of them also carries the per-image `prompt`, which is
    /// precisely why the eleven-image measurement comes out at eleven
    /// however the exclusion behaves, and why a test built on that
    /// corpus goes on passing with this branch disabled.
    #[test]
    fn excluding_a_nested_field_collapses_two_materials_that_differ_only_in_it() {
        let earlier = fields(&[(
            "vdsl",
            &vdsl_chunk(SCRIPT_HIRES, "2026-04-26T15:48:29.514778+09:00"),
        )]);
        let later = fields(&[(
            "vdsl",
            &vdsl_chunk(SCRIPT_HIRES, "2026-04-26T16:02:11.337914+09:00"),
        )]);

        let whole = strategy(Decode::RawJson, &[], &[]);
        assert_ne!(
            derive(&whole, png().as_ref(), &earlier),
            derive(&whole, png().as_ref(), &later),
            "the fixture says nothing unless the two are apart to begin with"
        );

        let without_timestamp = strategy(Decode::RawJson, &[], &[&["vdsl", "timestamp"]]);
        let collapsed = derive(&without_timestamp, png().as_ref(), &earlier);
        assert!(collapsed.key().is_some(), "{collapsed:?}");
        assert_eq!(
            collapsed,
            derive(&without_timestamp, png().as_ref(), &later),
            "one run, one recipe, two moments — the excluded field was the whole difference"
        );

        // The field left, and nothing else did.
        let mut selection = select(&without_timestamp, &earlier);
        remove_excluded(&without_timestamp, &mut selection);
        assert_eq!(
            render(&selection),
            format!(
                r#"[[["vdsl"],"json",{{"script":{},"version":"0.4.0"}}]]"#,
                serde_json::to_string(SCRIPT_HIRES).unwrap()
            ),
            "the timestamp is gone from inside the chunk, and the chunk is still there"
        );
    }

    /// A rule that is not about this material, told apart from a rule
    /// that is and found nothing — the distinction
    /// [`MaterialMeta`](crate::domain::material_meta::MaterialMeta)
    /// keeps on the axis below, kept here for the same reason.
    #[test]
    fn an_absent_keyword_is_not_applicable() {
        let rule = strategy(Decode::RawJson, &[&["vdsl", "script"]], &[]);

        // A PNG off some other generator: metadata, none of it this
        // rule's.
        let stranger = fields(&[("Software", "ComfyUI"), ("prompt", &prompt_chunk(7))]);
        assert_eq!(
            derive(&rule, png().as_ref(), &stranger),
            SeriesKey::NotApplicable
        );

        // The keyword is there, but the material is a JPEG — the rule
        // was written against a PNG container and says nothing here.
        let vdsl = fields(&[(
            "vdsl",
            &vdsl_chunk(SCRIPT_HIRES, "2026-04-26T15:48:29+09:00"),
        )]);
        assert_eq!(
            derive(&rule, Some(&mime("image/jpeg")), &vdsl),
            SeriesKey::NotApplicable
        );
        // …and a material whose format was never resolved is not a
        // material this rule claims either.
        assert_eq!(derive(&rule, None, &vdsl), SeriesKey::NotApplicable);
        // The same rule on the same map, once the format agrees.
        assert!(derive(&rule, png().as_ref(), &vdsl).key().is_some());

        // An empty container: with no include list every keyword is a
        // target, so having none is having nothing to be about.
        assert_eq!(
            derive(
                &strategy(Decode::RawJson, &[], &[]),
                png().as_ref(),
                &BTreeMap::new()
            ),
            SeriesKey::NotApplicable
        );
    }

    /// The other half of that distinction: the keyword is here, the
    /// rule ran, and the path resolved nowhere.
    #[test]
    fn an_include_that_selects_nothing_is_empty() {
        // v0.3.0 of the generator, before `script` was written.
        let older = fields(&[(
            "vdsl",
            &serde_json::to_string(
                &json!({"timestamp": "2026-01-02T03:04:05+09:00", "version": "0.3.0"}),
            )
            .unwrap(),
        )]);
        assert_eq!(
            derive(
                &strategy(Decode::RawJson, &[&["vdsl", "script"]], &[]),
                png().as_ref(),
                &older
            ),
            SeriesKey::NothingToSelect
        );

        // A segment cannot index an array: `["3","inputs","0"]`
        // addresses an object key spelled `0`, and this rule finds no
        // such key rather than the first element.
        let graph = fields(&[(
            "prompt",
            &serde_json::to_string(&json!({"3": {"inputs": ["a", "b"]}})).unwrap(),
        )]);
        assert_eq!(
            derive(
                &strategy(Decode::RawJson, &[&["prompt", "3", "inputs", "0"]], &[]),
                png().as_ref(),
                &graph
            ),
            SeriesKey::NothingToSelect
        );
        // The array itself is selectable, whole.
        assert!(
            derive(
                &strategy(Decode::RawJson, &[&["prompt", "3", "inputs"]], &[]),
                png().as_ref(),
                &graph
            )
            .key()
            .is_some()
        );
    }

    /// The character-card shape: base64 of a JSON document, and the
    /// field worth grouping on is two levels inside it.
    #[test]
    fn base64_json_reaches_a_nested_field() {
        fn card_document(name: &str, description: &str) -> String {
            serde_json::to_string(&json!({
                "spec": "chara_card_v3",
                "spec_version": "3.0",
                "data": { "name": name, "description": description },
            }))
            .unwrap()
        }
        fn card(name: &str, description: &str) -> BTreeMap<String, String> {
            fields(&[(
                "ccv3",
                &BASE64.encode(card_document(name, description).as_bytes()),
            )])
        }

        let rule = strategy(Decode::Base64Json, &[&["ccv3", "data", "name"]], &[]);
        let derive_card = |card: &BTreeMap<String, String>| derive(&rule, png().as_ref(), card);

        let first = derive_card(&card("Seraphina", "A forest guardian."));
        let revised = derive_card(&card("Seraphina", "A forest guardian, revised."));
        let other = derive_card(&card("Aria", "A forest guardian."));

        assert!(
            first.key().is_some(),
            "the payload decoded and the path resolved: {first:?}"
        );
        assert_eq!(first, revised, "one card edited is one card");
        assert_ne!(
            first, other,
            "and the field the rule selected is the one that decides"
        );

        // Two paths into one keyword. The keyword is the unit of
        // decoding — one base64 pass and one parse, cached — and each
        // path then selects out of that single reading.
        let two_fields = strategy(
            Decode::Base64Json,
            &[&["ccv3", "data", "name"], &["ccv3", "spec"]],
            &[],
        );
        let card = card("Seraphina", "A forest guardian.");
        let mut selection = select(&two_fields, &card);
        remove_excluded(&two_fields, &mut selection);
        assert_eq!(
            render(&selection),
            r#"[[["ccv3","data","name"],"json","Seraphina"],[["ccv3","spec"],"json","chara_card_v3"]]"#,
            "each path selected out of the one reading, and neither picked up the other's"
        );

        // What the decoder refuses is not read as something
        // approximate: the value stays the text the container carried,
        // so the rule's path resolves nowhere. Which strings `base64`
        // refuses is the crate's business; what is asserted here is
        // where a refusal lands.
        //
        // **Every fixture below is the *same real card*, spelled a way
        // this engine does not accept.** That is what gives the
        // assertion teeth: if the engine were widened, each of these
        // would decode to a document whose `data.name` resolves, and
        // `NothingToSelect` would become a key. A fixture built from
        // some other payload would land on `NothingToSelect` because
        // the *path* missed, and would go on passing however the
        // decoder was changed.
        // `ÿ` is `C3 BF`, and three of them cover all three offsets a
        // byte can take inside a base64 group, so at least one lands
        // where the six set bits of `BF` line up as a 63 sextet — `/`
        // in the standard alphabet and `_` in the URL-safe one. The
        // assertion below is what actually checks that; this is why it
        // is expected to hold.
        let document = card_document("Seraphina", "A forest guardian ÿÿÿ.");
        let padded = BASE64.encode(document.as_bytes());
        let url_safe = base64::engine::general_purpose::URL_SAFE.encode(document.as_bytes());
        let unpadded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(document.as_bytes());
        assert_ne!(
            url_safe, padded,
            "the alphabet fixture says nothing unless the two alphabets spell this payload \
             differently — pick a description that encodes a 62 or 63 sextet"
        );
        assert!(
            padded.ends_with('=') && !unpadded.ends_with('='),
            "the padding fixture says nothing unless this payload is padded to begin with"
        );

        for not_a_payload in [
            document.clone(), // the JSON itself, unencoded: the ordinary miss
            url_safe,         // the URL-safe alphabet
            unpadded,         // the same bytes, padding omitted
        ] {
            let carried = fields(&[("ccv3", not_a_payload.as_str())]);
            assert_eq!(
                derive(&rule, png().as_ref(), &carried),
                SeriesKey::NothingToSelect,
                "{not_a_payload:?} did not decode, so nothing was selected out of it"
            );
        }

        // Surrounding whitespace is the exception, and not a taste:
        // `png_chunk::envelope_from_chunk` trims for a recorded reason
        // (editors round-trip a trailing newline into the chunk), and
        // nothing on the walk path trims before `meta_kv`. A card the
        // importer accepts has to derive a key here, and the same key.
        let wrapped = fields(&[("ccv3", format!("\n{padded}\n").as_str())]);
        assert_eq!(
            derive(&rule, png().as_ref(), &wrapped),
            derive(&rule, png().as_ref(), &fields(&[("ccv3", padded.as_str())])),
            "a trailing newline is the editor's, not the card's"
        );
        assert!(derive(&rule, png().as_ref(), &wrapped).key().is_some());
    }

    /// **A Strategy groups photographs by one EXIF field, and the
    /// field's type is addressable behind it.**
    ///
    /// The JPEG shape: keys that are already flat addresses
    /// (`ifd0:0x010f` is the maker, in the primary IFD), values that
    /// carry the type the file stated. Three frames from two cameras,
    /// where the exposure differs *within* one camera — which is what
    /// makes the grouping a grouping rather than a count, since a rule
    /// that selected everything would return three keys and one that
    /// selected nothing would return one.
    ///
    /// The second half is what [`Decode::Exif`] is for. `["ifd0:0x010f"]`
    /// selects the field whole and would work under
    /// [`Decode::None`] too; `["ifd0:0x010f","ascii"]` selects the text
    /// **only where the field really is an ASCII string**, and the last
    /// block is the pair that separates: one file whose exposure is a
    /// rational and one whose exposure is an ASCII tag whose text is
    /// literally `1/125`. A decoder that stripped the marker instead of
    /// moving it would put those two on one key.
    #[test]
    fn an_exif_rule_groups_by_a_field_and_keeps_its_type_addressable() {
        fn photograph(make: &str, exposure: &str) -> BTreeMap<String, String> {
            fields(&[
                ("ifd0:0x010f", &format!("ascii:{make}")),
                ("ifd0:0x0112", "short:6"),
                ("exif:0x829a", &format!("rational:{exposure}")),
            ])
        }
        fn rule(include: &[&[&str]]) -> Strategy {
            Strategy {
                id: StrategyId::new(),
                name: "by camera".to_string(),
                applies_to: mime("image/jpeg"),
                decode: Decode::Exif,
                include: include
                    .iter()
                    .map(|p| Path::new(p.iter().copied()))
                    .collect(),
                exclude: Vec::new(),
            }
        }

        let jpeg = Some(mime("image/jpeg"));
        let corpus = [
            photograph("ACME", "1/125"),
            photograph("ACME", "1/250"),
            photograph("OTHER", "1/125"),
        ];
        let keys = |rule: &Strategy| -> Vec<SeriesKey> {
            corpus
                .iter()
                .map(|meta_kv| derive(rule, jpeg.as_ref(), meta_kv))
                .collect()
        };

        for include in [
            &[&["ifd0:0x010f"][..]][..],
            &[&["ifd0:0x010f", "ascii"][..]][..],
        ] {
            let derived = keys(&rule(include));
            let sizes = group_sizes(
                &derived
                    .iter()
                    .map(|key| {
                        key.key()
                            .expect("every frame carries the maker")
                            .to_string()
                    })
                    .collect::<Vec<_>>(),
            );
            assert_eq!(sizes, vec![1, 2], "{include:?}: two cameras, two and one");
            assert_eq!(derived[0], derived[1], "one camera, two exposures, one key");
            assert_ne!(derived[0], derived[2]);
        }

        // The whole map, for the control: with nothing selected away the
        // exposure separates the two ACME frames, so the grouping above
        // is the include list doing something.
        assert_eq!(
            group_sizes(&keys_over_jpeg(&rule(&[]), &corpus)),
            vec![1, 1, 1]
        );

        // The type is a segment, and it has to match: the exposure is a
        // rational, so a path naming `ascii` under it resolves nowhere.
        assert_eq!(
            derive(
                &rule(&[&["exif:0x829a", "ascii"]]),
                jpeg.as_ref(),
                &corpus[0]
            ),
            SeriesKey::NothingToSelect
        );
        assert!(
            derive(
                &rule(&[&["exif:0x829a", "rational"]]),
                jpeg.as_ref(),
                &corpus[0]
            )
            .key()
            .is_some()
        );

        // And the pair the marker exists for: the same rendering under
        // two types is two fields, not one.
        let as_rational = fields(&[("exif:0x829a", "rational:1/125")]);
        let as_text = fields(&[("exif:0x829a", "ascii:1/125")]);
        let whole = rule(&[&["exif:0x829a"]]);
        assert_ne!(
            derive(&whole, jpeg.as_ref(), &as_rational),
            derive(&whole, jpeg.as_ref(), &as_text),
            "a rational and an ASCII tag reading `1/125` are two different fields"
        );
        assert_eq!(
            derive(
                &rule(&[&["exif:0x829a", "rational"]]),
                jpeg.as_ref(),
                &as_text
            ),
            SeriesKey::NothingToSelect,
            "and the type segment is what tells them apart"
        );

        // A value that did not come from the probe — no marker at all —
        // stays the text the container stated rather than being split at
        // nothing, so a path into it finds nothing.
        let unmarked = fields(&[("exif:0x829a", "1/125")]);
        assert_eq!(
            derive(
                &rule(&[&["exif:0x829a", "rational"]]),
                jpeg.as_ref(),
                &unmarked
            ),
            SeriesKey::NothingToSelect
        );
        assert!(derive(&whole, jpeg.as_ref(), &unmarked).key().is_some());
    }

    /// [`keys_over`] for a rule written against JPEG.
    fn keys_over_jpeg(strategy: &Strategy, corpus: &[BTreeMap<String, String>]) -> Vec<String> {
        corpus
            .iter()
            .map(
                |meta_kv| match derive(strategy, Some(&mime("image/jpeg")), meta_kv) {
                    SeriesKey::Derived(key) => key,
                    other => panic!("every frame in this corpus carries the fields: {other:?}"),
                },
            )
            .collect()
    }

    /// The form the key is taken over, stated as a literal.
    ///
    /// Three properties at once: the path travels beside its value
    /// rather than flattened into a string, the entries are in path
    /// order, and every nested object's keys come out sorted. The last
    /// of those is [`canonical_value`]'s doing and deliberately not
    /// `serde_json`'s — this workspace runs with `preserve_order` on, so
    /// a parsed object re-serialises in the order its author wrote it
    /// unless something sorts it. The sibling test states that as the
    /// property; this one pins the exact bytes that get hashed.
    #[test]
    fn the_selected_form_carries_the_path_beside_the_value_and_is_ordered() {
        let meta_kv = fields(&[
            ("vdsl", r#"{"version":"0.4.0","script":"phase8"}"#),
            ("Software", "VDSL"),
        ]);
        let rule = strategy(Decode::RawJson, &[], &[]);

        let mut selection = select(&rule, &meta_kv);
        remove_excluded(&rule, &mut selection);
        assert_eq!(
            render(&selection),
            r#"[[["Software"],"text","VDSL"],[["vdsl"],"json",{"script":"phase8","version":"0.4.0"}]]"#,
            "paths beside values, entries in path order, nested keys sorted, no whitespace — \
             and `Software`, which is not JSON, kept as the text it was and marked as text"
        );

        let key = derive(&rule, png().as_ref(), &meta_kv);
        assert_eq!(key, SeriesKey::Derived(digest_of(&render(&selection))));
        let key = key.key().expect("derived").to_string();
        assert!(key.starts_with(SERIES_KEY_PREFIX));
        assert_eq!(key.len(), SERIES_KEY_PREFIX.len() + 64);
        assert!(
            key[SERIES_KEY_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    /// The key is a function of the document, not of how it was typed.
    ///
    /// A JSON object is an unordered collection (RFC 8259), so two
    /// containers that carry the same fields in a different order carry
    /// the same document, and a series key that told them apart would be
    /// answering the wrong question — the whole point of the axis is
    /// "made the same way".
    ///
    /// This is the property [`canonical_value`] exists for, and it is
    /// stated here as a property rather than as a byte literal so that
    /// deleting that function fails *here*, with a name that says what
    /// broke. Under `preserve_order` — which this workspace runs with —
    /// removing the sort makes these two keys differ.
    #[test]
    fn the_key_is_the_same_whatever_order_the_container_wrote_its_keys_in() {
        let rule = strategy(Decode::RawJson, &[], &[]);

        // The same object, typed two ways: one already sorted, one not.
        // Nested, because the sort has to be recursive — a top-level
        // sort would pass a shallow version of this test.
        let sorted = fields(&[(
            "vdsl",
            r#"{"alpha":{"inner_a":1,"inner_b":2},"beta":"x","gamma":[3,1,2]}"#,
        )]);
        let shuffled = fields(&[(
            "vdsl",
            r#"{"gamma":[3,1,2],"beta":"x","alpha":{"inner_b":2,"inner_a":1}}"#,
        )]);

        let key_of = |meta_kv: &BTreeMap<String, String>| {
            let mut selection = select(&rule, meta_kv);
            remove_excluded(&rule, &mut selection);
            render(&selection)
        };
        assert_eq!(
            key_of(&sorted),
            key_of(&shuffled),
            "key order is how a document was written, not what it says"
        );
        assert_eq!(
            derive(&rule, png().as_ref(), &sorted),
            derive(&rule, png().as_ref(), &shuffled)
        );

        // And the other direction, so the assertion above cannot be
        // satisfied by a function that flattens everything: an array's
        // order *is* content, and reordering one is a different
        // document.
        let reordered_array = fields(&[(
            "vdsl",
            r#"{"alpha":{"inner_a":1,"inner_b":2},"beta":"x","gamma":[1,2,3]}"#,
        )]);
        assert_ne!(
            key_of(&sorted),
            key_of(&reordered_array),
            "a JSON array is ordered; collapsing two of them would merge \
             materials that are not the same"
        );
    }

    /// A value the decoder cannot read keeps every distinction it
    /// carried.
    ///
    /// Two half-written chunks are two different half-written chunks.
    /// Treating an undecodable value as absent would put them under one
    /// key, together with every other material whose chunk failed to
    /// parse for its own reason — the over-grouping the module doc
    /// refuses.
    #[test]
    fn a_value_that_does_not_decode_stays_the_text_the_container_carried() {
        let truncated = fields(&[("vdsl", r#"{"script": "phase8_hires.lua", "timesta"#)]);
        let other_truncation = fields(&[("vdsl", r#"{"script": "phase9_portrait.lua", "tim"#)]);
        let whole_map = strategy(Decode::RawJson, &[], &[]);

        assert_ne!(
            derive(&whole_map, png().as_ref(), &truncated),
            derive(&whole_map, png().as_ref(), &other_truncation),
            "two broken chunks are two chunks"
        );

        // And the harder pair, which keeping the text does *not* on its
        // own separate: prose, against a JSON document that happens to
        // be a string with the same content. Both reach the rendering
        // as one `Value::String`, so without the kind travelling beside
        // it these two would be one key — two materials the container
        // plainly distinguishes, filed as made the same way.
        let prose = fields(&[("Software", "hello")]);
        let json_string = fields(&[("Software", r#""hello""#)]);
        assert_ne!(
            derive(&whole_map, png().as_ref(), &prose),
            derive(&whole_map, png().as_ref(), &json_string),
            "`hello` is not `\"hello\"`, and the container said which one it carried"
        );

        // And a path into it finds nothing, rather than finding
        // something wrong.
        assert_eq!(
            derive(
                &strategy(Decode::RawJson, &[&["vdsl", "script"]], &[]),
                png().as_ref(),
                &truncated
            ),
            SeriesKey::NothingToSelect
        );
    }

    /// The reserved value is the digest [`derive`] refuses to write.
    ///
    /// Pinned against the *rendering* rather than typed twice, because a
    /// constant that agreed with nothing would go on looking right: the
    /// whole claim is that this exact string is what an empty selection
    /// would hash to, so a change to [`render`]'s form (the empty case
    /// spelled some other way) has to name this constant, and it does so
    /// here.
    ///
    /// The second half is the property the constant is reserved
    /// *against*: no rule derives it, whatever the rule and whatever the
    /// material, because `NothingToSelect` answers first. A `derive` that
    /// stopped short-circuiting would put every material its rule missed
    /// on this one key.
    #[test]
    fn the_reserved_key_is_the_one_an_empty_selection_would_hash_to() {
        assert_eq!(
            SERIES_KEY_EMPTY,
            digest_of(&render(&BTreeMap::new())),
            "the reserved value has to be the digest it is reserving"
        );
        assert!(SERIES_RESERVED_VALUES.contains(&SERIES_KEY_EMPTY));

        // A rule that selects nothing, and one that is not asked at all.
        let older = fields(&[("vdsl", r#"{"version":"0.3.0"}"#)]);
        for outcome in [
            derive(
                &strategy(Decode::RawJson, &[&["vdsl", "script"]], &[]),
                png().as_ref(),
                &older,
            ),
            derive(
                &strategy(Decode::RawJson, &[], &[&["vdsl"]]),
                png().as_ref(),
                &older,
            ),
        ] {
            assert_eq!(outcome, SeriesKey::NothingToSelect);
            assert_eq!(outcome.key(), None, "and it carries no key to reserve");
        }

        // What the exclusion buys a reader: the same string, arriving in
        // the column by any other route, is not a grouping.
        assert!(!is_series_key(SERIES_KEY_EMPTY));
        assert!(is_series_key(&digest_of(
            r#"[[["vdsl","script"],"json","x"]]"#
        )));
        assert!(
            !is_series_key(&format!("m1-sha256:{}", "a".repeat(64))),
            "a digest off another axis is not a series key either"
        );
    }

    /// How many variants an `enum` declares, read off this file's own
    /// source.
    ///
    /// A variant line is one that starts with an uppercase letter inside
    /// the block — doc comments (`///`) and attributes (`#`) do not, and
    /// neither does anything else these two enums contain. Crude on
    /// purpose: what it has to survive is a variant being *added*, and
    /// there is no way to add one without a line of that shape.
    fn declared_variants(name: &str) -> usize {
        let body = include_str!("series.rs")
            .split_once(&format!("pub enum {name} {{"))
            .unwrap_or_else(|| panic!("`{name}` is declared in this file"))
            .1
            .split_once("\n}")
            .expect("and the declaration is closed")
            .0;
        body.lines()
            .filter(|line| line.trim().starts_with(|c: char| c.is_ascii_uppercase()))
            .count()
    }

    /// [`Decode::ALL`] holds every variant `Decode` has.
    ///
    /// The guard the schema's vocabulary test rests on, and the one a
    /// hand-written array cannot give: a `match` forces an arm per
    /// variant, never a list entry, so a new variant would compile with
    /// new `as_str` / `parse` arms, leave this list where it was, and
    /// reach a user's library as a `CHECK` violation the first time
    /// somebody registered a rule using it. See [`Decode::ALL`] for the
    /// shapes that were tried and where each leaks.
    ///
    /// Length plus distinctness is the whole argument: if the enum
    /// declares N variants, the list holds N entries, and no two entries
    /// are equal, then the list is the variant set. Both halves are
    /// asserted, because either alone is satisfiable by a list that is
    /// wrong.
    ///
    /// **It fired, on the variant it was written for.** Checked by
    /// mutation on 2026-08-10 by adding `Decode::Exif` with the minimum
    /// the compiler demands — `as_str`, `parse` and `decoded` arms — and
    /// nothing else: *left `4`, right `3`*. Adding it to
    /// [`Decode::ALL`] moved the failure one step down the chain, to the
    /// migration's own guard: *left `["none", "raw_json",
    /// "base64_json"]`, right `["none", "raw_json", "base64_json",
    /// "exif"]`*, which is the `CHECK` that would have refused the
    /// token. That variant is now shipped and the `CHECK` widened (V77),
    /// so both steps are green — and the sequence is recorded because it
    /// is the whole of what this test is for. [measured 2026-08-11: removing
    /// `Self::Exif` from [`Decode::ALL`] reproduces the first failure,
    /// and putting it back but reverting V77 reproduces the second.]
    #[test]
    fn the_decoder_list_names_every_variant_this_enum_has() {
        assert_eq!(
            declared_variants("Decode"),
            Decode::ALL.len(),
            "`Decode::ALL` is what the shipped schema's `CHECK` is measured against — \
             a variant missing from it is a decoder the schema will refuse to store"
        );
        for (index, decode) in Decode::ALL.iter().enumerate() {
            assert!(
                !Decode::ALL[..index].contains(decode),
                "{decode:?} is listed twice, so the count above proves nothing"
            );
        }

        // The same for the three answers, whose tokens are the other
        // half of that migration's guard.
        assert_eq!(declared_variants("SeriesKey"), SeriesKey::OUTCOMES.len());
        for (index, token) in SeriesKey::OUTCOMES.iter().enumerate() {
            assert!(!SeriesKey::OUTCOMES[..index].contains(token), "{token}");
        }
        // …and the tokens really are the ones the variants write. Three
        // distinct slugs out of three variants, against a list of three
        // proved complete above, is a bijection.
        let written = [
            SeriesKey::Derived(String::new()).outcome_slug(),
            SeriesKey::NothingToSelect.outcome_slug(),
            SeriesKey::NotApplicable.outcome_slug(),
        ];
        for token in written {
            assert!(SeriesKey::OUTCOMES.contains(&token), "{token} is unlisted");
        }

        // The parse is the part that can go quietly wrong: a reader that
        // matched nothing would report zero variants and agree with an
        // empty list.
        assert!(declared_variants("Decode") >= 3);
    }

    /// The two stored vocabularies round-trip, and an unknown decoder is
    /// refused rather than resolved to the one that reads nothing.
    ///
    /// `Decode::None` is asserted to be a *different* answer from the
    /// refusal, which is the whole risk in that arm: the two behave
    /// alike on the way in — neither descends into a value — so a
    /// `unwrap_or(None)` would look correct on every fixture while
    /// deriving keys under a rule this build cannot carry out.
    #[test]
    fn the_stored_tokens_round_trip_and_an_unknown_decoder_is_refused() {
        for decode in [
            Decode::None,
            Decode::RawJson,
            Decode::Base64Json,
            Decode::Exif,
        ] {
            assert_eq!(Decode::parse(decode.as_str()).unwrap(), decode);
        }
        // `exif` stood here while it was the decoder that had not
        // shipped. It shipped, so the case needs a token that has not —
        // and the shape of one is a rule registered against a later
        // build, which is what the refusal is really about.
        let err = Decode::parse("prose_pairs").expect_err("a decoder this build has not shipped");
        assert!(
            matches!(&err, DomainError::Validation(m) if m.contains("prose_pairs")),
            "the message names the token: {err}"
        );

        assert_eq!(
            SeriesKey::Derived("sk1-sha256:x".into()).outcome_slug(),
            "derived"
        );
        assert_eq!(
            SeriesKey::NothingToSelect.outcome_slug(),
            "nothing_to_select"
        );
        assert_eq!(SeriesKey::NotApplicable.outcome_slug(), "not_applicable");
    }

    /// What exclude does at its two edges.
    ///
    /// Removing everything is [`SeriesKey::NothingToSelect`] and not a
    /// key over an empty selection. An empty path removes *nothing*,
    /// which is the case worth a test rather than a comment: the empty
    /// sequence is a prefix of every path, so the literal reading of it
    /// deletes the whole selection and every material under the rule
    /// collapses into one silent group.
    #[test]
    fn an_exclude_that_removes_everything_is_empty_and_an_empty_path_removes_nothing() {
        let meta_kv = fields(&[
            (
                "vdsl",
                &vdsl_chunk(SCRIPT_HIRES, "2026-04-26T15:48:29+09:00"),
            ),
            ("prompt", &prompt_chunk(1_000)),
        ]);

        assert_eq!(
            derive(
                &strategy(Decode::RawJson, &[], &[&["vdsl"], &["prompt"]]),
                png().as_ref(),
                &meta_kv
            ),
            SeriesKey::NothingToSelect
        );

        assert_eq!(
            derive(
                &strategy(Decode::RawJson, &[], &[&[]]),
                png().as_ref(),
                &meta_kv
            ),
            derive(
                &strategy(Decode::RawJson, &[], &[]),
                png().as_ref(),
                &meta_kv
            ),
            "an exclude path with no segments names no field"
        );

        // An emptied entry is not a removed one: that the container had
        // the keyword is itself a distinction. Asserted on the rendered
        // form rather than on `key().is_some()`, which an exclude that
        // silently did nothing would also satisfy.
        let field_by_field = strategy(
            Decode::RawJson,
            &[&["vdsl"]],
            &[
                &["vdsl", "script"],
                &["vdsl", "timestamp"],
                &["vdsl", "version"],
            ],
        );
        let mut selection = select(&field_by_field, &meta_kv);
        remove_excluded(&field_by_field, &mut selection);
        assert_eq!(
            render(&selection),
            r#"[[["vdsl"],"json",{}]]"#,
            "every field named, none of them left, and the keyword still there"
        );
        assert!(
            derive(&field_by_field, png().as_ref(), &meta_kv)
                .key()
                .is_some()
        );
    }
}
