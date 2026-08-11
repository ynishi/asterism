//! `source_locator` — where an artefact's bytes are, held as a typed
//! value instead of being sniffed out of a string at every call site.
//!
//! A locator has four shapes and they are not variations of one string:
//! a file on this machine, one record inside a container file on this
//! machine, something a remote scheme addresses, and a caller-minted
//! name for something that never had bytes. Each shape is its own type
//! holding its information already taken apart, and
//! [`SourceLocator`] is the umbrella over them — a sum composed with
//! `From`, owning no recognition logic beyond the one boundary below.
//!
//! # Why this is its own module
//!
//! It is the only code that knows the storage encoding. Keeping that in
//! one file is what makes the claim checkable: a reader who wants to
//! know how a locator is spelled on disk reads
//! [`SourceLocator::to_storage`] and its inverse, and there is nowhere
//! else to look.
//!
//! # The storage encoding is tagged JSON
//!
//! [`to_storage`](SourceLocator::to_storage) writes, and
//! [`TryFrom<&str>`](SourceLocator::try_from) reads, one object per
//! shape:
//!
//! ```json
//! {"kind":"file",   "path":"/pics/a.png"}
//! {"kind":"record", "container":"/logs/s.jsonl","record":"0198c1c2-…"}
//! {"kind":"remote", "scheme":"hf","target":"org/model/f.safetensors"}
//! {"kind":"logical","name":"chat/0198c1c2/msg-1"}
//! ```
//!
//! Reading is `serde` over that form, and **there is no recognition to
//! do**: the tag says which type it is, and each type then validates its
//! own fields — a `file` whose path has no root is still refused, a
//! `remote` whose scheme is one character still is, and `file` is still
//! never a [`Scheme`].
//!
//! Two properties of the rendering are load-bearing rather than
//! stylistic:
//!
//! - **Canonical.** Fixed field order, no whitespace, no optional keys.
//!   The `(persona_id, source_kind, source_locator)` lookup is an
//!   equality test on this string, so two equal locators must render to
//!   byte-identical text. `serde` derives the whole rendering from the
//!   shape below; nothing here hand-writes JSON.
//! - **Opaque to SQL.** Nothing queries inside it, so it is a
//!   self-describing encoding in a TEXT column — not a reason to reach
//!   for SQLite's JSON functions.
//!
//! What the tag **removes rather than manages**, all of it inherited
//! from the delimited form this replaced:
//!
//! - **percent-escaping.** No character in a path or a record address is
//!   special any more, so nothing has to be escaped on the way in or
//!   unescaped on the way out.
//! - **the `/pics/a#b.png` ambiguity.** A legal POSIX filename with a
//!   `#` in it was indistinguishable from a container plus a record.
//!   Here it is a `file` whose `path` contains a `#`, and nothing looks
//!   at that character. (The rows *already stored* under the old form
//!   are settled by the rewrite migration, which can ask the filesystem
//!   which of the two a given string was — a test no parser can run.)
//! - **the split-direction question.** `split` or `rsplit`, first `#` or
//!   last: there is no split.
//!
//! # Two readers, because there are two boundaries
//!
//! [`TryFrom<&str>`](SourceLocator::try_from) is the **storage** reader
//! and speaks only the form above. A locator arriving from an *importer*
//! is a different contract — `FootprintSource::locator` is documented as
//! a path, or `<container>#<record>` for a per-record source, and the
//! parsers emit exactly that — so it has its own entry point,
//! [`from_wire`](SourceLocator::from_wire), which is where the ordered
//! guess still lives. Keeping them apart is what lets the column form
//! change without renegotiating the SDK contract, and it is why nothing
//! outside `from_wire` reads a `#`.
//!
//! A third reader exists and is not here: the **frozen** copy inside the
//! rewrite migration, which reads what the columns held *before* the
//! tag. It is deliberately a snapshot rather than a call into this
//! module, for the reason the V56 snapshot beside it already gives — a
//! landed migration has to keep meaning what it meant when it ran.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// An absolute path on this machine.
///
/// Absoluteness is a type invariant rather than a convention, which is
/// what lets [`ContainerRecord`] require an openable container by
/// construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPath(PathBuf);

impl LocalPath {
    /// The path itself, for a caller about to open it.
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The path as text — total, because construction refused a path
    /// that is not valid UTF-8 (see [`TryFrom<PathBuf>`]).
    ///
    /// Panics rather than substituting a default if that invariant is
    /// ever broken by a new constructor. This value is rendered into a
    /// NOT NULL TEXT column, and an empty `path` is one of the things
    /// [`SourceLocator::try_from`] refuses to read back (a rootless
    /// path is not a [`LocalPath`]) — so a silent `""` would write a row
    /// that cannot be loaded again, which is worse than the panic that
    /// says which invariant went.
    pub fn as_str(&self) -> &str {
        self.0
            .to_str()
            .expect("LocalPath refuses a non-UTF-8 path at construction")
    }
}

