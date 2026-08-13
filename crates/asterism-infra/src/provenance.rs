//! Writing a [`DisclosureRecord`] into a file that already exists.
//!
//! This is the adapter half of AI-disclosure provenance. `asterism-provenance`
//! decides *what is asserted* and renders both forms of it as values; this
//! module puts them into a file on disk, in the one order that works, and
//! signs the manifest when — and only when — a signing identity has been
//! configured.
//!
//! # Why it happens here rather than inside the generator
//!
//! Inside a generation graph only tensors move between nodes, so a signed
//! image fed through a save node is re-encoded from pixels and comes out
//! with the signature gone. Video is worse: the combine step discards
//! whatever frame-level record existed, and the manifest can only be
//! attached after the encode. Signing is therefore possible only in a
//! layer that holds the finished file, which is this application.
//!
//! # The order is not a preference
//!
//! ```text
//! read → XMP packet → C2PA manifest → one write
//! ```
//!
//! The manifest's hard binding is computed over the file's bytes, and on
//! a still image the XMP packet is part of them. Writing the packet after
//! signing therefore invalidates the signature — IPTC's own 2025.1
//! announcement carries a worked example whose caption records exactly
//! that outcome, so this is documented behaviour rather than a
//! deduction. [`ProvenanceWriter::apply`] does the two in this order and
//! [`tests::the_packet_is_written_before_the_manifest_is_signed`] is what
//! keeps them there.
//!
//! # What happens with no certificate
//!
//! The IPTC/XMP half still lands. That is the half platforms read most
//! widely, it needs no key material, and it is a legitimate disclosure on
//! its own — IPTC's guidance is to emit the XMP property *or* a C2PA
//! manifest, with both being better than either.
//!
//! The manifest half does not, and there is deliberately no fallback to
//! the test certificates the C2PA tooling ships with. A manifest signed
//! by them validates as untrusted, which is strictly worse than no
//! manifest: an absent manifest says nothing, and an untrusted one makes
//! a provenance claim that a validator actively rejects.
//! [`SigningIdentity::from_files`] refuses them by name.
//!
//! # The gap this leaves on video
//!
//! A still gets its XMP packet either way; MP4 and MOV get nothing
//! without a certificate. MP4 can carry XMP in a `uuid` box, but writing
//! BMFF boxes is not something this module does — the manifest path goes
//! through `c2pa`, which knows the container, and the packet path goes
//! through `asterism-provenance::embed`, which knows PNG and JPEG. Until
//! one of those two grows the other's format, an unsigned video export
//! carries no disclosure at all, and [`Stamped`] says so rather than
//! reporting a success it did not have.
//!
//! # Renditions are not signed
//!
//! The preview rendition path re-encodes through ffmpeg, which produces
//! a new file carrying none of the original's container tags, stream
//! tags, manifest or XMP. A rendition therefore cannot inherit a
//! signature and must not be given one of its own that would claim the
//! original's provenance for a derived file. Nothing here is wired to
//! that path; this note is why.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use asterism_provenance::record::DisclosureRecord;
use asterism_provenance::{Stamped, embed, manifest};

/// A container this module can write provenance into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// PNG — XMP as an `iTXt` chunk, manifest as a JUMBF chunk.
    Png,
    /// JPEG — XMP as an `APP1` segment, manifest in `APP11`.
    ///
    /// The two live in different segments, which is the detail that
    /// makes "we preserve EXIF" and "we preserve credentials" different
    /// claims: EXIF is `APP1` and a manifest is `APP11`, so a pipeline
    /// can honestly do the first while silently dropping the second.
    Jpeg,
    /// MP4 — manifest as a JUMBF box. No XMP half (module docs).
    Mp4,
    /// QuickTime MOV — same box, different brand.
    Mov,
}

impl Container {
    /// The MIME type `c2pa` dispatches its container handler on.
    fn mime(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Mp4 => "video/mp4",
            Self::Mov => "video/quicktime",
        }
    }

    /// Whether an XMP packet can be written into this container by
    /// [`asterism_provenance::embed`].
    fn takes_xmp(&self) -> bool {
        matches!(self, Self::Png | Self::Jpeg)
    }

    /// Identifies the container from the file's own leading bytes.
    ///
    /// From the bytes rather than from the extension or a stored MIME
    /// type, for the reason `embed::sniff` gives: both of those are
    /// statements made about a file by something that is not the file,
    /// and being wrong here rewrites framing in a container that does
    /// not have it.
    fn sniff(head: &[u8]) -> Option<Self> {
        match embed::sniff(head) {
            Some(embed::Container::Png) => return Some(Self::Png),
            Some(embed::Container::Jpeg) => return Some(Self::Jpeg),
            None => {}
        }
        // ISO base media: a `ftyp` box at the head, whose major brand
        // says which dialect. `qt  ` is QuickTime; everything else that
        // got this far is treated as MP4, which is what the brand list
        // (`isom`, `mp42`, `avc1`, `iso2`, …) actually contains.
        if head.get(4..8)? == b"ftyp" {
            return match head.get(8..12)? {
                b"qt  " => Some(Self::Mov),
                _ => Some(Self::Mp4),
            };
        }
        None
    }
}

