//! `a1111` — the line grammar of AUTOMATIC1111's `parameters` text,
//! and nothing about what any line means.
//!
//! The blob this reads is the one value that family writes: the prompt,
//! the negative prompt and the sampler settings in one line-oriented
//! text. The settings ride the last line as comma-separated `Key:
//! value` pairs, and a value that itself contains a comma or a colon is
//! double-quoted with backslash escapes — `Lora hashes: "name:
//! a1b2c3d4"` is one pair, not two. That quoting rule is the whole
//! reason this is a tokeniser rather than two `split` calls.
//!
//! # A pure function of a string, with no judgement
//!
//! The same contract as the rest of this crate, one level up from
//! bytes: boundaries out, judgement left to the caller. Which keys
//! matter — whether `Model` outranks `Model hash`, what an empty
//! `Seed` means — is the caller's question, and answering it here
//! would put a registry's opinions into a grammar
//! ([`png`](crate::png)'s module doc on why the judgement stayed
//! behind when the parsing moved).
//!
//! # Refusal is all-or-nothing
//!
//! A line is a settings line when **every** top-level comma-separated
//! part carries a `key: value` split. A prompt that happens to end the
//! blob and happens to contain a colon does not half-parse into a
//! plausible pair list — one part without a separator refuses the whole
//! line, and the caller sees no settings rather than wrong ones.

/// The settings pairs of a `parameters` blob, in the order written.
///
/// The last line of the blob, tokenised — that is where the family
/// writes them. An empty result is a blob with no settings line: a
/// prompt alone, an empty string, or a last line that refuses to parse
/// (module docs). Duplicate keys are preserved as written; deciding
/// between them is judgement, not grammar.
pub fn settings(blob: &str) -> Vec<(String, String)> {
    blob.lines().next_back().and_then(pairs).unwrap_or_default()
}

/// One line as `key: value` pairs, or `None` where any part refuses.
fn pairs(line: &str) -> Option<Vec<(String, String)>> {
    split_top_level(line)?
        .into_iter()
        .map(|part| {
            let (key, value) = part.split_once(':')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), unquote(value.trim())))
        })
        .collect()
}

/// Splits on commas that are not inside a double-quoted value.
///
/// Inside quotes a backslash escapes the next character, which is how
/// the writer keeps a literal quote in a value — the same convention
/// the unquoting below undoes.
fn split_top_level(line: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            '\\' if in_quotes => {
                current.push(c);
                current.push(chars.next()?);
            }
            ',' if !in_quotes => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    // A quote still open at the end of the line is a value the writer
    // never finished; the line is not the shape it claims.
    if in_quotes {
        return None;
    }
    parts.push(current);
    Some(parts)
}

/// Undoes the writer's quoting: outer quotes off, `\"` and `\\` back to
/// the character they carried. A value that was never quoted is
/// returned as it stands.
fn unquote(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(escaped @ ('"' | '\\')) => out.push(escaped),
                // An escape this grammar does not know is kept as
                // written rather than guessed at.
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// The ordinary settings line, as the family writes it under the
    /// prompt and the negative prompt.
    #[test]
    fn settings_reads_the_last_line_as_comma_separated_pairs() {
        let blob = "1girl, purple eyes\nNegative prompt: blurry\n\
                    Steps: 28, Sampler: Euler a, CFG scale: 7, Seed: 12345, \
                    Size: 512x768, Model hash: abc123def, Model: cetus-mix_v4";
        assert_eq!(
            settings(blob),
            owned(&[
                ("Steps", "28"),
                ("Sampler", "Euler a"),
                ("CFG scale", "7"),
                ("Seed", "12345"),
                ("Size", "512x768"),
                ("Model hash", "abc123def"),
                ("Model", "cetus-mix_v4"),
            ])
        );
    }

    /// The case the quoting rule exists for: a value carrying commas
    /// and colons stays one pair, and its escapes decode.
    #[test]
    fn a_quoted_value_keeps_its_commas_and_colons_inside_one_pair() {
        let line = r#"Steps: 20, Lora hashes: "client-name: a1b2, other: c3d4", Version: v1.6.0"#;
        assert_eq!(
            settings(line),
            owned(&[
                ("Steps", "20"),
                ("Lora hashes", "client-name: a1b2, other: c3d4"),
                ("Version", "v1.6.0"),
            ])
        );

        let escaped = r#"Title: "a \"quoted\" word, kept", Steps: 1"#;
        assert_eq!(
            settings(escaped),
            owned(&[("Title", r#"a "quoted" word, kept"#), ("Steps", "1")])
        );
    }

    /// One part without a separator refuses the whole line — a prompt
    /// with a colon in it does not half-parse into settings.
    #[test]
    fn a_line_that_is_not_all_pairs_yields_no_settings() {
        for blob in [
            "a photo of x: y, plain tail",
            "1girl, purple eyes",
            "",
            "score: 9, : empty key",
            r#"Steps: 20, Lora hashes: "never closed"#,
        ] {
            assert_eq!(settings(blob), Vec::new(), "for {blob:?}");
        }
    }

    /// A blob whose last line is the negative prompt still tokenises —
    /// it is shaped like a pair — and which keys matter is exactly the
    /// judgement this grammar does not hold.
    #[test]
    fn the_grammar_has_no_opinion_about_which_keys_matter() {
        let blob = "a cat\nNegative prompt: a dog";
        assert_eq!(settings(blob), owned(&[("Negative prompt", "a dog")]));
    }

    /// Duplicate keys are the writer's statement, not this layer's to
    /// collapse.
    #[test]
    fn duplicate_keys_are_preserved_in_written_order() {
        assert_eq!(
            settings("Seed: 1, Seed: 2"),
            owned(&[("Seed", "1"), ("Seed", "2")])
        );
    }
}
