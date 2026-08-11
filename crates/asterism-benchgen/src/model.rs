//! The cardinality model: a seeded, infinite stream of `AssetSpec`.
//!
//! Everything a bench corpus is made of — persona, dimensions, when the
//! asset was taken, its tags / groups / rating / labels, and the seed its
//! pixels are derived from — is drawn here from a single `ChaCha20Rng`.
//!
//! ## The determinism contract
//!
//! **Two `SpecStream`s built from the same seed yield the same spec
//! sequence, element for element.** That single property is what the rest
//! of the bench rests on:
//!
//! - *Prefix inclusion*: S (5,000) ⊂ M (12,000) ⊂ L (110,000) falls out for
//!   free — a preset is a `take(n)` of the same stream, so asset `i` means
//!   the same asset in every preset of that seed.
//! - *Tier agreement*: the T-file tier (real PNGs on disk) and the T-meta
//!   tier (rows seeded straight into the repository by `seed-meta`) consume this same
//!   stream, so a group named `bench-mega` holds a comparable population on
//!   either side.
//! - *Comparability*: a bench number is only meaningful next to the seed it
//!   was measured on; `(seed, generator_version, preset)` in the manifest is
//!   the identity that makes two runs comparable.
//!
//! To hold that contract, **no wall-clock, environment, or thread-derived
//! value may enter a draw**. `occurred_at_ms` is synthesised from a fixed
//! base instant, never from `SystemTime`. The only real clock in this crate
//! is `Manifest::generated_at_ms`, which nothing reads back.
//!
//! The number of draws taken per asset is allowed to depend on earlier
//! draws (tag rejection sampling, group-pool fallback): that keeps the
//! stream deterministic, but it does mean asset `i` cannot be computed
//! without walking the stream to `i`.
//!
//! Pool names (`bench-mega`, `bench-3k-0`, …) are part of the contract too:
//! the bench driver selects scroll targets by name, so they are public
//! constants rather than incidental strings.
//!
//! ## Groups belong to one persona
//!
//! A group (`asset_bucket`) is persona-scoped — `(persona_id, name)` is
//! unique — so an asset can only join a group its own persona owns.
//! Spreading `bench-mega` across all six personas would produce a group
//! whose members mostly vanish the moment a persona filter is applied,
//! which is exactly the state the 10k-per-group scroll bench must not be
//! measured in. So **only `persona_idx == 0` draws groups**: the whole pool
//! table lives under `bench-persona-0`, and
//! "scroll `bench-mega`" means the same population with or without the
//! persona filter.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::collections::VecDeque;

/// Base instant of the synthetic capture window: 2026-02-01T00:00:00Z.
pub const BASE_OCCURRED_AT_MS: i64 = 1_769_904_000_000;
/// Length of the capture window. Bursts are scattered uniformly over it.
pub const SPAN_DAYS: i64 = 180;

/// Personas are `bench-persona-0` … `bench-persona-5` on the consuming side
/// (`seed-meta` names them); the stream only carries the index.
pub const PERSONA_COUNT: usize = 6;
const PERSONA_WEIGHTS: [u32; PERSONA_COUNT] = [45, 23, 15, 8, 5, 4];

/// Dimension mix (width, height, weight-%) — the AI-generated staples.
/// Decode cost is driven by pixel count, so this is what makes `thumb_gen`
/// work as hard as it does on a real library.
const DIMENSIONS: [(u32, u32, u32); 5] = [
    (1024, 1024, 40),
    (832, 1216, 20),
    (1216, 832, 15),
    (1536, 1536, 15),
    (2048, 2048, 10),
];

pub const TAG_VOCAB_SIZE: usize = 240;
pub const LABEL_VOCAB_SIZE: usize = 40;
const TAG_LAMBDA: f64 = 3.0;
const TAG_MAX: usize = 8;
const LABEL_MAX: usize = 3;

const RATED_FRACTION: f64 = 0.07;
const TRASHED_FRACTION: f64 = 0.02;
const UNGROUPED_FRACTION: f64 = 0.15;
const SECOND_GROUP_FRACTION: f64 = 0.20;

