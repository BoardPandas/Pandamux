use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use pandamux_core::{
    EnvironmentId, ProviderInstanceId,
    provider_config::{ProviderCapabilities, ProviderInstanceConfig, ProviderKind},
};
use pandamux_protocol::RpcError;
use pandamux_providers::{ProviderAuthStatus, ProviderDriver, ProviderHealth, ProviderSnapshot};

use crate::{DriverRegistry, Store};

#[derive(Clone)]
pub struct ProviderResolver {
    store: Store,
    drivers: Arc<DriverRegistry>,
    local_environment_id: EnvironmentId,
    data_dir: Arc<RwLock<PathBuf>>,
    process_counts: Arc<Mutex<HashMap<ProviderProcessKey, usize>>>,
}

pub struct ResolvedProviderInstance {
    pub config: ProviderInstanceConfig,
    pub driver: Arc<dyn ProviderDriver>,
    pub capabilities: ProviderCapabilities,
}

pub struct ResolvedProviderExecution {
    pub instance: ResolvedProviderInstance,
    pub snapshot: ProviderSnapshot,
    pub(crate) _permit: Arc<ProviderProcessPermit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ProviderProcessKey {
    environment_id: EnvironmentId,
    instance_id: ProviderInstanceId,
}

pub(crate) struct ProviderProcessPermit {
    counts: Arc<Mutex<HashMap<ProviderProcessKey, usize>>>,
    key: ProviderProcessKey,
}

impl ProviderResolver {
    pub fn new(
        store: Store,
        drivers: Arc<DriverRegistry>,
        local_environment_id: EnvironmentId,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            drivers,
            local_environment_id,
            data_dir: Arc::new(RwLock::new(data_dir)),
            process_counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_data_dir(&self, data_dir: PathBuf) {
        *self.data_dir.write().expect("provider data directory lock") = data_dir;
    }

    pub fn local_environment_id(&self) -> &EnvironmentId {
        &self.local_environment_id
    }

    pub fn default_instance_id(
        &self,
        environment_id: &EnvironmentId,
    ) -> Result<ProviderInstanceId, RpcError> {
        let settings = self.settings()?;
        let mut candidates = settings.providers.iter().filter_map(|config| {
            self.resolve_config(config, environment_id, true)
                .ok()
                .map(|resolved| resolved.id)
        });
        let first = candidates.next().ok_or_else(|| {
            provider_error(
                None,
                environment_id,
                "provider_instance_missing",
                "no enabled provider instance is configured for this environment",
            )
        })?;
        Ok(first)
    }

    pub fn resolve(
        &self,
        instance_id: &ProviderInstanceId,
        environment_id: &EnvironmentId,
    ) -> Result<ResolvedProviderInstance, RpcError> {
        self.resolve_with_enabled(instance_id, environment_id, true)
    }

    pub fn resolve_allow_disabled(
        &self,
        instance_id: &ProviderInstanceId,
        environment_id: &EnvironmentId,
    ) -> Result<ResolvedProviderInstance, RpcError> {
        self.resolve_with_enabled(instance_id, environment_id, false)
    }

    pub fn resolve_kind(
        &self,
        requested_instance_id: Option<&ProviderInstanceId>,
        environment_id: &EnvironmentId,
        expected_kind: ProviderKind,
    ) -> Result<ResolvedProviderInstance, RpcError> {
        if let Some(instance_id) = requested_instance_id {
            let resolved = self.resolve(instance_id, environment_id)?;
            if resolved.config.provider != expected_kind {
                return Err(provider_error(
                    Some(instance_id),
                    environment_id,
                    "provider_instance_incompatible",
                    format!(
                        "provider instance '{}' is {}, not {}",
                        instance_id,
                        resolved.config.provider.as_str(),
                        expected_kind.as_str()
                    ),
                ));
            }
            return Ok(resolved);
        }

        let settings = self.settings()?;
        let mut matches = Vec::new();
        for config in &settings.providers {
            if config.provider != expected_kind {
                continue;
            }
            if let Ok(resolved) = self.resolve(&config.id, environment_id) {
                matches.push(resolved);
            }
        }
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(provider_error(
                None,
                environment_id,
                "provider_instance_missing",
                format!(
                    "no enabled {} provider instance is configured for this environment",
                    expected_kind.as_str()
                ),
            )),
            _ => Err(provider_error(
                None,
                environment_id,
                "provider_instance_ambiguous",
                format!(
                    "multiple {} provider instances are configured; select an exact instance id",
                    expected_kind.as_str()
                ),
            )),
        }
    }