/// What went wrong applying a record.
#[derive(Debug, thiserror::Error)]
pub enum ProvenanceError {
    /// The file could not be read, written, or replaced.
    #[error("provenance io on {path}: {source}")]
    Io {
        /// File the operation was against.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// The bytes are not a container this module writes into.
    #[error("{path} is not a container this build writes provenance into")]
    UnsupportedContainer {
        /// File that was offered.
        path: PathBuf,
    },
    /// The XMP packet could not be written.
    #[error("writing the XMP packet into {path}: {source}")]
    Xmp {
        /// File the packet was for.
        path: PathBuf,
        /// The underlying failure.
        source: embed::EmbedError,
    },
    /// A packet was written and this crate's own reader could not find
    /// it again.
    ///
    /// Not a fact about the record — a record with nothing to disclose
    /// produces no packet and never reaches here. It says the writer
    /// emitted something the reader does not recognise, which is a
    /// defect in one of the two halves and has to surface as one.
    ///
    /// It was previously folded into
    /// [`Stamped::prompt_dropped`](asterism_provenance::outcome::Stamped):
    /// a file carrying no readable disclosure at all was reported as a
    /// successful stamp that had merely shortened the prompt, and — when
    /// the record had no prompt to shorten — as an unqualified success.
    ///
    /// Nothing is known to reach it today. The one producer that was
    /// found is fixed (`asterism-provenance`'s JPEG writer put the
    /// packet after `EOI` for a file with no scan, where the reader
    /// cannot see it), and the two halves otherwise agree on where a
    /// packet goes. It stays as the guard that says so, because what
    /// makes it unreachable is an agreement between two modules rather
    /// than anything the compiler holds.
    #[error("wrote an XMP packet into {path} and could not read it back")]
    XmpUnreadable {
        /// File the packet was written into.
        path: PathBuf,
    },
    /// The manifest definition this build produces is not one `c2pa`
    /// accepts.
    ///
    /// Separate from [`Self::Sign`] because it happens strictly before
    /// signing: the certificate and key have already loaded, and nothing
    /// has been offered to the signer. Reported as
    /// [`Self::Identity`] it read as `signing identity: …`, which sends
    /// whoever is holding the log to their certificate configuration for
    /// a defect in this crate's record-to-JSON mapping.
    #[error("building the manifest definition for {path}: {source}")]
    Definition {
        /// File the manifest was for.
        path: PathBuf,
        /// The underlying failure.
        source: serde_json::Error,
    },
    /// The manifest could not be built or signed.
    #[error("signing a manifest for {path}: {source}")]
    Sign {
        /// File the manifest was for.
        path: PathBuf,
        /// The underlying failure.
        source: Box<c2pa::Error>,
    },
    /// The configured signing identity could not be loaded, or was one
    /// this build refuses to sign with.
    #[error("signing identity: {0}")]
    Identity(String),
}

/// The certificate and key a manifest is signed with.
///
/// Configuration, never a shipped asset: this repository has no
/// certificate to give, and the one that gets used decides whose claim
/// the manifest is.
pub struct SigningIdentity {
    cert_chain: Vec<u8>,
    private_key: Vec<u8>,
    alg: c2pa::crypto::raw_signature::SigningAlg,
    tsa_url: Option<String>,
}

impl std::fmt::Debug for SigningIdentity {
    /// Names the algorithm and nothing else.
    ///
    /// The private key is in this struct, and a derived `Debug` would
    /// put it into any log line, panic message or error report that
    /// formatted the value — including ones written years from now by
    /// somebody who never read this file.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningIdentity")
            .field("alg", &self.alg)
            .field("tsa_url", &self.tsa_url)
            .field("cert_chain", &"<redacted>")
            .field("private_key", &"<redacted>")
            .finish()
    }
}

/// Substring that identifies the certificates the C2PA tooling ships for
/// its own tests.
///
/// Matched against the DER inside the PEM, where an X.509 subject's
/// common name sits as plain ASCII, so no certificate parser is needed
/// to catch the case that matters. It is a heuristic and is described as
/// one: it catches the test certificates by the name they carry
/// ("C2PA Test Signing Cert", "C2PA Test Root CA"), and it is not a
/// trust decision. Whether a real certificate chains to anything is a
/// validator's question, asked against the published trust list, and it
/// is not one a signer can answer about itself.
const TEST_CERT_MARKER: &[u8] = b"C2PA Test";

impl SigningIdentity {
    /// Loads an identity from a certificate chain and a private key.
    ///
    /// `alg` is a COSE algorithm name (`es256`, `ps256`, `ed25519`, …)
    /// as `c2pa` spells them.
    pub fn from_files(
        cert_chain: &Path,
        private_key: &Path,
        alg: &str,
        tsa_url: Option<String>,
    ) -> Result<Self, ProvenanceError> {
        let cert_chain = std::fs::read(cert_chain).map_err(|e| {
            ProvenanceError::Identity(format!("reading {}: {e}", cert_chain.display()))
        })?;
        let private_key = std::fs::read(private_key).map_err(|e| {
            ProvenanceError::Identity(format!("reading {}: {e}", private_key.display()))
        })?;
        Self::from_bytes(cert_chain, private_key, alg, tsa_url)
    }

