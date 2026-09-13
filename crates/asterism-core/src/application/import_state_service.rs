//! Reading and writing where an importer got to.
//!
//! Thin on purpose, and the thinness is the design. Every judgement
//! about a resumption point is made in the importer, which is the only
//! place that can make one. This service is the other end of the wire:
//! it stores what it is given and hands back what it stored.
//!
//! Two things it deliberately does not do. It does not validate the
//! offset — see [`import_state`](crate::domain::import_state) for that
//! rule. And it does not check whether the persona exists: a write
//! arrives from an import that has been posting assets to that persona
//! all along, so the check would repeat one the asset path has already
//! made; and a read arrives before anything has been posted, where the
//! honest answer to an unknown persona is the same as to a known one
//! with nothing stored — nothing.

use std::sync::Arc;

use asterism_contract::command::{ReadImportStateCommand, WriteImportStateCommand};
use asterism_contract::dto::ImportStateDto;
use chrono::Utc;

use crate::domain::attribution::AttributionContext;
use crate::domain::import_state::{ImportState, ImportStateKey};
use crate::domain::repository::ImportStateRepository;
use crate::error::DomainError;

/// Where an importer got to, read and written.
pub struct ImportStateService {
    repo: Arc<dyn ImportStateRepository>,
}

impl ImportStateService {
    /// Wraps the persistence port.
    pub fn new(repo: Arc<dyn ImportStateRepository>) -> Self {
        Self { repo }
    }

    /// The point stored for one key, or `None` for a source nothing has
    /// imported yet.
    pub async fn read(
        &self,
        command: ReadImportStateCommand,
    ) -> Result<Option<ImportStateDto>, DomainError> {
        let key = ImportStateKey::new(command.persona_id, command.partition);
        Ok(self.repo.find(&key).await?.map(to_dto))
    }

    /// Stores a point, replacing whatever the key held.
    ///
    /// Takes an [`AttributionContext`] it does not persist, on the
    /// same terms as the settings service: the row has no room for a
    /// writer and none is being added, and the argument is required so
    /// that a write path cannot be added later without somebody having
    /// to decide what to say about who made it.
    pub async fn write(
        &self,
        command: WriteImportStateCommand,
        _by: &AttributionContext,
    ) -> Result<ImportStateDto, DomainError> {
        let state = ImportState {
            key: ImportStateKey::new(command.persona_id, command.partition),
            offset_json: command.offset_json,
            updated_at: Utc::now(),
        };
        self.repo.upsert(&state).await?;
        Ok(to_dto(state))
    }
}

/// The stored row as the wire carries it.
fn to_dto(state: ImportState) -> ImportStateDto {
    ImportStateDto {
        persona_id: state.key.persona_id,
        partition: state.key.partition,
        offset_json: state.offset_json,
        updated_at: state.updated_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use async_trait::async_trait;

    #[derive(Default)]
    struct MemoryRepo {
        rows: Mutex<Vec<ImportState>>,
    }

    #[async_trait]
    impl ImportStateRepository for MemoryRepo {
        async fn find(&self, key: &ImportStateKey) -> Result<Option<ImportState>, DomainError> {
            Ok(self
                .rows
                .lock()
                .expect("the fixture's rows")
                .iter()
                .find(|row| &row.key == key)
                .cloned())
        }

        async fn upsert(&self, state: &ImportState) -> Result<(), DomainError> {
            let mut rows = self.rows.lock().expect("the fixture's rows");
            rows.retain(|row| row.key != state.key);
            rows.push(state.clone());
            Ok(())
        }
    }

    fn service() -> ImportStateService {
        ImportStateService::new(Arc::new(MemoryRepo::default()))
    }

    fn write(partition: &str, offset: &str) -> WriteImportStateCommand {
        WriteImportStateCommand {
            persona_id: "persona".into(),
            partition: partition.into(),
            offset_json: offset.into(),
        }
    }

    fn read(persona: &str, partition: &str) -> ReadImportStateCommand {
        ReadImportStateCommand {
            persona_id: persona.into(),
            partition: partition.into(),
        }
    }

    /// The offset comes back exactly as it went in, key order included.
    ///
    /// A real question rather than a pedantic one: this workspace builds
    /// `serde_json` with `preserve_order`, so two objects that agree on
    /// content but not on the order their keys were written are the same
    /// value and different text. The offset belongs to the adapter, and
    /// an adapter comparing what it gets back against what it sent is
    /// comparing text.
    #[tokio::test]
    async fn an_offset_comes_back_as_it_was_written() {
        let service = service();
        let offset = r#"{"after_path":"/photos/z.png","seen":12}"#;
        service
            .write(
                write("root=/photos", offset),
                &AttributionContext::unrecorded(),
            )
            .await
            .expect("a write");

        let got = service
            .read(read("persona", "root=/photos"))
            .await
            .expect("a read")
            .expect("the point just written");
        assert_eq!(got.offset_json, offset, "byte for byte, and in that order");
    }

    /// A key nothing has stored answers "nothing", which is what a first
    /// run looks like — not a failure a caller has to tell apart from a
    /// broken store by reading a message.
    #[tokio::test]
    async fn a_key_nothing_has_stored_is_an_answer_and_not_a_failure() {
        let got = service()
            .read(read("persona", "root=/photos"))
            .await
            .expect("a read of nothing is still a read");
        assert!(got.is_none());
    }

    /// Two partitions inside one persona are two positions, and two
    /// personas over one partition are as well.
    ///
    /// Both halves of the key, asserted together because the failure
    /// they prevent is the same one: a run taking up after a point that
    /// was never about it, and importing nothing while looking like it
    /// worked.
    #[tokio::test]
    async fn both_halves_of_the_key_separate_a_position_from_another() {
        let service = service();
        service
            .write(
                write("root=/photos|ext=png", "1"),
                &AttributionContext::unrecorded(),
            )
            .await
            .expect("a write");

        assert!(
            service
                .read(read("persona", "root=/photos|ext=mp4"))
                .await
                .expect("a read")
                .is_none(),
            "another filter over the same tree is another partition"
        );
        assert!(
            service
                .read(read("other-persona", "root=/photos|ext=png"))
                .await
                .expect("a read")
                .is_none(),
            "and another persona's import is another position"
        );
    }

    /// A second write moves the point rather than adding beside it.
    #[tokio::test]
    async fn writing_again_moves_the_point() {
        let service = service();
        for offset in ["1", "2"] {
            service
                .write(
                    write("root=/photos", offset),
                    &AttributionContext::unrecorded(),
                )
                .await
                .expect("a write");
        }
        let got = service
            .read(read("persona", "root=/photos"))
            .await
            .expect("a read")
            .expect("one point");
        assert_eq!(got.offset_json, "2");
    }
}
