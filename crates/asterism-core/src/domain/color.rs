//! `ColorBucket` — the closed set of colours the palette facet filters
//! on.
//!
//! Assets already carry a five-entry dominant-colour palette (extracted
//! from the 128 px thumbnail by the `thumb_gen` job). Those are exact
//! hex values, which is the wrong shape for a filter: no two photos
//! share a hex, so "show me the red ones" cannot be answered by
//! equality, and answering it by distance would mean scanning every
//! palette on every query and exposing a threshold knob nobody wants to
//! tune.
//!
//! So the palette is quantised once, at extraction time, into this
//! closed set. A bucket is indexable (the sidebar swatch is an equality
//! predicate), countable (the facet shows how many assets carry each
//! colour, like the FORMAT section next to it), and stable (the same
//! hex always lands in the same bucket, so the derived
//! `asset_color` rows can be rebuilt from `asset.palette` at any time).
//!
//! Buckets are a *view* of the palette, never the source of truth —
//! `asset.palette` stays canonical, `asset_color` is a projection.

use crate::error::DomainError;

/// A quantised palette colour — one sidebar swatch.
///
/// Eight chromatic bands plus `Brown` (dark orange, too common in
/// photographs to leave folded into Orange) and three neutrals. The set
/// is closed: a hex that fails to parse produces no bucket rather than
/// a fallback one, because a wrong bucket is worse than a missing one
/// (it puts an asset under a swatch the user did not see it as).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ColorBucket {
    /// Hue ≈ 345–15°.
    Red,
    /// Hue ≈ 15–45°, light enough not to read as brown.
    Orange,
    /// Hue ≈ 45–70°.
    Yellow,
    /// Hue ≈ 70–165° (the widest band — human vision separates greens
    /// least by name).
    Green,
    /// Hue ≈ 165–195°.
    Cyan,
    /// Hue ≈ 195–255°.
    Blue,
    /// Hue ≈ 255–290°.
    Purple,
    /// Hue ≈ 290–345° (includes magenta).
    Pink,
    /// Dark orange / ochre — skin, wood, coffee, sepia.
    Brown,
    /// Near-white, chromatic or not.
    White,
    /// Desaturated mid-tone.
    Gray,
    /// Near-black, chromatic or not.
    Black,
}

impl ColorBucket {
    /// Every bucket in swatch order: the chromatic wheel first (red →
    /// pink), then brown, then the neutrals light-to-dark. The order is
    /// the display order — the sidebar renders `ALL` verbatim so the
    /// swatch row does not jump around as counts change.
    pub const ALL: [ColorBucket; 12] = [
        ColorBucket::Red,
        ColorBucket::Orange,
        ColorBucket::Yellow,
        ColorBucket::Green,
        ColorBucket::Cyan,
        ColorBucket::Blue,
        ColorBucket::Purple,
        ColorBucket::Pink,
        ColorBucket::Brown,
        ColorBucket::White,
        ColorBucket::Gray,
        ColorBucket::Black,
    ];

    /// Wire / storage spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Cyan => "cyan",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Pink => "pink",
            Self::Brown => "brown",
            Self::White => "white",
            Self::Gray => "gray",
            Self::Black => "black",
        }
    }

    /// Parses a caller-supplied bucket slug, case-insensitively.
    ///
    /// `Err` carries the accepted values so a typo answers itself —
    /// the same contract as
    /// [`DiagLevel::parse`](asterism_contract::query::DiagLevel::parse).
    /// Rejecting rather than defaulting matters here: an unknown slug
    /// silently treated as "no filter" would show the whole grid and
    /// read as "this colour matches everything".
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let needle = raw.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|b| b.as_str() == needle)
            .ok_or_else(|| {
                let accepted: Vec<&str> = Self::ALL.iter().map(|b| b.as_str()).collect();
                DomainError::Validation(format!(
                    "unknown colour bucket: {raw:?} (expected one of {})",
                    accepted.join(", ")
                ))
            })
    }
}

