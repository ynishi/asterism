//! `probe` — the port a format's identity measurement is written
//! against.
//!
//! Two of the three fingerprint axes are format-specific.
//! [`content_hash`](crate::domain::content_hash) streams a whole file and
//! works on anything; the other two have to know what the container is,
//! because the question they answer — which bytes decide what this
//! decodes to, and which are notes written *about* it — has no answer
//! that is true of every file. So the vocabulary of the answers lives in
//! [`content_region`](crate::domain::content_region) and
//! [`material_meta`](crate::domain::material_meta), and the reading of
//! any particular container lives behind this trait.
//!
//! # Why a port and not a `match`
//!
//! The walkers used to be functions in the domain layer with the format
//! written into them, and the gate that routed a file to them was a
//! `matches!` on the declared mime — one copy per axis. Adding a second
//! format meant widening two `matches!` arms and adding a second parser,
//! several hundred lines of untrusted-input handling, to a layer whose
//! whole claim is that it has no I/O and no format knowledge. The parser
//! is the part that grows per format, and it is the part that belongs
//! furthest from here.
//!
//! What is left in the domain is the vocabulary the columns carry and
//! this trait. An implementation is an adapter: it holds a format's
//! judgement about its own container and produces domain values. Adding
//! a format adds an implementation and a line in whatever registry the
//! adapter layer keeps — the probe, not a second list of the formats it
//! answers for, which the probe states itself
//! ([`ArtefactProbe::declares`]); nothing in this crate is edited.
//!
//! That is what it costs to read the *next* file, and it is not the
//! whole cost of the format arriving. Every artefact already in the
//! library carries `unsupported:<mime>` on both walking axes, and a
//! marker is a final answer to "has anybody looked"
//! ([`is_axis_answer`](crate::domain::content_hash::is_axis_answer)), so
//! [`needs_fingerprint`](crate::domain::content_hash::needs_fingerprint)
//! passes those rows over for good. Files imported after the probe lands
//! would take digests while the ones that were there first keep the
//! marker, and the column would hold two meanings with nothing to tell
//! them apart. So a format also arrives owing a way back to the rows it
//! was refused on, **and it owes one per axis**: a probe may claim the
//! second axis a slice after the first, and the column it was refused on
//! stays refused until something says otherwise.
//!
//! The cheap way to say it is a numbered `UPDATE` writing NULL over that
//! one marker, which hands the rows back to the ordinary walk — what
//! both of JPEG's axes took, one step each.
//! [`needs_content_walk`](crate::domain::content_hash::needs_content_walk)
//! is the other shape — a second predicate, selecting one marker, driven
//! by a migration-chain read — and it stays reserved for the case it was
//! written for. It reads the content column only, so there is no
//! equivalent predicate on the meta axis and none has been needed.

use crate::domain::content_region::{self, ContentRegion};
use crate::domain::material_meta::{self, MaterialMeta};
use crate::domain::material_meta_raw::MetaRaw;
use crate::domain::value::MimeType;

