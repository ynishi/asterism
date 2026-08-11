//! Audio `RawItem` → `Footprint::Audio` parser.
//!
//! Metadata via `lofty` (pure-Rust, MIT). Covers MP3 / M4A (AAC in
//! MP4) / WAV / FLAC / OGG (Vorbis + Opus) plus WavPack / APE /
//! MPC / AIFF. Header-only reads — no decoding.

use std::io::Cursor;
use std::path::PathBuf;

use asterism_importer_sdk::{
    Audio, Footprint, FootprintSource, ParseError, RawItem, SIDECAR_SUFFIX, SourceParser,
};
use chrono::Utc;
use lofty::config::ParseOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use serde_json::{Value, json};

/// What the parser declares when it finds a sidecar. The server reads
/// the file and decides what it names — the parser only reports that
/// one is there.
const SIDECAR_DECLARATION: &str = "sidecar";

/// Turns a scanned audio file into `Footprint::Audio`.
pub struct AudioParser {
    platform: Option<String>,
}

impl AudioParser {
    pub fn new(platform: Option<String>) -> Self {
        Self { platform }
    }
}

impl SourceParser for AudioParser {
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

        let probed = probe_audio(&item.payload);
        let occurred_at = item.occurred_at.unwrap_or_else(Utc::now);

