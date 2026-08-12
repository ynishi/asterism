//! `MaterialLayer` — one band of marks over an Asset's material, and
//! the thing that says **where a mark came from**.
//!
//! A material can be marked more than once by more than one hand. The
//! container itself declares chapters; a person disagrees with them and
//! writes their own; a job derives a third set from the audio. Before
//! this type those three were the same rows, distinguishable only by
//! whoever happened to have written them last — so "re-read the file"
//! either destroyed a person's work or duplicated the file's, and there
//! was no answer to "which of these did I write?".
//!
//! The layer carries that answer as data. [`LayerOrigin`] says who
//! produced the band, [`LayerRole`] says what kind of thing it holds,
//! and marks belong to a layer rather than directly to the asset:
//! [`ChapterMark`](crate::domain::chapter_mark) hangs off a `Structure`
//! layer, [`MaterialMark`](crate::domain::material_mark) off an
//! `Annotation` one.
//!
//! Design notes:
//!
//! - **The layer is addressed through the asset, not the material.**
//!   `(asset_id, material_ord)` names the original the band is over,
//!   the same pair
//!   [`Material`](crate::domain::material) is identified by, and the
//!   same one `MaterialMark` resolves to `ord == 0` by convention.
//!   Materials are aggregate-internal, so there is no material id to
//!   reference and this is the shape the aggregate offers.
//! - **`Imported` is immutable, and that rule is not here.** Whether a
//!   caller may write into a band depends on which caller it is — a
//!   person editing, or the re-probe job replacing the file's own
//!   declaration wholesale — and the entity cannot see which. The
//!   application layer holds it
//!   ([`material_layer_service`](crate::application::material_layer_service)),
//!   in the one place both routes pass through.
//! - **"The default band" is a cross-row fact**, so it is not enforced
//!   here. At most one layer per `(asset, material_ord, role)` carries
//!   `is_default`, and that is a partial unique index in the schema (see
//!   the V78 doc comment in `migrations.rs`): a rule about *other rows*
//!   cannot be checked by a value holding one of them, and a check that
//!   read them would be a race between its read and its write.
//!   [`Self::validate`] carries the half that is self-contained.
//! - **No display name.** A band is described by what it *is* —
//!   `(origin, role)` — and a surface renders that pair. Storing a
//!   caption as well would make "the imported layer" and whatever the
//!   caption says two answers to one question, and the caption would be
//!   the one that drifts.

use crate::domain::value::{AssetId, MaterialLayerId};
use crate::error::DomainError;

/// The `material_ord` of an asset's primary original — the axis
/// `asset.duration_ms` measures and `HTMLMediaElement.currentTime`
/// reports.
///
/// Named rather than written as a bare `0` at each call site: the
/// surfaces that mark a timeline
/// ([`MaterialMark`](crate::domain::material_mark) and everything above
/// it) address the primary original by convention, and a `0` in an
/// argument list next to another ordinal says nothing about which
/// convention it is.
pub const PRIMARY_MATERIAL_ORD: u32 = 0;

/// Who produced a layer's contents.
///
/// The axis that decides whether a person may edit the band. It is a
/// closed enum rather than an open slug (unlike
/// [`Modality`](crate::domain::value::Modality)) because each value
/// carries a *rule* — an imported band is replaced by re-reading the
/// file, a user band is edited by hand — and a value nobody has written
/// a rule for is a band nothing knows how to write into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerOrigin {
    /// Read out of the material itself: the chapter list the container
    /// declares, an embedded track. Replaced wholesale when the
    /// material is read again; never edited in place.
    Imported,
    /// Written by the person using Asterism. The only origin a hand
    /// edit may touch.
    User,
    /// Derived by a job from the material's content (a scene cut, a
    /// silence split). Machine-owned like `Imported`, but produced here
    /// rather than found in the file.
    Machine,
}

impl LayerOrigin {
    /// Every variant, in the order the `CHECK (origin IN (...))` in the
    /// schema lists them.
    ///
    /// Exists so the round trip below can be proved over the whole enum
    /// rather than over the three values someone remembered to write in
    /// a test.
    pub const ALL: &'static [Self] = &[Self::Imported, Self::User, Self::Machine];

    /// Slug stored in the `origin` column and used on the wire.
    ///
    /// One spelling, so the adapter, the contract layer and the DDL's
    /// `CHECK` cannot disagree about what a band's origin is called.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::User => "user",
            Self::Machine => "machine",
        }
    }

    /// Reads a slug back. `None` for anything this build has no variant
    /// for.
    ///
    /// Deliberately not `Result`: the two callers want different errors
    /// out of the same failure. A row holding an unknown slug is an
    /// infrastructure fact (`DomainError::Infra`); a command carrying
    /// one is a caller error (`DomainError::Validation`). Returning
    /// `Option` lets each say so in its own words instead of one of them
    /// restating the other's.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|o| o.slug() == slug)
    }
}

