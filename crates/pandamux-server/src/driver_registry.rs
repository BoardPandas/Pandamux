use pandamux_core::provider_config::ProviderKind;
use pandamux_providers::traits::ProviderDriver;
use pandamux_providers::{
    AntigravityDriver, ClaudeDriver, CodexDriver, CursorDriver, GrokDriver, MockProviderDriver,
    OpenCodeDriver,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// Thread-safe registry mapping provider kinds to their active drivers.
#[derive(Clone, Default)]
pub struct DriverRegistry {
    drivers: Arc<RwLock<HashMap<ProviderKind, Arc<dyn ProviderDriver>>>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self {
            drivers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers a driver for the specified provider kind.
    pub fn register(&self, kind: ProviderKind, driver: Arc<dyn ProviderDriver>) {
        let mut map = self.drivers.write().unwrap();
        map.insert(kind, driver);
    }

    /// Resolves the driver for a provider kind if available.
    pub fn get(&self, kind: ProviderKind) -> Option<Arc<dyn ProviderDriver>> {
        let map = self.drivers.read().unwrap();
        map.get(&kind).cloned()
    }

    /// Creates a registry initialized with default local drivers and mock fallback.
    pub fn default_local() -> Self {
        let registry = Self::new();

        let claude = Arc::new(ClaudeDriver::new(PathBuf::from("claude")));
        registry.register(ProviderKind::Claude, claude);

        let codex = Arc::new(CodexDriver::new(PathBuf::from("codex")));
        registry.register(ProviderKind::Codex, codex);

        let antigravity = Arc::new(AntigravityDriver::new(PathBuf::from(
            ".pandamux/antigravity",
        )));
        registry.register(ProviderKind::Antigravity, antigravity);

        let cursor = Arc::new(CursorDriver::new(PathBuf::from(".pandamux/cursor")));
        registry.register(ProviderKind::Cursor, cursor);

        let grok = Arc::new(GrokDriver::new(PathBuf::from(".pandamux/grok")));
        registry.register(ProviderKind::Grok, grok);

        let opencode = Arc::new(OpenCodeDriver::new(
            PathBuf::from(".pandamux/opencode"),
            false,
        ));
        registry.register(ProviderKind::OpenCode, opencode);

        let mock = Arc::new(MockProviderDriver::new(
            ProviderKind::Custom,
            "Mock Provider",
        ));
        registry.register(ProviderKind::Custom, mock);

        registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_registry_lookup() {
        let registry = DriverRegistry::default_local();
        assert!(registry.get(ProviderKind::Claude).is_some());
        assert!(registry.get(ProviderKind::Codex).is_some());
        assert!(registry.get(ProviderKind::Antigravity).is_some());
        assert!(registry.get(ProviderKind::Cursor).is_some());
        assert!(registry.get(ProviderKind::Grok).is_some());
        assert!(registry.get(ProviderKind::OpenCode).is_some());
        assert!(registry.get(ProviderKind::Custom).is_some());
    }
}
