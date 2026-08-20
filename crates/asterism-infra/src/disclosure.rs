//! Writing a [`DisclosureRecord`] into a file that already exists.
//!
//! This is the adapter half of AI disclosure. What is asserted is
//! decided in [`asterism_core::domain::disclosure`];
//! `asterism-disclosure-format` renders that decision into the two
//! forms it can take, as values; this module puts them into a file on
//! disk, in the one order that works, and signs the manifest when — and
//! only when — a signing identity has been configured.
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
//! deduction. [`DisclosureWriter::apply`] does the two in this order and
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
//!
//! [`SigningIdentity::from_files`] refuses them by the name they carry.
//! That is a heuristic and a rename defeats it, which is why
//! [`inspect_certificate`] reads the certificate's own extensions
//! beside it — though not for the same question. It refuses what cannot
//! sign at all and reports the rest, because the extended key usage a
//! test certificate carries is one legitimate certificates carry too:
//! the two are not told apart by structure.
//!
//! Neither check decides whether a certificate is *trusted*. That is a
//! validator's question, asked against a published trust list, and not
//! one a signer can answer about itself.
//!
//! # The gap this leaves on video
//!
//! A still gets its XMP packet either way; MP4 and MOV get nothing
//! without a certificate. MP4 can carry XMP in a `uuid` box, but writing
//! BMFF boxes is not something this module does — the manifest path goes
//! through `c2pa`, which knows the container, and the packet path goes
//! through `asterism-disclosure-format::embed`, which knows PNG and JPEG. Until
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

use asterism_core::domain::disclosure::{DisclosureRecord, Half, Skipped, Stamped};
use asterism_disclosure_format::{embed, manifest};

/// A container this module can write a disclosure into.
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
    /// [`asterism_disclosure_format::embed`].
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
        // ISO base media: a `ftyp` box at the head, whose brands say
        // which dialect. Treating everything that was not `qt  ` as MP4
        // — which is what this arm used to do — labelled every other
        // family in the container `video/mp4`: HEIC, AVIF and M4A all
        // begin with `ftyp`, and a manifest signed under the wrong
        // declared MIME type is a claim about the file that the file
        // contradicts. So membership is now read the way the box states
        // it: from the major brand when it is one of the MP4 dialects,
        // and otherwise from the compatible list, which is where a file
        // whose major brand is its vendor's name (Sony's `XAVC`)
        // declares itself. The families this module does not write into
        // are refused *before* that list is consulted, because it is not
        // exclusive — an M4A routinely declares `isom` compatible, and
        // compatibility with a video dialect does not make audio video.
        // A brand list naming nothing recognised is refused too, and the
        // caller reports a container this build does not write into
        // rather than signing under a guess.
        if head.get(4..8)? == b"ftyp" {
            /// Major brands of the dialects [`Container::Mp4`] means.
            const MP4: [[u8; 4]; 10] = [
                *b"isom", *b"iso2", *b"iso3", *b"iso4", *b"iso5", *b"iso6", *b"mp41", *b"mp42",
                *b"avc1", *b"dash",
            ];
            /// ISO base media families that are not MP4: HEIF stills,
            /// AVIF, and the iTunes audio and video family.
            const FOREIGN: [[u8; 4]; 13] = [
                *b"heic", *b"heix", *b"hevc", *b"hevx", *b"mif1", *b"mif2", *b"msf1", *b"avif",
                *b"avis", *b"M4A ", *b"M4B ", *b"M4P ", *b"M4V ",
            ];
            let major: [u8; 4] = head.get(8..12)?.try_into().ok()?;
            if major == *b"qt  " {
                return Some(Self::Mov);
            }
            if FOREIGN.contains(&major) {
                return None;
            }
            if MP4.contains(&major) {
                return Some(Self::Mp4);
            }
            // The compatible brands run from the minor version to the
            // end of the box, read no further than the head the caller
            // took and no further than the box's own declared size.
            let declared = u32::from_be_bytes(head.get(..4)?.try_into().ok()?) as usize;
            let end = declared.min(head.len());
            let mut at = 16;
            while at + 4 <= end {
                let brand: [u8; 4] = head[at..at + 4].try_into().ok()?;
                // The same non-exclusivity cuts both ways: a list that
                // names a foreign family refuses the file even when an
                // MP4 dialect sits further along it.
                if FOREIGN.contains(&brand) {
                    return None;
                }
                if MP4.contains(&brand) {
                    return Some(Self::Mp4);
                }
                at += 4;
            }
        }
        None
    }
}