/// Quantises one palette entry (`#rrggbb`, leading `#` optional, case
/// insensitive) into its bucket.
///
/// `None` means "not a colour this function can read" — a malformed
/// hex. Every well-formed hex lands somewhere, so a palette of five
/// valid entries always yields between one and five buckets.
///
/// The classification asks four questions in order:
///
/// 1. **Is it effectively colourless?** Very light, very dark, or
///    barely-tinted values are neutrals regardless of their hue — the
///    hue of `#fefdff` is meaningless to the eye.
/// 2. **Is it brown?** Dark orange is a colour of its own to a viewer,
///    and photographs are full of it. Folding it into Orange would make
///    that swatch mean "everything with skin or wood in it".
/// 3. **Is it a pale red?** Light red is pink in everyday naming, and
///    the magenta-side Pink band would never catch it.
/// 4. **Otherwise, which hue band?**
///
/// The colourless test reads **chroma** (`max - min` of the raw
/// channels), not HSL saturation. HSL normalises saturation by
/// `1 - |2·l - 1|`, which collapses toward zero near white and black
/// and inflates `s` to compensate: at `l = 0.94` a channel spread of
/// three parts in 255 is already "fully saturated" by that measure.
/// Keyed on `s`, cream (`#f5f0e8`) and off-white (`#eef0f5`) come out
/// Orange and Blue — the two swatches nobody would look for a nearly
/// white wall under. Chroma is lightness-independent and says what the
/// eye says: almost no colour here.
pub fn bucket_of(hex: &str) -> Option<ColorBucket> {
    let (r, g, b) = parse_hex(hex)?;
    let (h, chroma, l) = to_hcl(r, g, b);

    // 1. Neutrals. The lightness cutoffs come first and are absolute:
    //    a saturated hue at 2 % lightness is black on screen, and
    //    filing it under Red would put a photograph of a night sky in
    //    the red swatch.
    if l >= 0.95 {
        return Some(ColorBucket::White);
    }
    if l <= 0.08 {
        return Some(ColorBucket::Black);
    }
    if chroma < 0.10 {
        return Some(match l {
            l if l >= 0.85 => ColorBucket::White,
            l if l <= 0.18 => ColorBucket::Black,
            _ => ColorBucket::Gray,
        });
    }

    // 2. Brown — the dark end of the red-orange-to-yellow-orange span.
    if (15.0..50.0).contains(&h) && l < 0.35 {
        return Some(ColorBucket::Brown);
    }

    // 3. Pale red reads as pink (blush, salmon, rose). The Pink band
    //    below only covers the magenta side (290–345°), so without
    //    this a pastel pink would land under Red.
    if !(15.0..345.0).contains(&h) && l >= 0.75 {
        return Some(ColorBucket::Pink);
    }

    // 4. Hue bands.
    Some(match h {
        h if !(15.0..345.0).contains(&h) => ColorBucket::Red,
        h if h < 45.0 => ColorBucket::Orange,
        h if h < 70.0 => ColorBucket::Yellow,
        h if h < 165.0 => ColorBucket::Green,
        h if h < 195.0 => ColorBucket::Cyan,
        h if h < 255.0 => ColorBucket::Blue,
        h if h < 290.0 => ColorBucket::Purple,
        _ => ColorBucket::Pink,
    })
}

/// Quantises a whole palette, de-duplicated, in [`ColorBucket::ALL`]
/// order.
///
/// De-duplication is the point: a photograph whose five dominant
/// colours are five shades of blue carries the Blue bucket once, so the
/// facet counts assets rather than palette entries.
pub fn buckets_of<'a, I>(palette: I) -> Vec<ColorBucket>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut found: Vec<ColorBucket> = palette.into_iter().filter_map(bucket_of).collect();
    found.sort_unstable();
    found.dedup();
    // Restore swatch order (the derived `Ord` follows the enum
    // declaration, which *is* swatch order — this keeps the guarantee
    // explicit rather than incidental).
    ColorBucket::ALL
        .into_iter()
        .filter(|b| found.contains(b))
        .collect()
}

/// `#rrggbb` → `(r, g, b)` in 0–255. Accepts an optional leading `#`
/// and either case; anything else is `None`.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let body = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if body.len() != 6 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&body[0..2], 16).ok()?;
    let g = u8::from_str_radix(&body[2..4], 16).ok()?;
    let b = u8::from_str_radix(&body[4..6], 16).ok()?;
    Some((r, g, b))
}

