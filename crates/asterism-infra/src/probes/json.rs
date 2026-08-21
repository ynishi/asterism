//! JSON's reading of the content axis — the first digest on that axis
//! that re-renders, and the parameter list that makes it a definition.
//!
//! A JSON object is an unordered collection of name/value pairs
//! (RFC 8259), so two documents differing only in member order are one
//! document, and a digest that selects bytes cannot say so. The axis
//! doctrine ([`content_hash`](asterism_core::domain::content_hash))
//! prices the other route: a re-rendering digest must write its
//! canonical form out in full, because the rule for numbers and the
//! rule for duplicate keys are the parts that decide the answers.
//! These are the parameters, and the golden vectors below are the
//! parameters made checkable:
//!
//! 1. **A document is exactly one JSON text, UTF-8.** Anything else —
//!    invalid bytes, a syntax error, trailing content after the value —
//!    refuses to [`ContentRegion::EmptySpan`]. JSON has no signature
//!    apart from parsing whole, so every disagreement between the claim
//!    and the bytes lands there; `Unsupported` stays what the port's
//!    gate answers for formats this probe never claimed.
//! 2. **Inter-token whitespace is dropped.** The canonical form is
//!    compact: no space after `,` or `:`.
//! 3. **Object members are sorted by member name, decoded.** Names are
//!    compared as the UTF-8 byte sequences their tokens decode to, so
//!    `"b"` sorts as `b` — an order a reordering serialiser cannot
//!    disturb, which is the property the axis exists for.
//! 4. **A duplicate member name, at any depth, refuses the document.**
//!    Decoded before comparing, so a name spelled by Unicode escape
//!    collides with its plain spelling. The
//!    spec forbids the duplicate and practice resolves it silently,
//!    each serialiser its own way — a digest that picked a winner would
//!    call two documents the same on the strength of the loser.
//! 5. **Every scalar token is copied from the source verbatim** —
//!    numbers, strings (member names included), `true`/`false`/`null`.
//!    `1.50` stays `1.50`, `-0.0` stays `-0.0`, and `1` never becomes
//!    `1.0`'s equal. This is the parameter that separates the reading
//!    from RFC 8785, whose ECMAScript number rendering collides
//!    integers above 2^53 and erases `-0.0` and `1.0` against `0` and
//!    `1`. On a duplicate-detection axis the two error directions are
//!    not symmetric — a false positive is folded by resolution and
//!    destroys, a false negative costs a row — so the smaller claim
//!    wins. The price is paid in the same coin: two spellings of one
//!    name — plain, and by Unicode escape — decode identically and
//!    digest differently, a false negative accepted on the same
//!    grounds.
//! 6. **Array order is kept.** An array is a sequence; reordering one
//!    changes the document.
//!
//! The meta axis is not claimed. A JSON document has no container
//! metadata — no bytes riding alongside that are *about* the value
//! rather than part of it — so there is nothing for that axis to read,
//! the same shape JPEG had while its meta reading did not exist yet.
//!
//! `.jsonl` is deliberately not this format: it is records in a
//! container, not one value per file, and it keeps `text/plain`
//! ([`guess_mime`](asterism_core::domain::material::guess_mime)).
//!
//! # Why the walk is not `serde_json::Value`
//!
//! The workspace already sorts keys once — `series::canonical_value`,
//! for the series key — and it walks a parsed `Value`, so every scalar
//! is re-rendered on the way out and `1.50` comes back `1.5`. The right
//! trade for a key over what a document *refers to*, and exactly the
//! loss parameter 5 refuses for what a document *is*. So this walk
//! validates with the parser and then scans the validated text itself,
//! copying tokens instead of parsing them.

use asterism_core::domain::content_hash::ContentHasher;
use asterism_core::domain::content_region::ContentRegion;
use asterism_core::domain::material_meta::{self, MaterialMeta};
use asterism_core::domain::probe::{ArtefactProbe, FormatClaim, GateOpen};
use asterism_core::domain::value::MimeType;

use super::under_region_tag;

/// The probe. Stateless — the reading is a function of the bytes.
#[derive(Debug)]
pub struct JsonProbe;

