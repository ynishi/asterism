//! What a fresh reading of a material's chapter list means for the
//! rows — the machine half of the layer model.
//!
//! A container declares how its content is divided, and that
//! declaration can change: the file is replaced, a better parser lands,
//! a codec's chapters were unreadable last time. So "read the chapters
//! again" has to be an operation that can run any number of times and
//! leave the same state, without touching anything a person wrote.
//! [`replace_imported_chapters`] is that operation.
//!
//! Functions rather than a service, for the reason
//! [`duplicate_detection`](crate::application_support::duplicate_detection)
//! gives beside it: the job handler that will drive this already holds
//! both ports it names, so a struct would add a handle without adding a
//! decision. It lives here rather than in
//! [`MaterialLayerService`](crate::application::MaterialLayerService)
//! because only a job drives it — the split this module doc states.
//!
//! # Why replacement, not merge
//!
//! A merge needs an identity for "the same chapter across two
//! readings", and containers do not offer one: MP4's `chpl` numbers its
//! entries by position, Matroska's `ChapterAtom` carries a UID that is
//! only unique within the file it came from, and a re-encode changes
//! both. Matching on `(start_ms, label)` would treat a shifted timestamp
//! as a new chapter and a re-titled one as a deletion — so a merge
//! would be a guess presented as a reconciliation. Replacement makes
//! the imported band exactly what the file says, which is the only
//! claim it was ever making.
//!
//! The person's own band is untouched by construction: it is a
//! different row of `material_layer`, and this function names the
//! imported one.

use crate::application::material_layer_service::imported_structure_layer;
use crate::domain::chapter_mark::ChapterMark;
use crate::domain::material_layer::MaterialLayer;
use crate::domain::material_mark::TimelineSpan;
use crate::domain::repository::{ChapterMarkRepository, MaterialLayerRepository};
use crate::domain::value::AssetId;
use crate::error::DomainError;

/// One section as a probe read it out of the material.
///
/// The parser's output shape, deliberately not [`ChapterMark`]: a
/// reading has no ids in it, and minting them in the parser would mean
/// every re-read produced rows that look new. `ord` is the position in
/// the container's own list, which is what a reader sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedChapter {
    /// Where the section sits on the playback timeline. An instant
    /// means the container declared a start and no end (MP4 `chpl`).
    pub span: TimelineSpan,
    /// The title as declared — empty when the container names none.
    pub label: String,
}

/// Makes the imported chapter band of `(asset_id, material_ord)` say
/// exactly what `scanned` says, and returns the band.
///
/// Creates the band on the first reading (and makes it the default when
/// nothing else holds that flag — see
/// [`imported_structure_layer`]). Re-running with the same input is a
/// no-op in effect: the rows are replaced by equal ones, under fresh
/// ids.
///
/// An empty `scanned` is meaningful and is applied: a file that used to
/// declare chapters and no longer does ends with an empty imported
/// band, not with the previous reading left standing. The band itself
/// stays, because "this material was scanned and declares nothing" and
/// "this material has never been scanned" are different states and only
/// the row can tell them apart.
///
/// Ids are minted fresh on every run. A surface holding a chapter id
/// across a re-probe therefore finds it gone — which is the honest
/// answer, since the row it named was a statement about a file that has
/// since been read differently.
pub async fn replace_imported_chapters(
    layers: &dyn MaterialLayerRepository,
    chapters: &dyn ChapterMarkRepository,
    asset_id: &AssetId,
    material_ord: u32,
    scanned: &[ScannedChapter],
) -> Result<MaterialLayer, DomainError> {
    let layer = imported_structure_layer(layers, asset_id, material_ord).await?;
    let rows: Vec<ChapterMark> = scanned
        .iter()
        .enumerate()
        .map(|(index, section)| {
            // The container's own order is the reading order, so `ord`
            // is the index in the list handed over rather than a rank
            // computed from `start_ms`: a container is free to declare
            // its sections out of timeline order, and a re-sort here
            // would be this layer inventing a division the file did not
            // make.
            let ord = u32::try_from(index).map_err(|_| {
                DomainError::Validation(format!(
                    "a material declaring more than {} chapters is not a chapter list",
                    u32::MAX
                ))
            })?;
            ChapterMark::new(layer.id, section.span, section.label.clone(), ord)
        })
        .collect::<Result<_, _>>()?;
    chapters.replace_layer_content(&layer.id, &rows).await?;
    Ok(layer)
}