/// RGB (0–255) → `(hue, chroma, lightness)`: hue in degrees
/// `[0, 360)`, chroma and lightness in `[0, 1]`.
///
/// Chroma rather than HSL saturation, deliberately — see
/// [`bucket_of`]. It is the same `max - min` the hue derivation
/// already needs, so nothing is spent computing it.
fn to_hcl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (rf, gf, bf) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let mut h = if max == rf {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if max == gf {
        60.0 * ((bf - rf) / delta + 2.0)
    } else {
        60.0 * ((rf - gf) / delta + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    (h, delta.clamp(0.0, 1.0), l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primaries_land_in_their_own_bands() {
        assert_eq!(bucket_of("#ff0000"), Some(ColorBucket::Red));
        assert_eq!(bucket_of("#ff8800"), Some(ColorBucket::Orange));
        assert_eq!(bucket_of("#ffee00"), Some(ColorBucket::Yellow));
        assert_eq!(bucket_of("#00ff00"), Some(ColorBucket::Green));
        assert_eq!(bucket_of("#00ffff"), Some(ColorBucket::Cyan));
        assert_eq!(bucket_of("#0000ff"), Some(ColorBucket::Blue));
        assert_eq!(bucket_of("#8800ff"), Some(ColorBucket::Purple));
        assert_eq!(bucket_of("#ff00ff"), Some(ColorBucket::Pink));
    }

    #[test]
    fn neutrals_ignore_their_hue() {
        assert_eq!(bucket_of("#ffffff"), Some(ColorBucket::White));
        assert_eq!(bucket_of("#000000"), Some(ColorBucket::Black));
        assert_eq!(bucket_of("#808080"), Some(ColorBucket::Gray));
        // A saturated hue at the extremes reads as a neutral on screen.
        assert_eq!(bucket_of("#050002"), Some(ColorBucket::Black));
        assert_eq!(bucket_of("#fffdfe"), Some(ColorBucket::White));
        // Barely-tinted mid grey.
        assert_eq!(bucket_of("#7f8285"), Some(ColorBucket::Gray));
    }

    /// The colours a photograph is actually full of. Each of these came
    /// out of the HSL-saturation form of this function under a swatch
    /// nobody would look for it under, which is why the neutral test
    /// reads chroma now.
    #[test]
    fn nearly_white_tints_are_white_not_a_hue() {
        assert_eq!(bucket_of("#f5f0e8"), Some(ColorBucket::White), "cream wall");
        assert_eq!(
            bucket_of("#eef0f5"),
            Some(ColorBucket::White),
            "cool off-white"
        );
        assert_eq!(bucket_of("#e8e0d0"), Some(ColorBucket::White), "beige");
        // …and a tint strong enough to name still gets its hue.
        assert_eq!(
            bucket_of("#ffdbac"),
            Some(ColorBucket::Orange),
            "pale skin carries real chroma"
        );
    }

    /// The Pink band covers the magenta side only, so a pale red would
    /// otherwise land under Red — which is not where anyone looks for
    /// a blush pink.
    #[test]
    fn pale_red_is_pink() {
        assert_eq!(bucket_of("#f7cac9"), Some(ColorBucket::Pink), "pastel pink");
        assert_eq!(
            bucket_of("#ff0000"),
            Some(ColorBucket::Red),
            "a full red stays red"
        );
        assert_eq!(
            bucket_of("#c92a2a"),
            Some(ColorBucket::Red),
            "a dark red stays red"
        );
    }

    #[test]
    fn dark_orange_is_brown_not_orange() {
        assert_eq!(bucket_of("#5b3a1a"), Some(ColorBucket::Brown));
        // …while the same hue at a normal lightness stays Orange.
        assert_eq!(bucket_of("#d2823c"), Some(ColorBucket::Orange));
    }

    #[test]
    fn hex_parsing_is_lenient_about_form_and_strict_about_shape() {
        assert_eq!(bucket_of("FF0000"), Some(ColorBucket::Red));
        assert_eq!(bucket_of("  #Ff0000 "), Some(ColorBucket::Red));
        assert_eq!(
            bucket_of("#f00"),
            None,
            "short form is not produced by the extractor"
        );
        assert_eq!(bucket_of("#gggggg"), None);
        assert_eq!(bucket_of(""), None);
    }

    #[test]
    fn a_palette_yields_deduped_buckets_in_swatch_order() {
        // Three blues and a white → two buckets, chromatic first.
        let out = buckets_of(["#0000ff", "#3333cc", "#1a1aff", "#ffffff"]);
        assert_eq!(out, vec![ColorBucket::Blue, ColorBucket::White]);
    }

    #[test]
    fn malformed_entries_are_dropped_not_bucketed() {
        let out = buckets_of(["#ff0000", "nope", ""]);
        assert_eq!(out, vec![ColorBucket::Red]);
        assert!(buckets_of(["nope"]).is_empty());
    }

    #[test]
    fn slug_round_trips_and_rejects_typos() {
        for bucket in ColorBucket::ALL {
            assert_eq!(ColorBucket::parse(bucket.as_str()).unwrap(), bucket);
        }
        assert_eq!(ColorBucket::parse("  RED ").unwrap(), ColorBucket::Red);
        let err = ColorBucket::parse("crimson").unwrap_err().to_string();
        assert!(
            err.contains("crimson"),
            "the rejected value is named: {err}"
        );
        assert!(err.contains("red"), "the accepted set is listed: {err}");
    }
}
