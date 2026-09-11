//! Text `RawItem` → `Footprint::Doc` parser.
//!
//! The capability this crate is the door for already existed: hand
//! `asset_add` a path to a `.md` and `guess_mime` answers `text/plain`,
//! `MimeType::body_text` accepts that, and the words reach the body
//! cache and the full-text index. What was missing was a route that
//! files a written file as a *document* — the media routes take a file
//! whole and call it an image, a clip, a recording; the tape route
//! takes one whole and calls it a terminal transcript; nothing took one
//! and called it something somebody wrote.
//!
//! It shares `.txt` with that tape route, and the overlap is not an
//! accident to resolve: a transcript and a note are different things
//! wearing one extension, and which one a directory holds is the
//! caller's to say by picking the subcommand.
//!
//! What this reads out of the bytes is only what a card needs before
//! the body is read: a heading, an excerpt, and a word count. The body
//! itself is never sent — the server reads it off the locator, which is
//! also what keeps a document that changes on disk from having two
//! answers.

use std::path::PathBuf;

use asterism_importer_sdk::{
    Doc, DocFormat, Footprint, FootprintSource, ParseError, RawItem, SourceParser,
};
use chrono::Utc;
use serde_json::json;

/// How much of the document to carry as the excerpt.
///
/// The SDK truncates again to `COVER_MAX_CHARS` on the way to the
/// spec, so this is not the display rule; it is how much text to walk
/// before stopping. Generous enough that the cover is never short
/// because this was, and bounded so a large document is not copied into
/// a footprint on its way to a column that holds a sentence.
const EXCERPT_SCAN_CHARS: usize = 2_000;

/// Turns one scanned document into `Footprint::Doc`.
pub struct TextParser {
    platform: Option<String>,
}

impl TextParser {
    pub fn new(platform: Option<String>) -> Self {
        Self { platform }
    }
}

impl SourceParser for TextParser {
    fn parse(&self, item: RawItem) -> Result<Vec<Footprint>, ParseError> {
        let path = PathBuf::from(&item.locator);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let parent_dir = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());

        // Lossy, deliberately, and it is the same reading the server
        // will do: `source_text` decodes the body with
        // `String::from_utf8_lossy` when it fills the body cache. A
        // parser that refused a file on an invalid byte would be a
        // stricter gate than the reader behind it, and would refuse a
        // document that `asset_add` accepts by hand.
        let text = String::from_utf8_lossy(&item.payload);

        let format = match extension.as_deref() {
            Some("md") => DocFormat::Markdown,
            // `.txt` and everything else. Plain is the honest reading of
            // a file whose format nothing here knows, and the scanner
            // offers this parser only the two, so the arm is the
            // ordinary `.txt` case and a caller pointing it somewhere by
            // hand at the same time.
            _ => DocFormat::Plain,
        };

        // Front matter is stepped over once, here, so the heading and
        // the excerpt read the same document as each other.
        let body = without_front_matter(&text);
        let title = heading(body)
            .map(str::to_string)
            .unwrap_or_else(|| stem.clone());
        let excerpt = excerpt(body);
        // Counted over the whole file rather than the body: front
        // matter is words somebody wrote in the document, and a count
        // that disagreed with `wc` would be a third opinion nobody
        // asked for.
        let word_count = text.split_whitespace().count() as u64;

        let file_size_bytes = item
            .extra
            .get("file_size_bytes")
            .and_then(|v| v.as_u64())
            .or(Some(item.payload.len() as u64));

        let mut labels = vec!["document".to_string()];
        if let Some(ext) = &extension {
            labels.push(ext.clone());
        }
        if !parent_dir.is_empty() {
            labels.push(parent_dir.clone());
        }

        let extra = json!({
            "filename": path.file_name().map(|s| s.to_string_lossy().to_string()),
            "parent_dir": parent_dir,
            "word_count": word_count,
        });

        Ok(vec![Footprint::Doc(Doc {
            source: FootprintSource {
                kind: item.source_kind,
                locator: item.locator,
                external_id: None,
                platform: self.platform.clone(),
            },
            // A document that came back from an outside generator lands
            // here as an ordinary document: `Doc` carries no
            // `derived_from`, which the SDK places on the three media
            // variants and says why beside them.
            occurred_at: item.occurred_at.unwrap_or_else(Utc::now),
            occurred_source: Default::default(),
            title: Some(title),
            excerpt,
            format,
            bundle_id: None,
            file_size_bytes,
            word_count: Some(word_count),
            labels,
            extra,
        })])
    }
}