    /// Loads an identity from material already in memory. Same checks as
    /// [`from_files`](Self::from_files); split out so the tests can
    /// exercise the refusal without writing key material to disk.
    pub fn from_bytes(
        cert_chain: Vec<u8>,
        private_key: Vec<u8>,
        alg: &str,
        tsa_url: Option<String>,
    ) -> Result<Self, ProvenanceError> {
        if names_a_test_certificate(&cert_chain) {
            return Err(ProvenanceError::Identity(
                "this is a C2PA test certificate: a manifest signed with it validates as \
                 untrusted, which claims a provenance a reader will reject. Configure a real \
                 signing identity, or export without a manifest — the IPTC/XMP disclosure is \
                 written either way"
                    .into(),
            ));
        }
        let alg = alg
            .parse::<c2pa::crypto::raw_signature::SigningAlg>()
            .map_err(|e| ProvenanceError::Identity(format!("unknown signing algorithm: {e}")))?;
        Ok(Self {
            cert_chain,
            private_key,
            alg,
            tsa_url,
        })
    }

    /// Builds the `c2pa` signer for one signing operation.
    fn signer(&self) -> Result<c2pa::BoxedSigner, c2pa::Error> {
        c2pa::create_signer::from_keys(
            &self.cert_chain,
            &self.private_key,
            self.alg,
            self.tsa_url.clone(),
        )
    }
}

/// Whether a PEM bundle carries one of the C2PA test certificates.
///
/// Decodes each `CERTIFICATE` block's base64 and looks for
/// [`TEST_CERT_MARKER`] in the DER. Anything undecodable is skipped
/// rather than refused: a bundle this cannot read is a bundle `c2pa` is
/// about to reject on its own terms, with a better message than a guess
/// from here.
fn names_a_test_certificate(pem: &[u8]) -> bool {
    // The marker also appears literally in a PEM whose comment lines
    // name the file, which is the cheapest of the two checks.
    if contains(pem, TEST_CERT_MARKER) {
        return true;
    }
    let Ok(text) = std::str::from_utf8(pem) else {
        return false;
    };
    text.split("-----BEGIN CERTIFICATE-----")
        .skip(1)
        .filter_map(|block| block.split("-----END CERTIFICATE-----").next())
        .filter_map(|body| {
            use base64::Engine as _;
            let compact: String = body.split_whitespace().collect();
            base64::engine::general_purpose::STANDARD
                .decode(compact)
                .ok()
        })
        .any(|der| contains(&der, TEST_CERT_MARKER))
}

/// Substring search over bytes.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Applies disclosure records to files.
///
/// Cheap to build and holds no file state; one is constructed per
/// process from settings and shared.
#[derive(Debug, Default)]
pub struct ProvenanceWriter {
    /// Behind an `Arc` so the port impl can hand a copy to a blocking
    /// task without duplicating key material on every call — a signing
    /// identity holds a private key, and the fewer copies of it exist in
    /// the process at once, the better.
    identity: Option<std::sync::Arc<SigningIdentity>>,
}

impl ProvenanceWriter {
    /// A writer that emits the IPTC/XMP half and no manifest.
    ///
    /// The state every install starts in, and a supported one rather
    /// than a degraded one — see the module docs on why an untrusted
    /// manifest is worse than none.
    pub fn unsigned() -> Self {
        Self { identity: None }
    }

    /// A writer that also signs.
    pub fn signed_with(identity: SigningIdentity) -> Self {
        Self {
            identity: Some(std::sync::Arc::new(identity)),
        }
    }

    /// Whether this writer can produce a manifest.
    pub fn can_sign(&self) -> bool {
        self.identity.is_some()
    }

