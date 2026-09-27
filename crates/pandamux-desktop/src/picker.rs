use pandamux_core::ProviderInstanceId;

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
    /// Creates a new picker initialized with Claude 3.7 Sonnet and high reasoning effort.
    pub fn new() -> Self {
        let providers = vec![
            ProviderChoice {
                id: "claude".to_string(),
                display_name: "Claude".to_string(),
                badge: "🟣 Anthropic".to_string(),
                models: vec![
                    ModelChoice {
                        id: "claude-3-7-sonnet".to_string(),
                        display_name: "Claude 3.7 Sonnet".to_string(),
                        supports_reasoning: true,
                    },
                    ModelChoice {
                        id: "claude-3-5-sonnet".to_string(),
                        display_name: "Claude 3.5 Sonnet".to_string(),
                        supports_reasoning: false,
                    },
                    ModelChoice {
                        id: "claude-3-5-haiku".to_string(),
                        display_name: "Claude 3.5 Haiku".to_string(),
                        supports_reasoning: false,
                    },
                ],
            },
            ProviderChoice {
                id: "codex".to_string(),
                display_name: "Codex".to_string(),
                badge: "🟢 OpenAI".to_string(),
                models: vec![
                    ModelChoice {
                        id: "o3-mini".to_string(),
                        display_name: "o3-mini".to_string(),
                        supports_reasoning: true,
                    },
                    ModelChoice {
                        id: "gpt-4.5-preview".to_string(),
                        display_name: "GPT-4.5 Preview".to_string(),
                        supports_reasoning: true,
                    },
                    ModelChoice {
                        id: "gpt-4o".to_string(),
                        display_name: "GPT-4o".to_string(),
                        supports_reasoning: false,
                    },
                ],
            },
            ProviderChoice {
                id: "antigravity".to_string(),
                display_name: "Antigravity".to_string(),
                badge: "🔵 Google".to_string(),
                models: vec![
                    ModelChoice {
                        id: "gemini-2.0-flash".to_string(),
                        display_name: "Gemini 2.0 Flash".to_string(),
                        supports_reasoning: false,
                    },
                    ModelChoice {
                        id: "gemini-2.0-flash-thinking".to_string(),
                        display_name: "Gemini 2.0 Flash Thinking".to_string(),
                        supports_reasoning: true,
                    },
                    ModelChoice {
                        id: "gemini-2.0-pro-exp".to_string(),
                        display_name: "Gemini 2.0 Pro Exp".to_string(),
                        supports_reasoning: true,
                    },
                ],
            },
        ];

        Self {
            provider_id: "claude".to_string(),
            model_id: "claude-3-7-sonnet".to_string(),
            effort: Some("high".to_string()),
            providers,
            show_provider_dropdown: false,
            show_model_dropdown: false,
            show_effort_dropdown: false,
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
        if let Some(prov) = self.providers.iter().find(|p| p.id == self.provider_id) {
            if !prov.models.is_empty() {
                let current_idx = prov
                    .models
                    .iter()
                    .position(|m| m.id == self.model_id)
                    .unwrap_or(0);
                let next_idx = (current_idx + 1) % prov.models.len();
                self.model_id = prov.models[next_idx].id.clone();
            }
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
    pub fn provider_instance_id(&self) -> ProviderInstanceId {
        ProviderInstanceId::from(self.provider_id.as_str())
    }

    /// Returns the active model string.
    pub fn model(&self) -> &str {
        &self.model_id
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
            .unwrap_or(&self.provider_id)
    }

    /// Returns current model display name.
    pub fn current_model_name(&self) -> &str {
        if let Some(prov) = self.providers.iter().find(|p| p.id == self.provider_id) {
            if let Some(model) = prov.models.iter().find(|m| m.id == self.model_id) {
                return &model.display_name;
            }
        }
        &self.model_id
    }
}