/// Everything after a YAML front-matter block, or the whole document
/// when there is none.
///
/// A vault's `.md` opens with one, and without this step it is the
/// document: the first non-blank line is `---`, so no heading is found,
/// and the cover becomes `--- title: … tags: […] ---` while the `#`
/// below is never reached. Stepping over it is what makes the ordinary
/// Obsidian / Hugo / Jekyll file read like the plain one beside it.
///
/// The block is recognised, not parsed: a first non-blank line of
/// exactly `---` and everything through the next line of exactly `---`.
/// What the keys say is not read — a `title:` in there is the vault's
/// vocabulary, and reading it would make this parser the second
/// authority on what a document is called.
///
/// An unterminated opener is not front matter. A document that starts
/// with a horizontal rule keeps all of itself rather than losing the
/// remainder to a block that never closed.
fn without_front_matter(text: &str) -> &str {
    let mut rest = text.trim_start_matches(['\n', '\r']);
    let first_end = rest.find('\n').unwrap_or(rest.len());
    if rest[..first_end].trim_end() != "---" {
        return text;
    }
    rest = &rest[first_end.min(rest.len())..];
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        if line.trim_end() == "---" {
            return &rest[offset..];
        }
    }
    text
}

/// The document's own title, when it states one on its first non-blank
/// line.
///
/// One line and one syntax: an ATX heading (`#` through `######`)
/// before any prose. Setext headings are not read — the fallback is the
/// file stem, which is a name somebody chose, so the cost of not
/// reading a second form is small and the cost of half-reading Markdown
/// is a second opinion about what Markdown is.
///
/// A file that opens with prose has no heading here even if its first
/// line reads like one, because a line of prose is not a claim about
/// itself.
fn heading(text: &str) -> Option<&str> {
    let first = text.lines().find(|line| !line.trim().is_empty())?;
    let trimmed = first.trim();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = trimmed[hashes..].trim();
    // `#hashtag` is not a heading: ATX wants a space after the run.
    if rest.is_empty() || !trimmed[hashes..].starts_with(char::is_whitespace) {
        return None;
    }
    Some(rest)
}