        let mut labels = vec!["audio".to_string()];
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
            "probe_seen": probed.is_some(),
        });
        // The three stream facts the probe measures that no column
        // holds. `Footprint::Audio` carries all three, but only `codec`
        // survives the trip to `AssetSpec` — as a `codec:<slug>` label,
        // which is a facet key rather than the value — and `sample_rate`
        // / `channels` were being dropped there outright. Recording them
        // in the bag is what makes them readable at all; it is also why
        // the detail panel's Sample rate / Channels rows had nothing to
        // show since they were written.
        //
        // `codec` sits in both places on purpose: the label is matched
        // against, this is printed. See the video parser for why that is
        // two readers rather than a drifted copy.
        //
        // Inserted only when measured, not placed in the literal above as
        // `Option`s: `json!` renders `None` as a `null` member, so an
        // unprobed container would carry three null keys the panel then
        // has to tell apart from "measured as zero".
        if let (Some(probed), Some(bag)) = (probed.as_ref(), extra.as_object_mut()) {
            if let Some(codec) = &probed.codec {
                bag.insert("codec".into(), Value::String(codec.clone()));
            }
            if let Some(sample_rate) = probed.sample_rate {
                bag.insert("sample_rate".into(), Value::from(sample_rate));
            }
            if let Some(channels) = probed.channels {
                bag.insert("channels".into(), Value::from(channels));
            }
        }

        let alt = if stem.is_empty() { None } else { Some(stem) };

        // Same rule as image / video: an exporter-written
        // `<name>.meta.json` sitting next to the file marks a return
        // trip. Declare it; the server owns the lookup.
        let derived_from = std::path::Path::new(&format!("{}{}", item.locator, SIDECAR_SUFFIX))
            .is_file()
            .then(|| SIDECAR_DECLARATION.to_string());

        Ok(vec![Footprint::Audio(Audio {
            source: FootprintSource {
                kind: item.source_kind,
                locator: item.locator,
                platform: self.platform.clone(),
                external_id: None,
            },
            occurred_at,
            alt,
            duration_ms: probed.as_ref().and_then(|p| p.duration_ms),
            file_size_bytes,
            codec: probed.as_ref().and_then(|p| p.codec.clone()),
            sample_rate: probed.as_ref().and_then(|p| p.sample_rate),
            channels: probed.as_ref().and_then(|p| p.channels),
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

struct ProbeFields {
    duration_ms: Option<u64>,
    codec: Option<String>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
}

fn probe_audio(payload: &[u8]) -> Option<ProbeFields> {
    let reader = Cursor::new(payload);
    let probe = Probe::new(reader).options(ParseOptions::new());
    // guess_file_type reads the magic bytes; then read() parses
    // header/metadata for whatever container came back.
    let tagged = probe.guess_file_type().ok()?.read().ok()?;
    let props = tagged.properties();

    let duration_ms = {
        let d = props.duration();
        if d.as_millis() == 0 {
            None
        } else {
            Some(d.as_millis() as u64)
        }
    };
    let sample_rate = props.sample_rate();
    let channels = props.channels().map(u16::from);

    // lofty exposes the container's file_type — turn it into a
    // canonical codec slug so downstream can facet by codec.
    let codec = codec_slug(tagged.file_type());

    Some(ProbeFields {
        duration_ms,
        codec,
        sample_rate,
        channels,
    })
}

fn codec_slug(kind: lofty::file::FileType) -> Option<String> {
    use lofty::file::FileType::*;
    Some(match kind {
        Mpeg => "mp3".into(),
        Mp4 => "aac".into(),
        Flac => "flac".into(),
        Vorbis => "vorbis".into(),
        Opus => "opus".into(),
        Wav => "pcm".into(),
        Aiff => "aiff".into(),
        WavPack => "wavpack".into(),
        Ape => "ape".into(),
        Speex => "speex".into(),
        Custom(name) => name.to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a committed fixture produced by
    /// `scripts/gen-test-fixtures.py`.
    ///
    /// These used to be third-party recordings fetched by URL at test
    /// time. They are now one second of a 440 Hz sine per container,
    /// generated locally: the assertions below only ever needed a real
    /// container header, and downloading someone else's recording to
    /// get one left the repository redistributing files under licences
    /// it does not carry.
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

    fn parse_fixture(name: &str) -> Audio {
        let bytes = fixture_bytes(name);
        let parser = AudioParser::new(Some("test".into()));
        let item = RawItem {
            source_kind: "fs".into(),
            locator: format!("tests/fixtures/{name}"),
            payload: bytes,
            occurred_at: None,
            extra: json!({}),
        };
        let out = parser.parse(item).expect("parse ok");
        match out.into_iter().next().expect("one Audio footprint") {
            Footprint::Audio(a) => a,
            _ => panic!("first footprint should be Audio"),
        }
    }

    #[test]
    fn fixture_mp3_lands_with_duration_and_sample_rate() {
        let a = parse_fixture("tone.mp3");
        assert!(a.labels.iter().any(|l| l == "mp3"));
        assert_eq!(a.codec.as_deref(), Some("mp3"));
        assert!(a.duration_ms.is_some_and(|d| d > 0), "MP3 duration_ms");
        assert!(a.sample_rate.is_some_and(|r| r > 0), "MP3 sample_rate");

        // The three stream facts reach the bag, which is the only place
        // two of them reach at all: `audio_to_spec` keeps `duration_ms`
        // (a column) and turns `codec` into a `codec:<slug>` label, and
        // dropped `sample_rate` / `channels` on the floor. Measured and
        // then discarded is the state the detail panel's Sample rate /
        // Channels rows were reading against.
        assert_eq!(
            a.extra.get("codec").and_then(|v| v.as_str()),
            a.codec.as_deref(),
            "codec reaches the bag verbatim"
        );
        assert_eq!(
            a.extra.get("sample_rate").and_then(|v| v.as_u64()),
            a.sample_rate.map(u64::from),
            "sample_rate reaches the bag verbatim"
        );
        assert_eq!(
            a.extra.get("channels").and_then(|v| v.as_u64()),
            a.channels.map(u64::from),
            "channels reaches the bag verbatim"
        );
    }

    /// A container the probe cannot read states **nothing** about the
    /// stream, rather than stating three nulls.
    ///
    /// The keys are inserted only when measured, so `{#if extra.codec}`
    /// in the detail panel is deciding between "measured" and "not
    /// present" instead of having to tell a real value apart from an
    /// explicit `null` — the shape a `json!` literal carrying `Option`s
    /// would have produced. The sibling contract on the video side is
    /// `a_container_neither_probe_reads_still_lands`.
    #[test]
    fn an_unreadable_container_states_no_stream_facts() {
        let parser = AudioParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: "clips/opaque.aiff".into(),
                payload: b"not an audio container at all".to_vec(),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Audio(a) = &out[0] else {
            panic!("first footprint should be Audio")
        };
        assert_eq!(a.codec, None);
        assert_eq!(a.sample_rate, None);
        assert_eq!(a.channels, None);
        assert_eq!(
            a.extra.get("probe_seen"),
            Some(&Value::Bool(false)),
            "the probe miss is still recorded"
        );
        let bag = a.extra.as_object().expect("extra is an object");
        for key in ["codec", "sample_rate", "channels"] {
            assert!(
                !bag.contains_key(key),
                "an unprobed container must not carry a null `{key}`: {:?}",
                a.extra
            );
        }
    }

    #[test]
    fn fixture_m4a_lands() {
        let a = parse_fixture("tone.m4a");
        assert!(a.labels.iter().any(|l| l == "m4a"));
        assert_eq!(a.codec.as_deref(), Some("aac"));
    }

    #[test]
    fn fixture_wav_lands_with_pcm_codec() {
        let a = parse_fixture("tone.wav");
        assert!(a.labels.iter().any(|l| l == "wav"));
        assert_eq!(a.codec.as_deref(), Some("pcm"));
        assert!(a.duration_ms.is_some_and(|d| d > 0));
        assert!(a.sample_rate.is_some_and(|r| r > 0));
    }

    #[test]
    fn fixture_flac_lands() {
        let a = parse_fixture("tone.flac");
        assert!(a.labels.iter().any(|l| l == "flac"));
        assert_eq!(a.codec.as_deref(), Some("flac"));
        assert!(a.duration_ms.is_some_and(|d| d > 0));
    }

    #[test]
    fn fixture_ogg_lands() {
        let a = parse_fixture("tone.ogg");
        assert!(a.labels.iter().any(|l| l == "ogg"));
        // Ogg can carry Vorbis / Opus / Speex — accept any known slug
        // rather than pinning to one.
        assert!(a.codec.is_some(), "OGG codec detected");
        assert!(a.duration_ms.is_some_and(|d| d > 0));
    }

    #[test]
    fn a_sidecar_next_to_the_file_is_declared_as_the_origin() {
        // The return leg of a synthesis round trip — same rule as
        // image and video.
        let tmp = tempfile::tempdir().expect("tempdir");
        let audio_path = tmp.path().join("returned.wav");
        std::fs::write(
            format!("{}.meta.json", audio_path.display()),
            r#"{"id":"0198c1c2-0000-7000-8000-000000000001"}"#,
        )
        .expect("write sidecar");

        let parser = AudioParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: audio_path.display().to_string(),
                payload: Vec::new(),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Audio(a) = &out[0] else {
            panic!("first footprint should be Audio")
        };
        assert_eq!(a.derived_from.as_deref(), Some("sidecar"));
    }

    #[test]
    fn a_file_without_a_sidecar_declares_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let audio_path = tmp.path().join("plain.wav");

        let parser = AudioParser::new(None);
        let out = parser
            .parse(RawItem {
                source_kind: "fs".into(),
                locator: audio_path.display().to_string(),
                payload: Vec::new(),
                occurred_at: None,
                extra: json!({}),
            })
            .expect("parse ok");
        let Footprint::Audio(a) = &out[0] else {
            panic!("first footprint should be Audio")
        };
        assert_eq!(a.derived_from, None);
    }
}
