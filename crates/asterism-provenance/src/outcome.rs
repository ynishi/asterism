//! What applying a record to a file actually achieved.
//!
//! Here rather than beside the writer that produces it, because two
//! layers need to name it and neither may depend on the other: the port
//! is declared in `asterism-core` (ports live in the core; adapters
//! never define traits) and the implementation lives in
//! `asterism-infra`. This crate is the one both can already see.

/// The result of writing a [`DisclosureRecord`](crate::DisclosureRecord)
/// into a file.
///
/// Every field is an outcome rather than an intention. The two halves
/// fail independently — a container may take an XMP packet and no
/// manifest, or a manifest and no packet — so a caller that recorded
/// "provenance applied" would be recording something no particular file
/// necessarily has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamped {
    /// An XMP packet was written.
    pub xmp_written: bool,
    /// A signed C2PA manifest was written.
    pub manifest_written: bool,
    /// The prompt was dropped to fit the packet into a JPEG segment.
    ///
    /// Reported because it cannot be recovered afterwards: a file whose
    /// prompt did not fit and a file that never had one are
    /// indistinguishable once written, so the difference has to leave
    /// the call that made it.
    pub prompt_dropped: bool,
}

impl Stamped {
    /// Whether the file carries a machine-readable mark of any kind.
    ///
    /// The question the obligation asks, and deliberately not "did
    /// everything succeed". One mark is a disclosure; reporting a
    /// partial application as a failure would push a caller towards
    /// treating a marked file as unmarked.
    pub fn discloses(&self) -> bool {
        self.xmp_written || self.manifest_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_mark_on_its_own_is_a_disclosure() {
        assert!(!Stamped::default().discloses());
        assert!(
            Stamped {
                xmp_written: true,
                ..Stamped::default()
            }
            .discloses()
        );
        assert!(
            Stamped {
                manifest_written: true,
                ..Stamped::default()
            }
            .discloses()
        );
    }
}