/// What kind of marks a layer holds.
///
/// Not a property of the marks themselves: the same timeline position
/// means a different thing in a chapter list ("this section starts
/// here") than in a note ("look at this"). The role is what the two
/// aggregates hang off, and what keeps a `set_default` on one from
/// moving the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerRole {
    /// A reading of how the material is *divided*: chapters. Holds
    /// [`ChapterMark`](crate::domain::chapter_mark).
    Structure,
    /// Notes fastened to positions in the material. Holds
    /// [`MaterialMark`](crate::domain::material_mark).
    Annotation,
}

impl LayerRole {
    /// Every variant, in the order the `CHECK (role IN (...))` in the
    /// schema lists them.
    pub const ALL: &'static [Self] = &[Self::Structure, Self::Annotation];

    /// Slug stored in the `role` column and used on the wire.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Structure => "structure",
            Self::Annotation => "annotation",
        }
    }

    /// Reads a slug back; `None` for anything this build has no variant
    /// for. Same two-caller reasoning as [`LayerOrigin::from_slug`].
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.slug() == slug)
    }
}

/// One band of marks over an Asset's material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialLayer {
    /// Surrogate id (UUID v7; layers are read in `ord` order, so the
    /// embedded timestamp only serves as a tie-break).
    pub id: MaterialLayerId,
    /// Asset whose material the band is over.
    pub asset_id: AssetId,
    /// Which of that asset's originals (`0` = the primary one, the axis
    /// `asset.duration_ms` measures).
    pub material_ord: u32,
    /// Who produced the contents.
    pub origin: LayerOrigin,
    /// What kind of marks it holds.
    pub role: LayerRole,
    /// Whether this is the band a surface shows, and the one a new mark
    /// lands in, when the caller names no other. At most one per
    /// `(asset_id, material_ord, role)` — enforced by the schema, not
    /// here (see the module doc).
    pub is_default: bool,
    /// Display order within `(asset_id, material_ord, role)`.
    pub ord: u32,
}

impl MaterialLayer {
    /// Opens a band over `asset_id`'s material.
    pub fn new(
        asset_id: AssetId,
        material_ord: u32,
        origin: LayerOrigin,
        role: LayerRole,
        is_default: bool,
        ord: u32,
    ) -> Result<Self, DomainError> {
        let layer = Self {
            id: MaterialLayerId::new(),
            asset_id,
            material_ord,
            origin,
            role,
            is_default,
            ord,
        };
        layer.validate()?;
        Ok(layer)
    }

    /// Rebuilds a layer read back from storage.
    ///
    /// Adapters route every row through here rather than assembling the
    /// struct field by field, for the reason
    /// [`MaterialMark::rehydrate`](crate::domain::material_mark::MaterialMark::rehydrate)
    /// gives: this is the read door, and it is the only thing between a
    /// row that arrived some other way — a hand-written `INSERT`, a
    /// migration, a build that had a variant this one does not — and a
    /// caller holding it as a valid layer.
    ///
    /// Failures are `Validation`; an adapter restates them as `Infra`,
    /// since a row like this being present is an infrastructure fact
    /// rather than a caller error.
    pub fn rehydrate(
        id: MaterialLayerId,
        asset_id: AssetId,
        material_ord: u32,
        origin: LayerOrigin,
        role: LayerRole,
        is_default: bool,
        ord: u32,
    ) -> Result<Self, DomainError> {
        let layer = Self {
            id,
            asset_id,
            material_ord,
            origin,
            role,
            is_default,
            ord,
        };
        layer.validate()?;
        Ok(layer)
    }

