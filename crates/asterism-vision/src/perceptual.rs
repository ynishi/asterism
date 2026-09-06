//! A fingerprint that survives a transform — the cheap half of
//! duplicate detection.
//!
//! The three exact axes ask whether two files hold the same bytes, and
//! they are right to: a digest that widened its equivalence would fold
//! two different pictures into one, which destroys, where a narrow one
//! only fails to notice. The reasoning is written out once, beside the
//! digests it constrains, in
//! `asterism_core::domain::content_hash`. Nothing here weakens it.
//!
//! What this module adds is the question those axes cannot ask. A
//! resized or recompressed copy shares no bytes with its original, so
//! every exact axis reads it as an unrelated file. A perceptual
//! fingerprint reads the picture instead — coarsely, deliberately — and
//! two values that differ in a few bits mean two images that look the
//! same at a glance.
//!
//! ## This value never enters the duplicate axes
//!
//! It is not a fourth duplicate axis. Detection walks the axes
//! strongest-first, stops at the first agreement, writes an
//! `identical_to` edge, and may enqueue a fold; a claim this
//! approximate has no business anywhere in that sequence, and
//! `asterism_core::domain::content_hash::is_duplicate_key` refuses a
//! value carrying this tag by construction — it tests for the axis's
//! own prefix, and no axis spells this one.
//!
//! So the tag exists to say which question a stored value answers, the
//! way every other digest in this workspace does, and to stay
//! unmistakable for the ones that answer sameness. It is not a
//! sub-namespace of any of them: `p1-dhash:` begins with no other tag,
//! which is what keeps a reader from mistaking it for one.

use image::RgbImage;
use image::imageops::{FilterType, grayscale, resize};

/// Algorithm tag on a stored perceptual value: perceptual definition
/// revision 1, difference hash.
///
/// Versioned like its siblings (`cr1-sha256:`, `m1-sha256:`) because a
/// definition that has been stored cannot be edited afterwards without
/// changing what every value written under it meant. Widening the grid,
/// changing the resampling filter, or comparing columns instead of rows
/// each produce a different fingerprint of the same picture, so each of
/// them is a new tag rather than a quiet improvement to this one.
pub const PERCEPTUAL_DIGEST_PREFIX: &str = "p1-dhash:";

/// The side of the comparison grid. Eight comparisons along each of
/// eight lines, in each of two directions, fill exactly the 128 bits a
/// [`u128`] holds.
const SIDE: u32 = 8;

/// Hexadecimal digits a stored value spells after its tag.
const DIGITS: usize = 32;

/// The fingerprint as bits — [`None`] for an image with no pixels,
/// which has nothing to compare.
///
/// Greyscale first and resample second: the reduction is where every
/// difference between an original and its recompressed copy is supposed
/// to vanish, and dropping two channels before it means the filter
/// averages one plane rather than three that will be averaged together
/// afterwards anyway.
///
/// The filter is [`FilterType::Triangle`] rather than the CatmullRom
/// the fixtures resample with. A sharpening filter preserves exactly
/// what this value is trying to discard — ringing at an edge survives
/// into the comparison and turns a JPEG artefact into a flipped bit.
///
/// # Why both directions
///
/// Comparing along rows alone asks only where the picture gets darker
/// left to right, and two scenes whose difference is mostly vertical
/// reduce to the same answer. Measured on the fixture scenes at 64
/// bits, the closest pair that were not copies of each other sat 3 bits
/// apart — exactly where the worst recompressed copy sat, leaving no
/// threshold between them. Adding the column pass separates the two
/// populations by asking the same question down the picture as well.
pub fn fingerprint(img: &RgbImage) -> Option<u128> {
    if img.width() == 0 || img.height() == 0 {
        return None;
    }

    let grey = grayscale(img);
    let rows = resize(&grey, SIDE + 1, SIDE, FilterType::Triangle);
    let cols = resize(&grey, SIDE, SIDE + 1, FilterType::Triangle);

    let mut bits: u128 = 0;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (near, far) = (rows.get_pixel(x, y).0[0], rows.get_pixel(x + 1, y).0[0]);
            bits = (bits << 1) | u128::from(near > far);
        }
    }
    for x in 0..SIDE {
        for y in 0..SIDE {
            let (near, far) = (cols.get_pixel(x, y).0[0], cols.get_pixel(x, y + 1).0[0]);
            bits = (bits << 1) | u128::from(near > far);
        }
    }
    Some(bits)
}

/// The fingerprint as a stored value, tag and all — [`None`] for an
/// image with no pixels.
///
/// Lower-case hexadecimal, zero-padded to the full width, so that two
/// values are the same string whenever they are the same fingerprint. A
/// reader that trims a leading zero would be reading a different
/// picture's value.
pub fn of_image(img: &RgbImage) -> Option<String> {
    fingerprint(img)
        .map(|bits| format!("{PERCEPTUAL_DIGEST_PREFIX}{bits:0width$x}", width = DIGITS))
}