    pub async fn acquire_execution(
        &self,
        instance_id: &ProviderInstanceId,
        environment_id: &EnvironmentId,
    ) -> Result<ResolvedProviderExecution, RpcError> {
        let instance = self.resolve(instance_id, environment_id)?;
        let snapshot = instance.driver.probe(&instance.config).await;
        validate_snapshot(instance_id, environment_id, &snapshot)?;

        let key = ProviderProcessKey {
            environment_id: environment_id.clone(),
            instance_id: instance_id.clone(),
        };
        let permit = self.try_acquire_process(key, instance.config.concurrency_limit)?;
        Ok(ResolvedProviderExecution {
            instance,
            snapshot,
            _permit: Arc::new(permit),
        })
    }

    pub fn active_processes(
        &self,
        instance_id: &ProviderInstanceId,
        environment_id: &EnvironmentId,
    ) -> usize {
        let key = ProviderProcessKey {
            environment_id: environment_id.clone(),
            instance_id: instance_id.clone(),
        };
        self.process_counts
            .lock()
            .expect("provider process count lock")
            .get(&key)
            .copied()
            .unwrap_or(0)
    }

    fn resolve_with_enabled(
        &self,
        instance_id: &ProviderInstanceId,
        environment_id: &EnvironmentId,
        require_enabled: bool,
    ) -> Result<ResolvedProviderInstance, RpcError> {
        let settings = self.settings()?;
        let config = settings
            .providers
            .iter()
            .find(|candidate| &candidate.id == instance_id)
            .ok_or_else(|| {
                provider_error(
                    Some(instance_id),
                    environment_id,
                    "provider_instance_missing",
                    format!("provider instance '{instance_id}' does not exist"),
                )
            })?;
        let resolved = self.resolve_config(config, environment_id, require_enabled)?;
        let driver = self.drivers.get(resolved.provider).ok_or_else(|| {
            provider_error(
                Some(instance_id),
                environment_id,
                "provider_driver_unavailable",
                format!(
                    "no driver is registered for provider {}",
                    resolved.provider.as_str()
                ),
            )
        })?;
        let metadata = driver.metadata();
        if metadata.id != resolved.provider {
            return Err(provider_error(
                Some(instance_id),
                environment_id,
                "provider_driver_incompatible",
                "the registered driver does not match the stored provider kind",
            ));
        }
        Ok(ResolvedProviderInstance {
            config: resolved,
            driver,
            capabilities: metadata.capabilities,
        })
    }

    fn resolve_config(
        &self,
        config: &ProviderInstanceConfig,
        environment_id: &EnvironmentId,
        require_enabled: bool,
    ) -> Result<ProviderInstanceConfig, RpcError> {
        if let Some(config_environment_id) = config.environment_id.as_ref()
            && config_environment_id != environment_id
        {
            return Err(provider_error(
                Some(&config.id),
                environment_id,
                "provider_instance_incompatible",
                format!(
                    "provider instance '{}' belongs to environment '{}'",
                    config.id, config_environment_id
                ),
            ));
        }

        let settings = self.settings()?;
        let environment_override = settings
            .environments
            .iter()
            .find(|environment| &environment.id == environment_id)
            .and_then(|environment| environment.provider_overrides.get(config.id.as_str()));
        let mut resolved = config.resolve_for_environment(environment_id, environment_override);
        if require_enabled && !resolved.enabled {
            let mut error = provider_error(
                Some(&config.id),
                environment_id,
                "provider_instance_disabled",
                format!("provider instance '{}' is disabled", config.id),
            );
            if let Some(disabled_reason) = resolved.disabled_reason.as_ref()
                && let Some(data) = error
                    .data
                    .as_mut()
                    .and_then(serde_json::Value::as_object_mut)
            {
                data.insert(
                    "disabledReason".to_string(),
                    serde_json::to_value(disabled_reason)
                        .expect("provider disabled reason serializes"),
                );
            }
            return Err(error);
        }
        if resolved.concurrency_limit == 0 {
            return Err(provider_error(
                Some(&config.id),
                environment_id,
                "provider_concurrency_invalid",
                "provider concurrency limit must be at least one",
            ));
        }

        let profile_dir = PathBuf::from(&resolved.profile_dir);
        if !profile_dir.is_absolute() {
            resolved.profile_dir = self
                .data_dir
                .read()
                .expect("provider data directory lock")
                .join(profile_dir)
                .to_string_lossy()
                .to_string();
        }
        Ok(resolved)
    }