/// The prose a card shows: the first paragraph after the heading, if
/// the heading is one this reader took.
///
/// Walked rather than sliced whole: a document can be long and this
/// value is on its way to a column that holds a sentence.
fn excerpt(text: &str) -> String {
    let mut lines = text.lines().peekable();
    // Step over leading blanks and the heading, if the heading is what
    // `heading` above would have taken.
    while let Some(line) = lines.peek() {
        if line.trim().is_empty() {
            lines.next();
            continue;
        }
        break;
    }
    if heading(text).is_some() {
        lines.next();
    }
    let mut out = String::new();
    let mut budget = EXCERPT_SCAN_CHARS;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            if out.is_empty() {
                continue;
            }
            break;
        }
        if !out.is_empty() && budget > 0 {
            out.push(' ');
            budget -= 1;
        }
        // Taken to the budget rather than pushed whole and measured
        // after. A document with no line breaks is one line, and a
        // check that ran between lines would never run: the first push
        // would be the whole file, on its way to a column that holds a
        // sentence.
        for ch in line.chars() {
            if budget == 0 {
                return out;
            }
            out.push(ch);
            budget -= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn item(locator: &str, body: &str) -> RawItem {
        RawItem {
            source_kind: "fs".into(),
            locator: locator.into(),
            payload: body.as_bytes().to_vec(),
            occurred_at: None,
            extra: json!({}),
        }
    }

    fn parse(locator: &str, body: &str) -> Doc {
        let out = TextParser::new(Some("test".into()))
            .parse(item(locator, body))
            .expect("parse ok");
        match out.into_iter().next().expect("one Doc footprint") {
            Footprint::Doc(d) => d,
            other => panic!("expected a Doc, got {other:?}"),
        }
    }

    #[test]
    fn a_markdown_file_takes_its_title_from_its_first_heading() {
        let d = parse(
            "/notes/plan.md",
            "# The plan\n\nFirst we look, then we decide.\n\nA second paragraph.\n",
        );
        assert_eq!(d.title.as_deref(), Some("The plan"));
        assert_eq!(d.excerpt, "First we look, then we decide.");
        assert!(d.labels.iter().any(|l| l == "document"));
        assert!(d.labels.iter().any(|l| l == "md"));
    }

    #[test]
    fn a_file_that_opens_with_prose_is_titled_by_its_name() {
        // A first line that reads like a title is still prose, and the
        // stem is a name somebody chose.
        let d = parse("/notes/Grocery list.txt", "Milk, bread, and coffee.\n");
        assert_eq!(d.title.as_deref(), Some("Grocery list"));
        assert_eq!(d.excerpt, "Milk, bread, and coffee.");
    }

    #[test]
    fn a_hashtag_is_not_a_heading() {
        // ATX wants whitespace after the run of hashes. Without this a
        // note opening on a tag would lose its first line to the title
        // and show the second as its cover.
        let d = parse("/notes/tagged.md", "#tagged\n\nThe body.\n");
        assert_eq!(d.title.as_deref(), Some("tagged"), "the stem, not the line");
        assert_eq!(
            d.excerpt, "#tagged",
            "the line stays in the prose, since nothing took it as a title — and \
             the excerpt is the first paragraph, which that line is the whole of"
        );
    }

    #[test]
    fn the_extension_decides_the_format_label() {
        assert_eq!(
            parse("/a/x.md", "# t\n\nbody\n").format,
            DocFormat::Markdown
        );
        assert_eq!(parse("/a/x.txt", "body\n").format, DocFormat::Plain);
    }

    #[test]
    fn invalid_utf8_is_read_the_way_the_server_will_read_it() {
        // Lossy rather than refused: `source_text` decodes the body the
        // same way when it fills the cache, so refusing here would turn
        // away a document the rest of the app can hold.
        let mut bytes = b"# Caf\xe9\n\nbody\n".to_vec();
        bytes.push(b'\n');
        let out = TextParser::new(None)
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: "/notes/cafe.md".into(),
                payload: bytes,
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Doc(d) = &out[0] else {
            panic!("expected a Doc")
        };
        assert!(d.title.as_deref().is_some_and(|t| t.starts_with("Caf")));
    }

    /// The count travels in the extension bag, and only there.
    ///
    /// `Doc::word_count` is a field `doc_to_spec` drops — `AssetSpec`
    /// has nowhere to put it — so the bag is what survives the trip. It
    /// is set on both because the footprint is the thing a future spec
    /// field would read, and the bag is the thing a reader has today.
    #[test]
    fn the_word_count_reaches_the_bag() {
        let d = parse("/notes/count.md", "# Title\n\none two three four\n");
        assert_eq!(
            d.word_count,
            Some(6),
            "six whitespace-separated tokens, of which the bare `#` is one — \
             a word here is what split_whitespace says it is, not what a \
             typesetter would count"
        );
        assert_eq!(
            d.extra.get("word_count").and_then(Value::as_u64),
            d.word_count,
            "word_count reaches the bag verbatim"
        );
    }

    #[test]
    fn front_matter_is_stepped_over_rather_than_shown() {
        // The ordinary vault file. Without this the cover is the YAML
        // and the heading below it is never reached.
        let d = parse(
            "/vault/note.md",
            "---\ntitle: Quarterly plan\ntags: [work, q3]\n---\n\n# The plan\n\nFirst we look.\n",
        );
        assert_eq!(d.title.as_deref(), Some("The plan"));
        assert_eq!(d.excerpt, "First we look.");
        assert_eq!(
            d.word_count,
            Some(14),
            "the count is over the whole file, both `---` fences and the YAML included"
        );
    }

    #[test]
    fn an_unterminated_opener_is_not_front_matter() {
        // A document that opens on a horizontal rule keeps all of
        // itself rather than losing the remainder to a block that never
        // closed.
        let d = parse("/vault/rule.md", "---\n\nJust prose under a rule.\n");
        assert_eq!(d.title.as_deref(), Some("rule"));
        assert_eq!(d.excerpt, "---");
    }

    #[test]
    fn one_long_line_is_cut_to_the_budget() {
        // The check used to run between lines, which on a document with
        // no line breaks meant never: the first push was the whole
        // file.
        let long = "x".repeat(EXCERPT_SCAN_CHARS * 3);
        let d = parse("/notes/one-line.txt", &long);
        assert_eq!(d.excerpt.chars().count(), EXCERPT_SCAN_CHARS);
    }

    #[test]
    fn an_empty_document_still_lands() {
        // Nothing to read is not a parse failure: the row is the fact
        // that the file is in the library, and a card with an empty
        // cover is a truthful card.
        let d = parse("/notes/blank.md", "\n\n");
        assert_eq!(d.title.as_deref(), Some("blank"));
        assert_eq!(d.excerpt, "");
        assert_eq!(d.word_count, Some(0));
    }
}