/// One format's reading of an artefact's identity.
///
/// # One declaration, two gates
///
/// [`declares`](Self::declares) is where an implementation says which
/// formats it answers for and on which axes, and it is the only place it
/// says so. The gates a caller asks —
/// [`walks_content`](ProbeGates::walks_content) and
/// [`walks_meta`](ProbeGates::walks_meta) — are read off that list by
/// [`ProbeGates`], which no implementor writes.
///
/// The two gates are still two, and an implementation is still free to
/// answer them differently: a [`FormatClaim`] carries an axis apiece.
/// The two axes are two definitions over one container — a format whose
/// picture bytes are separable may still carry its metadata somewhere
/// nothing here can read, and the reverse happens too.
///
/// Collapsing them into one "do you handle this" would break on the day
/// that stops being hypothetical — a single gate either skips a walk
/// that would have worked, or reads a file whole for a walk that answers
/// nothing. The caller reads the file if **either** says yes, and then
/// asks both.
///
/// # `false` is an answer
///
/// Declining a format is ordinary and costs nothing real: leave it out
/// of [`declares`](Self::declares) and the row falls back to the file
/// axis, which groups byte-identical copies perfectly well. What an
/// implementation must not do is say yes and then read the container
/// carelessly — see [`content_of`](Self::content_of).
///
/// # The declared format selects the probe, and only it
///
/// A reading answers only what its own gate admitted, and that is not a
/// rule an implementation is asked to keep. The methods a caller can
/// reach are [`ProbeGates::content`] and [`ProbeGates::meta`], which ask
/// the gate first and return the marker for the declared format where it
/// says no — never a digest, however plainly the bytes announce
/// themselves. What an implementation writes is
/// [`content_of`](Self::content_of), and entering it takes a
/// [`GateOpen`] that only the gate hands out.
///
/// The rule is about who chooses. A caller holding a whole registry
/// picks a probe from the declared mime, before any byte is read; a
/// probe that also sniffed its way in would answer differently depending
/// on which of the two doors it was reached through, and the second door
/// is not hypothetical — an artefact whose bytes are held rather than
/// named (an inline body, a locator with no extension) reaches a probe
/// without going through a registry at all. One artefact would then
/// carry a digest or a marker according to how it happened to arrive,
/// and the two are not comparable values. Both doors open onto the same
/// gate now, which is what stopped the arrival deciding.
///
/// A signature check is still owed, in the other direction: having been
/// selected by the claim, refuse when the bytes disagree with it. That
/// stops a container parser being pointed at whatever the file really
/// is. It decides *whether to trust* a claim, never *whether to accept*
/// an artefact that made none.
pub trait ArtefactProbe: Send + Sync {
    /// Every format this probe answers for, and the axes it answers on.
    ///
    /// The list a caller's gates are computed from, and the list a
    /// registry is checked against — one list, because they are one
    /// fact. A build that also kept its own copy would have a way to
    /// disagree with itself, and the disagreement is quiet: a probe that
    /// answers for a format the registry never listed opens the gate in
    /// the fingerprint job, so every artefact of that format is read
    /// whole, refused on its signature, and stored under a different
    /// marker than the day before, with nothing red anywhere.
    ///
    /// Declared here rather than in the caller because this is where the
    /// walking is. A caller with its own list would go on reading video
    /// into memory for a year after a probe learned to walk it, or stop
    /// reading PNGs the day the list was edited on one side only.
    ///
    /// The gates are asked **before** anything is read —
    /// [`content_of`](Self::content_of) needs the whole artefact in
    /// memory, so the caller has to decide whether to spend that before
    /// it opens the file, and the declared mime is the only input it has
    /// at that point. So a claim answers "is it worth reading", never
    /// "is it really that format": the mime is a guess from a filename
    /// and lies in both directions, which is why
    /// [`content_of`](Self::content_of) checks the bytes too.
    ///
    /// `&'static` because the list outlives every caller that borrows
    /// it — the gates, the registry's completeness check — without a
    /// lifetime threaded through a `dyn` port. It is not a statement
    /// that the answer is fixed: `if FLAG.load(Relaxed) { CLAIMS } else
    /// { &[] }` has this signature and compiles.
    ///
    /// **The constancy is an obligation on the implementor.** Return the
    /// same list however often it is asked and whatever else the process
    /// is doing. A probe deciding at runtime is a gate whose answer
    /// depends on when it was asked, and what breaks is the checking: a
    /// registry the completeness rule proved coherent in one state runs
    /// in production in another, where two probes can claim one axis, or
    /// none can, and either writes a column that disagrees with the one
    /// written yesterday — an artefact's stored value moving with a flag
    /// nothing connects it to, and nothing red anywhere.
    fn declares(&self) -> &'static [FormatClaim];