/// Whether a string names a location with a root.
///
/// Hand-rolled rather than deferring to [`Path::is_absolute`] on
/// purpose: `is_absolute` is platform-conditional, so a Windows-spelled
/// path (`C:\pics\a.png`) would answer `false` on a unix build and the
/// row would be reclassified as a logical name by the machine that read
/// it. The predicate this replaced (`content_hash::is_hashable_locator`,
/// deleted with the consumer migration) made the same trade for the same
/// reason; the fixture that asserts it is
/// `a_one_character_scheme_is_a_windows_drive_not_a_scheme` below.
fn is_rooted(raw: &str) -> bool {
    let mut chars = raw.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some('/'), _, _) => true,
        (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic() => true,
        _ => false,
    }
}

impl TryFrom<&str> for LocalPath {
    type Error = DomainError;

    /// Rejects a rootless path. A path with no root is not openable by
    /// the process that reads it — the jobs run in a different process
    /// from the importer that recorded it, so there is no working
    /// directory that would resolve it — and "no bytes" is the true
    /// answer rather than a resolution attempt against whatever
    /// directory happens to be current.
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        if !is_rooted(raw) {
            return Err(DomainError::Validation(format!(
                "LocalPath must be absolute, got {raw:?}"
            )));
        }
        Ok(Self(PathBuf::from(raw)))
    }
}

impl TryFrom<PathBuf> for LocalPath {
    type Error = DomainError;

    /// The producer-side constructor: an importer that already holds a
    /// `PathBuf` never goes through a string.
    ///
    /// Refuses a path with no UTF-8 rendering as well as a rootless
    /// one. The column this ends up in is TEXT, so a path that cannot
    /// be rendered as text cannot be stored and could not be read back;
    /// refusing it here keeps [`SourceLocator::to_storage`] total
    /// instead of lossy.
    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        let raw = path.to_str().ok_or_else(|| {
            DomainError::Validation(format!("LocalPath must be valid UTF-8, got {path:?}"))
        })?;
        if !is_rooted(raw) {
            return Err(DomainError::Validation(format!(
                "LocalPath must be absolute, got {raw:?}"
            )));
        }
        Ok(Self(path))
    }
}

/// How a container's reader finds one record again.
///
/// Opaque here — only the reader for that container shape interprets
/// it: a line number, a message uuid, a journal row id. Non-empty, and
/// with **no character reserved**: the storage form is tagged rather
/// than delimited, so an address containing a `#` is stored and read
/// back unharmed. The one place a `#` still means something is
/// [`SourceLocator::from_wire`], which reads the spelling importers
/// send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordAddress(String);

impl RecordAddress {
    /// The address as the container's reader stated it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for RecordAddress {
    type Error = DomainError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        if raw.is_empty() {
            return Err(DomainError::Validation(
                "RecordAddress must not be empty".into(),
            ));
        }
        Ok(Self(raw.to_string()))
    }
}

/// One record inside a container file on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRecord {
    container: LocalPath,
    record: RecordAddress,
}

impl ContainerRecord {
    /// Pairs a container with an address inside it. Total: both halves
    /// carry their own invariant already.
    pub fn new(container: LocalPath, record: RecordAddress) -> Self {
        Self { container, record }
    }

    /// The file holding the record — openable, by [`LocalPath`]'s
    /// invariant.
    pub fn container(&self) -> &LocalPath {
        &self.container
    }

    /// The address the container's reader resolves.
    pub fn record(&self) -> &RecordAddress {
        &self.record
    }
}

/// A URI scheme, lowercased.
///
/// RFC 3986 grammar (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`)
/// **with one deliberate departure: a single character is refused.**
/// RFC 3986 permits it; allowing it here would read the `C` of
/// `C://pics/a.png` as a scheme and reclassify every Windows-spelled
/// path as remote. The gate this replaced made the same trade and said
/// so in as many words ("a scheme is longer than one character"); it is
/// deleted now, and this is where the rule lives. Do not "fix" this back
/// to the RFC.
///
/// Open set, not an enum: `https`, `s3`, `hf`, and whatever the next
/// source speaks. Constants name the ones in hand; the grammar admits
/// the rest.
///
/// `file` is **not** one of these and is refused by the constructor —
/// it is a spelling of a local path, not a fact about the artefact, and
/// the boundary consumes it (see [`SourceLocator::try_from`]). Refusing
/// it here means no `Remote` carrying a `file` scheme can be built at
/// all, so `file:///pics/a.png` and `/pics/a.png` compare equal by
/// construction rather than by the parser remembering to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheme(String);