    fn settings(&self) -> Result<pandamux_core::UserSettings, RpcError> {
        self.store
            .get_settings()
            .map_err(|error| RpcError::internal_error(error.to_string()))
            .map(|settings| settings.unwrap_or_default())
    }

    fn try_acquire_process(
        &self,
        key: ProviderProcessKey,
        limit: u32,
    ) -> Result<ProviderProcessPermit, RpcError> {
        let mut counts = self
            .process_counts
            .lock()
            .expect("provider process count lock");
        let active = counts.entry(key.clone()).or_insert(0);
        if *active >= limit as usize {
            return Err(provider_error(
                Some(&key.instance_id),
                &key.environment_id,
                "provider_capacity_reached",
                format!(
                    "provider instance '{}' has reached its concurrency limit of {limit}",
                    key.instance_id
                ),
            ));
        }
        *active += 1;
        Ok(ProviderProcessPermit {
            counts: self.process_counts.clone(),
            key,
        })
    }
}

impl Drop for ProviderProcessPermit {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().expect("provider process count lock");
        if let Some(active) = counts.get_mut(&self.key) {
            *active = active.saturating_sub(1);
            if *active == 0 {
                counts.remove(&self.key);
            }
        }
    }
}

fn validate_snapshot(
    instance_id: &ProviderInstanceId,
    environment_id: &EnvironmentId,
    snapshot: &ProviderSnapshot,
) -> Result<(), RpcError> {
    if !snapshot.installed {
        return Err(provider_error(
            Some(instance_id),
            environment_id,
            "provider_not_installed",
            format!("provider instance '{instance_id}' is not installed"),
        ));
    }
    if let ProviderHealth::Unavailable { reason } = &snapshot.health {
        return Err(provider_error(
            Some(instance_id),
            environment_id,
            "provider_unavailable",
            reason,
        ));
    }
    if snapshot.auth == ProviderAuthStatus::Unauthenticated {
        return Err(provider_error(
            Some(instance_id),
            environment_id,
            "provider_unauthenticated",
            format!("provider instance '{instance_id}' is not authenticated"),
        ));
    }
    Ok(())
}