/// One format, one axis. The meta flag is a statement that JSON has no
/// metadata region, not a reading that is still owed — see the module
/// doc.
const CLAIMS: &[FormatClaim] = &[FormatClaim {
    mime: MimeType::Json,
    content: true,
    meta: false,
}];

impl ArtefactProbe for JsonProbe {
    fn declares(&self) -> &'static [FormatClaim] {
        CLAIMS
    }

    fn content_of(
        &self,
        bytes: &[u8],
        _declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> ContentRegion {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return ContentRegion::EmptySpan;
        };
        // The parser is the signature check (parameter 1): it enforces
        // the grammar, the recursion ceiling, and "one value, nothing
        // after it". `IgnoredAny` because the answer wanted here is
        // yes/no — the values are read off the text by the scan below,
        // which copies tokens rather than parsing them.
        if serde_json::from_str::<serde::de::IgnoredAny>(text).is_err() {
            return ContentRegion::EmptySpan;
        }
        let Ok(canonical) = canonical_form(text) else {
            return ContentRegion::EmptySpan;
        };
        let mut hasher = ContentHasher::new();
        hasher.update(canonical.as_bytes());
        ContentRegion::Digest(under_region_tag(hasher))
    }

    fn meta_of(
        &self,
        _bytes: &[u8],
        declared_mime: Option<&MimeType>,
        _gate: GateOpen,
    ) -> MaterialMeta {
        // Unreachable through the port — the axis is not claimed — and
        // the marker is what the gate would have answered, so the two
        // doors agree even about a door that is never opened.
        material_meta::unsupported_format(declared_mime)
    }
}

/// Why a scan refused, distinguished for the tests rather than the
/// column — every arm stores [`ContentRegion::EmptySpan`].
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    /// A duplicate member name (parameter 4) — the one refusal the
    /// scanner finds that the parser accepts.
    DuplicateKey,
    /// The scan disagreed with text the parser passed. Unreachable in
    /// practice, and written out rather than unwrapped: the answer to
    /// "these bytes are not one JSON text" is a value already in hand,
    /// and panicking on untrusted input to save a line is how a parser
    /// earns its reputation.
    Broken,
}

/// The canonical form (module doc, parameters 2–6) of a text the
/// parser has already accepted.
fn canonical_form(text: &str) -> Result<String, Refusal> {
    let bytes = text.as_bytes();
    let mut at = 0usize;
    let mut out = String::with_capacity(text.len());
    skip_ws(bytes, &mut at);
    walk(bytes, &mut at, &mut out, 0)?;
    skip_ws(bytes, &mut at);
    if at != bytes.len() {
        return Err(Refusal::Broken);
    }
    Ok(out)
}

/// The recursion ceiling, matching the parser's own: the text was
/// accepted under `serde_json`'s default limit, so a scan that runs
/// past it is reading a different document than the one validated.
const MAX_DEPTH: usize = 128;

/// One value: dispatch on the first byte, per the grammar the parser
/// already enforced.
fn walk(bytes: &[u8], at: &mut usize, out: &mut String, depth: usize) -> Result<(), Refusal> {
    if depth >= MAX_DEPTH {
        return Err(Refusal::Broken);
    }
    match bytes.get(*at) {
        Some(b'{') => walk_object(bytes, at, out, depth),
        Some(b'[') => walk_array(bytes, at, out, depth),
        Some(b'"') => {
            let token = string_token(bytes, at)?;
            out.push_str(token);
            Ok(())
        }
        Some(_) => {
            let token = scalar_token(bytes, at)?;
            out.push_str(token);
            Ok(())
        }
        None => Err(Refusal::Broken),
    }
}