impl Scheme {
    /// The scheme a web-hosted original is addressed by.
    pub const HTTPS: &str = "https";
    /// The scheme an object-store original is addressed by.
    pub const S3: &str = "s3";
    /// The scheme a model-weights source speaks.
    pub const HF: &str = "hf";
    /// The scheme consumed at the boundary and never recorded.
    const FILE: &str = "file";

    /// The scheme, lowercased.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether a string satisfies the grammar above, including the
    /// two-character minimum. Used by the boundary before it commits to
    /// reading a `://` as a scheme separator.
    fn is_valid(raw: &str) -> bool {
        let mut chars = raw.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() {
            return false;
        }
        // The departure from RFC 3986: one character is not a scheme.
        if chars.clone().next().is_none() {
            return false;
        }
        chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    }
}

impl TryFrom<&str> for Scheme {
    type Error = DomainError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        if !Self::is_valid(raw) {
            return Err(DomainError::Validation(format!(
                "not a scheme (alphabetic first character, two or more characters): {raw:?}"
            )));
        }
        let lowered = raw.to_ascii_lowercase();
        if lowered == Self::FILE {
            return Err(DomainError::Validation(
                "`file` is consumed at the storage boundary and is never recorded as a Scheme"
                    .into(),
            ));
        }
        Ok(Self(lowered))
    }
}

/// Everything after `<scheme>://`.
///
/// Opaque here, for the same reason [`RecordAddress`] is: what it means
/// is defined by the scheme, not by Album.
/// `hf://org/model/file.safetensors` and `https://host/p?q=1` do not
/// share a decomposition, and inventing one would be wrong for
/// whichever scheme it was not modelled on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteTarget(String);

impl RemoteTarget {
    /// The target as the source stated it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for RemoteTarget {
    type Error = DomainError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        if raw.is_empty() {
            return Err(DomainError::Validation(
                "RemoteTarget must not be empty".into(),
            ));
        }
        Ok(Self(raw.to_string()))
    }
}

/// An artefact addressed by a scheme this machine does not resolve to a
/// local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRef {
    scheme: Scheme,
    target: RemoteTarget,
}

impl RemoteRef {
    /// Pairs a scheme with its target.
    pub fn new(scheme: Scheme, target: RemoteTarget) -> Self {
        Self { scheme, target }
    }

    /// The scheme, which is the fact about the artefact worth keeping.
    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Everything after `://`, uninterpreted.
    pub fn target(&self) -> &RemoteTarget {
        &self.target
    }
}

/// A caller-minted name. No bytes anywhere.
///
/// Its own invariant is only non-emptiness. "It is not any of the
/// others" comes from the order the umbrella tries them, not from this
/// constructor — stated because a reader would otherwise assume the
/// type checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalName(String);

impl LogicalName {
    /// The name as the caller minted it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for LogicalName {
    type Error = DomainError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        // Blank, not merely empty: `SourceRef::new` rejected
        // `locator.trim().is_empty()` before this type existed, and a
        // name made of spaces is the same nothing an empty one is. The
        // sink is the last type to refuse a string, so relaxing it here
        // would quietly widen what the whole umbrella accepts.
        if raw.trim().is_empty() {
            return Err(DomainError::Validation(
                "LogicalName must not be blank".into(),
            ));
        }
        Ok(Self(raw.to_string()))
    }
}

/// Where an artefact's bytes are. Says nothing about what it is, and
/// nothing about which asset it belongs to.
///
/// Two of the four are local ([`File`](Self::File),
/// [`Record`](Self::Record)), one is remote, one is neither.
/// [`is_local`](Self::is_local) is the question most consumers actually
/// ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceLocator {
    /// A file on this machine, addressed whole.
    File(LocalPath),
    /// One record inside a container file on this machine.
    Record(ContainerRecord),
    /// An artefact a scheme addresses elsewhere.
    Remote(RemoteRef),
    /// A caller-minted name for something that never had bytes.
    Logical(LogicalName),
}

impl From<LocalPath> for SourceLocator {
    fn from(value: LocalPath) -> Self {
        Self::File(value)
    }
}

impl From<ContainerRecord> for SourceLocator {
    fn from(value: ContainerRecord) -> Self {
        Self::Record(value)
    }
}

impl From<RemoteRef> for SourceLocator {
    fn from(value: RemoteRef) -> Self {
        Self::Remote(value)
    }
}

impl From<LogicalName> for SourceLocator {
    fn from(value: LogicalName) -> Self {
        Self::Logical(value)
    }
}

