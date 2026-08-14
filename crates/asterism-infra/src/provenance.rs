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
/// # Where a strictness setting goes
///
/// A deployment signing for publication would reasonably want the
/// warnings to refuse too. That configuration is deliberately not
/// invented here, and the shape it needs is this verdict being
/// reachable without loading an identity — which is what
/// [`inspect_certificate`] is for. Such a caller inspects first and
/// refuses on its own terms; what it cannot do is tell
/// [`SigningIdentity::from_bytes`] to refuse for it, so it writes its
/// own message and the warnings are logged a second time when it goes
/// on to load. Whoever wires the setting decides whether that is worth
/// a parameter.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CertificateVerdict {
    /// Findings that make the certificate unusable for signing at all.
    pub refusals: Vec<String>,
    /// Findings that keep it off a trust list without stopping it
    /// signing for a reader who has imported it.
    pub warnings: Vec<String>,
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

        // What the certificate says about itself, after what it is
        // called. The name check above is a heuristic a rename defeats;
        // this reads the extensions instead. Last of the three, so a
        // bundle that fails an earlier one is refused for the earlier
        // reason rather than warned about first.
        let verdict = inspect_certificate(&cert_chain);
        if !verdict.refusals.is_empty() {
            return Err(ProvenanceError::Identity(format!(
                "this certificate cannot sign a C2PA claim: {}. Nothing is written rather \
                 than a signature that will not hold — the IPTC/XMP disclosure is written \
                 either way",
                verdict.refusals.join("; ")
            )));
        }
        for warning in &verdict.warnings {
            // The dotted name is the `event` field rather than the
            // target, which is the convention the rest of this crate
            // uses: both sinks filter on `asterism=info` by target
            // prefix, so an event addressed `diag.…` reaches neither
            // stderr nor the `diag_log` table.
            tracing::warn!(
                event = "diag.provenance.identity",
                "signing certificate: {warning}"
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
/// [`ProvenanceError::Sign`] instead, which is a worse message but not a
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
        let err = SigningIdentity::from_bytes(wrong_usage, b"key".to_vec(), "es256", None)
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