/// An object: members collected, sorted by decoded name (parameter 3),
/// refused on a duplicate (parameter 4), emitted with their name
/// tokens verbatim (parameter 5).
fn walk_object(
    bytes: &[u8],
    at: &mut usize,
    out: &mut String,
    depth: usize,
) -> Result<(), Refusal> {
    *at += 1; // consume `{`
    skip_ws(bytes, at);
    let mut members: Vec<(String, &str, String)> = Vec::new();
    if bytes.get(*at) == Some(&b'}') {
        *at += 1;
        out.push_str("{}");
        return Ok(());
    }
    loop {
        skip_ws(bytes, at);
        let name_token = string_token(bytes, at)?;
        // The decoded name is for ordering and collision only; what is
        // emitted is the token. Decoding is handed to the same crate
        // that validated the text, not spelled here a second time.
        let decoded: String = serde_json::from_str(name_token).map_err(|_| Refusal::Broken)?;
        skip_ws(bytes, at);
        if bytes.get(*at) != Some(&b':') {
            return Err(Refusal::Broken);
        }
        *at += 1;
        skip_ws(bytes, at);
        let mut value = String::new();
        walk(bytes, at, &mut value, depth + 1)?;
        members.push((decoded, name_token, value));
        skip_ws(bytes, at);
        match bytes.get(*at) {
            Some(b',') => *at += 1,
            Some(b'}') => {
                *at += 1;
                break;
            }
            _ => return Err(Refusal::Broken),
        }
    }
    members.sort_by(|a, b| a.0.cmp(&b.0));
    if members.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(Refusal::DuplicateKey);
    }
    out.push('{');
    for (wrote, (_, name, value)) in members.iter().enumerate() {
        if wrote > 0 {
            out.push(',');
        }
        out.push_str(name);
        out.push(':');
        out.push_str(value);
    }
    out.push('}');
    Ok(())
}

/// An array: order kept (parameter 6), whitespace dropped.
fn walk_array(bytes: &[u8], at: &mut usize, out: &mut String, depth: usize) -> Result<(), Refusal> {
    *at += 1; // consume `[`
    skip_ws(bytes, at);
    out.push('[');
    if bytes.get(*at) == Some(&b']') {
        *at += 1;
        out.push(']');
        return Ok(());
    }
    let mut wrote = 0usize;
    loop {
        skip_ws(bytes, at);
        if wrote > 0 {
            out.push(',');
        }
        walk(bytes, at, out, depth + 1)?;
        wrote += 1;
        skip_ws(bytes, at);
        match bytes.get(*at) {
            Some(b',') => *at += 1,
            Some(b']') => {
                *at += 1;
                out.push(']');
                return Ok(());
            }
            _ => return Err(Refusal::Broken),
        }
    }
}

/// A string token, quotes included, exactly as the source spells it.
///
/// The only string structure the scan needs is "a backslash takes the
/// next byte": that covers `\"` and `\\`, and `\uXXXX`'s four hex
/// digits are plain bytes once the `u` is skipped. Everything subtler —
/// which escapes exist, surrogate pairing, control-byte rejection — was
/// the parser's to enforce and already has been.
fn string_token<'a>(bytes: &'a [u8], at: &mut usize) -> Result<&'a str, Refusal> {
    if bytes.get(*at) != Some(&b'"') {
        return Err(Refusal::Broken);
    }
    let start = *at;
    *at += 1;
    while let Some(&byte) = bytes.get(*at) {
        *at += 1;
        match byte {
            b'\\' => *at += 1,
            b'"' => {
                let token = &bytes[start..*at];
                // The slice starts and ends at `"` so its boundaries are
                // character boundaries; the middle is the source's own
                // UTF-8, checked before the scan began.
                return std::str::from_utf8(token).map_err(|_| Refusal::Broken);
            }
            _ => {}
        }
    }
    Err(Refusal::Broken)
}

/// A number / `true` / `false` / `null` token, verbatim: everything up
/// to the next structural byte or whitespace.
fn scalar_token<'a>(bytes: &'a [u8], at: &mut usize) -> Result<&'a str, Refusal> {
    let start = *at;
    while let Some(&byte) = bytes.get(*at) {
        match byte {
            b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r' => break,
            _ => *at += 1,
        }
    }
    if start == *at {
        return Err(Refusal::Broken);
    }
    std::str::from_utf8(&bytes[start..*at]).map_err(|_| Refusal::Broken)
}

