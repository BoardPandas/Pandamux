use pandamux_core::{ProviderInstanceId, ProviderKind, UserSettings};
use pandamux_protocol::{ProviderHealthReport, ProviderModelsReport};

/// Represents a model option for a provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub display_name: String,
    pub supports_reasoning: bool,
}

/// Represents a provider option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderChoice {
    pub id: String,
    pub display_name: String,
    pub badge: String,
    pub models: Vec<ModelChoice>,
}

/// State for the Provider / Model / Effort picker component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerState {
    pub provider_id: String,
    pub model_id: String,
    pub effort: Option<String>,
    pub providers: Vec<ProviderChoice>,
    pub show_provider_dropdown: bool,
    pub show_model_dropdown: bool,
    pub show_effort_dropdown: bool,
}

impl Default for PickerState {
    fn default() -> Self {
        Self::new()
    }
}

impl PickerState {
    /// Creates an empty picker. Configured, authenticated instances and their
    /// discovered models are synchronized after the desktop connects.
    pub fn new() -> Self {
        Self {
            provider_id: String::new(),
            model_id: String::new(),
            effort: Some("medium".to_string()),
            providers: Vec::new(),
            show_provider_dropdown: false,
            show_model_dropdown: false,
            show_effort_dropdown: false,
        }
    }

    /// Rebuilds picker choices from configured instance identities, successful
    /// authentication evidence, and bounded provider model discovery.
    pub fn synchronize(
        &mut self,
        settings: &UserSettings,
        health_reports: &[ProviderHealthReport],
        model_reports: &[ProviderModelsReport],
    ) {
        let previous_provider = self.provider_id.clone();
        let previous_model = self.model_id.clone();
        let mut providers = Vec::new();

        for config in settings.providers.iter().filter(|config| config.enabled) {
            let environment_id = config
                .environment_id
                .as_ref()
                .map(|environment_id| environment_id.as_str())
                .unwrap_or("env-local");
            let authenticated = health_reports.iter().any(|report| {
                report.instance_id == config.id
                    && report.environment_id.as_str() == environment_id
                    && report.enabled
                    && report.status == "healthy"
                    && report.authentication_state == "authenticated"
            });
            if !authenticated {
                continue;
            }
            let Some(discovery) = model_reports.iter().find(|report| {
                report.instance_id == config.id
                    && report.environment_id.as_str() == environment_id
                    && report.enabled
                    && report.error.is_none()
                    && !report.models.is_empty()
            }) else {
                continue;
            };
            let models = discovery
                .models
                .iter()
                .map(|model| ModelChoice {
                    id: model.id.clone(),
                    display_name: model.display_name.clone(),
                    supports_reasoning: model.supports_reasoning,
                })
                .collect::<Vec<_>>();
            providers.push(ProviderChoice {
                id: config.id.as_str().to_string(),
                display_name: config.display_name.clone(),
                badge: provider_badge(config.provider).to_string(),
                models,
            });
        }

        self.providers = providers;
        if self.providers.is_empty() {
            self.provider_id.clear();
            self.model_id.clear();
            self.show_provider_dropdown = false;
            self.show_model_dropdown = false;
            return;
        }

        self.provider_id = if self
            .providers
            .iter()
            .any(|provider| provider.id == previous_provider)
        {
            previous_provider
        } else {
            self.providers[0].id.clone()
        };
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.id == self.provider_id)
            .expect("selected provider comes from synchronized choices");
        self.model_id = if provider
            .models
            .iter()
            .any(|model| model.id == previous_model)
        {
            previous_model
        } else {
            provider
                .models
                .first()
                .map(|model| model.id.clone())
                .unwrap_or_default()
        };
        if !provider
            .models
            .iter()
            .find(|model| model.id == self.model_id)
            .is_some_and(|model| model.supports_reasoning)
        {
            self.effort = None;
        }
    }

    /// Selects a provider by id and automatically chooses its primary model.
    pub fn select_provider(&mut self, provider_id: &str) {
        if let Some(prov) = self.providers.iter().find(|p| p.id == provider_id) {
            self.provider_id = prov.id.clone();
            if let Some(first_model) = prov.models.first() {
                self.model_id = first_model.id.clone();
            }
        }
        self.show_provider_dropdown = false;
    }

    /// Selects a model by id.
    pub fn select_model(&mut self, model_id: &str) {
        self.model_id = model_id.to_string();
        self.show_model_dropdown = false;
    }

    /// Selects reasoning effort level.
    pub fn select_effort(&mut self, effort: Option<String>) {
        self.effort = effort;
        self.show_effort_dropdown = false;
    }

    /// Cycles to the next available provider.
    pub fn cycle_provider(&mut self) {
        if self.providers.is_empty() {
            return;
        }
        let current_idx = self
            .providers
            .iter()
            .position(|p| p.id == self.provider_id)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.providers.len();
        let next_provider_id = self.providers[next_idx].id.clone();
        self.select_provider(&next_provider_id);
    }

    /// Cycles to the next available model for the active provider.
    pub fn cycle_model(&mut self) {
        if let Some(prov) = self.providers.iter().find(|p| p.id == self.provider_id)
            && !prov.models.is_empty()
        {
            let current_idx = prov
                .models
                .iter()
                .position(|m| m.id == self.model_id)
                .unwrap_or(0);
            let next_idx = (current_idx + 1) % prov.models.len();
            self.model_id = prov.models[next_idx].id.clone();
        }
    }

    /// Cycles reasoning effort: None -> Low -> Medium -> High -> Max -> None.
    pub fn cycle_effort(&mut self) {
        self.effort = match self.effort.as_deref() {
            None => Some("low".to_string()),
            Some("low") => Some("medium".to_string()),
            Some("medium") => Some("high".to_string()),
            Some("high") => Some("max".to_string()),
            _ => None,
        };
    }

    /// Returns the active `ProviderInstanceId`.
    pub fn provider_instance_id(&self) -> Option<ProviderInstanceId> {
        (!self.provider_id.is_empty()).then(|| ProviderInstanceId::from(self.provider_id.as_str()))
    }

    /// Returns the active model string.
    pub fn model(&self) -> Option<&str> {
        (!self.model_id.is_empty()).then_some(self.model_id.as_str())
    }

    /// Returns the active effort string.
    pub fn effort_str(&self) -> Option<&str> {
        self.effort.as_deref()
    }

    /// Returns current provider display name.
    pub fn current_provider_name(&self) -> &str {
        self.providers
            .iter()
            .find(|p| p.id == self.provider_id)
            .map(|p| p.display_name.as_str())
            .unwrap_or("No provider")
    }

    /// Returns current model display name.
    pub fn current_model_name(&self) -> &str {
        if let Some(prov) = self.providers.iter().find(|p| p.id == self.provider_id)
            && let Some(model) = prov.models.iter().find(|m| m.id == self.model_id)
        {
            return &model.display_name;
        }
        if self.model_id.is_empty() {
            "No model"
        } else {
            &self.model_id
        }
    }
}

fn provider_badge(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "🟣 Anthropic",
        ProviderKind::Codex => "🟢 OpenAI",
        ProviderKind::Antigravity => "🔵 Google",
        ProviderKind::Cursor => "🟣 Cursor",
        ProviderKind::Grok => "⚫ xAI",
        ProviderKind::OpenCode => "🟠 OpenCode",
        ProviderKind::Custom => "⚪ Custom",
    }
}
