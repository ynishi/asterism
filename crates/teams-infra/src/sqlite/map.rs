//! Row ↔ domain conversion helpers for the teams tables.
//!
//! The reading convention is `asterism-infra`'s: inside an isle closure
//! only `rusqlite` primitives are handled, and promoting rows into
//! domain types (including validation) happens **outside** the closure.
//! Writes are the deliberate exception, documented in
//! [`repo`](crate::sqlite::repo): the same-tx rule requires the domain
//! invariants to be evaluated *inside* the transaction, on the state it
//! is about to change.

use chrono::{DateTime, Utc};
use teams_core::DomainError;
use teams_core::domain::identity::LedgerActor;
use teams_core::domain::ledger::SubjectRef;

/// An epoch-ms column as an instant, refusing a value no clock
/// produced.
///
/// The forge's acts carry a `DateTime<Utc>` and every stamp column on
/// this plane is `INTEGER` epoch ms, so this pair is what the hosted
/// forge's rows cross. It refuses out-of-range rather than saturating:
/// a stored timestamp that is not a time is a corrupt row, and a
/// clamped one would read as a real instant nobody wrote.
pub fn ms_to_datetime(ms: i64) -> Result<DateTime<Utc>, DomainError> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .ok_or_else(|| DomainError::Infra(anyhow::anyhow!("timestamp out of range: {ms}")))
}

/// An instant as the epoch-ms column that carries it.
pub fn datetime_to_ms(dt: &DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}

/// Wraps an infrastructure error (typically `IsleError`) into
/// `DomainError::Infra`.
pub fn infra_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> DomainError {
    DomainError::Infra(anyhow::Error::new(e))
}

/// Serialises a [`LedgerActor`] into the `actor` TEXT column — the
/// serde-tagged JSON form, so the member/admin distinction lands in
/// storage exactly as the domain spells it.
pub fn actor_to_json(actor: &LedgerActor) -> Result<String, DomainError> {
    serde_json::to_string(actor)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot serialise ledger actor: {e}")))
}

/// Parses the `actor` TEXT column back into a [`LedgerActor`].
pub fn actor_from_json(json: &str) -> Result<LedgerActor, DomainError> {
    serde_json::from_str(json)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("corrupt actor column: {e}")))
}

/// Splits a [`SubjectRef`] into the `(ref_type, ref_value)` pair the
/// `ledger_subject` table is keyed by.
///
/// Goes through the serde representation rather than a hand-written
/// match so the columns and the wire form cannot disagree: the tag is
/// the `ref_type`, the content — a string in every variant (digests,
/// the uuid's hyphenated form, the forge handle's canonical encoding) —
/// is the `ref_value`. A trace query looking for a subject encodes it
/// the same way, so index walks compare exactly what appends wrote.
pub fn subject_to_ref(subject: &SubjectRef) -> Result<(String, String), DomainError> {
    let value = serde_json::to_value(subject)
        .map_err(|e| DomainError::Infra(anyhow::anyhow!("cannot serialise subject: {e}")))?;
    let ref_type = value
        .get("ref_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DomainError::Infra(anyhow::anyhow!("subject serialised without ref_type")))?
        .to_string();
    let ref_value = value
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            DomainError::Infra(anyhow::anyhow!("subject serialised without a string value"))
        })?
        .to_string();
    Ok((ref_type, ref_value))
}

/// Rebuilds a [`SubjectRef`] from its `(ref_type, ref_value)` columns —
/// the inverse of [`subject_to_ref`], through the same serde
/// representation.
pub fn subject_from_ref(ref_type: &str, ref_value: &str) -> Result<SubjectRef, DomainError> {
    serde_json::from_value(serde_json::json!({
        "ref_type": ref_type,
        "value": ref_value,
    }))
    .map_err(|e| {
        DomainError::Infra(anyhow::anyhow!(
            "corrupt subject columns ({ref_type:?}, {ref_value:?}): {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use teams_core::domain::identity::ActorStamp;
    use teams_core::domain::ledger::ForgeIdentityRef;
    use uuid::Uuid;

    #[test]
    fn an_actor_round_trips_with_its_capacity_intact() {
        let stamp = ActorStamp {
            user_id: Uuid::now_v7(),
            display_name: "Hoshino".into(),
        };
        let member = LedgerActor::member(stamp.clone());
        let back = actor_from_json(&actor_to_json(&member).unwrap()).unwrap();
        assert_eq!(back, member);
        assert!(!back.is_admin());
    }

    #[test]
    fn every_subject_variant_round_trips_through_the_two_columns() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let subjects = [
            SubjectRef::digest(&digest).unwrap(),
            SubjectRef::blob(&digest).unwrap(),
            SubjectRef::user(Uuid::now_v7()),
            SubjectRef::forge_identity(ForgeIdentityRef::owner()),
            SubjectRef::forge_identity(ForgeIdentityRef::unrecorded()),
            SubjectRef::forge_identity(ForgeIdentityRef::server()),
            SubjectRef::forge_identity(ForgeIdentityRef::subject("hoshino").unwrap()),
        ];
        for subject in subjects {
            let (ref_type, ref_value) = subject_to_ref(&subject).unwrap();
            assert_eq!(subject_from_ref(&ref_type, &ref_value).unwrap(), subject);
        }
    }

    #[test]
    fn a_forge_handle_reaches_the_index_as_its_canonical_string() {
        // The column pair is what a trace query compares against, so
        // the typed pair has to arrive as the one string both sides
        // spell — not as JSON the index could never match.
        let (ref_type, ref_value) =
            subject_to_ref(&SubjectRef::forge_identity(ForgeIdentityRef::owner())).unwrap();
        assert_eq!(ref_type, "forge_identity");
        assert_eq!(ref_value, "owner");

        let (_, ref_value) = subject_to_ref(&SubjectRef::forge_identity(
            ForgeIdentityRef::subject("hoshino").unwrap(),
        ))
        .unwrap();
        assert_eq!(ref_value, "subject:hoshino");

        // A stored value outside the #102 vocabulary is a corrupt
        // column, refused rather than carried as a guess.
        assert!(subject_from_ref("forge_identity", "who-knows").is_err());
    }
}