    /// Checks the one rule this value can hold on its own: **the
    /// default annotation band is the user's**.
    ///
    /// Posting a note names no layer — a person clicking a timeline is
    /// not choosing a band — so the service resolves the default
    /// annotation layer and writes into it, creating one if the asset
    /// has none
    /// ([`material_layer_service`](crate::application::material_layer_service)).
    /// If that band could be `Imported` or `Machine`, every note would
    /// land in a band the immutability rule forbids writing to: either
    /// the guard refuses the person's own note, or the write goes
    /// through and the next re-probe deletes it along with the file's
    /// content. Refusing the row is what keeps that state unreachable
    /// in the first place, and the schema mirrors it
    /// (`CHECK (role <> 'annotation' OR is_default = 0 OR origin =
    /// 'user')`, V78).
    ///
    /// Structure bands are deliberately not covered: the chapter list a
    /// reader wants by default is usually the one the file declares, so
    /// `(Imported, Structure, is_default)` is the *expected* row there.
    ///
    /// Uniqueness of the default is a rule about other rows and lives
    /// in the schema (module doc). Every door calls this — `new`,
    /// `rehydrate`, and an adapter's `save` — because the fields are
    /// public, so a record update reaches them without passing a
    /// constructor.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.role == LayerRole::Annotation && self.is_default && self.origin != LayerOrigin::User
        {
            return Err(DomainError::Validation(format!(
                "the default annotation layer is where a new mark lands, so it must be \
                 written by the user; origin {:?} cannot carry it",
                self.origin
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slugs round-trip, over the whole enum rather than over a
    /// hand-written list.
    ///
    /// `ALL` is what makes that true: a variant added without a slug
    /// fails to compile in `slug`, and one added without a place in
    /// `ALL` is caught by the count assertion — which is the failure a
    /// `for` loop over a literal list would miss.
    #[test]
    fn origin_and_role_slugs_round_trip() {
        assert_eq!(LayerOrigin::ALL.len(), 3, "a new origin belongs in ALL");
        for origin in LayerOrigin::ALL {
            assert_eq!(LayerOrigin::from_slug(origin.slug()), Some(*origin));
        }
        assert_eq!(LayerRole::ALL.len(), 2, "a new role belongs in ALL");
        for role in LayerRole::ALL {
            assert_eq!(LayerRole::from_slug(role.slug()), Some(*role));
        }

        // The literal spellings, because the schema's CHECK names them
        // and nothing else in Rust does.
        assert_eq!(LayerOrigin::Imported.slug(), "imported");
        assert_eq!(LayerOrigin::User.slug(), "user");
        assert_eq!(LayerOrigin::Machine.slug(), "machine");
        assert_eq!(LayerRole::Structure.slug(), "structure");
        assert_eq!(LayerRole::Annotation.slug(), "annotation");

        assert_eq!(LayerOrigin::from_slug("Imported"), None, "case matters");
        assert_eq!(LayerOrigin::from_slug("derived"), None);
        assert_eq!(LayerRole::from_slug(""), None);
    }

    /// The default annotation band has to be the user's — and the three
    /// neighbouring rows that stay legal are asserted beside it.
    ///
    /// Without them the rule would pass with `validate` replaced by
    /// `return Err(...)`: an imported *structure* default is the
    /// ordinary case (the chapter list the file declares), and a
    /// non-default imported annotation band is exactly what a re-probe
    /// writes.
    #[test]
    fn only_the_user_owns_the_default_annotation_band() {
        let asset = AssetId::new();
        let open =
            |origin, role, is_default| MaterialLayer::new(asset, 0, origin, role, is_default, 0);

        assert!(
            open(LayerOrigin::User, LayerRole::Annotation, true).is_ok(),
            "the band a new note lands in"
        );
        assert!(
            open(LayerOrigin::Imported, LayerRole::Structure, true).is_ok(),
            "the file's own chapter list is the expected default there"
        );
        assert!(
            open(LayerOrigin::Imported, LayerRole::Annotation, false).is_ok(),
            "an imported annotation band is fine as long as it is not the default"
        );

        for origin in [LayerOrigin::Imported, LayerOrigin::Machine] {
            let err = open(origin, LayerRole::Annotation, true)
                .expect_err("a note would land in a band nobody may write to");
            assert!(
                matches!(err, DomainError::Validation(_)),
                "expected Validation, got {err:?}"
            );
        }
    }

    /// `rehydrate` applies the same rule the constructor does.
    ///
    /// The fields are public, so the value a row is read into can be
    /// reached by a record update; the read door is what keeps a row
    /// written around the schema from arriving as a valid layer.
    #[test]
    fn rehydrate_rejects_rows_the_constructor_would_refuse() {
        let id = MaterialLayerId::new();
        let asset = AssetId::new();
        assert!(
            MaterialLayer::rehydrate(
                id,
                asset,
                0,
                LayerOrigin::Machine,
                LayerRole::Annotation,
                true,
                0
            )
            .is_err(),
            "a machine-owned default annotation band stays refused on the way back in"
        );
        let ok = MaterialLayer::rehydrate(
            id,
            asset,
            2,
            LayerOrigin::Machine,
            LayerRole::Annotation,
            false,
            7,
        )
        .expect("the same band, not the default, reads back");
        assert_eq!(ok.material_ord, 2);
        assert_eq!(ok.ord, 7);
        assert!(!ok.is_default);
    }
}
