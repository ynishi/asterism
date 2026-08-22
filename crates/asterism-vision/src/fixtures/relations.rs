//! The relation stream: scenes together with the relatives the pipeline
//! must find, and the material it must refuse.
//!
//! Every relationship under evaluation exists on purpose, derived from
//! one seeded walk:
//!
//! - **look-alike**: the same scene re-drawn with jittered geometry —
//!   no shared bytes, visually similar, an encoder question rather than
//!   a hash question;
//! - **semantic sibling**: the same describable content (identical tag
//!   set) re-composed from scratch on a different background — related
//!   in meaning, not in looks;
//! - **hard negative**: same background family, different shapes —
//!   close enough in histogram terms to punish a lazy similarity score;
//! - **unrelated**: noise canvases ([`super::scene::noise_image`]) and
//!   the query strings below, which the honest-failure acceptance test
//!   requires to return nothing.
//!
//! File-level variants (exact copy, resize, recompress, crop) are not
//! spec-level relatives: the evaluation applies [`super::scene`]'s
//! transforms to a rendered base at call time.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;

use super::scene::{PALETTE, PaletteColor, SHAPES, SceneSpec, ShapeSpec};

/// Canvas mix — modest sizes; rendering cost is not under evaluation.
pub const DIMS: [(u32, u32); 3] = [(640, 640), (512, 768), (768, 512)];
const SHAPE_COUNT: (usize, usize) = (1, 3);
/// Shapes are placed one per cell of a 2×2 grid, so no shape can occlude
/// another — a tag the spec asserts must be visible in the pixels, or
/// the tag-suggestion ground truth is quietly wrong. Cell distance
/// (0.36), jitter and size are chosen together so even worst-case
/// neighbours do not touch, and everything stays inside the 80 % center
/// crop.
const CELLS: [(f64, f64); 4] = [(0.32, 0.32), (0.68, 0.32), (0.32, 0.68), (0.68, 0.68)];
const CELL_JITTER: f64 = 0.02;
/// Clamp bounds for look-alike jitter; also the widest any center may
/// sit.
pub const PLACEMENT: (f64, f64) = (0.24, 0.76);
const SHAPE_SIZE: (f64, f64) = (0.08, 0.12);
/// The center-crop fraction the placement is safe under.
pub const CROP_FRAC: f64 = 0.8;
/// Look-alike geometry jitter: positions move a little, the scene stays
/// recognisably the same picture. Small enough that jittered neighbours
/// still cannot touch (see the non-overlap test).
const JITTER_POS: f64 = 0.03;
const JITTER_SIZE: (f64, f64) = (0.9, 1.1);
const JITTER_ANGLE_DEG: f64 = 20.0;

/// Query strings with no counterpart among the fixtures: retrieval is
/// allowed — required — to return nothing for these.
pub fn unrelated_queries_en() -> Vec<String> {
    [
        "a photograph of a cat",
        "city skyline at night",
        "handwritten text on paper",
    ]
    .map(String::from)
    .to_vec()
}

/// Japanese counterpart of [`unrelated_queries_en`].
pub fn unrelated_queries_ja() -> Vec<String> {
    ["猫の写真", "夜の街並み", "紙に書かれた手書きの文字"]
        .map(String::from)
        .to_vec()
}

/// One base scene and its spec-level relatives.
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedScenes {
    /// Position in the stream; part of a measurement's identity.
    pub index: u64,
    /// The base scene.
    pub scene: SceneSpec,
    /// Every second base has a look-alike.
    pub lookalike: Option<SceneSpec>,
    /// Every third base has a semantic sibling.
    pub semantic_sibling: Option<SceneSpec>,
    /// Every fourth base has a hard negative.
    pub hard_negative: Option<SceneSpec>,
}

/// Seeded, infinite, deterministic: no wall-clock, no environment, a
/// fixed draw order, and scene `i` reachable only by walking to it.
pub struct RelationStream {
    rng: ChaCha20Rng,
    next_index: u64,
}