impl SourceLocator {
    /// The path to open, when there is one: [`File`](Self::File) only.
    ///
    /// A [`Record`](Self::Record) has no file of its own — a caller
    /// that wants the container matches the variant and asks
    /// [`ContainerRecord::container`], which is a different question
    /// (opening the container on a record's behalf is how a
    /// thousand-message log becomes a thousand identical fingerprints).
    pub fn local_path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path.as_path()),
            Self::Record(_) | Self::Remote(_) | Self::Logical(_) => None,
        }
    }

    /// [`File`](Self::File) or [`Record`](Self::Record) — the bytes are
    /// on this machine, whether or not this locator addresses them
    /// whole.
    pub fn is_local(&self) -> bool {
        match self {
            Self::File(_) | Self::Record(_) => true,
            Self::Remote(_) | Self::Logical(_) => false,
        }
    }

    /// The rendering meant for a person: a basename label on a card, a
    /// tooltip, a clipboard copy, the identity quoted back in an error.
    ///
    /// Deliberately **not** [`to_storage`](Self::to_storage), and the
    /// distinction is the reason the wire type stops being coupled to
    /// the storage encoding. A [`Record`](Self::Record) renders as its
    /// container, because `session.jsonl` is the thing a person
    /// recognises and `session.jsonl#0198c1c2-…` is a key. Nothing
    /// round-trips this value — a caller that needs to re-read a
    /// locator holds the locator.
    pub fn to_display(&self) -> String {
        match self {
            Self::File(path) => path.as_str().to_string(),
            Self::Record(record) => record.container().as_str().to_string(),
            Self::Remote(remote) => {
                format!(
                    "{}://{}",
                    remote.scheme().as_str(),
                    remote.target().as_str()
                )
            }
            Self::Logical(name) => name.as_str().to_string(),
        }
    }

    /// The storage rendering: the tagged object described in the module
    /// docstring, and the exact inverse of [`try_from`](Self::try_from).
    ///
    /// Canonical by construction. The field order is the declaration
    /// order of [`Stored`], the whitespace is `serde_json`'s compact
    /// default, and no field is optional — so two equal locators render
    /// to byte-identical strings, which is what the equality lookup on
    /// the column rests on.
    pub fn to_storage(&self) -> String {
        // Infallible in fact rather than by assumption: every field of
        // `Stored` is a `String`, and `serde_json` fails only on a
        // non-string map key or a non-finite float, neither of which
        // this shape can produce. Panicking beats substituting a
        // default — this value goes straight into a NOT NULL TEXT
        // column, and a silently degraded rendering would write a row
        // that cannot be read back.
        serde_json::to_string(&Stored::from(self))
            .expect("a locator is four strings; rendering them as JSON cannot fail")
    }

    /// Reads a locator spelled the way a **caller** spells it, as
    /// against the way the column holds it.
    ///
    /// This is the ingest boundary — `AddAssetCommand.locator`, and
    /// through it `FootprintSource::locator`, whose SDK contract is a
    /// path (or `<container>#<record>` for a source that addresses
    /// records inside a container file). It is the **only** live code
    /// that reads a `#`, and the order below is what keeps the readings
    /// apart:
    ///
    /// - a `<scheme>://` prefix whose scheme is longer than one
    ///   character is a [`Remote`](Self::Remote) — **checked before the
    ///   `#` split**, since a URL may carry a fragment of its own and
    ///   splitting `https://h/a#b` into a container and a record would
    ///   be reading someone else's syntax as ours;
    /// - `file:` is the one scheme that is **consumed rather than
    ///   recorded**: it is a spelling of a local path, not a fact about
    ///   the artefact, so `file:///pics/a.png` and `/pics/a.png` are one
    ///   locator and compare equal. What is left after the strip goes
    ///   through the remaining rules, which is why `file://pics/a.png`
    ///   lands as a [`Logical`](Self::Logical) — the strip leaves a
    ///   rootless path, and a rootless path is not openable by the
    ///   process that reads it;
    /// - a `#` with both halves non-empty and a container satisfying
    ///   [`LocalPath`] is a [`Record`](Self::Record). A rootless
    ///   container is therefore *not* a record: `ContainerRecord` cannot
    ///   be built without a `LocalPath`, and the honest answer for
    ///   `pics/a.jsonl#uuid` is a name, not a location;
    /// - a rooted path is a [`File`](Self::File);
    /// - anything else is a [`Logical`](Self::Logical).
    ///
    /// The ambiguity that order carries is the delimiter's and is
    /// unresolvable *here*: `/pics/a#b.png` reads as the record `b.png`
    /// inside `/pics/a`, and no rule over a string can tell that from
    /// the file it may well be. It survives only on this side, because
    /// what is stored is the tag, and a caller holding the pieces avoids
    /// it entirely (`LocalPath::try_from(path_buf)?.into()`).
    ///
    /// Fails only when nothing will take the string, which after
    /// [`LogicalName`] means only the blank one — the invariant
    /// `SourceRef::new` enforces, and the one thing a NOT NULL TEXT
    /// column should never hold.
    pub fn from_wire(raw: &str) -> Result<Self, DomainError> {
        if raw.trim().is_empty() {
            return Err(DomainError::Validation(
                "source locator must not be blank".into(),
            ));
        }

        // Scheme first: a URL's own fragment is not our delimiter.
        let body = match raw.split_once("://") {
            Some((scheme, rest)) if Scheme::is_valid(scheme) => {
                if scheme.eq_ignore_ascii_case(Scheme::FILE) {
                    // Consumed, never recorded. The remainder is read
                    // as if it had been written without the scheme.
                    rest
                } else if let Ok(target) = RemoteTarget::try_from(rest) {
                    // `is_valid` already refused `file`, so this cannot
                    // fail on the scheme half.
                    let scheme = Scheme::try_from(scheme)?;
                    return Ok(RemoteRef::new(scheme, target).into());
                } else {
                    // `<scheme>://` with nothing after it addresses
                    // nothing; it is a name like any other.
                    raw
                }
            }
            _ => raw,
        };

        // The container/record split, on the *last* `#`, which is what
        // the reader this replaced did (`source_text::split_locator`,
        // deleted with the consumer migration) and what keeps a
        // container path containing `#` readable.
        if let Some((container, record)) = body.rsplit_once('#')
            && let Ok(container) = LocalPath::try_from(container)
            && let Ok(record) = RecordAddress::try_from(record)
        {
            return Ok(ContainerRecord::new(container, record).into());
        }

        if let Ok(path) = LocalPath::try_from(body) {
            return Ok(path.into());
        }

        // The sink. `raw`, not `body`: a consumed `file://` that left
        // nothing openable behind is still the string the caller sent,
        // and keeping it whole is what makes the value say where it came
        // from.
        Ok(LogicalName::try_from(raw)?.into())
    }
}