/// A session burst: this many assets land inside one window.
const BURST_SIZE: (u32, u32) = (10, 80);
/// Width of that window, in minutes.
const BURST_WINDOW_MINUTES: (i64, i64) = (5, 40);
/// Share of bursts that start between 09:00 and 24:00 local-of-UTC.
const DAYTIME_FRACTION: f64 = 0.85;
const DAYTIME_START_MINUTE: i64 = 9 * 60;
const MINUTES_PER_DAY: i64 = 24 * 60;

// --- Group pools -----------------------------------------------------------
//
// Fixed structure with capacity counters, reproducing the reference
// shape (one 10k group, two 3k groups) plus a mid/small tail. Capacities are
// the *contract* the bench driver reads: "scroll `bench-mega`" has to mean a
// group of ~10,000 on every seed.

pub const GROUP_MEGA_NAME: &str = "bench-mega";
pub const GROUP_MEGA_CAPACITY: u32 = 10_000;
pub const GROUP_3K_COUNT: usize = 2;
pub const GROUP_3K_CAPACITY: u32 = 3_000;
pub const GROUP_MID_COUNT: usize = 20;
/// Inclusive capacity range; the value per pool is drawn from the seed and
/// then fixed for the life of the corpus.
pub const GROUP_MID_CAPACITY: (u32, u32) = (200, 800);
pub const GROUP_SMALL_COUNT: usize = 200;
pub const GROUP_SMALL_CAPACITY: (u32, u32) = (10, 50);

/// Tier weights used to pick where an asset lands: mega / 3k / mid / small.
const TIER_WEIGHTS: [(GroupTier, u32); 4] = [
    (GroupTier::Mega, 35),
    (GroupTier::ThreeK, 20),
    (GroupTier::Mid, 25),
    (GroupTier::Small, 20),
];
/// Deterministic fallback order when the drawn tier is full.
const TIER_ORDER: [GroupTier; 4] = [
    GroupTier::Mega,
    GroupTier::ThreeK,
    GroupTier::Mid,
    GroupTier::Small,
];
/// A second membership only ever comes from the tail tiers.
const SECOND_TIER_ORDER: [GroupTier; 2] = [GroupTier::Mid, GroupTier::Small];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupTier {
    Mega,
    ThreeK,
    Mid,
    Small,
}

/// One group (`asset_bucket` on the storage side) with a fixed capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupPool {
    pub name: String,
    pub tier: GroupTier,
    pub capacity: u32,
}

/// Canonical group name for a tier + ordinal. `seed-meta` and the bench
/// driver call this rather than re-deriving the format string.
pub fn group_name(tier: GroupTier, idx: usize) -> String {
    match tier {
        GroupTier::Mega => GROUP_MEGA_NAME.to_string(),
        GroupTier::ThreeK => format!("bench-3k-{idx}"),
        GroupTier::Mid => format!("bench-mid-{idx:02}"),
        GroupTier::Small => format!("bench-small-{idx:03}"),
    }
}

/// The full pool table for a seed: names are fixed, mid/small capacities are
/// drawn from a stream *derived* from the seed rather than from the spec
/// stream itself, so that adding a pool tier later cannot shift the asset
/// sequence of an existing seed.
pub fn group_pools(seed: u64) -> Vec<GroupPool> {
    let mut rng = ChaCha20Rng::seed_from_u64(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut pools = Vec::with_capacity(1 + GROUP_3K_COUNT + GROUP_MID_COUNT + GROUP_SMALL_COUNT);

    pools.push(GroupPool {
        name: group_name(GroupTier::Mega, 0),
        tier: GroupTier::Mega,
        capacity: GROUP_MEGA_CAPACITY,
    });
    for idx in 0..GROUP_3K_COUNT {
        pools.push(GroupPool {
            name: group_name(GroupTier::ThreeK, idx),
            tier: GroupTier::ThreeK,
            capacity: GROUP_3K_CAPACITY,
        });
    }
    for idx in 0..GROUP_MID_COUNT {
        pools.push(GroupPool {
            name: group_name(GroupTier::Mid, idx),
            tier: GroupTier::Mid,
            capacity: rng.random_range(GROUP_MID_CAPACITY.0..=GROUP_MID_CAPACITY.1),
        });
    }
    for idx in 0..GROUP_SMALL_COUNT {
        pools.push(GroupPool {
            name: group_name(GroupTier::Small, idx),
            tier: GroupTier::Small,
            capacity: rng.random_range(GROUP_SMALL_CAPACITY.0..=GROUP_SMALL_CAPACITY.1),
        });
    }
    pools
}

// --- Spec ------------------------------------------------------------------

/// Everything about one synthetic asset. `image_seed` is what
/// [`crate::image_synth::render_png`] renders from, so the pixels are a pure
/// function of the spec; `rel_path` is the corpus-relative location the
/// T-file tier writes to and the T-meta tier fabricates a locator from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetSpec {
    pub index: u64,
    pub persona_idx: usize,
    pub width: u32,
    pub height: u32,
    pub occurred_at_ms: i64,
    pub tags: Vec<String>,
    pub groups: Vec<String>,
    pub rating: Option<u8>,
    pub trashed: bool,
    pub labels: Vec<String>,
    pub image_seed: u64,
    pub rel_path: String,
}