/// RFC 8259's four whitespace bytes, between tokens only — inside a
/// string they are content and [`string_token`] never comes here.
fn skip_ws(bytes: &[u8], at: &mut usize) {
    while let Some(b' ' | b'\t' | b'\n' | b'\r') = bytes.get(*at) {
        *at += 1;
    }
}

#[cfg(test)]
mod tests {
    use asterism_core::domain::probe::ProbeGates;

    use super::*;

    fn region(bytes: &[u8]) -> ContentRegion {
        let declared = MimeType::Json;
        JsonProbe.content(bytes, Some(&declared))
    }

    fn digest(bytes: &[u8]) -> String {
        match region(bytes) {
            ContentRegion::Digest(value) => value,
            other => panic!("expected a digest, got {other:?}"),
        }
    }

    /// **The region definition produces these exact digests.**
    ///
    /// The values freeze the *definition* (module doc, parameters 1–6),
    /// not one implementation of it: the scan underneath may be
    /// replaced and these values must not move. A change to one of
    /// these is a `cr2-` decision — a new prefix and a re-walk of every
    /// stored row — never a refactor. A diff that edits a literal here
    /// to make a test pass has inverted the reason they exist.
    const OBJECT_REGION: &str =
        "cr1-sha256:1c70ef10c8e063a72b77fc366355bc8f3fca296c3d43ae4215b0e1098afb339f";

    /// The same, for a document whose scalars would not survive a
    /// re-rendering serialiser: `1.50`, `-0.0`, an integer above 2^53.
    /// Freezing it means a walk that started parsing numbers is caught
    /// here rather than in a duplicate group nobody can undo.
    const SCALAR_REGION: &str =
        "cr1-sha256:3b6831db1cfb5d48d22a02e70bf94c5278eee4ec35689636c3f654ab7e6ab190";

    /// The same, for a bare scalar document — a `.json` whose whole
    /// text is one number. Without it the frozen set would only ever
    /// exercise the composite arms.
    const BARE_SCALAR_REGION: &str =
        "cr1-sha256:1a60b208ff491c3e2d21cdd5abb003e51e97b072efec59098863da45021de6a9";

