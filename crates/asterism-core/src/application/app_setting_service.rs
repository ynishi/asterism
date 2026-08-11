//! `AppSettingService` — use cases for application settings.
//!
//! Resolves the closed registry
//! ([`SETTING_REGISTRY`](crate::domain::app_setting::SETTING_REGISTRY))
//! against the process environment and the stored overrides in
//! `app_setting`, in that order of increasing precedence.
//!
//! ## Why the stored row wins over the environment
//!
//! `default` → `env` → `stored`, uniformly for every key. A settings
//! screen makes exactly one promise — that what you pick is what runs —
//! and a process-wide variable quietly outranking it breaks that promise
//! with no recourse from inside the app. The environment therefore acts
//! as a *seed*: it supplies the value while nothing is stored, and gives
//! way once someone chooses one.
//!
//! One order for all keys is deliberate. Splitting the registry into
//! "user preference" and "operational knob" halves, each with its own
//! precedence, was considered and rejected as premature — a per-key rule
//! is only worth its complexity once a key actually needs enforcement,
//! and none does today. The escape hatch when that changes is a
//! separate, explicitly-named lock, not a second ordering.
//!
//! An env var whose contents do not parse as the declared kind, or which
//! falls outside the key's declared range, is **ignored** rather than
//! fatal: a typo in a shell export should not stop the application from
//! starting. It is logged, because an override that silently does
//! nothing is the hardest kind to diagnose.
//!
//! ## Every layer is kept
//!
//! [`EffectiveSetting::layers`] carries each layer that has a value, not
//! just the winner, so a client can show what a value is shadowing.
//! Collapsing to the winner is what previously left a stored row hidden
//! underneath an env var with no way to see or clear it.
//!
//! ## Attribution
//!
//! [`set`](AppSettingService::set) and [`reset`](AppSettingService::reset)
//! take an [`AttributionContext`] they do not persist: `app_setting` is a
//! closed key → value registry with no room for a writer, and none is
//! being added (see the [`application`](crate::application) module doc
//! for why the argument is required anyway).

use std::sync::Arc;

use asterism_contract::command::{ResetSettingCommand, SetSettingCommand};
use asterism_contract::dto::SettingDto;
use chrono::Utc;

use crate::application::mapping::effective_setting_to_dto;
use crate::domain::app_setting::{
    AppSetting, EffectiveSetting, SETTING_REGISTRY, SettingKey, SettingLayer, SettingSource,
};
use crate::domain::attribution::AttributionContext;
use crate::domain::repository::AppSettingRepository;
use crate::error::DomainError;

/// Reads the process environment. Injected so tests can resolve against
/// a fixed map instead of mutating global state (`std::env::set_var` is
/// unsound under a multi-threaded test runner).
pub trait EnvSource: Send + Sync {
    /// Returns the value of `name`, or `None` when it is unset.
    fn get(&self, name: &str) -> Option<String>;
}

/// [`EnvSource`] backed by the real process environment.
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// Application settings use-case service. Shared as an `Arc` through
/// Tauri state and server contexts.
pub struct AppSettingService {
    repo: Arc<dyn AppSettingRepository>,
    env: Arc<dyn EnvSource>,
}

impl AppSettingService {
    /// Constructs the service against the real process environment.
    pub fn new(repo: Arc<dyn AppSettingRepository>) -> Self {
        Self {
            repo,
            env: Arc::new(ProcessEnv),
        }
    }

    /// Constructs the service with an explicit [`EnvSource`].
    ///
    /// `#[cfg(test)]`: the only callers are this file's own tests,
    /// which need a `MapEnv` to exercise the default → env → stored
    /// layering without mutating process-global environment state.
    /// Compiled out of the production build so the seam cannot be
    /// mistaken for a supported way to construct the service — the
    /// composition root uses [`new`](Self::new) and the real process
    /// environment.
    #[cfg(test)]
    pub fn with_env(repo: Arc<dyn AppSettingRepository>, env: Arc<dyn EnvSource>) -> Self {
        Self { repo, env }
    }