/// Infinite iterator of specs. `next()` never returns `None`; callers bound
/// it with `take(n)`.
pub struct SpecStream {
    rng: ChaCha20Rng,
    index: u64,
    pools: Vec<GroupPool>,
    used: Vec<u32>,
    tag_cdf: Vec<f64>,
    /// Pre-sorted timestamps of the burst currently being handed out.
    burst: VecDeque<i64>,
}

impl SpecStream {
    pub fn new(seed: u64) -> Self {
        let pools = group_pools(seed);
        let used = vec![0; pools.len()];
        Self {
            rng: ChaCha20Rng::seed_from_u64(seed),
            index: 0,
            pools,
            used,
            tag_cdf: zipf_cdf(TAG_VOCAB_SIZE),
            burst: VecDeque::new(),
        }
    }

    fn next_occurred_at(&mut self) -> i64 {
        if self.burst.is_empty() {
            let size = self.rng.random_range(BURST_SIZE.0..=BURST_SIZE.1) as usize;
            let window_ms = self
                .rng
                .random_range(BURST_WINDOW_MINUTES.0..=BURST_WINDOW_MINUTES.1)
                * 60_000;
            let day = self.rng.random_range(0..SPAN_DAYS);
            let minute = if self.rng.random_bool(DAYTIME_FRACTION) {
                self.rng.random_range(DAYTIME_START_MINUTE..MINUTES_PER_DAY)
            } else {
                self.rng.random_range(0..DAYTIME_START_MINUTE)
            };
            let start = BASE_OCCURRED_AT_MS + day * 86_400_000 + minute * 60_000;

            let mut offsets: Vec<i64> = (0..size)
                .map(|_| self.rng.random_range(0..=window_ms))
                .collect();
            offsets.sort_unstable();
            self.burst = offsets.into_iter().map(|o| start + o).collect();
        }
        // `burst` was just refilled if it was empty, and a burst is never
        // drawn with size 0, so this cannot fail.
        self.burst.pop_front().unwrap_or(BASE_OCCURRED_AT_MS)
    }

    fn draw_tags(&mut self) -> Vec<String> {
        let want = poisson(&mut self.rng, TAG_LAMBDA).min(TAG_MAX);
        let mut out: Vec<String> = Vec::with_capacity(want);
        // Zipf draws collide often at the head, so resample; the bound keeps
        // a pathological seed from spinning.
        let mut attempts = 0;
        while out.len() < want && attempts < want * 8 + 16 {
            attempts += 1;
            let idx = zipf_index(&self.tag_cdf, &mut self.rng);
            let name = format!("bench-tag-{idx:03}");
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    fn draw_labels(&mut self) -> Vec<String> {
        let want = self.rng.random_range(0..=LABEL_MAX);
        let mut out: Vec<String> = Vec::with_capacity(want);
        let mut attempts = 0;
        while out.len() < want && attempts < want * 4 + 8 {
            attempts += 1;
            let idx = self.rng.random_range(0..LABEL_VOCAB_SIZE);
            let name = format!("bench-label-{idx:02}");
            if !out.contains(&name) {
                out.push(name);
            }
        }
        out
    }

    /// Pick the first non-full pool from the tiers in `order`, honouring
    /// `exclude` so an asset never joins the same group twice.
    fn pick_pool(&mut self, order: &[GroupTier], exclude: Option<usize>) -> Option<usize> {
        for tier in order {
            let candidates: Vec<usize> = self
                .pools
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    p.tier == *tier && self.used[*i] < p.capacity && Some(*i) != exclude
                })
                .map(|(i, _)| i)
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let pick = candidates[self.rng.random_range(0..candidates.len())];
            self.used[pick] += 1;
            return Some(pick);
        }
        None
    }

