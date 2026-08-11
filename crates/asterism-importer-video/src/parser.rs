//! Video `RawItem` → `Footprint::Video` parser.
//!
//! Two probe layers, tried in order: `mp4parse` for ISOBMFF (MP4 /
//! MOV), then `matroska` for EBML (WebM / MKV). Each rejects the
//! other's container at the first magic bytes, so the order is cost
//! only, not correctness. A container neither probe reads (AVI) still
//! lands — as a footprint without dims / duration / codec, which is
//! the contract for every probe miss.

use std::path::PathBuf;

use asterism_importer_sdk::{
    Footprint, FootprintSource, ParseError, RawItem, SIDECAR_SUFFIX, SourceParser, Video,
};
use asterism_media_probe::ProbeSource;
use chrono::Utc;
use serde_json::{Value, json};

/// What the parser declares when it finds a sidecar. The server reads
/// the file and decides what it names — the parser only reports that
/// one is there.
const SIDECAR_DECLARATION: &str = "sidecar";

/// Turns a scanned video file into `Footprint::Video`.
pub struct VideoParser {
    platform: Option<String>,
}

impl VideoParser {
    /// Builds a parser tagged with `platform`.
    pub fn new(platform: Option<String>) -> Self {
        Self { platform }
    }
}

impl SourceParser for VideoParser {
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

        let file_size_bytes = item
            .extra
            .get("file_size_bytes")
            .and_then(|v| v.as_u64())
            .or(Some(item.payload.len() as u64));

        let probed = asterism_media_probe::probe(&item.payload);

        let occurred_at = item.occurred_at.unwrap_or_else(Utc::now);
        let dims = probed.as_ref().and_then(|m| m.dims);
        let duration_ms = probed.as_ref().and_then(|m| m.duration_ms);
        let codec = probed.as_ref().and_then(|m| m.codec.clone());
        let framerate = None; // neither probe exposes this cheaply

        let mut labels = vec!["video".to_string()];
        if let Some(ext) = &extension {
            labels.push(ext.clone());
        }
        if !parent_dir.is_empty() {
            labels.push(parent_dir.clone());
        }

        let mut extra = json!({
            "filename": path.file_name().map(|s| s.to_string_lossy().to_string()),
            "parent_dir": parent_dir,
            "container": extension,
            // Which reader answered, kept as the two booleans this bag
            // has always carried rather than one slug: a consumer
            // reading `mp4_probe_seen == false` off an older row means
            // the same thing as one reading it off a new one.
            "mp4_probe_seen": probed.as_ref().is_some_and(|p| p.source == ProbeSource::Isobmff),
            "matroska_probe_seen": probed.as_ref().is_some_and(|p| p.source == ProbeSource::Ebml),
        });
        // The codec is the one thing the probe measures that no column
        // holds, so the bag is where it has to live for a reader that
        // wants the value rather than a facet.
        //
        // It is **not** a drifted copy of the `codec:<slug>` label
        // `video_to_spec` also emits: the two have different readers. The
        // label is a facet key — it exists to be matched against, and
        // shares a namespace with every other label. This is the value,
        // for a panel that prints it. Deriving one from the other at read
        // time would mean parsing a label prefix back out, which is the
        // shape that rots when a slug gains a colon.
        //
        // Inserted only when the probe answered, rather than placed in
        // the literal above as an `Option`: `json!` renders `None` as a
        // `null` member, so an unreadable container would carry a
        // `"codec": null` the panel then has to special-case.
        // `a_container_neither_probe_reads_still_lands` fixes that
        // contract for the fields beside it.
        if let Some(codec) = &codec
            && let Some(bag) = extra.as_object_mut()
        {
            bag.insert("codec".into(), Value::String(codec.clone()));
        }

        let alt = if stem.is_empty() { None } else { Some(stem) };