fn provider_error(
    instance_id: Option<&ProviderInstanceId>,
    environment_id: &EnvironmentId,
    reason: &str,
    message: impl Into<String>,
) -> RpcError {
    RpcError::with_data(
        -32020,
        message,
        serde_json::json!({
            "providerInstanceId": instance_id,
            "environmentId": environment_id,
            "reason": reason,
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pandamux_core::{
        Environment, EnvironmentStatus, UserSettings,
        environment::EnvironmentKind,
        provider_config::{ProviderEnvironmentOverride, ProviderInstanceConfig},
    };
    use pandamux_providers::MockProviderDriver;

    use super::*;

    fn config(id: &str, limit: u32) -> ProviderInstanceConfig {
        ProviderInstanceConfig {
            id: ProviderInstanceId::from(id),
            provider: ProviderKind::Claude,
            display_name: id.to_string(),
            profile_dir: format!("profiles/claude/{id}"),
            environment_id: None,
            binary_path: Some(format!("/bin/{id}")),
            concurrency_limit: limit,
            env_overrides: BTreeMap::new(),
            settings: serde_json::json!({ "instance": id }),
            enabled: true,
            disabled_reason: None,
        }
    }

    fn resolver(settings: UserSettings) -> ProviderResolver {
        let store = Store::in_memory().expect("store");
        store.save_settings(&settings).expect("save settings");
        let drivers = Arc::new(DriverRegistry::new());
        drivers.register(
            ProviderKind::Claude,
            Arc::new(MockProviderDriver::new(ProviderKind::Claude, "Mock Claude")),
        );
        ProviderResolver::new(
            store,
            drivers,
            EnvironmentId::from("env-local"),
            PathBuf::from("/data/pandamux"),
        )
    }

    #[test]
    fn resolves_exact_instance_and_environment_override() {
        let mut settings = UserSettings {
            providers: vec![config("claude-work", 2)],
            ..UserSettings::default()
        };
        settings.environments = vec![Environment {
            id: EnvironmentId::from("env-local"),
            kind: EnvironmentKind::Local,
            display_name: "Local".to_string(),
            status: EnvironmentStatus::Ready,
            platform: None,
            server_version: None,
            schedules_count: None,
            last_error: None,
            provider_overrides: BTreeMap::from([(
                "claude-work".to_string(),
                ProviderEnvironmentOverride {
                    binary_path: Some("/opt/claude-work".to_string()),
                    env_overrides: BTreeMap::from([("ACCOUNT".to_string(), "work".to_string())]),
                    settings: Some(serde_json::json!({ "account": "work" })),
                    ..ProviderEnvironmentOverride::default()
                },
            )]),
        }];
        let resolver = resolver(settings);
        let resolved = resolver
            .resolve(
                &ProviderInstanceId::from("claude-work"),
                &EnvironmentId::from("env-local"),
            )
            .expect("resolve exact instance");

        assert_eq!(
            resolved.config.binary_path.as_deref(),
            Some("/opt/claude-work")
        );
        assert_eq!(resolved.config.env_overrides["ACCOUNT"], "work");
        assert_eq!(resolved.config.settings["account"], "work");
        assert_eq!(
            resolved.config.profile_dir,
            "/data/pandamux/profiles/claude/claude-work"
        );
    }

    #[test]
    fn disabled_instance_error_preserves_the_structured_reason() {
        let mut disabled = config("cursor-default", 1);
        disabled.provider = ProviderKind::Cursor;
        disabled.enabled = false;
        disabled.disabled_reason = Some(pandamux_core::ProviderDisabledReason {
            code: "live_acceptance_incomplete".to_string(),
            message: "Cursor live acceptance has not passed".to_string(),
            remediation: Some("Run the common provider smoke".to_string()),
        });
        let settings = UserSettings {
            providers: vec![disabled],
            ..UserSettings::default()
        };
        let store = Store::in_memory().expect("store");
        store.save_settings(&settings).expect("save settings");
        let drivers = Arc::new(DriverRegistry::default_local());
        let resolver = ProviderResolver::new(
            store,
            drivers,
            EnvironmentId::from("env-local"),
            PathBuf::from("/data/pandamux"),
        );

        let error = match resolver.resolve(
            &ProviderInstanceId::from("cursor-default"),
            &EnvironmentId::from("env-local"),
        ) {
            Ok(_) => panic!("disabled provider must not resolve for execution"),
            Err(error) => error,
        };

        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.pointer("/disabledReason/code"))
                .and_then(serde_json::Value::as_str),
            Some("live_acceptance_incomplete")
        );
    }

    #[tokio::test]
    async fn enforces_and_releases_per_instance_capacity() {
        let settings = UserSettings {
            providers: vec![config("claude-one", 1)],
            ..UserSettings::default()
        };
        let resolver = resolver(settings);
        let instance_id = ProviderInstanceId::from("claude-one");
        let environment_id = EnvironmentId::from("env-local");
        let first = resolver
            .acquire_execution(&instance_id, &environment_id)
            .await
            .expect("first permit");
        let error = match resolver
            .acquire_execution(&instance_id, &environment_id)
            .await
        {
            Ok(_) => panic!("second permit must be refused"),
            Err(error) => error,
        };
        assert_eq!(
            error.data.expect("error data")["reason"],
            "provider_capacity_reached"
        );
        drop(first);
        resolver
            .acquire_execution(&instance_id, &environment_id)
            .await
            .expect("permit released");
    }

    #[test]
    fn missing_instance_never_falls_back_to_custom() {
        let resolver = resolver(UserSettings {
            providers: vec![config("claude-one", 1)],
            ..UserSettings::default()
        });
        let error = match resolver.resolve(
            &ProviderInstanceId::from("prov-default"),
            &EnvironmentId::from("env-local"),
        ) {
            Ok(_) => panic!("missing instance must fail"),
            Err(error) => error,
        };
        assert_eq!(
            error.data.expect("error data")["reason"],
            "provider_instance_missing"
        );
    }
}