/// The stored shape, and the only place the encoding is written down.
///
/// Private on purpose. Deriving `Serialize` on [`SourceLocator`] itself
/// would put the tagged form one `serde_json::to_string` away from any
/// wire type that happens to carry a locator, and the wire carries a
/// *display* rendering by design (`AssetCardDto::source_locator`). The
/// column is reached through [`SourceLocator::to_storage`] and nothing
/// else.
///
/// Internally tagged, so the discriminant is a field of the object
/// rather than a wrapper around it: `{"kind":"file","path":"…"}`, not
/// `{"file":{"path":"…"}}`. `serde` emits the tag first and the rest in
/// declaration order, which is the canonicality the lookup depends on.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Stored<'a> {
    File {
        path: std::borrow::Cow<'a, str>,
    },
    Record {
        container: std::borrow::Cow<'a, str>,
        record: std::borrow::Cow<'a, str>,
    },
    Remote {
        scheme: std::borrow::Cow<'a, str>,
        target: std::borrow::Cow<'a, str>,
    },
    Logical {
        name: std::borrow::Cow<'a, str>,
    },
}

impl<'a> From<&'a SourceLocator> for Stored<'a> {
    fn from(locator: &'a SourceLocator) -> Self {
        use std::borrow::Cow::Borrowed;
        match locator {
            SourceLocator::File(path) => Self::File {
                path: Borrowed(path.as_str()),
            },
            SourceLocator::Record(record) => Self::Record {
                container: Borrowed(record.container().as_str()),
                record: Borrowed(record.record().as_str()),
            },
            SourceLocator::Remote(remote) => Self::Remote {
                scheme: Borrowed(remote.scheme().as_str()),
                target: Borrowed(remote.target().as_str()),
            },
            SourceLocator::Logical(name) => Self::Logical {
                name: Borrowed(name.as_str()),
            },
        }
    }
}

impl TryFrom<&str> for SourceLocator {
    type Error = DomainError;