    /// Resolves every registry key, in registry order.
    ///
    /// One repository read serves the whole listing — the layer stack is
    /// applied in memory, so adding a key costs no extra query.
    pub async fn list(&self) -> Result<Vec<SettingDto>, DomainError> {
        let stored = self.repo.list().await?;
        Ok(SETTING_REGISTRY
            .iter()
            .map(|def| {
                let key = SettingKey::parse(def.key)
                    .expect("registry entries resolve against the registry");
                let stored_value = stored
                    .iter()
                    .find(|row| row.key == key)
                    .map(|row| row.value_json.clone());
                effective_setting_to_dto(&self.resolve(key, stored_value))
            })
            .collect())
    }

    /// Resolves a single key. Fails with `NotFound` when the key is not
    /// in the registry.
    pub async fn get(&self, key: &str) -> Result<SettingDto, DomainError> {
        let key = SettingKey::parse(key)?;
        let stored = self.repo.find(key).await?.map(|row| row.value_json);
        Ok(effective_setting_to_dto(&self.resolve(key, stored)))
    }

    /// Stores an override and returns the newly resolved value.
    ///
    /// The returned DTO is the *effective* value, which a successful
    /// write always makes `source: stored` — nothing outranks a stored
    /// row. Returning the resolution rather than the input still
    /// matters: it carries the refreshed chain, so a caller repaints
    /// the provenance trail without a second read.
    pub async fn set(
        &self,
        command: SetSettingCommand,
        _attribution: &AttributionContext,
    ) -> Result<SettingDto, DomainError> {
        let key = SettingKey::parse(&command.key)?;
        let value_json = key.def().canonicalise(&command.value_json)?;
        self.repo
            .upsert(&AppSetting {
                key,
                value_json,
                updated_at: Utc::now(),
            })
            .await?;
        self.get(key.as_str()).await
    }

    /// Clears an override and returns the value that now applies.
    pub async fn reset(
        &self,
        command: ResetSettingCommand,
        _attribution: &AttributionContext,
    ) -> Result<SettingDto, DomainError> {
        let key = SettingKey::parse(&command.key)?;
        self.repo.delete(key).await?;
        self.get(key.as_str()).await
    }

    /// Builds the layer stack `default → env → stored` and reads the
    /// winner off the top.
    ///
    /// A layer whose value fails validation stays in the chain carrying
    /// the reason, rather than vanishing. It never wins, but the person
    /// who exported a typo'd variable can see that it was seen and
    /// discarded — which is the whole point of keeping the chain.
    fn resolve(&self, key: SettingKey, stored: Option<String>) -> EffectiveSetting {
        let def = key.def();
        // The code default always contributes and cannot be rejected
        // (`registry_ranges_only_bound_int_keys_and_admit_their_defaults`
        // pins that), so a non-rejected layer always exists. It is run
        // through `canonicalise` like every other layer so the whole
        // chain is spelled the same way.
        let mut layers = vec![SettingLayer {
            source: SettingSource::Default,
            value_json: def
                .canonicalise(def.default_json)
                .unwrap_or_else(|_| def.default_json.to_string()),
            origin: None,
            rejected: None,
        }];

        if let Some(name) = def.env_var
            && let Some(raw) = self.env.get(name)
        {
            let (value_json, rejected) = match def.canonicalise(&env_to_json(def.kind, &raw)) {
                Ok(value_json) => (value_json, None),
                // Also logged: the chain reaches whoever opens the
                // settings screen, the log reaches whoever started the
                // process from a shell.
                Err(err) => {
                    tracing::warn!(
                        event = "diag.setting.env_rejected",
                        key = def.key,
                        var = name,
                        value = %raw,
                        error = %err,
                        "environment override is not a usable value; ignoring it"
                    );
                    (raw.clone(), Some(err.to_string()))
                }
            };
            layers.push(SettingLayer {
                source: SettingSource::Env,
                value_json,
                origin: Some(name),
                rejected,
            });
        }

        // The stored row is re-validated on every read, not only at
        // write time. `set` is not the only way a row can appear: a
        // newer build may have written it, and a key's declared kind or
        // range can change between builds while the schema is still
        // moving. A row that no longer matches is shown as rejected
        // rather than served as a value whose `kind` lies about how to
        // parse it.
        if let Some(raw) = stored {
            let (value_json, rejected) = match def.canonicalise(&raw) {
                Ok(value_json) => (value_json, None),
                Err(err) => (raw, Some(err.to_string())),
            };
            layers.push(SettingLayer {
                source: SettingSource::Stored,
                value_json,
                origin: None,
                rejected,
            });
        }

        let winner = layers
            .iter()
            .rev()
            .find(|layer| layer.rejected.is_none())
            .expect("the default layer is present and never rejected");
        EffectiveSetting {
            key,
            value_json: winner.value_json.clone(),
            source: winner.source,
            layers,
        }
    }
}