impl RelationStream {
    /// A stream whose entire output is a function of `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha20Rng::seed_from_u64(seed ^ 0xB15C_0DE0_0000_0001),
            next_index: 0,
        }
    }

    fn draw_scene(&mut self) -> SceneSpec {
        let (width, height) = DIMS[self.rng.random_range(0..DIMS.len())];
        let bg0 = PALETTE[self.rng.random_range(0..PALETTE.len())];
        let bg1 = loop {
            let c = PALETTE[self.rng.random_range(0..PALETTE.len())];
            if c != bg0 {
                break c;
            }
        };
        let angle_deg = self.rng.random_range(0.0..360.0);
        let shapes = self.draw_shapes(None);
        SceneSpec {
            width,
            height,
            bg: [bg0, bg1],
            angle_deg,
            shapes,
        }
    }

    /// Draw 1–3 shapes, each in its own grid cell.
    fn draw_shapes(&mut self, exclude: Option<&[PaletteColor]>) -> Vec<ShapeSpec> {
        let count = self.rng.random_range(SHAPE_COUNT.0..=SHAPE_COUNT.1);
        let mut cells = CELLS;
        // Fisher–Yates over the four cells, then take the first `count`.
        for i in (1..cells.len()).rev() {
            let j = self.rng.random_range(0..=i);
            cells.swap(i, j);
        }
        (0..count)
            .map(|i| self.draw_shape_in(cells[i], exclude))
            .collect()
    }

    fn draw_shape_in(&mut self, cell: (f64, f64), exclude: Option<&[PaletteColor]>) -> ShapeSpec {
        let color = loop {
            let c = PALETTE[self.rng.random_range(0..PALETTE.len())];
            if exclude.is_none_or(|banned| !banned.contains(&c)) {
                break c;
            }
        };
        ShapeSpec {
            kind: SHAPES[self.rng.random_range(0..SHAPES.len())],
            color,
            cx: cell.0 + self.rng.random_range(-CELL_JITTER..CELL_JITTER),
            cy: cell.1 + self.rng.random_range(-CELL_JITTER..CELL_JITTER),
            size: self.rng.random_range(SHAPE_SIZE.0..SHAPE_SIZE.1),
        }
    }

    /// Same shapes and colors, jittered geometry: related, not derived.
    fn draw_lookalike(&mut self, base: &SceneSpec) -> SceneSpec {
        let mut alike = base.clone();
        alike.angle_deg = (alike.angle_deg
            + self.rng.random_range(-JITTER_ANGLE_DEG..JITTER_ANGLE_DEG))
        .rem_euclid(360.0);
        for shape in &mut alike.shapes {
            shape.cx = (shape.cx + self.rng.random_range(-JITTER_POS..JITTER_POS))
                .clamp(PLACEMENT.0, PLACEMENT.1);
            shape.cy = (shape.cy + self.rng.random_range(-JITTER_POS..JITTER_POS))
                .clamp(PLACEMENT.0, PLACEMENT.1);
            shape.size = (shape.size * self.rng.random_range(JITTER_SIZE.0..JITTER_SIZE.1))
                .clamp(SHAPE_SIZE.0, SHAPE_SIZE.1);
        }
        alike
    }

    /// Same describable content, different composition: the base's
    /// shape kinds and colors reappear — so the tag set is identical —
    /// but the geometry is drawn from scratch and the background
    /// changes. Related in meaning, not in looks.
    fn draw_semantic_sibling(&mut self, base: &SceneSpec) -> SceneSpec {
        let mut cells = CELLS;
        for i in (1..cells.len()).rev() {
            let j = self.rng.random_range(0..=i);
            cells.swap(i, j);
        }
        let shapes = base
            .shapes
            .iter()
            .zip(cells)
            .map(|(shape, cell)| ShapeSpec {
                kind: shape.kind,
                color: shape.color,
                cx: cell.0 + self.rng.random_range(-CELL_JITTER..CELL_JITTER),
                cy: cell.1 + self.rng.random_range(-CELL_JITTER..CELL_JITTER),
                size: self.rng.random_range(SHAPE_SIZE.0..SHAPE_SIZE.1),
            })
            .collect();
        let bg0 = loop {
            let c = PALETTE[self.rng.random_range(0..PALETTE.len())];
            if !base.bg.contains(&c) {
                break c;
            }
        };
        let bg1 = loop {
            let c = PALETTE[self.rng.random_range(0..PALETTE.len())];
            if c != bg0 && !base.bg.contains(&c) {
                break c;
            }
        };
        SceneSpec {
            width: base.width,
            height: base.height,
            bg: [bg0, bg1],
            angle_deg: self.rng.random_range(0.0..360.0),
            shapes,
        }
    }

    /// Same background family, different content: the colors of the
    /// base's shapes are excluded, so the tag sets cannot overlap.
    fn draw_hard_negative(&mut self, base: &SceneSpec) -> SceneSpec {
        let banned: Vec<PaletteColor> = base.shapes.iter().map(|s| s.color).collect();
        let shapes = self.draw_shapes(Some(&banned));
        SceneSpec {
            shapes,
            ..base.clone()
        }
    }
}