    /// Reads the storage rendering — the tagged object, and nothing
    /// else.
    ///
    /// Two failures, and they are different faults:
    ///
    /// - the string is not the tagged form at all (an empty column, a
    ///   locator written before the rewrite migration, something a hand
    ///   put there). There is no fallback guess: reading a bare
    ///   `/pics/a.png` here would quietly re-admit the encoding the tag
    ///   replaced, and a value the column should not hold is worth an
    ///   error rather than a reading.
    /// - the tag is understood and a field is not admissible — a `file`
    ///   with a rootless path, a `remote` whose scheme is one character
    ///   or is `file`, an empty `record`. The tag says which type to
    ///   build; that type still says what it will accept, which is why
    ///   this function has no rules of its own.
    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        let stored: Stored<'_> = serde_json::from_str(raw)
            .map_err(|e| DomainError::Validation(format!("not a stored locator: {raw:?} ({e})")))?;
        Ok(match stored {
            Stored::File { path } => LocalPath::try_from(path.as_ref())?.into(),
            Stored::Record { container, record } => ContainerRecord::new(
                LocalPath::try_from(container.as_ref())?,
                RecordAddress::try_from(record.as_ref())?,
            )
            .into(),
            Stored::Remote { scheme, target } => RemoteRef::new(
                Scheme::try_from(scheme.as_ref())?,
                RemoteTarget::try_from(target.as_ref())?,
            )
            .into(),
            Stored::Logical { name } => LogicalName::try_from(name.as_ref())?.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The **wire** spelling — what an importer sends. Every fixture in
    /// this module that is written as a bare path, a `#` pair or a URL
    /// goes through here, because that is the only reader those
    /// spellings have; the storage form has its own tests below.
    fn parse(raw: &str) -> SourceLocator {
        SourceLocator::from_wire(raw).expect("locator")
    }

    #[test]
    fn the_file_scheme_is_consumed_so_two_spellings_are_one_locator() {
        let spelled = parse("file:///pics/a.png");
        let bare = parse("/pics/a.png");
        // The whole reason the scheme is consumed: these must compare
        // equal, and they only can if it is gone by the time the value
        // exists.
        assert_eq!(spelled, bare);
        assert_eq!(
            spelled,
            SourceLocator::File(LocalPath::try_from("/pics/a.png").unwrap())
        );
        // And the path that reaches `File::open` is a path, not a
        // spelling — the defect on the hashing job's side.
        assert_eq!(spelled.local_path(), Some(Path::new("/pics/a.png")));
    }

    #[test]
    fn no_scheme_but_file_is_consumed_every_other_one_is_kept() {
        // `file` cannot be built at all, so no `Remote` can carry it.
        assert!(Scheme::try_from("file").is_err());
        assert!(Scheme::try_from("FILE").is_err());
        // The neighbouring scheme is kept, which is what makes the
        // refusal above a rule about `file` and not about schemes.
        let kept = Scheme::try_from("files").expect("a different scheme");
        assert_eq!(kept.as_str(), "files");
    }

    #[test]
    fn a_consumed_scheme_leaving_a_rootless_path_is_a_name() {
        // `file://pics/a.png` strips to `pics/a.png`, which no process
        // that reads it can open.
        let locator = parse("file://pics/a.png");
        assert_eq!(
            locator,
            SourceLocator::Logical(LogicalName::try_from("file://pics/a.png").unwrap())
        );
        assert!(!locator.is_local());
        assert_eq!(locator.local_path(), None);
        // …and it is not the same locator as the rooted spelling, which
        // is the pair the previous test asserts *are* the same.
        assert_ne!(locator, parse("file:///pics/a.png"));
    }

    #[test]
    fn a_one_character_scheme_is_a_windows_drive_not_a_scheme() {
        // `C:/pics/a.png` never reaches the scheme test — it has no
        // `://` — so on its own it would assert nothing about the
        // departure from RFC 3986. `C://pics/a.png` does reach it, and
        // is the fixture where the two readings disagree.
        for raw in ["C:/pics/a.png", r"C:\pics\a.png", "C://pics/a.png"] {
            let locator = parse(raw);
            assert!(
                matches!(locator, SourceLocator::File(_)),
                "{raw} is a Windows-spelled path, not a remote"
            );
            assert!(locator.is_local(), "{raw}");
        }
        // The rule is a length rule, not a "single letters are not
        // schemes on Windows" rule: two characters is a scheme.
        assert!(matches!(
            parse("s3://bucket/a.png"),
            SourceLocator::Remote(_)
        ));
        assert!(Scheme::try_from("c").is_err());
    }

    #[test]
    fn a_url_fragment_belongs_to_the_url_not_to_us() {
        let locator = parse("https://h/a#b");
        let SourceLocator::Remote(remote) = &locator else {
            panic!("expected a remote, got {locator:?}");
        };
        assert_eq!(remote.scheme().as_str(), Scheme::HTTPS);
        // The `#` survives inside the target: the scheme is checked
        // before the split, so this never becomes a container plus a
        // record.
        assert_eq!(remote.target().as_str(), "h/a#b");
        assert!(!locator.is_local());
    }

    #[test]
    fn a_record_needs_a_container_that_can_be_opened() {
        let locator = parse("/logs/s.jsonl#0198c1c2-aaaa-bbbb");
        let SourceLocator::Record(record) = &locator else {
            panic!("expected a record, got {locator:?}");
        };
        assert_eq!(record.container().as_path(), Path::new("/logs/s.jsonl"));
        assert_eq!(record.record().as_str(), "0198c1c2-aaaa-bbbb");
        // Local — the bytes are on this machine — but with no file of
        // its own to open.
        assert!(locator.is_local());
        assert_eq!(locator.local_path(), None);

        // The same shape with a rootless container is a name, because
        // `ContainerRecord` cannot be built without a `LocalPath`. This
        // is the pair that makes the container rule non-vacuous: only
        // the root differs.
        let rootless = parse("pics/a.jsonl#0198c1c2-aaaa-bbbb");
        assert_eq!(
            rootless,
            SourceLocator::Logical(
                LogicalName::try_from("pics/a.jsonl#0198c1c2-aaaa-bbbb").unwrap()
            )
        );
        assert!(!rootless.is_local());
    }

    #[test]
    fn a_container_path_containing_a_hash_still_reads_as_a_record() {
        // The split is `rsplit`, so the container keeps its own `#`.
        let locator = parse("/pics/a#b.png#note-1");
        let SourceLocator::Record(record) = &locator else {
            panic!("expected a record, got {locator:?}");
        };
        assert_eq!(record.container().as_path(), Path::new("/pics/a#b.png"));
        assert_eq!(record.record().as_str(), "note-1");
    }

    #[test]
    fn a_hash_with_an_empty_half_is_not_a_record() {
        // Both halves must be non-empty; neither of these is a record,
        // and they differ from the record fixture only in that.
        assert!(matches!(parse("/logs/s.jsonl#"), SourceLocator::File(_)));
        assert!(matches!(parse("#0198c1c2"), SourceLocator::Logical(_)));
    }

    #[test]
    fn a_caller_minted_name_is_the_sink() {
        for raw in [
            "chat/test-chat-0198c1c2/msg-1",
            "opaque-locator",
            "./pics/a.png",
        ] {
            let locator = parse(raw);
            assert_eq!(
                locator,
                SourceLocator::Logical(LogicalName::try_from(raw).unwrap()),
                "{raw}"
            );
            assert!(!locator.is_local(), "{raw}");
        }
    }

    #[test]
    fn a_blank_string_is_the_one_refusal() {
        // Blank, not just empty: the constructor this replaced rejected
        // `locator.trim().is_empty()`, and widening that to accept a
        // locator made of spaces would be a loosening no one asked for.
        for raw in ["", " ", "   ", "\t", "\n"] {
            assert!(
                SourceLocator::from_wire(raw).is_err(),
                "{raw:?} is blank and must be refused"
            );
        }
        // Everything else lands somewhere — including the shapes that
        // look malformed. These differ from the blanks above only in
        // carrying a non-space character, which is the whole rule.
        for raw in ["#", "://", "https://", "file://", " x "] {
            assert!(
                SourceLocator::from_wire(raw).is_ok(),
                "{raw:?} should have landed as a logical name"
            );
        }
    }

    #[test]
    fn every_variant_survives_a_round_trip_through_storage() {
        for raw in [
            "/pics/a.png",
            "/pics/a#b.png#note-1",
            "/logs/s.jsonl#0198c1c2-aaaa",
            "hf://org/model/f.safetensors",
            "https://h/a#b",
            "chat/0198c1c2/msg-1",
            "file://",
        ] {
            let locator = parse(raw);
            let rendered = locator.to_storage();
            assert_eq!(
                SourceLocator::try_from(rendered.as_str()).expect("re-read"),
                locator,
                "{raw} rendered as {rendered}"
            );
        }
    }

    /// The encoding, written out. Asserted against literal text rather
    /// than against a re-read, because a round trip is satisfied by any
    /// self-consistent rendering — including one whose field order moved
    /// under a refactor, which is exactly the change the lookup cannot
    /// survive.
    #[test]
    fn the_storage_form_is_the_tagged_object() {
        assert_eq!(
            parse("/pics/a.png").to_storage(),
            r#"{"kind":"file","path":"/pics/a.png"}"#
        );
        assert_eq!(
            parse("/logs/s.jsonl#0198c1c2").to_storage(),
            r#"{"kind":"record","container":"/logs/s.jsonl","record":"0198c1c2"}"#
        );
        assert_eq!(
            parse("hf://org/model/f.safetensors").to_storage(),
            r#"{"kind":"remote","scheme":"hf","target":"org/model/f.safetensors"}"#
        );
        assert_eq!(
            parse("chat/0198c1c2/msg-1").to_storage(),
            r#"{"kind":"logical","name":"chat/0198c1c2/msg-1"}"#
        );
    }

    /// The property the `(persona_id, source_kind, source_locator)`
    /// lookup rests on: equal locators are equal **bytes** in the
    /// column, whatever spelling they arrived in.
    ///
    /// The fixtures are pairs that disagree as strings and agree as
    /// values, which is what makes this non-vacuous — rendering the
    /// input back verbatim would pass a same-spelling test and fail
    /// every line here.
    #[test]
    fn equal_locators_render_to_byte_identical_text() {
        for (left, right) in [
            // The consumed scheme.
            ("file:///pics/a.png", "/pics/a.png"),
            // The lowercased one.
            ("HF://org/m/f.safetensors", "hf://org/m/f.safetensors"),
        ] {
            let (left, right) = (parse(left), parse(right));
            assert_eq!(left, right, "the values agree");
            assert_eq!(
                left.to_storage(),
                right.to_storage(),
                "…so the column must hold one string for them"
            );
        }
        // And the producer-side path reaches the same bytes as the
        // spelled one, since an importer holding a `PathBuf` never goes
        // through the wire reader at all.
        let built: SourceLocator = LocalPath::try_from(PathBuf::from("/pics/a.png"))
            .expect("absolute")
            .into();
        assert_eq!(built.to_storage(), parse("file:///pics/a.png").to_storage());
    }

    /// The storage reader takes the tagged form and **only** that: it
    /// re-runs each type's invariant, and it does not fall back to the
    /// spelling `from_wire` reads.
    #[test]
    fn the_storage_reader_validates_the_fields_and_guesses_nothing() {
        // A tag whose field the type refuses.
        for raw in [
            // rootless path
            r#"{"kind":"file","path":"pics/a.png"}"#,
            // rootless container
            r#"{"kind":"record","container":"logs/s.jsonl","record":"x"}"#,
            // empty record address
            r#"{"kind":"record","container":"/logs/s.jsonl","record":""}"#,
            // one-character scheme — the Windows-drive departure
            r#"{"kind":"remote","scheme":"c","target":"/pics/a.png"}"#,
            // `file` is never a Scheme, whatever the column says
            r#"{"kind":"remote","scheme":"file","target":"/pics/a.png"}"#,
            // blank name
            r#"{"kind":"logical","name":"  "}"#,
        ] {
            assert!(
                SourceLocator::try_from(raw).is_err(),
                "{raw} names a value its own type refuses"
            );
        }
        // Not the tagged form at all — including the legacy spellings,
        // which must not be quietly re-admitted here. A stored row in
        // that shape predates the rewrite migration, and an error is the
        // honest answer.
        for raw in [
            "",
            "/pics/a.png",
            "/logs/s.jsonl#0198c1c2",
            "hf://org/m/f.safetensors",
            r#"{"kind":"folder","path":"/pics"}"#,
            r#"{"path":"/pics/a.png"}"#,
        ] {
            assert!(
                SourceLocator::try_from(raw).is_err(),
                "{raw:?} is not the storage form and must not be read as one"
            );
        }
    }

    /// The two characters the delimiter reserved are ordinary now, in
    /// both halves of a record — the ambiguity is removed rather than
    /// managed.
    #[test]
    fn a_hash_is_an_ordinary_character_on_both_sides_of_the_boundary() {
        let record: SourceLocator = ContainerRecord::new(
            LocalPath::try_from("/pics/a#b.png").expect("absolute"),
            RecordAddress::try_from("note#1").expect("non-empty"),
        )
        .into();
        // The address carrying a `#` survives the round trip, which it
        // could not under a delimiter at any split direction.
        assert_eq!(
            SourceLocator::try_from(record.to_storage().as_str()).expect("re-read"),
            record
        );
        // And a file whose *name* carries one is a file, with nothing
        // looking at the character.
        let file: SourceLocator = LocalPath::try_from(PathBuf::from("/pics/a#b.png"))
            .expect("absolute")
            .into();
        assert_eq!(
            SourceLocator::try_from(file.to_storage().as_str()).expect("re-read"),
            file
        );
        assert_ne!(file, record);
    }

    #[test]
    fn a_producer_holding_the_pieces_never_goes_through_a_string() {
        let locator: SourceLocator = LocalPath::try_from(PathBuf::from("/pics/a.png"))
            .expect("absolute")
            .into();
        assert_eq!(locator, parse("/pics/a.png"));

        let record: SourceLocator = ContainerRecord::new(
            LocalPath::try_from("/logs/s.jsonl").expect("absolute"),
            RecordAddress::try_from("0198c1c2").expect("non-empty"),
        )
        .into();
        assert_eq!(record, parse("/logs/s.jsonl#0198c1c2"));

        let remote: SourceLocator = RemoteRef::new(
            Scheme::try_from(Scheme::S3).expect("scheme"),
            RemoteTarget::try_from("bucket/a.png").expect("non-empty"),
        )
        .into();
        assert_eq!(remote, parse("s3://bucket/a.png"));

        // …and the rootless path a producer might hold is refused at
        // construction, not stored and discovered later.
        assert!(LocalPath::try_from(PathBuf::from("pics/a.png")).is_err());
        assert!(RecordAddress::try_from("").is_err());
        assert!(RemoteTarget::try_from("").is_err());
        assert!(LogicalName::try_from("").is_err());
    }
}