    /// The content-axis reading of these bytes, or the marker saying why
    /// there is none — **written by an implementor, called by nobody**.
    ///
    /// [`ProbeGates::content`] is the method a caller reaches. It asks
    /// the gate and hands the answer down as [`GateOpen`], which is the
    /// only way into here, so this body runs for a format this probe
    /// declared on this axis or it does not run at all. An
    /// implementation writes the reading and no part of the refusal that
    /// precedes it.
    ///
    /// `declared_mime` is what the row believes the file is, derived
    /// from its extension, and it is what selected this probe — a probe
    /// declaring several formats reads it to know which one it was
    /// reached for. An implementation should also check the claim against
    /// the bytes' own signature and refuse when the two disagree:
    /// running a container parser over whatever the file really is, on a
    /// caller's word for it, is the shape of problem this kind of code
    /// is known for, and a renamed file groups perfectly well on the
    /// file axis anyway.
    ///
    /// # The obligation on an implementor
    ///
    /// **Choose the content side with a denylist. Anything unrecognised
    /// goes into the digest.** Saying "the same" about two artefacts
    /// that are not is a loss no later correction can undo.
    ///
    /// The two rules are wrong in opposite directions, and the asymmetry
    /// is not close. An allowlist of the elements known to matter drops
    /// any element nobody thought of: a private extension, a type added
    /// to the spec later, or — measured on this repo's corpus, not
    /// imagined — PNG's colour-management chunks and APNG frame data,
    /// where **two visibly different pictures came out with one
    /// digest**. That error is a false positive. Downstream, a fold
    /// turns the loser of a duplicate group into a tombstone, and the
    /// user cannot undo it by looking at the two pictures again.
    ///
    /// A denylist's error runs the other way: forget to exclude some
    /// metadata element and two files that differ only in it get
    /// separate digests — which is exactly the state of a format with no
    /// content axis at all. One failure loses data, the other loses an
    /// improvement, so the unrecognised element goes on the side that
    /// loses the improvement.
    ///
    /// Note what this does *not* ask for. Leaving a format out of
    /// [`declares`](Self::declares), or claiming it on the other axis
    /// only, is entirely legitimate — this probe does not read that
    /// format's content, the row keeps its file axis, and nothing is
    /// claimed. The obligation is on the other
    /// branch: having said the format is handled, do not quietly drop
    /// the parts of it you did not recognise.
    ///
    /// A format's own critical / optional split is not this rule.
    /// PNG's, for one, says which chunks a *decoder* may skip, which is
    /// a statement about decoder obligations rather than about what the
    /// viewer sees — and under it the two measured collisions above are
    /// both admitted.
    fn content_of(
        &self,
        bytes: &[u8],
        declared_mime: Option<&MimeType>,
        gate: GateOpen,
    ) -> ContentRegion;

    /// The meta-axis reading of these bytes, or the marker saying why
    /// there is none — the same arrangement, behind
    /// [`ProbeGates::meta`].
    ///
    /// The complement of [`content_of`](Self::content_of) over the same
    /// container, on the same terms about the declared mime. Where that
    /// method's obligation is to keep the unrecognised, this one's is
    /// the mirror: an element read into the metadata is an element the
    /// content digest must not also carry, or the two axes stop being
    /// two.
    ///
    /// A [`MaterialMeta::Digest`] carries the canonical rendering beside
    /// the digest because they are one measurement — see that type for
    /// why a caller must not be able to hold one without the other.
    fn meta_of(
        &self,
        bytes: &[u8],
        declared_mime: Option<&MimeType>,
        gate: GateOpen,
    ) -> MaterialMeta;

    /// The container's metadata bytes as it carries them, behind
    /// [`ProbeGates::meta_raw`] — the input
    /// [`meta_of`](Self::meta_of)'s answer was derived from, kept so the
    /// derivation can be replaced without reading the file again
    /// ([`material_meta_raw`](crate::domain::material_meta_raw)).
    ///
    /// # Defaulted, and what the default says
    ///
    /// [`MetaRaw::Absent`] — "nothing here keeps bytes for this format",
    /// which is the truth for a probe that has not written this method
    /// and stores NULL. The two readings above have no default because
    /// declining them is done by leaving the format out of
    /// [`declares`](Self::declares); this one is declined *inside* a
    /// format a probe does read, since keeping bytes is worth doing
    /// where a rendering is lossy and not worth doing where it is not.
    ///
    /// # It rides the metadata gate rather than a third flag
    ///
    /// A [`FormatClaim`] carries an axis apiece and this is not a third
    /// axis: it is the same axis's other column, and the bytes it keeps
    /// are by definition the ones the metadata reading was taken over. A
    /// probe that does not read a format's metadata has no metadata
    /// region to keep, so a third flag would only ever be able to say
    /// something a caller must not act on — bytes claimed to be metadata
    /// by something that cannot say what metadata is in this container.
    ///
    /// # The obligation on an implementor
    ///
    /// **State a ceiling and answer [`MetaRaw::TooLarge`] past it.** The
    /// bytes are stored in a row, and a container whose structure does
    /// not bound them (PNG's chunk count times a chunk's declared
    /// length) would otherwise put an arbitrary fraction of a file into
    /// a column. Where the structure *does* bound them — JPEG's `APP1`
    /// segment length is two bytes — say so and the ceiling is the
    /// format's rather than a number somebody picked.
    ///
    /// **Keep them as the container carries them.** The value of this
    /// column is that a later reading can disagree with today's about
    /// what the bytes mean, which it can only do if nothing on the way
    /// in has already decided — no decompression, no re-framing, no
    /// text decoding. That is the same rule
    /// [`material_meta`](crate::domain::material_meta) states for values
    /// inside the canonical form, one layer further out.
    fn meta_raw_of(
        &self,
        bytes: &[u8],
        declared_mime: Option<&MimeType>,
        gate: GateOpen,
    ) -> MetaRaw {
        let _ = (bytes, declared_mime, gate);
        MetaRaw::Absent
    }
}

