//! The XMP packet — the half of the disclosure that needs no
//! certificate.
//!
//! IPTC Extension properties can only be carried in XMP; there is no
//! EXIF or IIM spelling of them. So this packet is the entire IPTC side
//! of the acceptance criteria, and it is the side that works today,
//! unsigned, on a machine with no signing identity configured.
//!
//! # Why this is written by hand rather than by an RDF library
//!
//! An XMP packet is RDF/XML, and a general RDF serialiser would be
//! entitled to emit any of several equivalent forms — properties as
//! attributes instead of elements, a different namespace prefix, a
//! different node ordering. Equivalent to a parser, not equivalent here:
//! the bytes go inside a C2PA hard binding (see the module docs on
//! ordering), so *which* equivalent form is produced decides whether a
//! signature made over one rendering still verifies against another. A
//! packet this crate writes has to be a function of the record and
//! nothing else, which is what a hand-written template is and a
//! serialiser is not obliged to be.
//!
//! The scope also does not justify a dependency: five properties, all
//! simple text or a URI, all in one namespace, none of them a container
//! (no `rdf:Alt` / `rdf:Bag` / `rdf:Seq`), no language alternatives.
//!
//! # Ordering against the C2PA manifest
//!
//! **The packet is written before the manifest is signed, never after.**
//! The manifest's hard binding covers the XMP, so editing the packet
//! afterwards invalidates the signature. This is not a deduction: IPTC's
//! own 2025.1 announcement carries a worked example whose caption
//! records that adding the new AI properties invalidated the C2PA
//! metadata that was already in the file. The apply path enforces the
//! order, and its tests are what hold it.
//!
//! # No padding
//!
//! The XMP specification suggests trailing whitespace so a packet can be
//! rewritten in place without moving the bytes after it. This writes
//! none, and marks the packet read-only (`<?xpacket end="r"?>`)
//! accordingly. In-place rewriting is not something that can happen to a
//! signed file anyway — any edit breaks the binding — and JPEG's APP1
//! segment has 65,533 bytes for the whole packet, which is a budget a
//! generator prompt can exhaust on its own. Spending part of it on
//! padding for an update path that cannot exist would be paying twice.

use crate::record::DisclosureRecord;

/// The IPTC Extension namespace, unchanged since 2008 and still the
/// namespace the 2025.1 AI properties were added to.
pub const IPTC_EXT_NS: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";

/// The prefix IPTC's own specification uses for [`IPTC_EXT_NS`].
///
/// A prefix is nominally arbitrary in XML, and this one is not treated
/// as such: readers in the wild pattern-match on `Iptc4xmpExt:` rather
/// than resolving the namespace, so a technically-equivalent prefix is
/// a practically-invisible packet.
const IPTC_EXT_PREFIX: &str = "Iptc4xmpExt";

/// The packet identifier the XMP specification fixes. Not a checksum and
/// not ours to choose — a scanner looking for an XMP packet in a file
/// looks for exactly this string.
const PACKET_ID: &str = "W5M0MpCehiHzreSzNTczkc9d";

