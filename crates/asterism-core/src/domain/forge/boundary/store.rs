//! The face that asks downward, and the client that speaks it.
//!
//! [`Store`] is stated in the shared vocabulary: ids the two sides
//! already agree on, and the shared error. [`StoreClient`] is the
//! forge's side of it, and the only thing in the forge that turns a
//! [`Content`] into the id a contract can carry.
//!
//! # Ownership, not existence
//!
//! The question is `owns`, not `exists`, and the difference is the
//! persona. A reference to something real but belonging to somebody
//! else is exactly as unusable as a reference to nothing, and a
//! crossing that forgets whose data it is asking about is the kind of
//! mistake that does not surface until two tenants are in the same
//! database. Carrying the persona in the signature means forgetting it
//! does not compile.
//!
//! The forge does not decide what ownership means — it asks, and the
//! side that holds the content answers.
//!
//! [`Content`]: crate::domain::forge::model::value::Content

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::forge::model::value::Content;
// SHARED VOCABULARY: ids and the error are boundary types — the shared
// vocabulary a contract is allowed to be stated in.
use crate::domain::value::{AssetId, PersonaId};
use crate::error::DomainError;

/// What the layer below answers.
///
/// One question today. It grows when the forge has a second thing to
/// ask, and not before: a method nothing calls is a shape nothing has
/// checked, and every implementation has to satisfy it anyway.
#[async_trait]
pub trait Store: Send + Sync {
    /// Does this id name something this persona holds?
    async fn owns(&self, persona: &PersonaId, asset: &AssetId) -> Result<bool, DomainError>;
}

/// The forge's side of [`Store`].
///
/// Holds the contract and speaks the forge's words at it, so that no
/// other part of the forge has to hold one or know the vocabulary it
/// is stated in.
#[derive(Clone)]
pub struct StoreClient(Arc<dyn Store>);

impl StoreClient {
    /// Wraps a contract.
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self(store)
    }

    /// Is this content real, and this persona's?
    ///
    /// The translation is here and nowhere else: a [`Content`] goes
    /// in, an id the contract understands goes out, and the answer
    /// comes back as a plain yes or no that the forge can act on
    /// without learning anything about what it asked.
    pub async fn holds(&self, persona: &PersonaId, content: &Content) -> Result<bool, DomainError> {
        self.0.owns(persona, &content.asset()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Everything;

    #[async_trait]
    impl Store for Everything {
        async fn owns(&self, _persona: &PersonaId, _asset: &AssetId) -> Result<bool, DomainError> {
            Ok(true)
        }
    }

    struct Nothing;

    #[async_trait]
    impl Store for Nothing {
        async fn owns(&self, _persona: &PersonaId, _asset: &AssetId) -> Result<bool, DomainError> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn the_client_passes_the_question_through_and_returns_the_answer() {
        let content = Content::from_uuid(Uuid::now_v7());
        let persona = PersonaId::new();

        let held = StoreClient::new(Arc::new(Everything))
            .holds(&persona, &content)
            .await
            .unwrap();
        let missing = StoreClient::new(Arc::new(Nothing))
            .holds(&persona, &content)
            .await
            .unwrap();

        assert!(held);
        assert!(!missing);
    }

    /// The id the contract is asked about is the one inside the
    /// content, not a fresh one — the translation has to be identity,
    /// or the forge asks about something it never referred to.
    #[tokio::test]
    async fn the_client_asks_about_the_id_the_content_holds() {
        struct Expecting(AssetId);

        #[async_trait]
        impl Store for Expecting {
            async fn owns(
                &self,
                _persona: &PersonaId,
                asset: &AssetId,
            ) -> Result<bool, DomainError> {
                Ok(*asset == self.0)
            }
        }

        let asset = AssetId::new();
        let client = StoreClient::new(Arc::new(Expecting(asset)));

        let same = client
            .holds(&PersonaId::new(), &Content::of(asset))
            .await
            .unwrap();

        assert!(same);
    }
}
