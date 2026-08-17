//! Repository adapters — SQLite implementations of the ports declared in
//! `asterism-core`.
//!
//! Conventions shared by every adapter:
//!
//! - Each adapter holds a cloneable writer `AsyncIsle` handle and issues
//!   queries through `isle.call`. Activating a WAL reader pool is deferred
//!   until read contention is measurable.
//! - Only `rusqlite` primitives are handled inside an isle closure;
//!   promotion into domain types happens outside (see the convention in
//!   [`crate::sqlite::map`]).
//! - Visibility filtering (for restricted assets) is always applied
//!   inside SQL by the asset adapter.

pub mod app_setting;
pub mod asset;
pub mod asset_body;
pub mod asset_comment;
pub mod asset_text_index;
pub mod attribution_guard;
pub mod chapter_mark;
pub mod dir;
pub mod dispatch;
pub mod edge;
pub mod group;
pub mod instance;
pub mod material_layer;
pub mod material_mark;
pub mod modality;
pub mod persona;
pub mod persona_profile;
pub mod persona_theme;
pub mod project;
pub mod pursuit;
pub mod query_group;
pub mod series;
pub mod session;
pub mod snapshot;
pub mod tag;
pub mod thread;
pub mod thumb;

pub use app_setting::SqliteAppSettingRepository;
pub use asset::SqliteAssetRepository;
pub use asset_body::SqliteAssetBodyRepository;
pub use asset_comment::SqliteAssetCommentRepository;
pub use asset_text_index::SqliteAssetTextIndex;
pub use chapter_mark::SqliteChapterMarkRepository;
pub use dir::SqliteDirRepository;
pub use dispatch::SqliteDispatchRepository;
pub use edge::SqliteEdgeRepository;
pub use instance::SqliteInstanceRepository;
pub use material_layer::SqliteMaterialLayerRepository;
pub use material_mark::SqliteMaterialMarkRepository;
pub use modality::SqliteModalityRepository;
pub use persona::SqlitePersonaRepository;
pub use persona_profile::SqlitePersonaProfileRepository;
pub use persona_theme::SqlitePersonaThemeRepository;
pub use project::SqliteProjectRepository;
pub use pursuit::SqlitePursuitRepository;
pub use query_group::SqliteQueryGroupRepository;
pub use series::SqliteSeriesRepository;
pub use session::SqliteSessionRepository;
pub use snapshot::SqliteSnapshotRepository;
pub use tag::SqliteTagRepository;
pub use thread::SqliteThreadRepository;
pub use thumb::SqliteThumbRepository;