/// Renders the record's IPTC properties as an XMP packet.
///
/// Returns `None` when the record has nothing to disclose: an empty
/// packet is a file modification that buys nothing, and "nothing was
/// established about this file" is a normal outcome rather than an
/// error ([`DisclosureRecord::discloses_anything`]).
pub fn render(record: &DisclosureRecord) -> Option<String> {
    if !record.discloses_anything() {
        return None;
    }

    let mut properties = String::new();
    // `DigitalSourceType` is typed as a URI by the IPTC specification,
    // which is why the value written is `uri()` and not the short term.
    if let Some(source_type) = record.source_type {
        push_property(&mut properties, "DigitalSourceType", source_type.uri());
    }
    if let Some(system) = &record.ai_system {
        push_property(&mut properties, "AISystemUsed", system);
    }
    if let Some(version) = &record.ai_system_version {
        push_property(&mut properties, "AISystemVersionUsed", version);
    }
    if let Some(prompt) = &record.prompt {
        push_property(&mut properties, "AIPromptInformation", prompt);
    }
    if let Some(writer) = &record.prompt_writer {
        push_property(&mut properties, "AIPromptWriterName", writer);
    }

    // `x:xmptk` names the toolkit that wrote the packet. It is
    // documentation for whoever opens the file, not an identifier
    // anything resolves.
    //
    // The name without a version, for two reasons that happen to agree.
    // The build version here would be the same `0.0.0` every crate in
    // this workspace carries, so it would say the same thing in every
    // file ever written — and these bytes go inside the C2PA hard
    // binding, which makes it the same uncorrectable claim the manifest
    // declines to make in `claim_generator_info`. It would also be the
    // one thing in this packet that is not read off the record, which
    // the module doc says nothing here may be: two stamps of one
    // unchanged record would render different bytes across a version
    // bump, for a difference nobody stated.
    let toolkit = "asterism";
    Some(format!(
        "<?xpacket begin=\"\u{feff}\" id=\"{PACKET_ID}\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"{toolkit}\">\n\
         \x20<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         \x20 <rdf:Description rdf:about=\"\"\n\
         \x20  xmlns:{IPTC_EXT_PREFIX}=\"{IPTC_EXT_NS}\">\n\
         {properties}\
         \x20 </rdf:Description>\n\
         \x20</rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"r\"?>"
    ))
}

/// Appends one simple text/URI property element.
fn push_property(out: &mut String, name: &str, value: &str) {
    out.push_str("   <");
    out.push_str(IPTC_EXT_PREFIX);
    out.push(':');
    out.push_str(name);
    out.push('>');
    escape_into(out, value);
    out.push_str("</");
    out.push_str(IPTC_EXT_PREFIX);
    out.push(':');
    out.push_str(name);
    out.push_str(">\n");
}

