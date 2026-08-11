//! `ModalityService` — use cases for the Modality master.
//!
//! The master is the open half of the two-layer model: rows are listed,
//! created, partially updated, and deleted at runtime. Behaviour is not
//! touched here — the service only validates identity + presentation
//! metadata and the `kind` reference into the closed
//! [`ContentKind`](crate::domain::value::ContentKind).
//!
//! Delete is guarded: a slug still carried by any asset is rejected
//! (`409 Conflict`) so the master can never orphan assets. The
//! operational retirement path is the `hidden` flag.
//!
//! Every write here takes an [`AttributionContext`] it does not persist:
//! the master carries no attribution column, and none is being added
//! (see the [`application`](crate::application) module doc for why the
//! argument is required anyway).

use std::sync::Arc;

use asterism_contract::command::{
    CreateModalityCommand, DeleteModalityCommand, UpdateModalityCommand,
};
use asterism_contract::dto::ModalityDefDto;

use crate::application::mapping::modality_view_to_dto;
use crate::domain::attribution::AttributionContext;
use crate::domain::modality::ModalityDef;
use crate::domain::repository::ModalityRepository;
use crate::domain::value::{CoverTemplate, Modality};
use crate::error::DomainError;

/// Modality master use-case service. Shared as an `Arc` through Tauri
/// state and server contexts.
pub struct ModalityService {
    repo: Arc<dyn ModalityRepository>,
}

impl ModalityService {
    /// Constructs the service around the master repository port.
    pub fn new(repo: Arc<dyn ModalityRepository>) -> Self {
        Self { repo }
    }

    /// Returns every master row (hidden included) with its live asset
    /// count, ordered by `sort_order` then `slug`.
    pub async fn list(&self) -> Result<Vec<ModalityDefDto>, DomainError> {
        Ok(self
            .repo
            .list()
            .await?
            .iter()
            .map(modality_view_to_dto)
            .collect())
    }

    /// Creates a master row. Slug grammar is validated here; a
    /// duplicate slug surfaces from the adapter as `Conflict`.
    pub async fn create(
        &self,
        command: CreateModalityCommand,
        _attribution: &AttributionContext,
    ) -> Result<ModalityDefDto, DomainError> {
        let slug = Modality::new(command.slug)?;
        let cover_template = command
            .cover_template
            .as_deref()
            .map(CoverTemplate::parse)
            .transpose()?;
        let def = ModalityDef {
            slug,
            label: command.label,
            terminal: command.terminal,
            sort_order: command.sort_order,
            hidden: command.hidden,
            cover_template,
        };
        self.repo.create(&def).await?;
        // A fresh insert has no assets yet, but re-read through the
        // count-bearing listing shape so the returned DTO is uniform.
        self.find_dto(&def.slug).await
    }

    /// Partially updates a master row: fetches the current row, applies
    /// the `Some` fields, and writes the resolved row back. Fails with
    /// `NotFound` when the slug is not registered.
    pub async fn update(
        &self,
        command: UpdateModalityCommand,
        _attribution: &AttributionContext,
    ) -> Result<ModalityDefDto, DomainError> {
        let slug = Modality::new(command.slug)?;
        let mut def = self
            .repo
            .find(&slug)
            .await?
            .ok_or_else(|| DomainError::not_found("modality", slug))?;
        if let Some(label) = command.label {
            def.label = label;
        }
        if let Some(terminal) = command.terminal {
            def.terminal = terminal;
        }
        if let Some(sort_order) = command.sort_order {
            def.sort_order = sort_order;
        }
        if let Some(hidden) = command.hidden {
            def.hidden = hidden;
        }
        if let Some(cover_template) = command.cover_template {
            def.cover_template = Some(CoverTemplate::parse(&cover_template)?);
        }
        self.repo.update(&def).await?;
        self.find_dto(&def.slug).await
    }

    /// Deletes a master row, but only when no asset carries the slug
    /// (`409 Conflict` otherwise — the master never orphans assets).
    /// A missing slug is a `NotFound` (nothing to delete).
    pub async fn delete(
        &self,
        command: DeleteModalityCommand,
        _attribution: &AttributionContext,
    ) -> Result<(), DomainError> {
        let slug = Modality::new(command.slug)?;
        let count = self.repo.asset_count(&slug).await?;
        if count > 0 {
            return Err(DomainError::Conflict(format!(
                "modality {slug} still has {count} asset(s); hide it instead of deleting"
            )));
        }
        self.repo.delete(&slug).await
    }

    /// Re-reads one master row through the count-bearing listing so a
    /// write path can return the uniform [`ModalityDefDto`] shape.
    async fn find_dto(&self, slug: &Modality) -> Result<ModalityDefDto, DomainError> {
        self.repo
            .list()
            .await?
            .iter()
            .find(|view| view.def.slug == *slug)
            .map(modality_view_to_dto)
            .ok_or_else(|| DomainError::not_found("modality", slug))
    }
}