    #[test]
    fn the_region_definition_produces_these_exact_digests() {
        assert_eq!(
            digest(r#"{"b":1,"a":{"y":true,"x":"A"},"list":[1.50,"é",null]}"#.as_bytes()),
            OBJECT_REGION
        );
        assert_eq!(
            digest(br#"{"big":9007199254740993,"neg":-0.0,"trail":1.50}"#),
            SCALAR_REGION
        );
        assert_eq!(digest(b"1.50"), BARE_SCALAR_REGION);
    }

    /// The axis's whole point: member order and inter-token whitespace
    /// are not part of the document.
    #[test]
    fn member_order_and_whitespace_do_not_reach_the_digest() {
        let reordered = digest(
            r#"
            {
              "list": [ 1.50, "é", null ],
              "a": { "x": "A", "y": true },
              "b": 1
            }
            "#
            .as_bytes(),
        );
        assert_eq!(reordered, OBJECT_REGION);
    }

    /// Parameter 5, held against the four collisions RFC 8785 buys
    /// (issue #16's table): each pair differs and must keep differing.
    #[test]
    fn scalar_tokens_are_the_source_s_own() {
        for (left, right) in [
            (
                &br#"{"id":9007199254740993}"#[..],
                &br#"{"id":9007199254740992}"#[..],
            ),
            (br#"{"a":-0.0}"#, br#"{"a":0}"#),
            (br#"{"n":1}"#, br#"{"n":1.0}"#),
            (br#"{"n":1.50}"#, br#"{"n":1.5}"#),
        ] {
            assert_ne!(digest(left), digest(right), "{left:?} vs {right:?}");
        }
    }

    /// Parameter 5's price, pinned so it reads as a decision: a name
    /// spelled by escape is a different token, so the documents differ
    /// — the false negative taken over a re-rendered false positive.
    ///
    /// The escape is assembled at runtime (`{"BSu0061":1}` with `BS`
    /// the backslash) rather than spelled in the literal, here and in
    /// the two tests below, so the source shows the intent and no
    /// string-escape layer between editor and test can decode it away.
    #[test]
    fn an_escape_spelled_name_is_its_own_token() {
        let escaped = format!(r#"{{"{}u0061":1}}"#, "\\");
        assert!(
            matches!(region(escaped.as_bytes()), ContentRegion::Digest(_)),
            "the escape-spelled document is valid on its own"
        );
        assert_ne!(digest(escaped.as_bytes()), digest(br#"{"a":1}"#));
    }

    /// Parameter 3: names sort by what they decode to, wherever the
    /// escape puts them lexically — and the token still lands verbatim.
    #[test]
    fn names_sort_decoded() {
        // `{"b":1,"a":2}`: the second member decodes ahead of the
        // first (`a` < `b`), while the first's token starts with a
        // backslash and would sort ahead of `"a"` compared raw.
        let escaped = format!(r#"{{"{}u0062":1,"a":2}}"#, "\\");
        let canonical = canonical_form(&escaped).expect("one valid document");
        assert_eq!(canonical, format!(r#"{{"a":2,"{}u0062":1}}"#, "\\"));
    }

    /// Parameter 6: an array is a sequence.
    #[test]
    fn array_order_reaches_the_digest() {
        assert_ne!(digest(b"[1,2]"), digest(b"[2,1]"));
    }

    /// Parameter 4, at the top and nested, and through the escape —
    /// the refusal is the marker, not a winner.
    #[test]
    fn a_duplicate_name_refuses_the_document() {
        for doc in [
            &br#"{"a":1,"a":2}"#[..],
            br#"{"outer":{"a":1,"a":2}}"#,
            br#"[{"k":0,"k":0}]"#,
        ] {
            assert_eq!(region(doc), ContentRegion::EmptySpan, "{doc:?}");
        }

        // Through the escape: the second member spells the first's
        // name by Unicode escape — the same name twice once decoded,
        // however differently the two tokens spell it.
        let escaped = format!(r#"{{"a":1,"{}u0061":2}}"#, "\\");
        assert_eq!(region(escaped.as_bytes()), ContentRegion::EmptySpan);
    }

    /// Parameter 1: not one JSON text, no region. The parser is the
    /// signature check, so every shape of disagreement lands on the
    /// same marker.
    #[test]
    fn anything_but_one_json_text_is_an_empty_span() {
        for doc in [
            &b""[..],
            b"   ",
            b"{",
            br#"{"a":}"#,
            b"1 2",
            br#"{"a":1} trailing"#,
            b"\xff\xfe",
            b"nul",
        ] {
            assert_eq!(region(doc), ContentRegion::EmptySpan, "{doc:?}");
        }
    }

    /// A document is any JSON value, not only an object (RFC 8259):
    /// the bare forms digest rather than refuse.
    #[test]
    fn a_bare_value_is_a_document() {
        for doc in [&b"true"[..], b"null", br#""hi""#, b"[]", b"{}", b"-12e3"] {
            assert!(matches!(region(doc), ContentRegion::Digest(_)), "{doc:?}");
        }
    }

    /// The canonical form itself, asserted once in the open so the
    /// digests above are checkable by eye: compact, sorted, tokens
    /// verbatim.
    #[test]
    fn the_canonical_form_reads_as_the_parameters_say() {
        let canonical = canonical_form(r#" { "b" : 1.50 , "a" : [ true, "x" ] , "c" : {} } "#)
            .expect("one valid document");
        assert_eq!(canonical, r#"{"a":[true,"x"],"b":1.50,"c":{}}"#);
    }

    /// The port refuses what the probe never claimed: another format,
    /// no format, and the meta axis of this one.
    #[test]
    fn the_gate_answers_for_everything_this_probe_declined() {
        use asterism_core::domain::content_region;

        let text = MimeType::parse("text/plain");
        assert_eq!(
            JsonProbe.content(b"{}", Some(&text)),
            content_region::unsupported_format(Some(&text))
        );
        assert_eq!(
            JsonProbe.content(b"{}", None),
            content_region::unsupported_format(None)
        );
        let json = MimeType::Json;
        assert_eq!(
            JsonProbe.meta(b"{}", Some(&json)),
            material_meta::unsupported_format(Some(&json))
        );
    }
}