        // A generated clip travels with a `<name>.meta.json` sidecar
        // naming the export it came out of — and for video the sidecar
        // is the *only* trustworthy carrier, since mp4 embedded
        // metadata is dropped by common tooling. Declare it and let
        // the server resolve the link.
        let derived_from = std::path::Path::new(&format!("{}{}", item.locator, SIDECAR_SUFFIX))
            .is_file()
            .then(|| SIDECAR_DECLARATION.to_string());

        Ok(vec![Footprint::Video(Video {
            source: FootprintSource {
                kind: item.source_kind,
                locator: item.locator,
                platform: self.platform.clone(),
                external_id: None,
            },
            occurred_at,
            alt,
            dims,
            duration_ms,
            file_size_bytes,
            codec,
            framerate,
            labels,
            bundle_id: None,
            extra,
            derived_from,
            // No built-in reader mints a statement out of a container
            // (`asterism_importer_sdk::Image::album_meta` records why).
            album_meta: Default::default(),
        })])
    }
}

// The two container probes and their codec-slug maps moved to
// `asterism-media-probe`. They are the measurement, and the server has
// to reach the same one when it backfills; what stays here is which
// fields this parser puts on a `Footprint::Video`.

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a committed fixture produced by
    /// `scripts/gen-test-fixtures.py` — ffmpeg's `testsrc` pattern, one
    /// file per container probe path.
    ///
    /// These were previously a Mozilla ISOBMFF stub, an
    /// openpreserve/format-corpus MOV and a Big Buck Bunny clip fetched
    /// by URL. Generating them locally keeps a CC-BY video out of an
    /// MIT-OR-Apache-2.0 repository, and makes each asserted number
    /// (320×240, 640×360, ~1 s, ~10 s, vp9) something this repo sets
    /// rather than something it discovered about someone else's file.
    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "fixture missing at {} ({e}). Regenerate with:\n  \
                 python3 scripts/gen-test-fixtures.py",
                path.display(),
            )
        })
    }

    fn parse_fixture(name: &str) -> Video {
        let bytes = fixture_bytes(name);
        let parser = VideoParser::new(Some("test".into()));
        let item = RawItem {
            source_kind: "fs".into(),
            locator: format!("tests/fixtures/{name}"),
            payload: bytes,
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).expect("parse ok");
        match out.into_iter().next().expect("one Video footprint") {
            Footprint::Video(v) => v,
            _ => panic!("first footprint should be Video"),
        }
    }

    #[test]
    fn fixture_mp4_minimal_lands() {
        // A one-second H.264 MP4: ftyp + moov with a real track, which
        // is what the ISOBMFF probe is asked to read here.
        let v = parse_fixture("testsrc.mp4");
        assert!(v.labels.iter().any(|l| l == "mp4"));
        assert!(v.labels.iter().any(|l| l == "video"));
        // The parsed shape is the point of the fixture, so assert it
        // rather than printing it for a human to eyeball — a printed
        // value nobody reads is not a test.
        assert!(v.dims.is_some(), "minimal.mp4 should yield dimensions");
        assert!(
            v.duration_ms.is_some(),
            "minimal.mp4 should yield a duration"
        );
        assert!(v.codec.is_some(), "minimal.mp4 should yield a codec");
        // The bag carries the value, not just the facet. `video_to_spec`
        // turns the same codec into a `codec:<slug>` label, which is what
        // a filter matches on; a panel that wants to *print* the codec
        // has no column to read and would otherwise be parsing a label
        // prefix back apart.
        assert_eq!(
            v.extra.get("codec").and_then(|c| c.as_str()),
            v.codec.as_deref(),
            "the measured codec reaches the bag verbatim"
        );
    }

    #[test]
    fn fixture_mov_animation_lands_with_dims_and_duration() {
        // 320×240 / 25 fps / 1 s testsrc in a QuickTime container.
        // mp4parse handles MOV (ISOBMFF-derived), so this and the MP4
        // above differ only in the brand the probe sees.
        let v = parse_fixture("testsrc.mov");
        assert!(v.labels.iter().any(|l| l == "mov"));
        assert_eq!(v.dims, Some((320, 240)), "MOV header dims");
        assert!(
            v.duration_ms
                .map(|d| (900..=1100).contains(&d))
                .unwrap_or(false),
            "MOV ~1s duration (got {:?})",
            v.duration_ms
        );
    }

    #[test]
    fn fixture_webm_lands_with_dims_duration_and_codec() {
        // WebM is Matroska, not ISOBMFF — mp4parse rejects it and the
        // matroska probe takes over. 640×360, ~10 s, VP9. These were
        // None for as long as the second probe layer was a carry, so
        // each field is pinned individually.
        let v = parse_fixture("testsrc.webm");
        assert!(v.labels.iter().any(|l| l == "webm"));
        assert!(v.labels.iter().any(|l| l == "video"));
        assert!(v.file_size_bytes.is_some());
        assert_eq!(v.dims, Some((640, 360)), "WebM header dims");
        assert!(
            v.duration_ms
                .map(|d| (9_500..=10_500).contains(&d))
                .unwrap_or(false),
            "WebM ~10s duration (got {:?})",
            v.duration_ms
        );
        assert_eq!(v.codec.as_deref(), Some("vp9"), "WebM codec slug");
        assert_eq!(
            v.extra.get("matroska_probe_seen"),
            Some(&serde_json::Value::Bool(true)),
            "the probe provenance is recorded"
        );
    }

    #[test]
    fn a_container_neither_probe_reads_still_lands() {
        // The probe-miss contract the module doc promises (AVI today):
        // the footprint is emitted, the metadata fields stay None, and
        // both probe flags say so. Synthetic garbage stands in for any
        // unreadable container.
        let parser = VideoParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: "clips/opaque.avi".into(),
                payload: b"RIFF\x00\x00\x00\x00AVI LIST".to_vec(),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Video(v) = &out[0] else {
            panic!("first footprint should be Video")
        };
        assert_eq!(v.dims, None);
        assert_eq!(v.duration_ms, None);
        assert_eq!(v.codec, None);
        assert_eq!(
            v.extra.get("mp4_probe_seen"),
            Some(&serde_json::Value::Bool(false))
        );
        assert_eq!(
            v.extra.get("matroska_probe_seen"),
            Some(&serde_json::Value::Bool(false))
        );
        // **Absent, not `null`.** The codec is inserted only when the
        // probe answered, so an unreadable container carries no key at
        // all. Placing it in the `json!` literal as an `Option` would
        // leave `"codec": null` here, which every reader then has to
        // tell apart from a measured value — and the detail panel's
        // `{#if extra.codec}` would be the only thing standing between
        // that and a blank `Codec` row.
        assert!(
            !v.extra
                .as_object()
                .expect("extra is an object")
                .contains_key("codec"),
            "an unreadable container states no codec rather than a null one: {:?}",
            v.extra
        );
    }

    #[test]
    fn a_sidecar_next_to_the_file_is_declared_as_the_origin() {
        // The return leg of a generation round trip. For video this
        // is the only reliable carrier — mp4 embedded metadata is
        // dropped by common tooling — so the declaration must not be
        // an image-only privilege.
        let tmp = tempfile::tempdir().expect("tempdir");
        let video_path = tmp.path().join("returned.mp4");
        std::fs::write(
            format!("{}.meta.json", video_path.display()),
            r#"{"id":"0198c1c2-0000-7000-8000-000000000001"}"#,
        )
        .expect("write sidecar");

        let parser = VideoParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: video_path.display().to_string(),
                payload: Vec::new(),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Video(v) = &out[0] else {
            panic!("first footprint should be Video")
        };
        assert_eq!(v.derived_from.as_deref(), Some("sidecar"));
    }

    #[test]
    fn a_file_without_a_sidecar_declares_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let video_path = tmp.path().join("plain.mp4");

        let parser = VideoParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: video_path.display().to_string(),
                payload: Vec::new(),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Video(v) = &out[0] else {
            panic!("first footprint should be Video")
        };
        assert_eq!(v.derived_from, None);
    }
}
