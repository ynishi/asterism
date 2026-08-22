//! The face that asks downward, and the client that speaks it.
//!
//! [`Store`] is stated in the shared vocabulary: ids the two sides
//! already agree on, and the shared error. [`StoreClient`] is the
//! forge's side of it, and the only thing in the forge that turns a
//! [`Content`] into the id a contract can carry.
//!
//! # Existence, not ownership
//!
//! The question is `exists`, and it used to be `owns(persona, asset)`.
//! That was wrong twice over, and the second HTTP surface built on it
//! is what made both visible.
//!
//! **A line carries no owner.** [`Lines::list`] says so: grouping and
//! access are outside the forge, and an instance has the lines
//! somebody made on purpose rather than one per person. So "real but
//! belonging to somebody else" is not a reason a reference is
//! unusable *here* — putting one person's asset on a shared line is
//! the thing a shared line is for.
//!
//! **And the check could not refuse a caller who wanted to pass.** A
//! persona is a column on the asset row and the caller chose both
//! halves of the pair, so naming the asset's own persona always
//! succeeded — and nothing here knew whether the caller was that
//! persona. What it caught was a client that paired the two wrongly.
//! It read as a guard on whose asset this is, and it was a consistency
//! check on two values one caller supplied.
//!
//! What the forge actually needs to know is whether the reference is
//! real, because an operation naming content that is not there is a
//! line lying about the present. That is one id and no persona.
//!
//! Nothing is deferred by this. "Who" is a question the forge already
//! asks, once, through [`Actors`](super::actors::Actors): a write
//! carries an [`Actor`](crate::domain::forge::model::act::Actor), the
//! handle is resolved by the side that knows what a user is, and it is
//! a handle precisely so that it exists before authentication binds it
//! and keeps pointing at the same actor afterwards. A persona was
//! never the forge's word for who, and an asset's owner was never the
//! forge's question.
//!
//! Access is per line and outside the forge, so what governs putting
//! content on one is who may write to that line. If the forge ever had
//! to record an owner rather than an author, it would be an `Actor` on
//! the entry — a fourth axis beside existence, content and name,
//! resolved through the same contract as every other handle. Nothing
//! asks for that today.
//!
//! [`Lines::list`]: crate::domain::forge::lines::Lines::list
//!
//! [`Content`]: crate::domain::forge::model::value::Content

use std::sync::Arc;

use async_trait::async_trait;

use crate::domain::forge::model::value::Content;
// SHARED VOCABULARY: ids and the error are boundary types — the shared
// vocabulary a contract is allowed to be stated in.
use crate::domain::value::AssetId;
use crate::error::DomainError;

/// What the layer below answers.
///
/// One question today. It grows when the forge has a second thing to
/// ask, and not before: a method nothing calls is a shape nothing has
/// checked, and every implementation has to satisfy it anyway.
#[async_trait]
pub trait Store: Send + Sync {
    /// Does this id name something?
    async fn exists(&self, asset: &AssetId) -> Result<bool, DomainError>;
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

    /// Is this content real?
    ///
    /// The translation is here and nowhere else: a [`Content`] goes
    /// in, an id the contract understands goes out, and the answer
    /// comes back as a plain yes or no that the forge can act on
    /// without learning anything about what it asked.
    pub async fn real(&self, content: &Content) -> Result<bool, DomainError> {
        self.0.exists(&content.asset()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct Everything;

    #[async_trait]
    impl Store for Everything {
        async fn exists(&self, _asset: &AssetId) -> Result<bool, DomainError> {
            Ok(true)
        }
    }

    struct Nothing;

    #[async_trait]
    impl Store for Nothing {
        async fn exists(&self, _asset: &AssetId) -> Result<bool, DomainError> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn the_client_passes_the_question_through_and_returns_the_answer() {
        let content = Content::from_uuid(Uuid::now_v7());

        let real = StoreClient::new(Arc::new(Everything))
            .real(&content)
            .await
            .unwrap();
        let missing = StoreClient::new(Arc::new(Nothing))
            .real(&content)
            .await
            .unwrap();

        assert!(real);
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
            async fn exists(&self, asset: &AssetId) -> Result<bool, DomainError> {
                Ok(*asset == self.0)
            }
        }

        let asset = AssetId::new();
        let client = StoreClient::new(Arc::new(Expecting(asset)));

        let same = client.real(&Content::of(asset)).await.unwrap();

        assert!(same);
    }
}