/// Writes `value` as XML character data.
///
/// Three escapes and one filter. The escapes are the two that XML
/// requires in element content (`&`, `<`) plus `>`, which is only
/// required inside `]]>` but is conventionally escaped everywhere and
/// costs two bytes to be unconditionally safe.
///
/// The filter is the part worth explaining. XML 1.0 admits no C0 control
/// character except tab, newline and carriage return — not even as a
/// numeric reference — so a value carrying one cannot be represented at
/// all, and a packet containing one is refused by every conforming
/// parser rather than being read leniently. These values come out of
/// containers written by other people's tooling: a prompt chunk is
/// whatever bytes a generator put there. Dropping the character makes a
/// packet that parses and a value that differs from the source by a
/// character no reader could have displayed; keeping it makes a file
/// whose entire metadata block is unreadable. The first is the smaller
/// loss, and it is confined to values nothing legitimate produces.
fn escape_into(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\t' | '\n' | '\r' => out.push(ch),
            // C0 controls other than the three above, and DEL's
            // companion block C1, which XML 1.0 admits but which no
            // metadata value has a use for.
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_type::DigitalSourceType;

    fn full_record() -> DisclosureRecord {
        DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_ai_system("ComfyUI", Some("0.3.0".into()))
            .with_prompt("1girl, purple eyes", Some("owner".into()))
    }

    #[test]
    fn a_record_with_nothing_to_disclose_renders_no_packet() {
        assert_eq!(render(&DisclosureRecord::for_asset("asset-1")), None);
    }

    #[test]
    fn the_packet_carries_every_2025_1_property_under_the_iptc_namespace() {
        let packet = render(&full_record()).expect("a full record renders");
        assert!(packet.contains(IPTC_EXT_NS), "namespace is declared");
        assert!(packet.contains(
            "<Iptc4xmpExt:DigitalSourceType>\
             http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia\
             </Iptc4xmpExt:DigitalSourceType>"
        ));
        assert!(packet.contains("<Iptc4xmpExt:AISystemUsed>ComfyUI</Iptc4xmpExt:AISystemUsed>"));
        assert!(
            packet.contains(
                "<Iptc4xmpExt:AISystemVersionUsed>0.3.0</Iptc4xmpExt:AISystemVersionUsed>"
            )
        );
        assert!(packet.contains(
            "<Iptc4xmpExt:AIPromptInformation>1girl, purple eyes\
             </Iptc4xmpExt:AIPromptInformation>"
        ));
        assert!(
            packet
                .contains("<Iptc4xmpExt:AIPromptWriterName>owner</Iptc4xmpExt:AIPromptWriterName>")
        );
    }

    #[test]
    fn the_source_type_is_written_as_a_uri_not_as_the_short_term() {
        // IPTC types this property as a URI. A bare term here is a
        // malformed value that still looks right in a diff.
        let packet = render(&full_record()).unwrap();
        assert!(!packet.contains(">trainedAlgorithmicMedia<"));
    }

    #[test]
    fn the_packet_is_wrapped_in_the_markers_a_scanner_looks_for() {
        let packet = render(&full_record()).unwrap();
        assert!(
            packet.starts_with("<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>")
        );
        assert!(
            packet.ends_with("<?xpacket end=\"r\"?>"),
            "read-only: this writes no padding, so claiming to be writable would be false"
        );
    }

    #[test]
    fn absent_fields_produce_no_elements_rather_than_empty_ones() {
        // An empty element is a stated blank value. The distinction is
        // the same one the record makes between `None` and a value, and
        // erasing it here would make every export claim an empty prompt.
        let packet = render(
            &DisclosureRecord::for_asset("asset-1")
                .with_source_type(DigitalSourceType::DigitalCapture),
        )
        .unwrap();
        assert!(!packet.contains("AIPromptInformation"));
        assert!(!packet.contains("AISystemUsed"));
    }

    #[test]
    fn a_prompt_with_markup_characters_cannot_break_out_of_its_element() {
        // Prompts are arbitrary text from outside. Left unescaped, a `<`
        // makes the whole packet unparseable, which takes the disclosure
        // down with it.
        let packet = render(
            &DisclosureRecord::for_asset("asset-1")
                .with_prompt("a & b <tag> \"quoted\" 'single'", None),
        )
        .unwrap();
        assert!(packet.contains(
            "<Iptc4xmpExt:AIPromptInformation>\
             a &amp; b &lt;tag&gt; \"quoted\" 'single'\
             </Iptc4xmpExt:AIPromptInformation>"
        ));
    }

    #[test]
    fn control_characters_are_dropped_rather_than_making_the_packet_unparseable() {
        // XML 1.0 cannot represent a NUL at all — not even as `&#0;` —
        // so the choice is between losing the character and losing the
        // packet.
        let packet =
            render(&DisclosureRecord::for_asset("a").with_prompt("before\u{0}\u{1}after", None))
                .unwrap();
        assert!(packet.contains(">beforeafter<"));
        assert!(!packet.contains('\u{0}'));

        // Tab / newline / carriage return are legal and are what a
        // multi-line prompt is made of.
        let kept = render(&DisclosureRecord::for_asset("a").with_prompt("a\nb\tc", None)).unwrap();
        assert!(kept.contains("a\nb\tc"));
    }

    #[test]
    fn rendering_is_a_function_of_the_record_and_nothing_else() {
        // The bytes go inside a C2PA hard binding, so two renderings of
        // one record have to be the same bytes or a signature made over
        // the first stops verifying against the second.
        let record = full_record();
        assert_eq!(render(&record), render(&record.clone()));

        // Comparing two renderings inside one build cannot see the way
        // this property was actually broken: the toolkit string carried
        // `env!("CARGO_PKG_VERSION")`, so the packet was a function of
        // the record *and the build*, and both sides of the equality
        // above moved together across a version bump. The attribute is
        // pinned literally instead, since that is the input from
        // outside the record that was there.
        assert!(
            render(&record).unwrap().contains(r#"x:xmptk="asterism""#),
            "the toolkit string is the one thing here not read off the \
             record, so it may not carry anything that varies"
        );
    }
}