    /// Writes `record` into the file at `path`, replacing it.
    ///
    /// The file is rewritten through a sibling temporary and a rename,
    /// so a failure part-way leaves the original intact rather than a
    /// half-stamped file that still looks like an export.
    pub fn apply(
        &self,
        path: &Path,
        record: &DisclosureRecord,
    ) -> Result<Stamped, ProvenanceError> {
        let io = |source: std::io::Error| ProvenanceError::Io {
            path: path.to_path_buf(),
            source,
        };

        // Enough bytes to identify any container here: PNG's signature
        // is 8, JPEG's is 2, and a `ftyp` brand ends at 12.
        let head = read_head(path, 12).map_err(io)?;
        let container =
            Container::sniff(&head).ok_or_else(|| ProvenanceError::UnsupportedContainer {
                path: path.to_path_buf(),
            })?;

        let mut outcome = Stamped::default();

        // --- 1. the XMP packet, on the containers that take one -----
        //
        // Held in memory rather than written yet: if a manifest follows,
        // it has to be signed over these bytes, and a write in between
        // would be a file on disk in a state neither half asked for.
        let mut staged: Option<Vec<u8>> = None;
        if container.takes_xmp() && record.discloses_anything() {
            let original = std::fs::read(path).map_err(io)?;
            let stamped =
                embed::stamp(&original, record).map_err(|source| ProvenanceError::Xmp {
                    path: path.to_path_buf(),
                    source,
                })?;
            if let Some(bytes) = stamped {
                // `embed::stamp` falls back to the reduced record when a
                // JPEG segment cannot hold the packet. Reading the
                // result back is how this side learns that happened —
                // the alternative is re-deriving the decision here,
                // which would be a second place it could differ.
                //
                // The read has three outcomes and only one of them is
                // that. A packet that fails to parse, or that the reader
                // cannot find at all, says the write produced something
                // this crate does not recognise — a defect, not a fact
                // about the record — so those two are errors. Folding
                // all three into the flag (which `.ok().flatten()` did)
                // reported "the mark landed, the prompt did not" about a
                // file that had neither.
                let written = embed::read_xmp(&bytes)
                    .map_err(|source| ProvenanceError::Xmp {
                        path: path.to_path_buf(),
                        source,
                    })?
                    .ok_or_else(|| ProvenanceError::XmpUnreadable {
                        path: path.to_path_buf(),
                    })?;
                outcome.prompt_dropped =
                    record.prompt.is_some() && !written.contains("AIPromptInformation");
                outcome.xmp_written = true;
                staged = Some(bytes);
            }
        }

        // --- 2. the manifest, when there is an identity to sign it ---
        if let Some(identity) = &self.identity {
            let signer = identity.signer().map_err(|source| ProvenanceError::Sign {
                path: path.to_path_buf(),
                source: Box::new(source),
            })?;
            // `default()` rather than `new()`: the latter is deprecated
            // in favour of passing settings through a `Context`, and
            // this path deliberately configures none — no trust list is
            // consulted while signing, and no remote manifest is
            // fetched.
            let mut builder = c2pa::Builder::default();
            builder.definition =
                serde_json::from_value(manifest::definition(record)).map_err(|source| {
                    ProvenanceError::Definition {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;

            // The signed output goes straight into the temporary the
            // rename will move, rather than into a `Vec` handed to
            // `replace` afterwards. That buffer held the whole signed
            // asset — for a video, the file plus its manifest, resident
            // at once — which is the half of the old comment on the
            // streaming branch below that was not true: the source
            // streamed and the destination did not.
            //
            // Opened read-write, not `File::create`. `Builder::sign`
            // takes `W: Write + Read + Seek`, and the `Read` is not
            // decorative — the BMFF handler re-reads box headers out of
            // the destination to adjust offsets. `File::create` gives a
            // write-only descriptor, which today's signing happens not
            // to read from, and a version of `c2pa` that did would fail
            // with `EBADF` on the video path only.
            //
            // The temporary is now visible for the whole signing
            // operation rather than for the length of one `write`. An
            // importer scanning the export directory can see it growing;
            // that is the cost of not holding the asset in memory, and
            // the extension is the same one the short-lived version
            // used.
            let temporary = temporary_for(path);
            let signing = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temporary)
                .map_err(|e| ProvenanceError::Io {
                    // The target, not the temporary: the temporary is
                    // removed below, so naming it would hand the reader
                    // a filename that no longer exists.
                    path: path.to_path_buf(),
                    source: e,
                })
                .and_then(|mut destination| match &staged {
                    // Sign over the XMP-stamped bytes: the hard binding
                    // covers the packet, so signing the original and
                    // then writing the packet would invalidate what was
                    // signed. These are already in memory — the packet
                    // was written into them — so there is nothing to
                    // stream from.
                    Some(bytes) => {
                        let mut source = Cursor::new(bytes.as_slice());
                        builder
                            .sign(
                                signer.as_ref(),
                                container.mime(),
                                &mut source,
                                &mut destination,
                            )
                            .map_err(|failure| ProvenanceError::Sign {
                                path: path.to_path_buf(),
                                source: Box::new(failure),
                            })
                    }
                    // Nothing staged: either the container takes no
                    // packet (video) or there was nothing to disclose in
                    // one. Stream from the file, so neither end of a
                    // large video is read into memory whole.
                    None => std::fs::File::open(path)
                        .map_err(|e| ProvenanceError::Io {
                            path: path.to_path_buf(),
                            source: e,
                        })
                        .and_then(|mut source| {
                            builder
                                .sign(
                                    signer.as_ref(),
                                    container.mime(),
                                    &mut source,
                                    &mut destination,
                                )
                                .map_err(|failure| ProvenanceError::Sign {
                                    path: path.to_path_buf(),
                                    source: Box::new(failure),
                                })
                        }),
                });
            if let Err(e) = signing {
                // Same reasoning as `commit`'s: a `.c2pa-partial` left
                // in an export directory an importer is watching turns
                // one failure into a second artefact.
                let _ = std::fs::remove_file(&temporary);
                return Err(e);
            }
            commit(&temporary, path).map_err(io)?;
            outcome.manifest_written = true;
            // The file on disk is already the finished one.
            staged = None;
        }

        if let Some(bytes) = staged {
            replace(path, &bytes).map_err(io)?;
        }
        Ok(outcome)
    }
}

#[async_trait::async_trait]
impl asterism_core::application::provenance_service::ProvenanceWriter for ProvenanceWriter {
    /// Adapts the writer to the core's port.
    ///
    /// Two things happen at this boundary and both are deliberate.
    ///
    /// The work runs on a blocking thread. Applying a record reads a
    /// file, may sign it, and writes it back — for a video that is
    /// hundreds of megabytes of synchronous I/O and a hash over all of
    /// it, which is exactly the shape that stalls an async runtime's
    /// worker for the duration.
    ///
    /// And every failure collapses to one `DomainError`. The core has
    /// no vocabulary for a JUMBF box or an APP1 segment and should not
    /// grow one; what it can act on is that the file was not stamped
    /// and why, in words. The typed variants stay on this side for the
    /// caller that is in this crate.
    async fn apply(
        &self,
        path: &std::path::Path,
        record: &DisclosureRecord,
    ) -> Result<Stamped, asterism_core::error::DomainError> {
        let path = path.to_path_buf();
        let record = record.clone();
        let writer = ProvenanceWriter {
            identity: self.identity.clone(),
        };
        tokio::task::spawn_blocking(move || writer.apply(&path, &record))
            .await
            .map_err(|e| {
                asterism_core::error::DomainError::Infra(anyhow::anyhow!(
                    "provenance task did not finish: {e}"
                ))
            })?
            .map_err(|e| asterism_core::error::DomainError::Infra(anyhow::anyhow!("{e}")))
    }
}

/// Reads the first `n` bytes, or the whole file when it is shorter.
///
/// `take` and `read_to_end` rather than one `read`, which is permitted
/// to return fewer bytes than the buffer holds without being at end of
/// file. On a local regular file it does not — the short return needs a
/// filesystem whose `read` legitimately gives partial counts, which is
/// NFS, SMB and FUSE, i.e. every network-mounted library. Eleven bytes
/// back instead of twelve made `Container::sniff` fail its `ftyp` test
/// and the caller report a perfectly good MP4 as a container this build
/// does not write into.
fn read_head(path: &Path, n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut head = Vec::with_capacity(n);
    file.take(n as u64).read_to_end(&mut head)?;
    Ok(head)
}

/// The sibling temporary a rewrite is staged in.
///
/// Same directory as the target so the rename stays within one
/// filesystem — across a mount boundary `rename` fails with `EXDEV`,
/// and the fallback (copy, then delete) is exactly the non-atomic
/// behaviour this is here to avoid.
fn temporary_for(path: &Path) -> PathBuf {
    path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.c2pa-partial"),
        None => "c2pa-partial".to_string(),
    })
}