/// One format a probe answers for, and the axes it answers on.
///
/// The two flags are independent on purpose. A probe that can say which
/// of a container's bytes are the picture but not read its metadata, or
/// the other way round, is answering the two questions the way this port
/// says they may be answered — separately. Requiring both would leave
/// one shortest way to satisfy it: claim the axis you cannot read, which
/// opens the caller's gate and sends every artefact of the format
/// through a `read_to_end` on its way to a probe with nothing to say
/// about it.
///
/// Both `false` is not a claim at all — the format simply goes
/// unlisted — and a registry is entitled to refuse a probe that writes
/// one, since it reads as a statement that the probe answers for a
/// format it has no answer for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatClaim {
    /// The declared format this claim is about, compared as parsed — so
    /// it is written once and matches every spelling the boundary
    /// normalises (`IMAGE/PNG; charset=binary`, ` image/png `).
    pub mime: MimeType,
    /// Whether this probe reads the content axis for this format — the
    /// flag [`ProbeGates::walks_content`] answers from, and the one that
    /// decides whether [`ArtefactProbe::content_of`] is entered at all.
    pub content: bool,
    /// The same for the meta axis.
    pub meta: bool,
}

/// Everything a caller may ask a probe: the two gates, read off its
/// [`declaration`](ArtefactProbe::declares), and the two readings that
/// stand behind them.
///
/// A blanket implementation over every [`ArtefactProbe`], and
/// deliberately not methods each probe writes for itself. The gate and
/// the declaration are one fact, and a probe able to write the gate
/// directly could answer for a format it never declared — which is the
/// failure worth designing out rather than testing for, because it is
/// silent from every side. `hash_artefact` opens on this answer, so a
/// format claimed only here is read whole up to the walk ceiling,
/// refused on its signature, and stored under `unsupported:unknown`
/// where it used to carry `unsupported:<mime>`: a column rewritten
/// across a whole format with no test red.
///
/// The readings live here for the mirror of that reason. A gate a
/// reading is not obliged to consult is a second fact about one
/// declaration, kept in step by each probe by hand; the drift is the
/// same drift and just as quiet, since a probe that answers an axis it
/// declined answers it *well* — a digest, where the registry stores a
/// marker for the identical bytes. So the public
/// [`content`](Self::content) asks the gate and only then delegates,
/// and what an implementor writes
/// ([`content_of`](ArtefactProbe::content_of)) cannot be entered without
/// the [`GateOpen`] that step produces.
///
/// The cost is that `impl ProbeGates for MyProbe` does not compile —
/// which is the point. There is one place to say what a probe answers
/// for, and it is [`declares`](ArtefactProbe::declares).
pub trait ProbeGates {
    /// Whether this probe reads the content axis for a declared format —
    /// the question a caller asks **before** reading anything.
    fn walks_content(&self, declared_mime: Option<&MimeType>) -> bool;

    /// Whether this probe reads the meta axis for a declared format.
    ///
    /// Its own question rather than a call to
    /// [`walks_content`](Self::walks_content), even for a probe whose
    /// two answers agree today — see [`ArtefactProbe`]'s note on why the
    /// two gates stay two.
    fn walks_meta(&self, declared_mime: Option<&MimeType>) -> bool;