impl Iterator for RelationStream {
    type Item = RelatedScenes;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.next_index;
        self.next_index += 1;

        let scene = self.draw_scene();
        // Drawn unconditionally so the stream position after scene `i`
        // never depends on which relatives scene `i` carries.
        let lookalike = self.draw_lookalike(&scene);
        let semantic_sibling = self.draw_semantic_sibling(&scene);
        let hard_negative = self.draw_hard_negative(&scene);

        Some(RelatedScenes {
            index,
            scene,
            lookalike: index.is_multiple_of(2).then_some(lookalike),
            semantic_sibling: index.is_multiple_of(3).then_some(semantic_sibling),
            hard_negative: index.is_multiple_of(4).then_some(hard_negative),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_is_deterministic_in_the_seed() {
        let a: Vec<RelatedScenes> = RelationStream::new(7).take(5).collect();
        let b: Vec<RelatedScenes> = RelationStream::new(7).take(5).collect();
        assert_eq!(a, b);
        let c: Vec<RelatedScenes> = RelationStream::new(8).take(5).collect();
        assert_ne!(a, c, "a different seed is a different fixture set");
    }

    #[test]
    fn relatives_follow_the_cadence() {
        let specs: Vec<RelatedScenes> = RelationStream::new(42).take(13).collect();
        for spec in &specs {
            assert_eq!(spec.lookalike.is_some(), spec.index % 2 == 0);
            assert_eq!(spec.semantic_sibling.is_some(), spec.index % 3 == 0);
            assert_eq!(spec.hard_negative.is_some(), spec.index % 4 == 0);
        }
    }

    #[test]
    fn relatives_relate_the_way_their_names_claim() {
        for spec in RelationStream::new(42).take(20) {
            if let Some(alike) = &spec.lookalike {
                assert_eq!(alike.tags_en(), spec.scene.tags_en());
                assert_eq!(alike.bg, spec.scene.bg, "look-alike keeps the background");
            }
            if let Some(sibling) = &spec.semantic_sibling {
                assert_eq!(
                    sibling.tags_en(),
                    spec.scene.tags_en(),
                    "a semantic sibling shows the same describable content"
                );
                for c in sibling.bg {
                    assert!(
                        !spec.scene.bg.contains(&c),
                        "a semantic sibling changes the background family"
                    );
                }
            }
            if let Some(negative) = &spec.hard_negative {
                assert_eq!(
                    negative.bg, spec.scene.bg,
                    "hard negative keeps the background"
                );
                for tag in negative.tags_en() {
                    assert!(
                        !spec.scene.tags_en().contains(&tag),
                        "hard negative must not share a tag with its base: {tag}"
                    );
                }
            }
        }
    }

    #[test]
    fn shapes_stay_inside_the_crop_safe_region() {
        // The crop keeps [0.5 - f/2, 0.5 + f/2]; a shape at the widest
        // allowed center must still fit inside it whole.
        let crop_margin = (0.5 + CROP_FRAC / 2.0) - PLACEMENT.1;
        for spec in RelationStream::new(42).take(20) {
            for shape in &spec.scene.shapes {
                assert!((PLACEMENT.0..PLACEMENT.1).contains(&shape.cx));
                assert!((PLACEMENT.0..PLACEMENT.1).contains(&shape.cy));
                assert!(shape.size < crop_margin);
            }
        }
    }

    /// A tag the spec asserts must be visible in the pixels: no shape
    /// may occlude another, in any scene the stream produces. Chebyshev
    /// distance bounds the square, which bounds the other two kinds.
    #[test]
    fn no_shape_ever_occludes_another() {
        for spec in RelationStream::new(42).take(50) {
            let mut scenes = vec![&spec.scene];
            scenes.extend(spec.lookalike.as_ref());
            scenes.extend(spec.semantic_sibling.as_ref());
            scenes.extend(spec.hard_negative.as_ref());
            for scene in scenes {
                for (i, a) in scene.shapes.iter().enumerate() {
                    for b in scene.shapes.iter().skip(i + 1) {
                        let chebyshev = (a.cx - b.cx).abs().max((a.cy - b.cy).abs());
                        assert!(
                            chebyshev > a.size + b.size,
                            "shapes touch: d={chebyshev:.3} sizes={:.3}+{:.3}",
                            a.size,
                            b.size
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unrelated_queries_exist_in_both_languages() {
        assert!(!unrelated_queries_en().is_empty());
        assert!(!unrelated_queries_ja().is_empty());
    }
}