    fn draw_groups(&mut self) -> Vec<String> {
        if self.rng.random_bool(UNGROUPED_FRACTION) {
            return Vec::new();
        }
        let weights: Vec<u32> = TIER_WEIGHTS.iter().map(|(_, w)| *w).collect();
        let drawn = TIER_WEIGHTS[weighted_index(&weights, &mut self.rng)].0;
        let mut order = vec![drawn];
        order.extend(TIER_ORDER.iter().copied().filter(|t| *t != drawn));

        let Some(primary) = self.pick_pool(&order, None) else {
            // Every pool is full: the asset stays ungrouped rather than
            // silently pushing a pool past its advertised capacity.
            return Vec::new();
        };
        let mut groups = vec![self.pools[primary].name.clone()];

        if self.rng.random_bool(SECOND_GROUP_FRACTION)
            && let Some(second) = self.pick_pool(&SECOND_TIER_ORDER, Some(primary))
        {
            groups.push(self.pools[second].name.clone());
        }
        groups
    }
}

impl Iterator for SpecStream {
    type Item = AssetSpec;

    fn next(&mut self) -> Option<AssetSpec> {
        let index = self.index;
        self.index += 1;

        let persona_idx = weighted_index(&PERSONA_WEIGHTS, &mut self.rng);
        let dim_weights: Vec<u32> = DIMENSIONS.iter().map(|(_, _, w)| *w).collect();
        let (width, height, _) = DIMENSIONS[weighted_index(&dim_weights, &mut self.rng)];
        let occurred_at_ms = self.next_occurred_at();
        let tags = self.draw_tags();
        // Groups are persona 0's alone (see the module doc): every other
        // persona is deliberately group-free, so a group filter and a
        // persona filter never fight over the same page. No draw is taken
        // for them at all — an unused draw would still consume the stream
        // and make the pool fill rate depend on the persona mix.
        let groups = if persona_idx == 0 {
            self.draw_groups()
        } else {
            Vec::new()
        };
        let rating = if self.rng.random_bool(RATED_FRACTION) {
            Some(self.rng.random_range(1u8..=5))
        } else {
            None
        };
        let trashed = self.rng.random_bool(TRASHED_FRACTION);
        let labels = self.draw_labels();
        let image_seed = self.rng.random::<u64>();

        Some(AssetSpec {
            index,
            persona_idx,
            width,
            height,
            occurred_at_ms,
            tags,
            groups,
            rating,
            trashed,
            labels,
            image_seed,
            rel_path: format!("files/{index:08}.png"),
        })
    }
}

// --- Draw helpers ----------------------------------------------------------

fn weighted_index(weights: &[u32], rng: &mut ChaCha20Rng) -> usize {
    let total: u32 = weights.iter().sum();
    let mut r = rng.random_range(0..total);
    for (i, w) in weights.iter().enumerate() {
        if r < *w {
            return i;
        }
        r -= *w;
    }
    weights.len() - 1
}

/// Normalised cumulative weights of `1/r` over `n` ranks — the long-tail
/// shape a real tag vocabulary has (a few tags on most assets, most tags on
/// a few).
fn zipf_cdf(n: usize) -> Vec<f64> {
    let mut cdf = Vec::with_capacity(n);
    let mut acc = 0.0;
    for r in 1..=n {
        acc += 1.0 / r as f64;
        cdf.push(acc);
    }
    for c in cdf.iter_mut() {
        *c /= acc;
    }
    cdf
}

fn zipf_index(cdf: &[f64], rng: &mut ChaCha20Rng) -> usize {
    let u: f64 = rng.random();
    cdf.partition_point(|c| *c <= u).min(cdf.len() - 1)
}

