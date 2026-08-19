//! Wire parsing for the forge's own ids.
//!
//! Two functions, and they are here rather than in
//! [`application::mapping`](crate::application::mapping) for one
//! reason: that module is the catalogue's, and a forge id parsed there
//! made the catalogue name a forge type (#81). Nothing but the forge's
//! services ever called them.
//!
//! The uuid reading itself stays shared — [`parse_uuid`] is about
//! uuids, not about either side, and two copies of "is this a uuid"
//! would be two error messages for one mistake.

use crate::application::mapping::parse_uuid;
use crate::domain::forge::value::{ProjectId, PursuitId};
use crate::error::DomainError;

/// Parses the wire representation of a pursuit id.
pub fn parse_pursuit_id(value: &str) -> Result<PursuitId, DomainError> {
    Ok(PursuitId::from_uuid(parse_uuid(value, "pursuit_id")?))
}

/// Parses the wire representation of a project id.
pub fn parse_project_id(value: &str) -> Result<ProjectId, DomainError> {
    Ok(ProjectId::from_uuid(parse_uuid(value, "project_id")?))
}