/// Moves a finished temporary over its target.
fn commit(temporary: &Path, path: &Path) -> std::io::Result<()> {
    match std::fs::rename(temporary, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // The temporary is this module's litter, and leaving it
            // beside a file an importer may be watching turns one
            // failure into a second artefact.
            let _ = std::fs::remove_file(temporary);
            Err(e)
        }
    }
}

/// Replaces `path`'s contents through a sibling temporary and a rename.
///
/// Sibling for the reason [`temporary_for`] gives. A failed write clears
/// the temporary as a failed rename does: `fs::write` can fail with the
/// file already created and partly filled, and the old shape returned
/// straight out of that, leaving exactly the litter [`commit`] exists to
/// avoid.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = temporary_for(path);
    match std::fs::write(&temporary, bytes) {
        Ok(()) => commit(&temporary, path),
        Err(e) => {
            let _ = std::fs::remove_file(&temporary);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_provenance::DigitalSourceType;

    /// The same hand-built 1×1 PNG the provenance crate's own tests use.
    /// Built here rather than imported so this crate's tests do not
    /// depend on another crate's test module.
    fn png_fixture() -> Vec<u8> {
        fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(kind);
            hasher.update(payload);
            out.extend_from_slice(&hasher.finalize().to_be_bytes());
            out
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]));
        png.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x02]));
        png.extend_from_slice(&chunk(b"IEND", &[]));
        png
    }

    fn record() -> DisclosureRecord {
        DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
            .with_ai_system("ComfyUI", None)
    }

    /// A throwaway signing identity, generated per call and never
    /// written outside the test's temporary directory.
    ///
    /// It chains to nothing, so a validator asked about trust will say
    /// so — which is the correct answer and not what these tests are
    /// about. What it buys is that everything downstream of "a
    /// certificate exists" is executed rather than described: the
    /// manifest is really built, really signed, and really read back.
    /// Without it the whole signing path would be untested on a
    /// repository that ships no certificate and refuses the C2PA test
    /// ones.
    fn throwaway_identity() -> SigningIdentity {
        let key = rcgen::KeyPair::generate().expect("a P-256 key pair");
        let mut params = rcgen::CertificateParams::new(vec!["asterism.invalid".to_string()])
            .expect("certificate parameters");
        // The C2PA certificate profile, which `c2pa` enforces when it
        // signs rather than leaving to a validator. Four requirements,
        // and a certificate generated with defaults meets none of the
        // last three, so this list is also the answer to "why was my
        // certificate refused with `InvalidCertificate`":
        //
        // 1. `CA:FALSE` for an end-entity certificate — and a self-signed
        //    *CA* certificate is refused outright, so the signing
        //    certificate cannot be its own issuer while claiming to be
        //    an authority.
        // 2. `digitalSignature` key usage.
        // 3. An extended key usage that is present, is not
        //    `anyExtendedKeyUsage`, and is on the allowed list —
        //    `emailProtection` is the one the C2PA specification names
        //    for document signing.
        // 4. Both an authority key identifier and a subject key
        //    identifier extension.
        params.is_ca = rcgen::IsCa::ExplicitNoCa;
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection];
        params.use_authority_key_identifier_extension = true;
        let cert = params.self_signed(&key).expect("a self-signed certificate");
        SigningIdentity::from_bytes(
            cert.pem().into_bytes(),
            key.serialize_pem().into_bytes(),
            // rcgen's default key pair is ECDSA P-256 with SHA-256.
            "es256",
            None,
        )
        .expect("a generated certificate is not a C2PA test certificate")
    }

    #[test]
    fn containers_are_identified_from_their_own_bytes() {
        assert_eq!(Container::sniff(&png_fixture()), Some(Container::Png));
        assert_eq!(
            Container::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(Container::Jpeg)
        );
        let mut mp4 = vec![0, 0, 0, 0x20];
        mp4.extend_from_slice(b"ftypisom");
        assert_eq!(Container::sniff(&mp4), Some(Container::Mp4));
        let mut mov = vec![0, 0, 0, 0x14];
        mov.extend_from_slice(b"ftypqt  ");
        assert_eq!(Container::sniff(&mov), Some(Container::Mov));
        assert_eq!(Container::sniff(b"GIF89a......"), None);
    }

    #[test]
    fn an_unsigned_writer_still_writes_the_iptc_disclosure() {
        // The state every install starts in. If this produced nothing,
        // the certificate question would block the half of the
        // obligation that does not need one.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        let outcome = ProvenanceWriter::unsigned()
            .apply(&path, &record())
            .unwrap();
        assert!(outcome.xmp_written);
        assert!(!outcome.manifest_written);
        assert!(outcome.discloses());

        let packet = embed::read_xmp(&std::fs::read(&path).unwrap())
            .unwrap()
            .expect("the file carries a packet");
        assert!(packet.contains("trainedAlgorithmicMedia"));
    }

    #[test]
    fn a_record_with_nothing_to_disclose_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let original = png_fixture();
        std::fs::write(&path, &original).unwrap();

        let outcome = ProvenanceWriter::unsigned()
            .apply(&path, &DisclosureRecord::for_asset("asset-1"))
            .unwrap();
        assert!(!outcome.discloses());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn an_unsigned_video_export_reports_that_it_disclosed_nothing() {
        // The gap in the module docs, asserted rather than described: a
        // caller must not be able to record "provenance applied" for a
        // file that carries none.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mp4");
        let mut mp4 = vec![0, 0, 0, 0x14];
        mp4.extend_from_slice(b"ftypisom");
        mp4.extend_from_slice(&[0; 8]);
        std::fs::write(&path, &mp4).unwrap();

        let outcome = ProvenanceWriter::unsigned()
            .apply(&path, &record())
            .unwrap();
        assert!(!outcome.xmp_written, "no XMP half for a BMFF container");
        assert!(!outcome.manifest_written, "and no identity to sign with");
        assert!(!outcome.discloses());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            mp4,
            "and nothing was written"
        );
    }

    #[test]
    fn a_file_that_is_not_a_container_is_refused_rather_than_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"not an image at all").unwrap();
        assert!(matches!(
            ProvenanceWriter::unsigned().apply(&path, &record()),
            Err(ProvenanceError::UnsupportedContainer { .. })
        ));
    }

    #[test]
    fn the_c2pa_test_certificates_are_refused_by_name() {
        // Signing with them produces a manifest that validates as
        // untrusted — a provenance claim a reader rejects, which is
        // worse than the absence the unsigned path leaves.
        let pem = b"-----BEGIN CERTIFICATE-----\n\
                    subject=C2PA Test Signing Cert\n\
                    -----END CERTIFICATE-----\n"
            .to_vec();
        let err = SigningIdentity::from_bytes(pem, b"key".to_vec(), "es256", None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("test certificate"), "{message}");
        assert!(
            message.contains("IPTC/XMP disclosure is written either way"),
            "the message has to say what still works: {message}"
        );
    }

    #[test]
    fn a_test_certificate_is_caught_inside_the_base64_too() {
        // The name sits in the DER as plain ASCII, so it is findable
        // without a certificate parser — which matters because a real
        // PEM has no comment lines to match on.
        use base64::Engine as _;
        let der = b"\x30\x82 fake DER naming C2PA Test Root CA";
        let body = base64::engine::general_purpose::STANDARD.encode(der);
        let pem = format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n");
        assert!(names_a_test_certificate(pem.as_bytes()));
    }

    #[test]
    fn an_ordinary_certificate_is_not_refused() {
        use base64::Engine as _;
        let body =
            base64::engine::general_purpose::STANDARD.encode(b"\x30\x82 DER naming Example Ltd");
        let pem = format!("-----BEGIN CERTIFICATE-----\n{body}\n-----END CERTIFICATE-----\n");
        assert!(!names_a_test_certificate(pem.as_bytes()));
        // It fails on the algorithm instead, which is the next check.
        let err = SigningIdentity::from_bytes(pem.into_bytes(), b"key".to_vec(), "nonsense", None)
            .unwrap_err();
        assert!(err.to_string().contains("unknown signing algorithm"));
    }

    #[test]
    fn a_failed_write_leaves_no_partial_file_beside_the_original() {
        // The export directory is somewhere an importer may be
        // watching. A leftover `.c2pa-partial` there turns one failure
        // into a second artefact that gets ingested.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();
        ProvenanceWriter::unsigned()
            .apply(&path, &record())
            .unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("c2pa-partial"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    /// The signing path stages in a temporary too, and has to clean up
    /// after itself — including when the signing is what fails.
    ///
    /// The sibling above covers the unsigned path, and only its success
    /// route. Signing writes its output straight into the temporary now
    /// rather than into a buffer handed to `replace`, so it owns a
    /// second creation site for the same litter, and the interesting
    /// half of it is the failure: a temporary is created *before*
    /// anything can go wrong, so a signing error that returned without
    /// removing it would leave one behind for good.
    ///
    /// The failure is arranged with bytes that pass the container sniff
    /// and then fail the container handler — a PNG signature followed by
    /// nothing a PNG parser accepts. The record discloses nothing, so no
    /// packet is staged and the streaming branch is the one taken.
    #[test]
    fn a_failed_signing_removes_its_temporary_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let original = b"\x89PNG\r\n\x1a\nnot a chunk anybody can walk";
        std::fs::write(&path, original).unwrap();

        let outcome = ProvenanceWriter::signed_with(throwaway_identity())
            .apply(&path, &DisclosureRecord::for_asset("asset-1"));
        assert!(
            matches!(outcome, Err(ProvenanceError::Sign { .. })),
            "expected the container handler to refuse these bytes: {outcome:?}"
        );

        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "the original is untouched when signing fails"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("c2pa-partial"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    /// The success route of the same path.
    #[test]
    fn signing_leaves_no_partial_file_beside_the_original_either() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        ProvenanceWriter::signed_with(throwaway_identity())
            .apply(&path, &record())
            .unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("c2pa-partial"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    /// A file shorter than the sniff window reads as what it holds.
    ///
    /// The head read used one `read`, which is allowed to return fewer
    /// bytes than asked for without being at end of file — on NFS, SMB
    /// and FUSE it does, and eleven bytes instead of twelve made
    /// `Container::sniff` fail its `ftyp` test and the caller report a
    /// good MP4 as unsupported. That filesystem behaviour cannot be
    /// reproduced here; what can is the other short case, and both are
    /// answered by the same `take` + `read_to_end`.
    #[test]
    fn a_file_shorter_than_the_sniff_window_reads_as_what_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stub");
        std::fs::write(&path, b"\x89PNG").unwrap();
        assert_eq!(read_head(&path, 12).unwrap(), b"\x89PNG");

        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").unwrap();
        assert!(read_head(&empty, 12).unwrap().is_empty());

        // And a file longer than the window stops at it.
        let long = dir.path().join("long");
        std::fs::write(&long, vec![0xABu8; 64]).unwrap();
        assert_eq!(read_head(&long, 12).unwrap().len(), 12);
    }

    #[test]
    fn a_configured_identity_writes_a_manifest_a_reader_can_find() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        let writer = ProvenanceWriter::signed_with(throwaway_identity());
        assert!(writer.can_sign());
        let outcome = writer.apply(&path, &record()).unwrap();
        assert!(outcome.xmp_written);
        assert!(outcome.manifest_written);

        let signed = std::fs::read(&path).unwrap();
        let reader = read_manifest("image/png", signed).expect("the manifest reads back");
        let json = reader.json();
        assert!(
            json.contains("trainedAlgorithmicMedia"),
            "the actions assertion carries the IPTC URI: {json}"
        );
        assert!(
            json.contains(asterism_provenance::manifest::ASTERISM_LABEL),
            "the lineage assertion is there: {json}"
        );
        assert!(json.contains("asset-1"), "and names the asset: {json}");
    }

    #[test]
    fn the_packet_is_written_before_the_manifest_is_signed() {
        // The ordering claim in the module docs, made observable. The
        // manifest's hard binding covers the XMP packet, so a file whose
        // packet was written *after* signing has a binding over bytes
        // that are no longer there.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        ProvenanceWriter::signed_with(throwaway_identity())
            .apply(&path, &record())
            .unwrap();
        let signed = std::fs::read(&path).unwrap();

        // Both halves are in the file, and the binding holds over them.
        assert!(
            embed::read_xmp(&signed).unwrap().is_some(),
            "the packet survived signing"
        );
        let reader =
            read_manifest("image/png", signed.clone()).expect("a manifest signed over these bytes");
        assert!(
            !reports_a_binding_failure(&reader),
            "signing over the stamped bytes leaves the binding intact: {}",
            reader.json()
        );

        // The negative control: edit the packet afterwards, which is
        // what writing XMP after signing amounts to.
        let corrected = DisclosureRecord::for_asset("asset-1")
            .with_source_type(DigitalSourceType::CompositeWithTrainedAlgorithmicMedia);
        let tampered = embed::stamp(&signed, &corrected).unwrap().unwrap();
        match read_manifest("image/png", tampered) {
            Err(_) => {}
            Ok(reader) => assert!(
                reports_a_binding_failure(&reader),
                "a packet written after signing has to invalidate the manifest, \
                 or the ordering in this module is unnecessary: {}",
                reader.json()
            ),
        }
    }

    /// Reads a manifest back out of a file's bytes.
    fn read_manifest(format: &str, bytes: Vec<u8>) -> Result<c2pa::Reader, c2pa::Error> {
        c2pa::Reader::from_context(c2pa::Context::new()).with_stream(format, Cursor::new(bytes))
    }

    /// Whether a reader reports the *hard binding* as broken.
    ///
    /// Narrower than "is this manifest valid", and deliberately so. The
    /// throwaway identity chains to nothing, and two of the failures
    /// that produces say nothing about the file's bytes:
    ///
    /// - `signingCredential.untrusted` — the expected verdict on a
    ///   certificate with no trust anchor.
    /// - `claimSignature.mismatch` — which reads like a cryptographic
    ///   failure and is not one here: `c2pa` emits it whenever the
    ///   certificate info comes back with `validated == false`, and that
    ///   flag is set by the trust check rather than by verifying the
    ///   signature bytes (`claim.rs`, `verify_internal`). An untrusted
    ///   certificate therefore always produces it.
    ///
    /// What is left is the hash over the asset's own bytes, which is
    /// exactly the claim under test: does editing the XMP packet after
    /// signing break the binding.
    fn reports_a_binding_failure(reader: &c2pa::Reader) -> bool {
        reader
            .validation_results()
            .and_then(|results| results.active_manifest())
            .is_some_and(|statuses| {
                statuses.failure().iter().any(|status| {
                    status.code() == "assertion.dataHash.mismatch"
                        || status.code() == "assertion.bmffHash.mismatch"
                })
            })
    }

    /// Copies one of the workspace's generated video fixtures into
    /// `dir` and returns the copy's path.
    ///
    /// The fixture is borrowed from `asterism-importer-video` rather
    /// than duplicated. It has no upstream and no licence to honour —
    /// `scripts/gen-test-fixtures.py` produces it from ffmpeg's
    /// `testsrc` — so a second copy here would be a second file to
    /// regenerate and one more place for the two to drift apart. What
    /// this test needs is a real MP4/MOV box structure, which is
    /// precisely what a hand-built fixture cannot give it.
    fn video_fixture(dir: &Path, name: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../asterism-importer-video/tests/fixtures")
            .join(name);
        let target = dir.join(name);
        std::fs::copy(&source, &target)
            .unwrap_or_else(|e| panic!("copying {}: {e}", source.display()));
        target
    }

    #[test]
    fn a_configured_identity_signs_video_after_the_encode() {
        // The half of the acceptance criteria that no local generation
        // pipeline covers: MP4 and MOV take a manifest as a JUMBF box,
        // and it can only go in after the encode, because the encode is
        // what discards any frame-level record.
        for (name, mime) in [
            ("testsrc.mp4", "video/mp4"),
            ("testsrc.mov", "video/quicktime"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = video_fixture(dir.path(), name);

            let outcome = ProvenanceWriter::signed_with(throwaway_identity())
                .apply(&path, &record())
                .unwrap();
            assert!(!outcome.xmp_written, "{name}: no XMP half for BMFF");
            assert!(outcome.manifest_written, "{name}: the manifest landed");

            let reader = read_manifest(mime, std::fs::read(&path).unwrap())
                .expect("the manifest reads back");
            assert!(
                reader.json().contains("trainedAlgorithmicMedia"),
                "{name}: {}",
                reader.json()
            );
            assert!(
                !reports_a_binding_failure(&reader),
                "{name}: the BMFF binding holds over the file as written: {}",
                reader.json()
            );
        }
    }

    #[test]
    fn a_dropped_prompt_is_reported_rather_than_inferred_afterwards() {
        // Afterwards, a file whose prompt did not fit and a file that
        // never had one look identical. The difference has to leave the
        // call that made it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.jpg");
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        std::fs::write(&path, &jpeg).unwrap();

        let huge = "x".repeat(embed::JPEG_MAX_PACKET + 1);
        let outcome = ProvenanceWriter::unsigned()
            .apply(&path, &record().with_prompt(huge, None))
            .unwrap();
        assert!(outcome.xmp_written);
        assert!(outcome.prompt_dropped);

        let packet = embed::read_xmp(&std::fs::read(&path).unwrap())
            .unwrap()
            .unwrap();
        assert!(packet.contains("trainedAlgorithmicMedia"));
        assert!(!packet.contains("AIPromptInformation"));
    }
}