/// Renders a raw environment string as JSON text of the declared kind.
///
/// Environment variables are untyped strings, so `ASTERISM_JOB_CONCURRENCY=8`
/// arrives as `"8"` and has to become `8` before it can be validated as
/// an `Int`. `Bool` accepts the shell spellings people actually type;
/// anything else fails validation upstream and the layer is skipped.
fn env_to_json(kind: crate::domain::app_setting::SettingValueKind, raw: &str) -> String {
    use crate::domain::app_setting::SettingValueKind as K;
    let trimmed = raw.trim();
    match kind {
        K::Int => trimmed.to_string(),
        K::Bool => match trimmed.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => "true".to_string(),
            "0" | "false" | "no" | "off" => "false".to_string(),
            other => other.to_string(),
        },
        K::Text => serde_json::Value::String(trimmed.to_string()).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::app_setting::SettingValueKind;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryRepo {
        rows: Mutex<Vec<AppSetting>>,
    }

    #[async_trait]
    impl AppSettingRepository for MemoryRepo {
        async fn list(&self) -> Result<Vec<AppSetting>, DomainError> {
            Ok(self.rows.lock().unwrap().clone())
        }

        async fn find(&self, key: SettingKey) -> Result<Option<AppSetting>, DomainError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.key == key)
                .cloned())
        }

        async fn upsert(&self, setting: &AppSetting) -> Result<(), DomainError> {
            let mut rows = self.rows.lock().unwrap();
            rows.retain(|row| row.key != setting.key);
            rows.push(setting.clone());
            Ok(())
        }

        async fn delete(&self, key: SettingKey) -> Result<(), DomainError> {
            self.rows.lock().unwrap().retain(|row| row.key != key);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MapEnv(HashMap<&'static str, &'static str>);

    impl EnvSource for MapEnv {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).map(|v| (*v).to_string())
        }
    }

    fn service(env: MapEnv) -> AppSettingService {
        AppSettingService::with_env(Arc::new(MemoryRepo::default()), Arc::new(env))
    }

    /// These tests are about layer resolution, not about who changed a
    /// setting: nothing here reads the context, so the writes carry the
    /// value a system write does.
    fn anyone() -> AttributionContext {
        AttributionContext::unrecorded()
    }

    #[tokio::test]
    async fn unset_key_resolves_to_default() {
        let dto = service(MapEnv::default())
            .get("import.auto_organize")
            .await
            .unwrap();
        assert_eq!(dto.value_json, "true");
        assert_eq!(dto.source, "default");
    }

    #[tokio::test]
    async fn stored_override_wins_over_default() {
        let svc = service(MapEnv::default());
        let dto = svc
            .set(
                SetSettingCommand {
                    key: "ui.clean_mode".into(),
                    value_json: "true".into(),
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(dto.value_json, "true");
        assert_eq!(dto.source, "stored");
    }

    #[tokio::test]
    async fn stored_wins_over_env() {
        // The promise a settings screen makes: what you pick is what
        // runs. The environment seeds the value, it does not cap it.
        let svc = service(MapEnv(HashMap::from([("ASTERISM_JOB_CONCURRENCY", "8")])));
        svc.set(
            SetSettingCommand {
                key: "jobs.concurrency".into(),
                value_json: "2".into(),
            },
            &anyone(),
        )
        .await
        .unwrap();
        let dto = svc.get("jobs.concurrency").await.unwrap();
        assert_eq!(dto.value_json, "2");
        assert_eq!(dto.source, "stored");
        assert_eq!(dto.env_var.as_deref(), Some("ASTERISM_JOB_CONCURRENCY"));
    }

    #[tokio::test]
    async fn env_applies_while_nothing_is_stored_and_returns_after_a_reset() {
        let svc = service(MapEnv(HashMap::from([("ASTERISM_JOB_CONCURRENCY", "8")])));
        // Seed role: nothing stored, so the export is what runs.
        assert_eq!(svc.get("jobs.concurrency").await.unwrap().source, "env");

        svc.set(
            SetSettingCommand {
                key: "jobs.concurrency".into(),
                value_json: "2".into(),
            },
            &anyone(),
        )
        .await
        .unwrap();
        assert_eq!(svc.get("jobs.concurrency").await.unwrap().source, "stored");

        // Clearing the choice hands the key back to the environment,
        // not to the code default — which is what makes Reset a
        // meaningful, visible action rather than a dead button.
        let after = svc
            .reset(
                ResetSettingCommand {
                    key: "jobs.concurrency".into(),
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(after.source, "env");
        assert_eq!(after.value_json, "8");
    }

    #[tokio::test]
    async fn every_contributing_layer_is_reported_in_precedence_order() {
        let svc = service(MapEnv(HashMap::from([("ASTERISM_JOB_CONCURRENCY", "8")])));
        svc.set(
            SetSettingCommand {
                key: "jobs.concurrency".into(),
                value_json: "2".into(),
            },
            &anyone(),
        )
        .await
        .unwrap();

        let dto = svc.get("jobs.concurrency").await.unwrap();
        let chain: Vec<_> = dto
            .layers
            .iter()
            .map(|l| (l.source.as_str(), l.value_json.as_str()))
            .collect();
        assert_eq!(chain, vec![("default", "0"), ("env", "8"), ("stored", "2")]);
        // The winner is the top of the chain, not a separate opinion.
        assert_eq!(dto.value_json, dto.layers.last().unwrap().value_json);
        assert_eq!(dto.source, dto.layers.last().unwrap().source);
        // Only the env layer names where it came from.
        assert_eq!(
            dto.layers[1].origin.as_deref(),
            Some("ASTERISM_JOB_CONCURRENCY")
        );
        assert_eq!(dto.layers[0].origin, None);
        assert_eq!(dto.layers[2].origin, None);
    }

    #[tokio::test]
    async fn an_invalid_layer_stays_in_the_chain_marked_rejected() {
        // The whole reason to keep the chain: a variable that was
        // exported and thrown away must be visible somewhere, or the
        // person who exported it cannot tell it from a typo in the
        // variable *name*.
        let svc = service(MapEnv(HashMap::from([(
            "ASTERISM_JOB_CONCURRENCY",
            "many",
        )])));
        let dto = svc.get("jobs.concurrency").await.unwrap();

        assert_eq!(dto.layers.len(), 2);
        assert_eq!(dto.layers[1].source, "env");
        assert_eq!(dto.layers[1].value_json, "many");
        assert!(dto.layers[1].rejected.is_some());
        // Rejected never wins: the default is still in force.
        assert_eq!(dto.source, "default");
        assert_eq!(dto.value_json, "0");
    }

    #[tokio::test]
    async fn a_rejected_stored_row_is_shown_and_never_wins() {
        let repo = Arc::new(MemoryRepo::default());
        repo.upsert(&AppSetting {
            key: SettingKey::parse("jobs.concurrency").unwrap(),
            value_json: "9999".into(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
        let svc = AppSettingService::with_env(repo, Arc::new(MapEnv::default()));

        let dto = svc.get("jobs.concurrency").await.unwrap();
        assert_eq!(dto.layers.len(), 2);
        assert_eq!(dto.layers[1].source, "stored");
        assert!(dto.layers[1].rejected.is_some());
        assert_eq!(dto.source, "default");
    }

    #[tokio::test]
    async fn unparseable_env_falls_back_instead_of_failing() {
        let svc = service(MapEnv(HashMap::from([(
            "ASTERISM_JOB_CONCURRENCY",
            "many",
        )])));
        let dto = svc.get("jobs.concurrency").await.unwrap();
        assert_eq!(dto.value_json, "0");
        assert_eq!(dto.source, "default");
    }

    #[tokio::test]
    async fn reset_returns_to_default_and_is_idempotent() {
        let svc = service(MapEnv::default());
        svc.set(
            SetSettingCommand {
                key: "ui.clean_mode".into(),
                value_json: "true".into(),
            },
            &anyone(),
        )
        .await
        .unwrap();
        let command = ResetSettingCommand {
            key: "ui.clean_mode".into(),
        };
        let first = svc.reset(command.clone(), &anyone()).await.unwrap();
        assert_eq!(first.value_json, "false");
        assert_eq!(first.source, "default");
        let second = svc.reset(command, &anyone()).await.unwrap();
        assert_eq!(second.source, "default");
    }

    #[tokio::test]
    async fn unknown_key_is_not_found_on_read_and_write() {
        let svc = service(MapEnv::default());
        assert!(matches!(
            svc.get("ui.nope").await.unwrap_err(),
            DomainError::NotFound { .. }
        ));
        assert!(matches!(
            svc.set(
                SetSettingCommand {
                    key: "ui.nope".into(),
                    value_json: "true".into(),
                },
                &anyone(),
            )
            .await
            .unwrap_err(),
            DomainError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn out_of_range_write_is_rejected() {
        let svc = service(MapEnv::default());
        let err = svc
            .set(
                SetSettingCommand {
                    key: "jobs.concurrency".into(),
                    value_json: "100000".into(),
                },
                &anyone(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation(_)));
        // Nothing was stored, so the key still reads as its default.
        assert_eq!(svc.get("jobs.concurrency").await.unwrap().source, "default");
    }

    #[tokio::test]
    async fn out_of_range_env_is_ignored_like_an_unparseable_one() {
        let svc = service(MapEnv(HashMap::from([(
            "ASTERISM_JOB_CONCURRENCY",
            "100000",
        )])));
        let dto = svc.get("jobs.concurrency").await.unwrap();
        assert_eq!(dto.source, "default");
        assert_eq!(dto.value_json, "0");
    }

    #[tokio::test]
    async fn out_of_range_env_leaves_the_chain_without_an_env_entry() {
        // The range is enforced on every layer, so an unusable export
        // is simply not part of the chain. Worth pinning separately
        // from the no-stored-row case, because only here can you tell
        // "the env layer was dropped" from "the env layer lost".
        let repo = Arc::new(MemoryRepo::default());
        repo.upsert(&AppSetting {
            key: SettingKey::parse("jobs.concurrency").unwrap(),
            value_json: "4".into(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
        let svc = AppSettingService::with_env(
            repo,
            Arc::new(MapEnv(HashMap::from([("ASTERISM_JOB_CONCURRENCY", "512")]))),
        );

        let dto = svc.get("jobs.concurrency").await.unwrap();
        assert_eq!(dto.source, "stored");
        assert_eq!(dto.value_json, "4");
        // The export is listed and marked rejected, so the operator can
        // see that it was read and refused rather than assuming the
        // variable name was wrong.
        let env = dto.layers.iter().find(|l| l.source == "env").unwrap();
        assert_eq!(env.value_json, "512");
        assert!(env.rejected.is_some());
    }

    #[tokio::test]
    async fn range_is_exposed_on_the_dto_for_rendering() {
        let dto = service(MapEnv::default())
            .get("jobs.concurrency")
            .await
            .unwrap();
        assert_eq!(dto.min, Some(0));
        assert_eq!(dto.max, Some(256));
        let unbounded = service(MapEnv::default())
            .get("ui.clean_mode")
            .await
            .unwrap();
        assert_eq!(unbounded.min, None);
        assert_eq!(unbounded.max, None);
    }

    #[tokio::test]
    async fn kind_mismatch_is_rejected() {
        let svc = service(MapEnv::default());
        let err = svc
            .set(
                SetSettingCommand {
                    key: "ui.clean_mode".into(),
                    value_json: "\"yes\"".into(),
                },
                &anyone(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[tokio::test]
    async fn list_reflects_stored_and_env_layers_per_key() {
        let svc = service(MapEnv(HashMap::from([("ASTERISM_JOB_CONCURRENCY", "8")])));
        svc.set(
            SetSettingCommand {
                key: "ui.clean_mode".into(),
                value_json: "true".into(),
            },
            &anyone(),
        )
        .await
        .unwrap();

        let by_key: HashMap<String, SettingDto> = svc
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|dto| (dto.key.clone(), dto))
            .collect();

        assert_eq!(by_key["ui.clean_mode"].source, "stored");
        assert_eq!(by_key["ui.clean_mode"].value_json, "true");
        assert_eq!(by_key["jobs.concurrency"].source, "env");
        assert_eq!(by_key["jobs.concurrency"].value_json, "8");
        assert_eq!(by_key["import.auto_organize"].source, "default");
    }

    #[tokio::test]
    async fn set_returns_the_value_it_just_made_effective() {
        // The write takes effect immediately even with an export
        // present, and the returned DTO says so — no round trip needed
        // to find out whether the choice landed.
        let svc = service(MapEnv(HashMap::from([("ASTERISM_JOB_CONCURRENCY", "8")])));
        let dto = svc
            .set(
                SetSettingCommand {
                    key: "jobs.concurrency".into(),
                    value_json: "2".into(),
                },
                &anyone(),
            )
            .await
            .unwrap();
        assert_eq!(dto.source, "stored");
        assert_eq!(dto.value_json, "2");
    }

    #[tokio::test]
    async fn stored_row_that_no_longer_matches_its_kind_falls_back_to_default() {
        // Written out of band (a newer build, or a kind change between
        // builds) — `set` would have rejected it.
        let repo = Arc::new(MemoryRepo::default());
        repo.upsert(&AppSetting {
            key: SettingKey::parse("jobs.concurrency").unwrap(),
            value_json: "\"auto\"".into(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
        let svc = AppSettingService::with_env(repo, Arc::new(MapEnv::default()));

        let dto = svc.get("jobs.concurrency").await.unwrap();
        assert_eq!(dto.value_json, "0");
        assert_eq!(dto.source, "default");
        // The bulk path must agree with the single-key path.
        let listed = svc.list().await.unwrap();
        let row = listed.iter().find(|d| d.key == "jobs.concurrency").unwrap();
        assert_eq!(row.source, "default");
    }

    #[tokio::test]
    async fn list_covers_the_whole_registry() {
        let dtos = service(MapEnv::default()).list().await.unwrap();
        assert_eq!(dtos.len(), SETTING_REGISTRY.len());
        for (dto, def) in dtos.iter().zip(SETTING_REGISTRY) {
            assert_eq!(dto.key, def.key);
            // With nothing stored and no env, the chain is the default
            // alone — and the default is always `layers[0]`.
            assert_eq!(dto.layers.len(), 1);
            assert_eq!(dto.layers[0].source, "default");
            assert_eq!(dto.layers[0].value_json, def.default_json);
            assert_eq!(dto.value_json, def.default_json);
        }
    }

    #[test]
    fn env_bool_accepts_shell_spellings() {
        for raw in ["1", "true", "TRUE", "yes", "on"] {
            assert_eq!(env_to_json(SettingValueKind::Bool, raw), "true");
        }
        for raw in ["0", "false", "no", "off"] {
            assert_eq!(env_to_json(SettingValueKind::Bool, raw), "false");
        }
    }
}