/// Read a stored value back to bits — [`None`] when it carries another
/// tag, or none, or does not spell the full width in hexadecimal after
/// the one it carries.
///
/// Strict about the length rather than accepting whatever
/// [`u128::from_str_radix`] would take, because a short value is a
/// writer that lost a leading zero and a long one is not this
/// vocabulary at all; either way, answering with a number would hand
/// back a fingerprint nobody computed.
pub fn parse(value: &str) -> Option<u128> {
    let digits = value.strip_prefix(PERCEPTUAL_DIGEST_PREFIX)?;
    if digits.len() != DIGITS || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u128::from_str_radix(digits, 16).ok()
}

/// How many of the 128 comparisons two fingerprints disagree on.
///
/// Zero means the two pictures reduced to the same grid, which is what
/// an exact copy and a faithful resize both do.
pub fn distance(a: u128, b: u128) -> u32 {
    (a ^ b).count_ones()
}

/// How far apart two fingerprints may sit and still be proposed as
/// copies of one picture.
///
/// Chosen from the measurement in this module's tests rather than from
/// the literature, and it buys precision at recall's expense: no pair
/// of different fixture scenes — nor any scene against noise — comes
/// within it, while the recompressed copy that drifted furthest sits
/// outside it and is missed. That is the direction to fail in. A pair
/// this layer declines costs a row nobody sees; a pair it invents puts
/// two unrelated pictures next to each other and asks a person to
/// believe it.
///
/// # What this distance does not reach
///
/// Cropping. A centre crop at the fixtures' safe fraction moves every
/// row and column of the grid at once, and the measurement puts it an
/// order of magnitude further out than a resize — well inside the range
/// where different pictures live, so no threshold admits it without
/// admitting them too. A crop-tolerant fingerprint is a different
/// construction, not a wider bound on this one.
pub const NEAR_DUPLICATE_DISTANCE: u32 = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::relations::{CROP_FRAC, RelationStream};
    use crate::fixtures::scene::{
        PALETTE, SHAPES, SceneSpec, ShapeSpec, crop_center, noise_image, render,
        resize as resize_scene, to_jpeg,
    };

    /// A scene built to be told apart at this module's resolution:
    /// shapes large enough to survive the reduction to an 8-row grid.
    ///
    /// The relation stream's own scenes are not that, and finding out
    /// why is what this file measured first. Its look-alike keeps the
    /// base's two background colours and jitters the gradient angle and
    /// a pair of shapes that occupy a few percent of the canvas — and
    /// an 8-row grid is entitled to lose all of that, so the closest of
    /// them sat 0 bits from its base. That is the encoder's distinction
    /// to draw rather than this one's: this layer separates a copy from
    /// a different picture, and a look-alike is neither.
    /// `i` walks every (background, shape) pairing exactly once, so no
    /// two scenes differ by the gradient angle alone — a difference an
    /// 8-row grid is entitled to lose.
    fn spec_for(i: usize) -> SceneSpec {
        let n = PALETTE.len();
        let k = SHAPES.len();
        let (hue, kind) = (i / k, i % k);
        SceneSpec {
            width: 640,
            height: 640,
            bg: [PALETTE[hue], PALETTE[(hue + 1 + kind) % n]],
            angle_deg: 30.0,
            shapes: vec![
                ShapeSpec {
                    kind: SHAPES[kind],
                    color: PALETTE[(hue + 3) % n],
                    cx: 0.32,
                    cy: 0.34,
                    size: 0.18,
                },
                ShapeSpec {
                    kind: SHAPES[(kind + 1) % k],
                    color: PALETTE[(hue + 2) % n],
                    cx: 0.68,
                    cy: 0.66,
                    size: 0.16,
                },
            ],
        }
    }

    /// Decode a fixture's encoded bytes the way the job will: the
    /// pipeline stores a file, and what comes back is whatever the
    /// container round-tripped.
    fn decode(bytes: &[u8]) -> RgbImage {
        image::load_from_memory(bytes)
            .expect("fixture bytes decode")
            .to_rgb8()
    }

    #[test]
    fn an_image_with_no_pixels_has_no_fingerprint() {
        assert_eq!(fingerprint(&RgbImage::new(0, 0)), None);
        assert_eq!(of_image(&RgbImage::new(0, 0)), None);
    }

    #[test]
    fn a_stored_value_round_trips() {
        let bits = 0x0f1e_2d3c_4b5a_6978_8796_a5b4_c3d2_e1f0_u128;
        let value = format!("{PERCEPTUAL_DIGEST_PREFIX}{bits:032x}");
        assert_eq!(parse(&value), Some(bits));

        let img = render(&spec_for(1)).expect("render");
        let spelled = of_image(&img).expect("a value");
        assert_eq!(parse(&spelled), fingerprint(&img));
    }

    /// A leading zero is part of the value, not decoration.
    #[test]
    fn a_short_value_is_not_a_fingerprint() {
        assert_eq!(parse("p1-dhash:1"), None);
        assert_eq!(parse(&format!("p1-dhash:{:031x}", 1)), None);
        assert_eq!(parse(&format!("p1-dhash:{:033x}", 1)), None);
    }

    /// The tag has to be unmistakable for the ones that answer
    /// sameness, which is the property the module doc rests on.
    #[test]
    fn another_axis_tag_is_not_read_as_this_one() {
        assert_eq!(
            parse("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            None
        );
        assert_eq!(parse("cr1-sha256:0000000000000000"), None);
        assert!(!PERCEPTUAL_DIGEST_PREFIX.starts_with("sha256:"));
        assert!(!PERCEPTUAL_DIGEST_PREFIX.starts_with("cr1-sha256:"));
        assert!(!PERCEPTUAL_DIGEST_PREFIX.starts_with("m1-sha256:"));
    }

    #[test]
    fn the_same_picture_twice_is_the_same_fingerprint() {
        let spec = RelationStream::new(7).next().expect("a scene").scene;
        let once = render(&spec).expect("render");
        let twice = render(&spec).expect("render");
        assert_eq!(fingerprint(&once), fingerprint(&twice));
    }

    /// Where each population lands, measured rather than assumed.
    ///
    /// A threshold is only defensible if every transform it has to
    /// accept sits below every picture it has to refuse, so this
    /// reports the worst copy and the closest stranger rather than an
    /// average either could hide behind.
    #[test]
    fn transforms_and_strangers_do_not_overlap() {
        let bases: Vec<_> = (0..18)
            .map(|i| render(&spec_for(i)).expect("render"))
            .collect();
        let prints: Vec<u128> = bases
            .iter()
            .map(|img| fingerprint(img).expect("a fingerprint"))
            .collect();

        let mut resized = Vec::new();
        let mut recompressed = Vec::new();
        let mut cropped = Vec::new();
        for (img, &base) in bases.iter().zip(&prints) {
            resized.push(distance(
                base,
                fingerprint(&resize_scene(img, 0.5)).expect("a fingerprint"),
            ));
            recompressed.push(distance(
                base,
                fingerprint(&decode(&to_jpeg(img, 70).expect("encode"))).expect("a fingerprint"),
            ));
            cropped.push(distance(
                base,
                fingerprint(&crop_center(img, CROP_FRAC)).expect("a fingerprint"),
            ));
        }

        let mut strangers = Vec::new();
        for (i, &a) in prints.iter().enumerate() {
            for &b in &prints[i + 1..] {
                strangers.push(distance(a, b));
            }
        }
        let noise = fingerprint(&noise_image(3, 640, 640)).expect("a fingerprint");
        let against_noise: Vec<u32> = prints.iter().map(|&p| distance(p, noise)).collect();

        let report = |name: &str, xs: &[u32]| {
            let min = xs.iter().copied().min().expect("a population");
            let max = xs.iter().copied().max().expect("a population");
            let mean = f64::from(xs.iter().sum::<u32>()) / xs.len() as f64;
            eprintln!("{name}: n={} min={min} max={max} mean={mean:.1}", xs.len());
        };
        report("resize 0.5", &resized);
        report("jpeg q70", &recompressed);
        report("crop 0.8", &cropped);
        report("stranger", &strangers);
        report("noise", &against_noise);

        let copies: Vec<u32> = resized.iter().chain(&recompressed).copied().collect();
        let others: Vec<u32> = strangers.iter().chain(&against_noise).copied().collect();

        for threshold in 0..=12 {
            let found = copies.iter().filter(|&&d| d <= threshold).count();
            let wrong = others.iter().filter(|&&d| d <= threshold).count();
            eprintln!(
                "threshold {threshold}: recall {found}/{} false positives {wrong}",
                copies.len()
            );
        }

        let wrong = others
            .iter()
            .filter(|&&d| d <= NEAR_DUPLICATE_DISTANCE)
            .count();
        assert_eq!(
            wrong, 0,
            "{wrong} pictures that are not copies of each other sat within \
             {NEAR_DUPLICATE_DISTANCE} bits"
        );

        let found = copies
            .iter()
            .filter(|&&d| d <= NEAR_DUPLICATE_DISTANCE)
            .count();
        assert!(
            found * 10 >= copies.len() * 9,
            "only {found} of {} resized or recompressed copies were recognised",
            copies.len()
        );

        let closest_crop = cropped.iter().copied().min().expect("a population");
        assert!(
            closest_crop > NEAR_DUPLICATE_DISTANCE,
            "a centre crop sat {closest_crop} bits away, inside the threshold this \
             module documents as out of its reach"
        );
    }
}