    /// This probe's content-axis answer for these bytes: the marker for
    /// a format it did not declare on this axis, and otherwise whatever
    /// [`content_of`](ArtefactProbe::content_of) makes of them.
    ///
    /// The marker is the one
    /// [`content_region::unsupported_format`](crate::domain::content_region::unsupported_format)
    /// builds — the same value a caller stores for a file it decided not
    /// to open — so the two ways of arriving at "no reading here" are one
    /// stored value rather than two.
    fn content(&self, bytes: &[u8], declared_mime: Option<&MimeType>) -> ContentRegion;

    /// The meta-axis answer, on the same terms.
    fn meta(&self, bytes: &[u8], declared_mime: Option<&MimeType>) -> MaterialMeta;

    /// The metadata bytes this probe keeps for these bytes, on the same
    /// terms and behind the same gate as [`meta`](Self::meta) — see
    /// [`ArtefactProbe::meta_raw_of`] for why the two share one.
    ///
    /// [`MetaRaw::Absent`] where the gate says no, which is the value
    /// that leaves the column NULL: a format nothing reads the metadata
    /// of has no metadata bytes, and inventing a marker for it would put
    /// a statement about this build in a column that holds a container's
    /// bytes.
    fn meta_raw(&self, bytes: &[u8], declared_mime: Option<&MimeType>) -> MetaRaw;
}

impl<P: ArtefactProbe + ?Sized> ProbeGates for P {
    fn walks_content(&self, declared_mime: Option<&MimeType>) -> bool {
        claims_for(self.declares(), declared_mime).any(|claim| claim.content)
    }

    fn walks_meta(&self, declared_mime: Option<&MimeType>) -> bool {
        claims_for(self.declares(), declared_mime).any(|claim| claim.meta)
    }

    fn content(&self, bytes: &[u8], declared_mime: Option<&MimeType>) -> ContentRegion {
        if !self.walks_content(declared_mime) {
            return content_region::unsupported_format(declared_mime);
        }
        self.content_of(bytes, declared_mime, GateOpen(()))
    }

    fn meta(&self, bytes: &[u8], declared_mime: Option<&MimeType>) -> MaterialMeta {
        if !self.walks_meta(declared_mime) {
            return material_meta::unsupported_format(declared_mime);
        }
        self.meta_of(bytes, declared_mime, GateOpen(()))
    }

    fn meta_raw(&self, bytes: &[u8], declared_mime: Option<&MimeType>) -> MetaRaw {
        if !self.walks_meta(declared_mime) {
            return MetaRaw::Absent;
        }
        self.meta_raw_of(bytes, declared_mime, GateOpen(()))
    }
}

/// That the gate for one axis was asked, and answered `true`.
///
/// A unit with a private field, so the only expression that builds one
/// is in the blanket implementation above, on the far side of the gate.
/// An implementor's reading takes one, which is what makes "consult the
/// gate before answering" something a probe cannot skip rather than
/// something every probe is asked to remember: outside this module there
/// is no call reaching [`ArtefactProbe::content_of`] that has not been
/// through [`ProbeGates::content`] — not from the adapter crate that
/// writes the probes, not from the registry, not from a test.
///
/// It carries nothing, and it is not meant to. What it is worth is the
/// call it makes unwritable, and the failure that call is: one artefact
/// taking a digest because it reached a probe directly and
/// `unsupported:unknown` because the identical file on disk went through
/// a registry, with both spellings stored in the same column and nothing
/// distinguishing them afterwards.
#[derive(Debug)]
pub struct GateOpen(());

/// The claims that answer for this declared format.
///
/// `None` — a locator whose extension named nothing — matches no claim,
/// so both gates answer `false`. The bytes might still be a format some
/// probe here handles, and that costs one content digest; the
/// alternative is to read every unrecognised file whole on the chance
/// that it is a picture, which is a real cost paid on every unknown row
/// for a case a `guess_mime` arm fixes properly.
fn claims_for<'a>(
    declared: &'a [FormatClaim],
    declared_mime: Option<&'a MimeType>,
) -> impl Iterator<Item = &'a FormatClaim> {
    declared
        .iter()
        .filter(move |claim| declared_mime.is_some_and(|mime| claim.mime == *mime))
}