/// Knuth's multiplicative Poisson sampler — small `lambda` only, which is
/// all this model needs, and it keeps the dependency list at `rand`.
fn poisson(rng: &mut ChaCha20Rng, lambda: f64) -> usize {
    let limit = (-lambda).exp();
    let mut k = 0usize;
    let mut p = 1.0f64;
    loop {
        p *= rng.random::<f64>();
        if p <= limit {
            return k;
        }
        k += 1;
        if k > 64 {
            return k;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn same_seed_yields_same_specs() {
        let a: Vec<AssetSpec> = SpecStream::new(42).take(200).collect();
        let b: Vec<AssetSpec> = SpecStream::new(42).take(200).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_diverge() {
        let a: Vec<AssetSpec> = SpecStream::new(42).take(50).collect();
        let b: Vec<AssetSpec> = SpecStream::new(43).take(50).collect();
        assert_ne!(a, b, "seed must actually select a different corpus");
    }

    #[test]
    fn prefix_is_stable_across_lengths() {
        let short: Vec<AssetSpec> = SpecStream::new(7).take(100).collect();
        let long: Vec<AssetSpec> = SpecStream::new(7).take(1_000).collect();
        assert_eq!(short.as_slice(), &long[..100]);
    }

    #[test]
    fn bursts_are_monotonic_within_a_window() {
        // Timestamps are not globally sorted (bursts are scattered), but a
        // burst hands its members out in order; check that at least most
        // consecutive pairs move forward, and that everything lands inside
        // the declared window.
        let specs: Vec<AssetSpec> = SpecStream::new(11).take(2_000).collect();
        let span_end = BASE_OCCURRED_AT_MS + SPAN_DAYS * 86_400_000 + 86_400_000;
        assert!(
            specs
                .iter()
                .all(|s| s.occurred_at_ms >= BASE_OCCURRED_AT_MS && s.occurred_at_ms < span_end)
        );
        let forward = specs
            .windows(2)
            .filter(|w| w[1].occurred_at_ms >= w[0].occurred_at_ms)
            .count();
        assert!(
            forward * 100 / (specs.len() - 1) > 80,
            "expected mostly-increasing timestamps inside bursts, got {forward}/{}",
            specs.len() - 1
        );
    }

    #[test]
    fn cardinality_is_in_range() {
        const N: usize = 10_000;
        let specs: Vec<AssetSpec> = SpecStream::new(42).take(N).collect();

        let persona0 = specs.iter().filter(|s| s.persona_idx == 0).count();
        assert!(
            (3_500..=5_500).contains(&persona0),
            "persona 0 share out of band: {persona0}/{N}"
        );

        let trashed = specs.iter().filter(|s| s.trashed).count();
        assert!(
            (100..=300).contains(&trashed),
            "trashed share out of band: {trashed}/{N}"
        );

        let rated = specs.iter().filter(|s| s.rating.is_some()).count();
        assert!(
            (500..=900).contains(&rated),
            "rated share out of band: {rated}/{N}"
        );
        assert!(
            specs
                .iter()
                .filter_map(|s| s.rating)
                .all(|r| (1..=5).contains(&r))
        );

        let tag_total: usize = specs.iter().map(|s| s.tags.len()).sum();
        let tag_mean = tag_total as f64 / N as f64;
        assert!(
            (2.5..=3.5).contains(&tag_mean),
            "tag count mean out of band: {tag_mean}"
        );
        assert!(specs.iter().all(|s| s.tags.len() <= TAG_MAX));

        // Group bands, re-derived for the persona-0-only model:
        //
        //   grouped ≈ persona0 × (1 − 15 % ungrouped) ≈ 4,500 × 0.85 ≈ 3,825
        //   mega    ≈ grouped × 35 % (tier weight) ≈ 1,340, plus whatever
        //           the tail tiers overflow into it — no pool is anywhere
        //           near full at n = 10,000, so the fallback contributes
        //           almost nothing here
        //   multi   ≈ grouped × 20 % ≈ 765
        //
        // The bands are ±30 % around those figures: wide enough that a
        // seed change is not a failure, narrow enough that dropping the
        // persona-0 restriction (which would roughly double every count)
        // trips them.
        let grouped = specs.iter().filter(|s| !s.groups.is_empty()).count();
        assert!(
            (2_900..=4_600).contains(&grouped),
            "grouped share out of band: {grouped}/{N} (persona0={persona0})"
        );

        let mega = specs
            .iter()
            .filter(|s| s.groups.iter().any(|g| g == GROUP_MEGA_NAME))
            .count();
        assert!(
            (1_000..=1_800).contains(&mega),
            "bench-mega share out of band: {mega}/{N}"
        );

        let multi = specs.iter().filter(|s| s.groups.len() > 1).count();
        assert!(
            (600..=950).contains(&multi),
            "double membership share out of band: {multi}/{N}"
        );

        for spec in &specs {
            let unique: HashSet<&String> = spec.groups.iter().collect();
            assert_eq!(
                unique.len(),
                spec.groups.len(),
                "asset {} joined the same group twice: {:?}",
                spec.index,
                spec.groups
            );
            let unique_tags: HashSet<&String> = spec.tags.iter().collect();
            assert_eq!(unique_tags.len(), spec.tags.len());
            assert!(spec.labels.len() <= LABEL_MAX);
        }
    }

    #[test]
    fn only_persona_zero_is_filed_into_groups() {
        // The invariant the persona filter depends on: a group filter and
        // a persona filter must never subtract from each other, so nobody
        // but persona 0 carries a group at all.
        let specs: Vec<AssetSpec> = SpecStream::new(42).take(5_000).collect();
        for spec in &specs {
            if spec.persona_idx != 0 {
                assert!(
                    spec.groups.is_empty(),
                    "asset {} (persona {}) carries {:?}; groups are persona 0's alone",
                    spec.index,
                    spec.persona_idx,
                    spec.groups
                );
            }
        }
        assert!(
            specs
                .iter()
                .any(|s| s.persona_idx == 0 && !s.groups.is_empty()),
            "persona 0 must still be filed — an all-empty corpus would pass vacuously"
        );
    }

    #[test]
    fn the_l_preset_fills_the_scroll_targets() {
        // Why the L preset exists. The reference workload points are a
        // 10,000-member group and a 3,000-member one, and the tier
        // weights only reach those capacities once persona 0 has
        // contributed ~42,000 filings — which happens at 110,000 assets
        // and not at M (12,000). If this stops holding, the scroll bench
        // is measuring a group of whatever size the walk happened to
        // produce.
        const N: usize = 110_000;
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for spec in SpecStream::new(42).take(N) {
            for g in spec.groups {
                *counts.entry(g).or_default() += 1;
            }
        }
        assert_eq!(
            counts.get(GROUP_MEGA_NAME).copied().unwrap_or(0),
            GROUP_MEGA_CAPACITY,
            "bench-mega must be full at the L preset"
        );
        for idx in 0..GROUP_3K_COUNT {
            let name = group_name(GroupTier::ThreeK, idx);
            assert_eq!(
                counts.get(&name).copied().unwrap_or(0),
                GROUP_3K_CAPACITY,
                "{name} must be full at the L preset"
            );
        }
    }

    #[test]
    fn pools_respect_capacity() {
        const N: usize = 20_000;
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        let stream = SpecStream::new(42);
        let pools = group_pools(42);
        for spec in stream.take(N) {
            for g in spec.groups {
                *counts.entry(g).or_default() += 1;
            }
        }
        for pool in &pools {
            let realized = counts.get(&pool.name).copied().unwrap_or(0);
            assert!(
                realized <= pool.capacity,
                "{} overfilled: {realized} > {}",
                pool.name,
                pool.capacity
            );
        }
    }

    #[test]
    fn group_pool_table_is_seed_stable() {
        assert_eq!(group_pools(42), group_pools(42));
        assert_eq!(
            group_pools(42).len(),
            1 + GROUP_3K_COUNT + GROUP_MID_COUNT + GROUP_SMALL_COUNT
        );
        assert_eq!(group_name(GroupTier::Mid, 3), "bench-mid-03");
        assert_eq!(group_name(GroupTier::Small, 7), "bench-small-007");
        assert_eq!(group_name(GroupTier::ThreeK, 1), "bench-3k-1");
    }
}