/// What went wrong applying a record.
#[derive(Debug, thiserror::Error)]
pub enum DisclosureError {
    /// The file could not be read, written, or replaced.
    #[error("disclosure io on {path}: {source}")]
    Io {
        /// File the operation was against.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
    /// The bytes are not a container this module writes into.
    #[error("{path} is not a container this build writes a disclosure into")]
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
    /// [`Stamped::prompt_dropped`](asterism_core::domain::disclosure::Stamped):
    /// a file carrying no readable disclosure at all was reported as a
    /// successful stamp that had merely shortened the prompt, and — when
    /// the record had no prompt to shorten — as an unqualified success.
    ///
    /// Nothing is known to reach it today. The one producer that was
    /// found is fixed (`asterism-disclosure-format`'s JPEG writer put the
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

/// `c2pa-kp-claimSigning`, under the C2PA's own private enterprise arc.
///
/// The Conformance Program's certificate profile requires this on a
/// claim signing certificate *in addition to* `emailProtection` or
/// `documentSigning`, so a certificate without it is one that profile
/// would not have issued. That is a statement about being **listed**,
/// not about being able to sign — see [`ACCEPTED_SIGNING_EKUS`].
const C2PA_CLAIM_SIGNING_EKU: &str = "1.3.6.1.4.1.62558.2.1";

/// Every extended key usage `c2pa` accepts on a claim signature, as
/// dotted strings.
///
/// Copied from that crate's own accept-list (`valid_eku_oids.cfg`),
/// which its validator reads as alternatives: `has_allowed_eku` returns
/// on the first match and one is enough. In order —
/// `id-kp-emailProtection`, `id-kp-documentSigning`, `id-kp-timeStamping`,
/// `id-kp-OCSPSigning`, Microsoft's C2PA signing OID, and
/// [`C2PA_CLAIM_SIGNING_EKU`].
///
/// True of `c2pa`'s **default** trust configuration, which is the one
/// this build uses: `signing_cert_valid` seeds the policy with these
/// six and then adds whatever `settings.trust.trust_config` holds, and
/// nothing here sets c2pa settings.
///
/// This list is what decides a **refusal**, because a certificate
/// carrying none of these is one nothing downstream will sign a claim
/// with. Requiring `c2pa-kp-claimSigning` instead was this module's
/// first attempt and it was wrong in a way worth recording: it would
/// have refused a `documentSigning`-only certificate — a profile IPTC's
/// own publisher policy explicitly permits, its requirement being an
/// EKU valid for `emailProtection` *and/or* `documentSigning` — while
/// telling its operator the certificate "cannot sign a C2PA claim",
/// which is false and which they could not have acted on, an EKU not
/// being something you can add to an issued certificate.
const ACCEPTED_SIGNING_EKUS: &[&str] = &[
    "1.3.6.1.5.5.7.3.4",
    "1.3.6.1.5.5.7.3.36",
    "1.3.6.1.5.5.7.3.8",
    "1.3.6.1.5.5.7.3.9",
    "1.3.6.1.4.1.311.76.59.1.9",
    C2PA_CLAIM_SIGNING_EKU,
];

/// What reading a certificate's own extensions concluded.
///
/// Two lists rather than a `bool`, because "this cannot sign" and "this
/// will not be trusted" are different sentences and only the first is
/// this side's to act on.
///
/// **Refusals** are certificates nothing downstream will sign with: an
/// extended key usage carrying none of [`ACCEPTED_SIGNING_EKUS`], or a
/// certificate marked as a CA. Signing would fail or produce a manifest
/// refused on its face, so it fails here with a reason instead.
///
/// **Warnings** are reasons a certificate would not be *listed*, which
/// is a question about a trust list rather than about whether the bytes
/// can sign. The specification's own guidance describes a private
/// credential store — a certificate trusted by the parties who imported
/// it and nobody else — and self-issued credentials for exactly that
/// use. Refusing those would close a door the specification holds open.
///
/// The split is not this module's invention. C2PA's implementation
/// guidance says of an extended key usage misconfiguration that a claim
/// generator "should warn its user with an explanation of the problem,
/// but should allow the user to choose to proceed with signing" — which
/// is the shape here, with the line drawn at what cannot sign at all
/// rather than at what will not be trusted.
///
/// # Where the strictness setting went
///
/// A deployment signing for publication reasonably wants the warnings
/// to refuse too, and that is [`Strictness::Strict`], handed to
/// [`SigningIdentity::from_bytes`] rather than decided by a caller
/// after an [`inspect_certificate`] of its own. The parameter this
/// module wondered about turned out to be worth it: a caller refusing
/// on its own terms would have to own the chain check as well, which
/// needs the certificate parser that already lives here, and its
/// warnings would then be logged a second time when it went on to
/// load.
///
/// The verdict stays reachable without loading an identity regardless,
/// because showing an operator why a certificate was refused should not
/// require attempting a load.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CertificateVerdict {
    /// Findings that make the certificate unusable for signing at all.
    pub refusals: Vec<String>,
    /// Findings that keep it off a trust list without stopping it
    /// signing for a reader who has imported it.
    pub warnings: Vec<String>,
}

/// Whether a certificate no trust list would carry may still sign.
///
/// This selects between the two halves of [`CertificateVerdict`]: the
/// refusals apply either way, and this decides what becomes of the
/// warnings.
///
/// A deployment's call rather than a default, because the strict answer
/// signs nothing at all on an installation holding a self-issued
/// credential — an arrangement the specification's own guidance
/// describes rather than warns about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Strictness {
    /// Refuse only what cannot sign at all, and log the rest.
    ///
    /// C2PA's implementation guidance for a certificate whose extended
    /// key usage is misconfigured: warn with an explanation of the
    /// problem, and allow the operator to proceed.
    #[default]
    Permissive,
    /// Also refuse what a trust list would not carry, and require the
    /// certificate to arrive with its chain.
    ///
    /// For an installation that publishes, where an export carrying a
    /// manifest nobody can validate is worth less than an export that
    /// stops and says why.
    ///
    /// The chain requirement is the part no extension states. A
    /// certificate issued under a conformance profile comes with the
    /// intermediate that issued it; a bundle holding one certificate is
    /// either self-issued or has had its chain dropped somewhere in
    /// deployment, and both are fixed the same way — by supplying the
    /// chain the issuer gave you.
    Strict,
}

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
        strictness: Strictness,
    ) -> Result<Self, DisclosureError> {
        let cert_chain = std::fs::read(cert_chain).map_err(|e| {
            DisclosureError::Identity(format!("reading {}: {e}", cert_chain.display()))
        })?;
        let private_key = std::fs::read(private_key).map_err(|e| {
            DisclosureError::Identity(format!("reading {}: {e}", private_key.display()))
        })?;
        Self::from_bytes(cert_chain, private_key, alg, tsa_url, strictness)
    }

    /// Loads an identity from material already in memory. Same checks as
    /// [`from_files`](Self::from_files); split out so the tests can
    /// exercise the refusal without writing key material to disk.
    pub fn from_bytes(
        cert_chain: Vec<u8>,
        private_key: Vec<u8>,
        alg: &str,
        tsa_url: Option<String>,
        strictness: Strictness,
    ) -> Result<Self, DisclosureError> {
        if names_a_test_certificate(&cert_chain) {
            return Err(DisclosureError::Identity(
                "this is a C2PA test certificate: a manifest signed with it validates as \
                 untrusted, which claims a provenance a reader will reject. Configure a real \
                 signing identity, or export without a manifest — the IPTC/XMP disclosure is \
                 written either way"
                    .into(),
            ));
        }
        let alg = alg
            .parse::<c2pa::crypto::raw_signature::SigningAlg>()
            .map_err(|e| DisclosureError::Identity(format!("unknown signing algorithm: {e}")))?;

        // What the certificate says about itself, after what it is
        // called. The name check above is a heuristic a rename defeats;
        // this reads the extensions instead. Last of the three, so a
        // bundle that fails an earlier one is refused for the earlier
        // reason rather than warned about first.
        let verdict = inspect_certificate(&cert_chain);
        if !verdict.refusals.is_empty() {
            return Err(DisclosureError::Identity(format!(
                "this certificate cannot sign a C2PA claim: {}. Nothing is written rather \
                 than a signature that will not hold — the IPTC/XMP disclosure is written \
                 either way",
                verdict.refusals.join("; ")
            )));
        }
        // Strict mode, which is the warnings turned into refusals plus
        // the one check no extension carries. Separate from the branch
        // above because the message has to be: this certificate *can*
        // sign, and an installation that does not publish can use it by
        // leaving strict mode off.
        if strictness == Strictness::Strict {
            let count = certificate_count(&cert_chain);

            // Bytes that parse as nothing are their own answer and not a
            // strictness one. `inspect_certificate` passes what it
            // cannot read rather than guessing at it, so an unreadable
            // bundle arrives here with no refusals and no warnings —
            // and telling its operator that the certificate can sign
            // with strict signing off would be false. It cannot sign
            // either way; turning strict off only moves the failure from
            // startup to the first export.
            if count == 0 {
                return Err(DisclosureError::Identity(
                    "strict signing could not read a certificate out of this bundle. \
                     Turning strict signing off would not make it sign — it would move \
                     the failure to the first export — so check that the file is PEM and \
                     holds the certificate you meant"
                        .into(),
                ));
            }

            let mut refusals = verdict.warnings.clone();
            if count == 1 {
                refusals.push(
                    "the bundle carries one certificate and no issuer chain; supply the \
                     chain your issuer gave you, ending before the root"
                        .into(),
                );
            }
            if !refusals.is_empty() {
                return Err(DisclosureError::Identity(format!(
                    "strict signing refuses this certificate: {}. It can sign — this is \
                     what a publicly issued one would carry and this one does not — so an \
                     installation that does not publish can use it with strict signing off",
                    refusals.join("; ")
                )));
            }
        }

        for warning in &verdict.warnings {
            // The dotted name is the `event` field rather than the
            // target, which is the convention the rest of this crate
            // uses: both sinks filter on `asterism=info` by target
            // prefix, so an event addressed `diag.…` reaches neither
            // stderr nor the `diag_log` table.
            tracing::warn!(
                event = "diag.disclosure.identity",
                "signing certificate: {warning}"
            );
        }

        if strictness == Strictness::Strict {
            // Recorded on the way through rather than only on refusal.
            // A check that says nothing when it passes leaves whoever
            // audits the deployment inferring that it ran from the
            // absence of an error, which is not evidence — the gap
            // cosign's own strict verification was filed for.
            tracing::info!(
                event = "diag.disclosure.identity",
                "signing certificate accepted under strict signing"
            );
        }

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

/// How many certificates the bundle holds.
///
/// Counts what parses rather than what looks like a PEM header, on the
/// same terms as [`inspect_certificate`]: a block this cannot read is
/// one it cannot count, and reporting a chain on the strength of a
/// label would be worse than reporting none.
///
/// Only [`Strictness::Strict`] asks. Under [`Strictness::Permissive`] a
/// single self-issued certificate is a supported configuration and
/// counting them would decide nothing.
fn certificate_count(pem: &[u8]) -> usize {
    use x509_parser::prelude::FromDer as _;

    x509_parser::pem::Pem::iter_from_buffer(pem)
        .filter_map(Result::ok)
        .filter(|block| {
            x509_parser::certificate::X509Certificate::from_der(&block.contents).is_ok()
        })
        .count()
}

/// Reads a signing certificate's own extensions and reports what they
/// say, without deciding what to do about it.
///
/// Public because that decision is the one a strictness setting would
/// make (see [`CertificateVerdict`]), and because a caller that wants to
/// show an operator why an identity was refused should not have to
/// attempt a load to find out.
///
/// The certificate inspected is the **first block of the bundle that
/// parses as one**, which is the end-entity certificate in a chain
/// written the usual way. Walking to the first that parses, rather than
/// taking the first block outright, is what keeps a bundle that leads
/// with a key from reading as unparseable.
///
/// Bytes this cannot read at all yield **no findings** rather than a
/// refusal. `c2pa` parses the same certificate next with a real
/// validator and says something better than a guess from here, and a
/// check that refused what it could not read would turn every encoding
/// this crate has not met into an export failing for a reason nobody can
/// act on. What that costs is real and worth naming: DER rather than
/// PEM, an empty file, and a bundle whose every block is something else
/// all pass inspection silently.
///
/// # What this does not check
///
/// Everything else `c2pa` requires at signing time, which is a great
/// deal more: certificate version, an unexpired validity window, the
/// signature and key algorithms, `digitalSignature` key usage, an
/// authority key identifier, no `any` usage, no `ocspSigning` or
/// `timeStamping` mixed with another usage. Those arrive as
/// [`DisclosureError::Sign`] instead, which is a worse message but not a
/// wrong one — this check is a strict subset, so nothing it passes ends
/// up in a *less* informative failure than it would have without it.
///
/// The one worth knowing by name is **expiry**, because it is the
/// failure a working configuration grows into: a Conformance Program
/// certificate is valid for at most 366 days, so every deployment that
/// signs will meet it, and it will arrive as a signing error rather than
/// as an identity one.
pub fn inspect_certificate(pem: &[u8]) -> CertificateVerdict {
    use x509_parser::prelude::FromDer as _;

    let mut verdict = CertificateVerdict::default();
    // The block is found first and parsed again inside this scope: the
    // parsed certificate borrows the block's bytes, so it cannot outlive
    // a closure that owns them.
    let Some(block) = x509_parser::pem::Pem::iter_from_buffer(pem)
        .filter_map(Result::ok)
        .find(|block| x509_parser::certificate::X509Certificate::from_der(&block.contents).is_ok())
    else {
        return verdict;
    };
    let Ok((_, certificate)) = x509_parser::certificate::X509Certificate::from_der(&block.contents)
    else {
        return verdict;
    };

    // Refusals first: what nothing downstream will sign with.
    //
    // The accept-list rather than `c2pa-kp-claimSigning` alone. A
    // certificate carrying one of these is one `c2pa` will sign a claim
    // with; requiring the C2PA OID here would refuse a
    // `documentSigning`-only certificate that signs perfectly well, and
    // tell its operator something untrue about why.
    let declared = certificate.extended_key_usage().ok().flatten().map(|eku| {
        let mut out: Vec<String> = eku.value.other.iter().map(|o| o.to_id_string()).collect();
        // `x509-parser` lifts seven usages it knows into named fields,
        // so `other` alone loses whichever of them a certificate holds.
        // Three of those seven are on the accept-list and are lifted
        // back here; `server_auth`, `client_auth`, `code_signing` and
        // `any` are the other four and are deliberately not, because
        // lifting them would manufacture an accept for a usage no claim
        // is signed under. `documentSigning` and Microsoft's C2PA usage
        // have no named field at all, so they arrive through `other` —
        // which is why that path carries four of the six and is the one
        // a test has to hold.
        if eku.value.email_protection {
            out.push("1.3.6.1.5.5.7.3.4".into());
        }
        if eku.value.time_stamping {
            out.push("1.3.6.1.5.5.7.3.8".into());
        }
        if eku.value.ocsp_signing {
            out.push("1.3.6.1.5.5.7.3.9".into());
        }
        out
    });

    // Absent and present-but-unusable are different sentences. A
    // certificate with no extension at all has not named a wrong usage;
    // it has named none, and telling its operator that "its extended key
    // usage names nothing" describes an extension they do not have.
    match &declared {
        None => verdict.refusals.push(
            "it carries no extended key usage extension, and a certificate signing a C2PA \
             claim has to name what it is for"
                .into(),
        ),
        Some(usages)
            if !usages
                .iter()
                .any(|oid| ACCEPTED_SIGNING_EKUS.contains(&oid.as_str())) =>
        {
            verdict.refusals.push(
                "its extended key usage names nothing a C2PA claim can be signed under \
                 (expected one of emailProtection, documentSigning, timeStamping, \
                 OCSPSigning, or a C2PA claim-signing usage)"
                    .into(),
            );
        }
        Some(_) => {}
    }
    let usages = declared.unwrap_or_default();

    if certificate
        .basic_constraints()
        .ok()
        .flatten()
        .is_some_and(|constraints| constraints.value.ca)
    {
        verdict.refusals.push(
            "the certificate inspected is a CA certificate, and a claim is signed by an \
             end-entity one — a bundle written root-first reads this way too"
                .into(),
        );
    }

    // Warnings: reasons it would not be listed. See `CertificateVerdict`.
    if !usages.iter().any(|oid| oid == C2PA_CLAIM_SIGNING_EKU) {
        verdict.warnings.push(format!(
            "its extended key usage does not include c2pa-kp-claimSigning \
             ({C2PA_CLAIM_SIGNING_EKU}); it will sign, and the Conformance Program's \
             certificate profile requires that usage of a certificate issued for claim \
             signing, so this one would not be one it issued"
        ));
    }

    if certificate
        .subject()
        .iter_organization()
        .next()
        .is_none_or(|organisation| organisation.as_str().is_ok_and(str::is_empty))
    {
        verdict.warnings.push(
            "its subject names no organisation, and a validator that displays a signer's \
             name reads that field"
                .into(),
        );
    }

    verdict
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
pub struct DisclosureWriter {
    signing: Signing,
}

/// What this process has to sign manifests with.
///
/// Three states rather than an `Option`, because the third one exists
/// and collapsing it is a false statement rather than a simplification:
/// an installation that configured a certificate which then failed to
/// load is not an installation with no certificate. Reported as the
/// latter, the export record says the deployment is doing exactly what
/// it was configured to do, at the one moment that is untrue and
/// somebody needs to know.
#[derive(Debug, Default, Clone)]
enum Signing {
    /// No certificate is configured. The state every install starts in.
    #[default]
    Unconfigured,
    /// An identity loaded and is ready to sign.
    ///
    /// Behind an `Arc` so the port impl can hand a copy to a blocking
    /// task without duplicating key material on every call — a signing
    /// identity holds a private key, and the fewer copies of it exist in
    /// the process at once, the better.
    Ready(std::sync::Arc<SigningIdentity>),
    /// A certificate was configured and cannot be used, in the words
    /// whoever configured it needs in order to fix it.
    ///
    /// Resolved once, at startup, and carried for the life of the
    /// process: the certificate is read when the writer is built, so
    /// this is where the reason exists, and the export is where
    /// somebody will notice.
    Unavailable(String),
}

impl DisclosureWriter {
    /// A writer that emits the IPTC/XMP half and no manifest.
    ///
    /// The state every install starts in, and a supported one rather
    /// than a degraded one — see the module docs on why an untrusted
    /// manifest is worse than none.
    pub fn unsigned() -> Self {
        Self {
            signing: Signing::Unconfigured,
        }
    }

    /// A writer that also signs.
    pub fn signed_with(identity: SigningIdentity) -> Self {
        Self {
            signing: Signing::Ready(std::sync::Arc::new(identity)),
        }
    }

    /// A writer whose configured identity could not be loaded, carrying
    /// `reason` into every manifest half it declines to write.
    ///
    /// Not the same call as [`unsigned`](Self::unsigned) and
    /// deliberately not reachable by leaving an argument out: this is a
    /// fault, and it reports as one. What it is *not* is a reason to
    /// refuse to start — a certificate under a conformance profile is
    /// valid for at most 366 days, so every deployment that signs will
    /// eventually meet an expired one, and a build that exited there
    /// would answer an expiry by making the library unopenable.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            signing: Signing::Unavailable(reason.into()),
        }
    }

    /// Whether this writer can produce a manifest.
    pub fn can_sign(&self) -> bool {
        matches!(self.signing, Signing::Ready(_))
    }

    /// Writes `record` into the file at `path`, replacing it.
    ///
    /// The file is rewritten through a sibling temporary and a rename,
    /// so a failure part-way leaves the original intact rather than a
    /// half-stamped file that still looks like an export.
    ///
    /// # Both halves are attempted, and both report
    ///
    /// ```text
    /// read → [ XMP packet ] → [ manifest ] → one write
    ///            │                 │
    ///            └── outcome ──────┴── outcome ──→ Stamped
    /// ```
    ///
    /// `Err` is reserved for the case where **nothing could be
    /// attempted**: the file cannot be read, or the container is not
    /// one this build writes into. Anything that goes wrong inside a
    /// half is reported as that half's
    /// [`Half::Failed`](asterism_core::domain::disclosure::Half::Failed), and the
    /// other half proceeds regardless.
    ///
    /// This was not always so, and the way it failed is the reason for
    /// the shape. Every failure used to return early, which meant that
    /// a signing error discarded the XMP packet already sitting in
    /// memory — so the day a certificate expired, exports stopped
    /// carrying the IPTC disclosure, which needs no certificate at all
    /// and which the module docs above promise "still lands". The same
    /// early return sent a packet too large for a JPEG segment out as
    /// a failure of the whole call, taking the manifest with it.
    ///
    /// The two halves are still ordered: the manifest's hard binding is
    /// computed over the file's bytes, so a packet has to be in them
    /// before the signature is taken. What changed is that failing to
    /// produce one no longer cancels the other.
    pub fn apply(
        &self,
        path: &Path,
        record: &DisclosureRecord,
    ) -> Result<Stamped, DisclosureError> {
        let io = |source: std::io::Error| DisclosureError::Io {
            path: path.to_path_buf(),
            source,
        };

        // Enough bytes to identify any container here: PNG's signature
        // is 8, JPEG's is 2, and a `ftyp` major brand ends at 12 — but
        // when that brand is a vendor's name the answer sits in the
        // compatible list behind it, so the window is 64, room for a
        // dozen compatible brands. `sniff` reads no further than the
        // `ftyp` box's own declared size.
        let head = read_head(path, 64).map_err(io)?;
        let container =
            Container::sniff(&head).ok_or_else(|| DisclosureError::UnsupportedContainer {
                path: path.to_path_buf(),
            })?;

        // --- 1. the XMP packet, on the containers that take one -----
        //
        // Held in memory rather than written yet: if a manifest follows,
        // it has to be signed over these bytes, and a write in between
        // would be a file on disk in a state neither half asked for.
        let mut staged: Option<Vec<u8>> = None;
        let mut prompt_dropped = false;
        let mut system_dropped = false;
        let mut xmp = if !container.takes_xmp() {
            Half::Skipped(Skipped::ContainerCannotCarryIt)
        } else if !record.discloses_anything() {
            Half::Skipped(Skipped::NothingToDisclose)
        } else {
            // Reading the file is not this half's business — the
            // manifest streams from the same file — so a read that
            // fails is the whole call failing rather than one half of
            // it.
            let original = std::fs::read(path).map_err(io)?;
            match embed::stamp(&original, record) {
                // A packet that will not fit even after the reductions,
                // or a container the writer chokes on. The manifest can
                // still be signed over the original bytes.
                Err(source) => Half::Failed(
                    DisclosureError::Xmp {
                        path: path.to_path_buf(),
                        source,
                    }
                    .to_string(),
                ),
                Ok(None) => Half::Skipped(Skipped::NothingToDisclose),
                Ok(Some(stamp)) => {
                    // The read-back stays, with one job left: a packet
                    // that fails to parse, or that the reader cannot
                    // find at all, says the write produced something
                    // this crate does not recognise — a defect, not a
                    // fact about the record. Those two discard the
                    // bytes rather than putting them on disk: a file
                    // carrying an unreadable packet is worse than one
                    // carrying none, because the manifest's binding
                    // would then be taken over it.
                    match embed::read_xmp(&stamp.bytes) {
                        Ok(Some(_)) => {
                            // `embed::stamp` says how far it reduced
                            // the record to fit — it used to say
                            // nothing, and this side re-derived the
                            // prompt's fate by substring-search over
                            // the rendered XML, which was blind to the
                            // tier below the prompt once there was
                            // one. A reduction is only a withholding
                            // of what the record actually asked for: a
                            // record with no prompt loses nothing at
                            // the prompt tier. Set here rather than
                            // where the reduction is first seen, so a
                            // packet the read-back refuses cannot
                            // leave a note about a packet that never
                            // reached the file.
                            prompt_dropped = record.prompt.is_some()
                                && stamp.reduction != embed::Reduction::Nothing;
                            system_dropped = record.ai_system.is_some()
                                && stamp.reduction == embed::Reduction::Obligation;
                            staged = Some(stamp.bytes);
                            Half::Written
                        }
                        Ok(None) => Half::Failed(
                            DisclosureError::XmpUnreadable {
                                path: path.to_path_buf(),
                            }
                            .to_string(),
                        ),
                        Err(source) => Half::Failed(
                            DisclosureError::Xmp {
                                path: path.to_path_buf(),
                                source,
                            }
                            .to_string(),
                        ),
                    }
                }
            }
        };

        // --- 2. the manifest, when there is an identity to sign it ---
        //
        // The three cases are three different sentences. No certificate
        // configured is the install doing what it was configured to do;
        // one configured that would not load is a fault, and saying so
        // here is the only place it reaches whoever reads the record.
        // Neither takes the packet down with it — that is what step 3
        // is for.
        let manifest = match &self.signing {
            Signing::Unconfigured => Half::Skipped(Skipped::NoSigningIdentity),
            Signing::Unavailable(reason) => Half::Failed(reason.clone()),
            Signing::Ready(identity) => {
                match self.sign(identity, path, container, record, staged.as_deref()) {
                    Ok(()) => {
                        // The committed file already carries the packet
                        // — it was signed over these bytes — so there
                        // is nothing left to write.
                        staged = None;
                        Half::Written
                    }
                    Err(failure) => Half::Failed(failure.to_string()),
                }
            }
        };

        // --- 3. land the packet the manifest did not carry ----------
        if let Some(bytes) = staged
            && let Err(failure) = replace(path, &bytes)
        {
            // The packet was produced and could not be put on disk.
            // Nothing else wrote the file, so the original stands and
            // the half that was about to claim it landed has to
            // retract — including the withholding notes, which are
            // facts about a packet that is not there.
            xmp = Half::Failed(io(failure).to_string());
            prompt_dropped = false;
            system_dropped = false;
        }

        let mut outcome = Stamped::new(xmp, manifest);
        outcome.prompt_dropped = prompt_dropped;
        outcome.system_dropped = system_dropped;
        Ok(outcome)
    }

    /// Signs a manifest over `staged`, or over the file when nothing is
    /// staged, and moves the result into place.
    ///
    /// Split out of [`apply`](Self::apply) so that every way this can
    /// fail arrives at one place in the caller, where it becomes the
    /// manifest half's outcome instead of the whole call's.
    fn sign(
        &self,
        identity: &SigningIdentity,
        path: &Path,
        container: Container,
        record: &DisclosureRecord,
        staged: Option<&[u8]>,
    ) -> Result<(), DisclosureError> {
        let signer = identity.signer().map_err(|source| DisclosureError::Sign {
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
                DisclosureError::Definition {
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
        // The temporary is visible for the whole signing operation
        // rather than for the length of one `write`. An importer
        // scanning the export directory can see it growing; that is the
        // cost of not holding the asset in memory. It is why [`stage`]
        // opens it at 0600 — for a video, "briefly" is minutes.
        //
        // `Builder::sign` takes `W: Write + Read + Seek`, and the
        // `Read` is not decorative: the BMFF handler re-reads box
        // headers out of the destination to adjust offsets. A
        // write-only descriptor works with today's signing and would
        // fail with `EBADF` on the video path under a version that
        // used it, which is why this is a real file handle rather than
        // a buffer.
        let mut temporary = stage(path).map_err(|e| DisclosureError::Io {
            // The target, not the temporary: the temporary is removed
            // when it drops, so naming it would hand the reader a
            // filename that no longer exists.
            path: path.to_path_buf(),
            source: e,
        })?;
        let destination = temporary.as_file_mut();
        let sign_failed = |failure: c2pa::Error| DisclosureError::Sign {
            path: path.to_path_buf(),
            source: Box::new(failure),
        };
        let signing = match staged {
            // Sign over the XMP-stamped bytes: the hard binding covers
            // the packet, so signing the original and then writing the
            // packet would invalidate what was signed. These are
            // already in memory — the packet was written into them —
            // so there is nothing to stream from.
            Some(bytes) => builder
                .sign(
                    signer.as_ref(),
                    container.mime(),
                    &mut Cursor::new(bytes),
                    destination,
                )
                .map_err(sign_failed),
            // Nothing staged: the container takes no packet (video),
            // there was nothing to disclose in one, or the packet half
            // failed. Stream from the file, so neither end of a large
            // video is read into memory whole.
            None => match std::fs::File::open(path) {
                Err(e) => Err(DisclosureError::Io {
                    path: path.to_path_buf(),
                    source: e,
                }),
                Ok(mut source) => builder
                    .sign(signer.as_ref(), container.mime(), &mut source, destination)
                    .map_err(sign_failed),
            },
        };
        // A failure needs no cleanup here: dropping the temporary
        // removes it, which is what stops a half-signed file being left
        // in an export directory an importer is watching.
        signing?;
        commit(temporary, path).map_err(|source| DisclosureError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[async_trait::async_trait]
impl asterism_core::application::disclosure_service::DisclosureWriter for DisclosureWriter {
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
        let writer = DisclosureWriter {
            signing: self.signing.clone(),
        };
        tokio::task::spawn_blocking(move || writer.apply(&path, &record))
            .await
            .map_err(|e| {
                asterism_core::error::DomainError::Infra(anyhow::anyhow!(
                    "disclosure task did not finish: {e}"
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

/// Opens the temporary a rewrite is staged in.
///
/// Three properties, and each of them was a defect before it was one.
///
/// **Same directory as the target**, so the rename stays within one
/// filesystem — across a mount boundary `rename` fails with `EXDEV`,
/// and the fallback (copy, then delete) is exactly the non-atomic
/// behaviour this is here to avoid.
///
/// **A name nothing can predict, created with `O_EXCL`.** The name used
/// to be a deterministic sibling — `shot.png` staged through
/// `shot.png.c2pa-partial` — opened with neither `O_EXCL` nor
/// `O_NOFOLLOW`. Anything else that could create a file in that
/// directory could put a symlink there first and have this write
/// through it, and two concurrent applies to one path shared a
/// temporary and interleaved into it. An export directory is not
/// necessarily private: it is wherever the user pointed the export,
/// which may be shared, synced, or watched by something else.
///
/// **Mode 0600 while it is open, and the target's own mode once it is
/// in place.** A staged file holds the whole asset for as long as the
/// signing takes, which for a video is not a moment. Inheriting the
/// umask made that world-readable. Copying the target's permissions
/// across before the rename is the other half: without it, replacing a
/// file the user had kept at 0600 would publish it at 0644.
fn stage(path: &Path) -> std::io::Result<tempfile::NamedTempFile> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = tempfile::Builder::new()
        .prefix(".asterism-")
        .suffix(".c2pa-partial")
        .tempfile_in(directory)?;
    // Best-effort: a target whose permissions cannot be read is one
    // whose replacement is about to fail anyway, and failing here would
    // turn a metadata quirk into a refusal to stamp.
    if let Ok(metadata) = std::fs::metadata(path) {
        let _ = temporary.as_file().set_permissions(metadata.permissions());
    }
    Ok(temporary)
}

/// Moves a finished temporary over its target, durably.
///
/// The temporary removes itself when dropped, so the failure path needs
/// no cleanup of its own — which is the part the hand-rolled version
/// kept having to remember at each new return.
///
/// Durable, not merely atomic. The rename is atomic in the namespace,
/// but on power loss between the write and the writeback the name
/// could point at bytes that never reached the disk — a short file
/// where the user's original was, which is the opposite of what this
/// module advertises. So the data is fsynced before the rename and the
/// directory after it, the same order the blob store writes its bytes
/// in. The cost is real on a large video and it is paid on purpose:
/// the alternative prices durability per file, and "whether power loss
/// eats your original" is not a property that should depend on a
/// setting. An fsync that fails is a failure like any other, and the
/// two sit on different sides of the rename: the data fsync stops the
/// caller with the original untouched, while a directory fsync that
/// fails has already replaced it — the new bytes are in the namespace
/// and the name is just not crash-durable yet, so that error means
/// "written, not yet safe" rather than "not written". A caller that
/// re-applies on it rewrites a file already carrying the disclosure,
/// which the derived-from-rows design makes a repeat, not a loss.
fn commit(temporary: tempfile::NamedTempFile, path: &Path) -> std::io::Result<()> {
    // The data first: a rename made durable ahead of its content would
    // pin the name to bytes the disk does not hold yet.
    temporary.as_file().sync_all()?;
    temporary.persist(path).map(|_| ()).map_err(|e| e.error)?;
    // Then the rename itself: the new directory entry lives in the
    // directory's own blocks, and until they are written back the old
    // entry is what a crash recovers.
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::File::open(directory)?.sync_all()
}

/// Replaces `path`'s contents through a temporary and a rename.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut temporary = stage(path)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    commit(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use asterism_core::domain::disclosure::DigitalSourceType;

    /// The same hand-built 1×1 PNG the format crate's own tests use.
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
        let (cert, key) = self_signed_pair();
        SigningIdentity::from_bytes(
            cert,
            key, // rcgen's default key pair is ECDSA P-256 with SHA-256.
            "es256",
            None,
            // Self-issued and chaining to nothing, which is exactly what
            // strict signing exists to refuse.
            Strictness::Permissive,
        )
        .expect("a generated certificate is not a C2PA test certificate")
    }

    /// An identity that passes every check made when it is *configured*
    /// and fails when it is *used*.
    ///
    /// The certificate is the same well-formed one
    /// [`throwaway_identity`] builds; the key is not a key. That is the
    /// shape of the failure this module says every signing deployment
    /// eventually meets — an expiry, a revoked key, a token that is not
    /// plugged in — and it is the one that must not take the XMP half
    /// down with it.
    fn unusable_identity() -> SigningIdentity {
        let (cert, _) = self_signed_pair();
        SigningIdentity::from_bytes(
            cert,
            b"-----BEGIN PRIVATE KEY-----\nnot a key at all\n-----END PRIVATE KEY-----\n".to_vec(),
            "es256",
            None,
            Strictness::Permissive,
        )
        .expect("the certificate is inspected here; the key is not")
    }

    /// A self-signed certificate and its key, both PEM.
    fn self_signed_pair() -> (Vec<u8>, Vec<u8>) {
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
        //    `emailProtection` is one the C2PA specification names for
        //    document signing.
        // 4. Both an authority key identifier and a subject key
        //    identifier extension.
        //
        // `emailProtection` alone, deliberately. It is the first entry
        // in `c2pa`'s accept-list and the shape most certificates in use
        // actually have, so it is the shape a fixture standing in for
        // one should carry — and it keeps every signing test running
        // through the input `inspect_certificate` is most likely to be
        // handed. It briefly carried a C2PA claim-signing OID as well,
        // to satisfy a version of that check which required one; the
        // check was wrong (`ACCEPTED_SIGNING_EKUS`) and the fixture is
        // back to the shape it was testing against.
        params.is_ca = rcgen::IsCa::ExplicitNoCa;
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection];
        params.use_authority_key_identifier_extension = true;
        let cert = params.self_signed(&key).expect("a self-signed certificate");
        (cert.pem().into_bytes(), key.serialize_pem().into_bytes())
    }

    /// A certificate carrying everything [`inspect_certificate`] warns
    /// about the absence of, followed by a second certificate.
    ///
    /// What [`Strictness::Strict`] is meant to accept, built so that the
    /// strict path is exercised on its passing side too — a check only
    /// ever seen refusing is one nobody can tell apart from a switch
    /// that disables the feature.
    fn issued_shaped_pair() -> (Vec<u8>, Vec<u8>) {
        let key = rcgen::KeyPair::generate().expect("a P-256 key pair");
        let mut params = rcgen::CertificateParams::new(vec!["asterism.invalid".to_string()])
            .expect("certificate parameters");
        params.is_ca = rcgen::IsCa::ExplicitNoCa;
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::EmailProtection,
            // `c2pa-kp-claimSigning`, the usage the Conformance
            // Program's profile requires beside one of the above.
            rcgen::ExtendedKeyUsagePurpose::Other(vec![1, 3, 6, 1, 4, 1, 62558, 2, 1]),
        ];
        params.use_authority_key_identifier_extension = true;
        params
            .distinguished_name
            .push(rcgen::DnType::OrganizationName, "Asterism Test Fixtures");
        let cert = params.self_signed(&key).expect("a self-signed certificate");

        let issuer_key = rcgen::KeyPair::generate().expect("a P-256 key pair");
        let mut issuer_params =
            rcgen::CertificateParams::new(vec!["asterism-issuer.invalid".to_string()])
                .expect("certificate parameters");
        issuer_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let issuer = issuer_params
            .self_signed(&issuer_key)
            .expect("an issuer certificate");

        let mut chain = cert.pem().into_bytes();
        chain.extend_from_slice(issuer.pem().as_bytes());
        (chain, key.serialize_pem().into_bytes())
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

    /// A `ftyp` box: size, `ftyp`, the major brand, a zero minor
    /// version, then the compatible brands.
    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compatible.len();
        let mut head = (size as u32).to_be_bytes().to_vec();
        head.extend_from_slice(b"ftyp");
        head.extend_from_slice(major);
        head.extend_from_slice(&[0, 0, 0, 0]);
        for brand in compatible {
            head.extend_from_slice(*brand);
        }
        head
    }

    #[test]
    fn ftyp_brands_outside_the_mp4_family_are_refused() {
        // HEIC and AVIF, with the compatible lists their encoders write.
        assert_eq!(Container::sniff(&ftyp(b"heic", &[b"mif1", b"heic"])), None);
        assert_eq!(Container::sniff(&ftyp(b"avif", &[b"avif", b"mif1"])), None);
        // M4A declares `isom` compatible, which is why the major brand
        // answers first: compatibility with a video dialect does not
        // make audio `video/mp4`.
        assert_eq!(
            Container::sniff(&ftyp(b"M4A ", &[b"M4A ", b"mp42", b"isom"])),
            None
        );
    }

    #[test]
    fn a_vendor_major_brand_is_read_from_its_compatible_list() {
        // Sony's XAVC: the vendor name as the major brand, MP4
        // membership declared behind it.
        assert_eq!(
            Container::sniff(&ftyp(b"XAVC", &[b"XAVC", b"mp42", b"iso2"])),
            Some(Container::Mp4)
        );
        // A brand list naming nothing recognised is refused rather than
        // guessed at, and so is one naming a foreign family ahead of an
        // MP4 dialect — non-exclusivity cuts both ways.
        assert_eq!(Container::sniff(&ftyp(b"ZZZZ", &[b"YYYY"])), None);
        assert_eq!(Container::sniff(&ftyp(b"ZZZZ", &[b"heic", b"isom"])), None);
    }

    #[test]
    fn an_unsigned_writer_still_writes_the_iptc_disclosure() {
        // The state every install starts in. If this produced nothing,
        // the certificate question would block the half of the
        // obligation that does not need one.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        let outcome = DisclosureWriter::unsigned()
            .apply(&path, &record())
            .unwrap();
        assert_eq!(outcome.xmp, Half::Written);
        assert_eq!(
            outcome.manifest,
            Half::Skipped(Skipped::NoSigningIdentity),
            "skipped, not failed: no certificate is a configuration, not a fault"
        );
        assert!(outcome.discloses());
        assert!(outcome.failures().is_empty());

        let packet = embed::read_xmp(&std::fs::read(&path).unwrap())
            .unwrap()
            .expect("the file carries a packet");
        assert!(packet.contains("trainedAlgorithmicMedia"));
    }

    #[test]
    fn a_configured_identity_that_did_not_load_fails_the_manifest_half() {
        // The distinction the third state exists for. An installation
        // that configured a certificate and got nothing out of it is not
        // one that configured none, and reporting the export the same
        // way would tell whoever reads the record that the deployment is
        // doing exactly what it was set up to do.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        let reason = "reading /etc/asterism/cert.pem: No such file or directory";
        let outcome = DisclosureWriter::unavailable(reason)
            .apply(&path, &record())
            .unwrap();

        assert_eq!(
            outcome.xmp,
            Half::Written,
            "the half that needs no certificate still lands — the whole reason a \
             failure lives in the value rather than the error channel"
        );
        assert_eq!(outcome.manifest, Half::Failed(reason.to_string()));
        assert!(
            outcome.discloses(),
            "a file carrying one mark is a disclosed file, whatever went wrong beside it"
        );
        assert_eq!(
            outcome.failures(),
            vec![reason],
            "and this is the list a caller reads to decide whether to tell anybody"
        );
    }

    #[test]
    fn strict_signing_refuses_what_a_trust_list_would_not_carry() {
        let (cert, key) = self_signed_pair();
        let err = SigningIdentity::from_bytes(
            cert.clone(),
            key.clone(),
            "es256",
            None,
            Strictness::Strict,
        )
        .unwrap_err();

        let message = err.to_string();
        for expected in ["c2pa-kp-claimSigning", "organisation", "chain"] {
            assert!(
                message.contains(expected),
                "strict refusal should name {expected}: {message}"
            );
        }
        assert!(
            message.contains("It can sign"),
            "and must not read as 'this cannot sign', which is a different and \
             untrue statement: {message}"
        );

        SigningIdentity::from_bytes(cert, key, "es256", None, Strictness::Permissive)
            .expect("the same certificate signs with strict signing off");
    }

    #[test]
    fn strict_signing_does_not_tell_an_unreadable_bundle_to_turn_it_off() {
        // The branch reachable because `inspect_certificate` passes
        // bytes it cannot parse: no refusals, no warnings, and then the
        // strict block. Advising "use it with strict signing off" here
        // would send an operator whose file is DER rather than PEM to a
        // configuration that fails on every export instead of once.
        let err = SigningIdentity::from_bytes(
            b"-----BEGIN CERTIFICATE-----\nnot base64 at all\n-----END CERTIFICATE-----\n".to_vec(),
            b"key".to_vec(),
            "es256",
            None,
            Strictness::Strict,
        )
        .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("could not read a certificate"),
            "{message}"
        );
        assert!(
            !message.contains("It can sign"),
            "nothing established that it can: {message}"
        );
    }

    #[test]
    fn strict_signing_accepts_a_certificate_shaped_like_an_issued_one() {
        // Strict has to be reachable, or it is a switch that turns
        // signing off. This fixture carries what the warnings ask for —
        // the claim-signing usage, an organisation in the subject — and
        // arrives with a second certificate behind it.
        //
        // Unrelated certificates, deliberately: the chain requirement is
        // a count and not a verification. Whether a chain links is a
        // validator's question asked against a trust list, and this side
        // cannot answer it — what it can see is that a bundle carrying
        // one certificate has had its chain dropped or never had one.
        let (cert, key) = issued_shaped_pair();
        SigningIdentity::from_bytes(cert, key, "es256", None, Strictness::Strict)
            .expect("nothing here is a reason a trust list would refuse it");
    }

    #[test]
    fn a_record_with_nothing_to_disclose_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let original = png_fixture();
        std::fs::write(&path, &original).unwrap();

        let outcome = DisclosureWriter::unsigned()
            .apply(&path, &DisclosureRecord::for_asset("asset-1"))
            .unwrap();
        assert!(!outcome.discloses());
        assert_eq!(
            outcome.xmp,
            Half::Skipped(Skipped::NothingToDisclose),
            "the record asserted nothing — distinct from a packet that failed"
        );
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

        let outcome = DisclosureWriter::unsigned()
            .apply(&path, &record())
            .unwrap();
        assert_eq!(
            outcome.xmp,
            Half::Skipped(Skipped::ContainerCannotCarryIt),
            "no XMP half for a BMFF container"
        );
        assert_eq!(
            outcome.manifest,
            Half::Skipped(Skipped::NoSigningIdentity),
            "and no identity to sign with"
        );
        assert!(!outcome.discloses());
        assert!(
            outcome.failures().is_empty(),
            "the file is unmarked and nothing went wrong — two different \
             statements, and the caller needs both to decide whether to \
             tell anyone"
        );
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
            DisclosureWriter::unsigned().apply(&path, &record()),
            Err(DisclosureError::UnsupportedContainer { .. })
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
        let err = SigningIdentity::from_bytes(
            pem,
            b"key".to_vec(),
            "es256",
            None,
            Strictness::Permissive,
        )
        .unwrap_err();
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
        let err = SigningIdentity::from_bytes(
            pem.into_bytes(),
            b"key".to_vec(),
            "nonsense",
            None,
            Strictness::Permissive,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown signing algorithm"));
    }

    /// Builds a self-signed certificate with the extensions a test wants
    /// to reason about.
    fn certificate_with(
        ekus: Vec<rcgen::ExtendedKeyUsagePurpose>,
        is_ca: rcgen::IsCa,
        organisation: Option<&str>,
    ) -> Vec<u8> {
        let key = rcgen::KeyPair::generate().expect("a P-256 key pair");
        let mut params = rcgen::CertificateParams::new(vec!["example.invalid".to_string()])
            .expect("certificate parameters");
        params.is_ca = is_ca;
        params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = ekus;
        if let Some(organisation) = organisation {
            params
                .distinguished_name
                .push(rcgen::DnType::OrganizationName, organisation);
        }
        params
            .self_signed(&key)
            .expect("a self-signed certificate")
            .pem()
            .into_bytes()
    }

    fn claim_signing_eku() -> rcgen::ExtendedKeyUsagePurpose {
        rcgen::ExtendedKeyUsagePurpose::Other(vec![1, 3, 6, 1, 4, 1, 62558, 2, 1])
    }

    /// The refusal is about what a certificate can sign, and nothing
    /// else.
    ///
    /// The first version of this check required `c2pa-kp-claimSigning`
    /// and refused everything without it. That is the Conformance
    /// Program's *issuance* profile, not the set `c2pa` will sign with —
    /// which is an accept-list where one entry is enough — so it would
    /// have refused a `documentSigning`-only certificate that signs
    /// perfectly well, with a message saying it could not.
    #[test]
    fn a_certificate_is_refused_only_for_what_it_cannot_do() {
        // Signs, and is not what the profile would have issued: one
        // warning, no refusal. This is the shape most certificates in
        // use have.
        let email_only = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );
        let verdict = inspect_certificate(&email_only);
        assert!(verdict.refusals.is_empty(), "{verdict:?}");
        assert_eq!(verdict.warnings.len(), 1, "{verdict:?}");
        assert!(verdict.warnings[0].contains("c2pa-kp-claimSigning"));

        // `documentSigning` alone — the profile IPTC's own publisher
        // policy mandates, and the certificate the first version of this
        // check would have refused while telling its operator it could
        // not sign. It is not one of the usages `x509-parser` lifts into
        // a named field, so it arrives through `other`, which is the
        // path that has to work for four of the six accepted usages.
        let document_signing = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::Other(vec![
                1, 3, 6, 1, 5, 5, 7, 3, 36,
            ])],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );
        let verdict = inspect_certificate(&document_signing);
        assert!(verdict.refusals.is_empty(), "{verdict:?}");

        // And the Microsoft C2PA signing usage, which arrives the same
        // way and is the other one a hand-written accept-list is likely
        // to lose.
        let microsoft = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::Other(vec![
                1, 3, 6, 1, 4, 1, 311, 76, 59, 1, 9,
            ])],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );
        assert!(
            inspect_certificate(&microsoft).refusals.is_empty(),
            "{:?}",
            inspect_certificate(&microsoft)
        );

        // What the profile issues: nothing to say at all.
        let conforming = certificate_with(
            vec![
                rcgen::ExtendedKeyUsagePurpose::EmailProtection,
                claim_signing_eku(),
            ],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );
        assert_eq!(
            inspect_certificate(&conforming),
            CertificateVerdict::default()
        );

        // No organisation is its own warning, separate from the usage
        // one — a certificate can miss either without the other.
        let anonymous = certificate_with(
            vec![
                rcgen::ExtendedKeyUsagePurpose::EmailProtection,
                claim_signing_eku(),
            ],
            rcgen::IsCa::ExplicitNoCa,
            None,
        );
        let verdict = inspect_certificate(&anonymous);
        assert!(verdict.refusals.is_empty(), "{verdict:?}");
        assert_eq!(verdict.warnings.len(), 1, "{verdict:?}");
        assert!(verdict.warnings[0].contains("organisation"));

        // Signs nothing: an extended key usage naming only something a
        // claim cannot be signed under.
        let wrong_usage = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );
        let verdict = inspect_certificate(&wrong_usage);
        assert_eq!(verdict.refusals.len(), 1, "{verdict:?}");
        assert!(verdict.refusals[0].contains("extended key usage"));

        // A CA certificate offering to sign a claim itself.
        let authority = certificate_with(
            vec![claim_signing_eku()],
            rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained),
            Some("Example Ltd"),
        );
        let verdict = inspect_certificate(&authority);
        assert!(
            verdict
                .refusals
                .iter()
                .any(|r| r.contains("CA certificate")),
            "{verdict:?}"
        );

        // A certificate with no extended key usage extension at all is
        // refused for that, and told so — it has not named a wrong
        // usage, it has named none.
        let no_usage = certificate_with(vec![], rcgen::IsCa::ExplicitNoCa, Some("Example Ltd"));
        let verdict = inspect_certificate(&no_usage);
        assert_eq!(verdict.refusals.len(), 1, "{verdict:?}");
        assert!(
            verdict.refusals[0].contains("no extended key usage"),
            "{verdict:?}"
        );

        // Bytes nothing here can read yield no findings rather than a
        // refusal, which is the escape hatch and its cost both.
        assert_eq!(
            inspect_certificate(b"not a certificate"),
            CertificateVerdict::default()
        );
    }

    /// The walk finds the certificate wherever the bundle puts it.
    ///
    /// Both shapes below are what the inspection's own doc argues from,
    /// and neither was asserted: a simplification of that `find` back to
    /// "the first block" would break the first case silently, and the
    /// CA refusal's message claims to describe the second.
    #[test]
    fn the_inspection_reads_past_a_block_that_is_not_a_certificate() {
        let key = rcgen::KeyPair::generate().expect("a P-256 key pair");
        let leaf = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );

        // A bundle led by the private key, which some tools write.
        let mut key_first = key.serialize_pem().into_bytes();
        key_first.extend_from_slice(&leaf);
        let verdict = inspect_certificate(&key_first);
        assert!(verdict.refusals.is_empty(), "{verdict:?}");
        assert_eq!(
            verdict,
            inspect_certificate(&leaf),
            "the key block changes nothing about what is found"
        );

        // A chain written root-first: the CA is what gets inspected, and
        // the refusal says which certificate it is talking about rather
        // than calling it the leaf.
        let root = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection],
            rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained),
            Some("Example Ltd"),
        );
        let mut root_first = root;
        root_first.extend_from_slice(&leaf);
        let verdict = inspect_certificate(&root_first);
        assert!(
            verdict
                .refusals
                .iter()
                .any(|r| r.contains("CA certificate") && r.contains("root-first")),
            "{verdict:?}"
        );
    }

    /// The refusal reaches the caller, with the sentence that tells them
    /// what still happens.
    ///
    /// `inspect_certificate` is exercised directly above; this is the
    /// wiring, which nothing else asserts — including the half of the
    /// message saying the IPTC/XMP disclosure is written anyway.
    #[test]
    fn loading_an_identity_refuses_a_certificate_that_cannot_sign() {
        let wrong_usage = certificate_with(
            vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth],
            rcgen::IsCa::ExplicitNoCa,
            Some("Example Ltd"),
        );
        let err = SigningIdentity::from_bytes(
            wrong_usage,
            b"key".to_vec(),
            "es256",
            None,
            Strictness::Permissive,
        )
        .expect_err("a certificate that signs nothing is not an identity");
        let message = err.to_string();
        assert!(message.contains("cannot sign a C2PA claim"), "{message}");
        assert!(message.contains("IPTC/XMP disclosure"), "{message}");
    }

    #[test]
    fn a_failed_write_leaves_no_partial_file_beside_the_original() {
        // The export directory is somewhere an importer may be
        // watching. A leftover `.c2pa-partial` there turns one failure
        // into a second artefact that gets ingested.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();
        DisclosureWriter::unsigned()
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
    ///
    /// Note what the call returns: `Ok`. A manifest that failed is the
    /// manifest half's outcome, not the call's — the cleanup is the
    /// property under test here, and it has to happen on a path that no
    /// longer returns early.
    #[test]
    fn a_failed_signing_removes_its_temporary_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let original = b"\x89PNG\r\n\x1a\nnot a chunk anybody can walk";
        std::fs::write(&path, original).unwrap();

        let outcome = DisclosureWriter::signed_with(throwaway_identity())
            .apply(&path, &DisclosureRecord::for_asset("asset-1"))
            .expect("a failed manifest is reported, not raised");
        assert!(
            outcome.manifest.failure().is_some(),
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

        DisclosureWriter::signed_with(throwaway_identity())
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

    #[cfg(unix)]
    #[test]
    fn the_staging_name_is_not_one_anything_can_predict() {
        // What the deterministic sibling allowed. `shot.png` staged
        // through `shot.png.c2pa-partial`, opened with neither `O_EXCL`
        // nor `O_NOFOLLOW`, so anything else able to create a file in
        // the export directory could put a symlink there first and have
        // the stamp write the whole asset through it, over a file of
        // the attacker's choosing.
        //
        // An export directory is not necessarily private: it is
        // wherever the user pointed the export, which may be shared,
        // synced, or watched.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"not this file").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("shot.png.c2pa-partial")).unwrap();

        DisclosureWriter::unsigned()
            .apply(&path, &record())
            .unwrap();

        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"not this file",
            "the staged write went somewhere nothing had guessed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stamping_keeps_the_permissions_the_file_had() {
        // The staged file is created at 0600 and the rename carries
        // that mode onto the target, so without copying the original's
        // permissions across, stamping would change them — in whichever
        // direction the umask happened to point. Narrowing is a
        // surprise; widening, on a file about to be published, is worse.
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

        DisclosureWriter::unsigned()
            .apply(&path, &record())
            .unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "stamping is not a permission change");
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

        let writer = DisclosureWriter::signed_with(throwaway_identity());
        assert!(writer.can_sign());
        let outcome = writer.apply(&path, &record()).unwrap();
        assert_eq!(outcome.xmp, Half::Written);
        assert_eq!(outcome.manifest, Half::Written);

        let signed = std::fs::read(&path).unwrap();
        let reader = read_manifest("image/png", signed).expect("the manifest reads back");
        let json = reader.json();
        assert!(
            json.contains("trainedAlgorithmicMedia"),
            "the actions assertion carries the IPTC URI: {json}"
        );
        assert!(
            json.contains(asterism_disclosure_format::manifest::ASTERISM_LABEL),
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

        DisclosureWriter::signed_with(throwaway_identity())
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
        let tampered = embed::stamp(&signed, &corrected).unwrap().unwrap().bytes;
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

            let outcome = DisclosureWriter::signed_with(throwaway_identity())
                .apply(&path, &record())
                .unwrap();
            assert_eq!(
                outcome.xmp,
                Half::Skipped(Skipped::ContainerCannotCarryIt),
                "{name}: no XMP half for BMFF"
            );
            assert_eq!(
                outcome.manifest,
                Half::Written,
                "{name}: the manifest landed"
            );

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
    fn a_signing_failure_does_not_take_the_packet_with_it() {
        // The regression this shape exists for. Every failure inside
        // the signing block used to return early, which discarded the
        // XMP packet already sitting in memory — so a certificate that
        // stopped working withheld the IPTC half, which needs no
        // certificate and which the module docs promise "still lands".
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, png_fixture()).unwrap();

        let outcome = DisclosureWriter::signed_with(unusable_identity())
            .apply(&path, &record())
            .expect("a manifest that fails is not the call failing");

        assert_eq!(
            outcome.xmp,
            Half::Written,
            "the half that needs no key landed"
        );
        assert!(
            outcome.manifest.failure().is_some(),
            "and the other half says what went wrong: {:?}",
            outcome.manifest
        );
        assert!(outcome.discloses(), "the file is disclosed either way");

        // Not just in the outcome — on disk.
        let packet = embed::read_xmp(&std::fs::read(&path).unwrap())
            .unwrap()
            .expect("the packet reached the file");
        assert!(packet.contains("trainedAlgorithmicMedia"));
    }

    #[test]
    fn a_system_name_that_will_not_fit_falls_back_to_the_bare_mark() {
        // `essential()` drops the prompt but keeps the AI system, which
        // is read out of someone else's file — so a long enough one
        // overflowed a JPEG segment twice and cost the whole packet.
        // The fallback ladder now has a bottom, the mark alone, and it
        // always fits.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.jpg");
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        std::fs::write(&path, &jpeg).unwrap();

        let oversized = "x".repeat(embed::JPEG_MAX_PACKET + 1);
        let outcome = DisclosureWriter::unsigned()
            .apply(
                &path,
                &DisclosureRecord::for_asset("asset-1")
                    .with_source_type(DigitalSourceType::TrainedAlgorithmicMedia)
                    .with_ai_system(oversized, None),
            )
            .expect("a system name that does not fit is not the call failing");

        assert_eq!(outcome.xmp, Half::Written, "the mark landed");
        assert!(outcome.discloses());
        assert!(
            outcome.system_dropped,
            "a packet reduced to the bare mark and a container that never \
             named its generator read identically afterwards, so the \
             difference leaves through the outcome"
        );
        assert!(
            !outcome.prompt_dropped,
            "this record asked for no prompt, so none was withheld"
        );
        let packet = embed::read_xmp(&std::fs::read(&path).unwrap())
            .unwrap()
            .expect("the packet reached the file");
        assert!(packet.contains("trainedAlgorithmicMedia"));
        assert!(
            !packet.contains("AISystemUsed"),
            "the unbounded string was dropped, not split or truncated"
        );
    }

    #[test]
    fn a_packet_that_cannot_be_written_at_all_does_not_fail_the_call() {
        // With no source type there is no bounded tier to step down to,
        // so the packet half genuinely cannot be written. That used to
        // fail the whole call; now the half reports its own failure and
        // the file is left alone.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.jpg");
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        std::fs::write(&path, &jpeg).unwrap();

        let oversized = "x".repeat(embed::JPEG_MAX_PACKET + 1);
        let outcome = DisclosureWriter::unsigned()
            .apply(
                &path,
                &DisclosureRecord::for_asset("asset-1").with_ai_system(oversized, None),
            )
            .expect("a packet that does not fit is not the call failing");

        assert!(
            outcome.xmp.failure().is_some(),
            "the packet half reports its own failure: {:?}",
            outcome.xmp
        );
        assert!(!outcome.discloses(), "and nothing was disclosed");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            jpeg,
            "the file is untouched rather than half-written"
        );
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
        let outcome = DisclosureWriter::unsigned()
            .apply(&path, &record().with_prompt(huge))
            .unwrap();
        assert_eq!(outcome.xmp, Half::Written);
        assert!(outcome.prompt_dropped);
        assert!(
            !outcome.system_dropped,
            "the prompt tier keeps the system name, and the note says only what went"
        );

        let packet = embed::read_xmp(&std::fs::read(&path).unwrap())
            .unwrap()
            .unwrap();
        assert!(packet.contains("trainedAlgorithmicMedia"));
        assert!(!packet.contains("AIPromptInformation"));
    }
}
