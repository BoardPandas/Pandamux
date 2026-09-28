//! Secrets management and security policy.
//!
//! Provides secret storage abstractions, secret masking for UI display,
//! redaction of secrets in logs and memory, and exclusion of secret values
//! during private repository backups.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

/// Errors encountered during secret storage or retrieval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretError {
    NotFound(String),
    Storage(String),
    Validation(String),
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(k) => write!(f, "Secret not found: {k}"),
            Self::Storage(msg) => write!(f, "Secret storage error: {msg}"),
            Self::Validation(msg) => write!(f, "Secret validation error: {msg}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// Abstract interface for secret storage (keyring on hub, in-memory for testing/node).
pub trait SecretStore: Send + Sync {
    /// Stores or updates a secret by key.
    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretError>;

    /// Retrieves a secret by key if present.
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretError>;

    /// Deletes a secret by key. Returns true if key was present.
    fn delete_secret(&self, key: &str) -> Result<bool, SecretError>;

    /// Lists all stored secret keys in alphabetical order.
    fn list_keys(&self) -> Result<Vec<String>, SecretError>;
}

/// Pure in-memory secret storage for unit tests and in-memory node sessions.
#[derive(Clone, Default)]
pub struct InMemorySecretStore {
    secrets: Arc<RwLock<HashMap<String, String>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretError> {
        let mut map = self
            .secrets
            .write()
            .map_err(|e| SecretError::Storage(e.to_string()))?;
        map.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretError> {
        let map = self
            .secrets
            .read()
            .map_err(|e| SecretError::Storage(e.to_string()))?;
        Ok(map.get(key).cloned())
    }

    fn delete_secret(&self, key: &str) -> Result<bool, SecretError> {
        let mut map = self
            .secrets
            .write()
            .map_err(|e| SecretError::Storage(e.to_string()))?;
        Ok(map.remove(key).is_some())
    }

    fn list_keys(&self) -> Result<Vec<String>, SecretError> {
        let map = self
            .secrets
            .read()
            .map_err(|e| SecretError::Storage(e.to_string()))?;
        let mut keys: Vec<String> = map.keys().cloned().collect();
        keys.sort();
        Ok(keys)
    }
}

/// Security policies for secret redaction, masking, and backup filtering.
pub struct SecretsPolicy;

impl SecretsPolicy {
    /// Mask secret value for write-only UI display (never echoes cleartext).
    pub fn mask_secret(_val: &str) -> &'static str {
        "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"
    }

    /// Determines if an environment variable name designates a secret.
    pub fn is_secret_env_var(name: &str) -> bool {
        let upper = name.to_ascii_uppercase();
        upper.contains("_KEY")
            || upper.contains("_SECRET")
            || upper.contains("_TOKEN")
            || upper.contains("_PASSWORD")
            || upper.contains("_AUTH")
            || upper.contains("_CREDENTIAL")
            || upper.contains("_BEARER")
            || upper.contains("_PASSPHRASE")
            || upper.starts_with("API_KEY")
            || upper.starts_with("ACCESS_TOKEN")
    }

    /// Detects common API key and token signatures in arbitrary text.
    pub fn contains_potential_secret(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        text.contains("sk-ant-")
            || text.contains("sk-proj-")
            || text.contains("ghp_")
            || text.contains("github_pat_")
            || text.contains("glpat-")
            || text.contains("xoxb-")
            || text.contains("xoxp-")
            || lower.contains("bearer ")
    }

    /// Redacts known secrets from logs and text outputs.
    pub fn redact_secrets(text: &str, known_secrets: &[&str]) -> String {
        let mut result = text.to_string();
        for &secret in known_secrets {
            if !secret.trim().is_empty() && secret.len() >= 4 {
                result = result.replace(secret, "[REDACTED_SECRET]");
            }
        }
        result
    }

    /// Scrubs secrets from memory notes before commit or backup push.
    pub fn scrub_memory(content: &str, known_secrets: &[&str]) -> String {
        let redacted = Self::redact_secrets(content, known_secrets);
        let mut lines = Vec::new();
        for line in redacted.lines() {
            if Self::contains_potential_secret(line) {
                // If the line contains a secret indicator, redact potential tokens
                let words: Vec<&str> = line.split_whitespace().collect();
                let mut sanitized_words = Vec::new();
                for word in words {
                    if word.starts_with("sk-ant-")
                        || word.starts_with("sk-proj-")
                        || word.starts_with("ghp_")
                        || word.starts_with("github_pat_")
                        || word.starts_with("glpat-")
                        || word.starts_with("xoxb-")
                        || word.starts_with("xoxp-")
                    {
                        sanitized_words.push("[REDACTED_TOKEN]");
                    } else {
                        sanitized_words.push(word);
                    }
                }
                lines.push(sanitized_words.join(" "));
            } else {
                lines.push(line.to_string());
            }
        }
        lines.join("\n")
    }

    /// Filters environment variables for backup export.
    /// In accordance with backup rules, variable names are preserved, but values
    /// are stripped so cleartext secrets and environment values never leave the host.
    pub fn filter_env_vars_for_backup(
        env_vars: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut filtered = BTreeMap::new();
        for key in env_vars.keys() {
            filtered.insert(key.clone(), String::new());
        }
        filtered
    }
}

/// Helper to mask secrets in write-only UI views.
pub fn mask_secret(val: &str) -> &'static str {
    SecretsPolicy::mask_secret(val)
}

/// Helper to redact known secrets from a string.
pub fn redact_secrets(text: &str, known_secrets: &[&str]) -> String {
    SecretsPolicy::redact_secrets(text, known_secrets)
}

/// Helper to scrub memory notes before commit or backup.
pub fn scrub_memory(content: &str, known_secrets: &[&str]) -> String {
    SecretsPolicy::scrub_memory(content, known_secrets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_secret_store() {
        let store = InMemorySecretStore::new();
        assert_eq!(store.get_secret("test_key").unwrap(), None);

        store.set_secret("test_key", "secret_val").unwrap();
        assert_eq!(
            store.get_secret("test_key").unwrap(),
            Some("secret_val".to_string())
        );

        let keys = store.list_keys().unwrap();
        assert_eq!(keys, vec!["test_key"]);

        let deleted = store.delete_secret("test_key").unwrap();
        assert!(deleted);
        assert_eq!(store.get_secret("test_key").unwrap(), None);
    }

    #[test]
    fn test_secret_env_var_detection() {
        assert!(SecretsPolicy::is_secret_env_var("ANTHROPIC_API_KEY"));
        assert!(SecretsPolicy::is_secret_env_var("OPENAI_API_SECRET"));
        assert!(SecretsPolicy::is_secret_env_var("GITHUB_TOKEN"));
        assert!(SecretsPolicy::is_secret_env_var("DB_PASSWORD"));
        assert!(SecretsPolicy::is_secret_env_var("BEARER_AUTH"));
        assert!(!SecretsPolicy::is_secret_env_var("PATH"));
        assert!(!SecretsPolicy::is_secret_env_var("HOME"));
        assert!(!SecretsPolicy::is_secret_env_var("RUST_LOG"));
    }

    #[test]
    fn test_redact_secrets() {
        let known = ["super-secret-key-1234", "another-secret"];
        let log = "Connecting with key super-secret-key-1234 to remote endpoint";
        let redacted = SecretsPolicy::redact_secrets(log, &known);
        assert_eq!(
            redacted,
            "Connecting with key [REDACTED_SECRET] to remote endpoint"
        );
    }

    #[test]
    fn test_scrub_memory_detects_tokens() {
        let notes = "User mentioned using token sk-ant-api03-abcdefg for anthropic driver";
        let scrubbed = SecretsPolicy::scrub_memory(notes, &[]);
        assert!(scrubbed.contains("[REDACTED_TOKEN]"));
        assert!(!scrubbed.contains("sk-ant-api03-abcdefg"));
    }

    #[test]
    fn test_filter_env_vars_for_backup() {
        let mut envs = BTreeMap::new();
        envs.insert("API_KEY".to_string(), "actual-secret-123".to_string());
        envs.insert("DEBUG".to_string(), "1".to_string());

        let backed_up = SecretsPolicy::filter_env_vars_for_backup(&envs);
        assert_eq!(backed_up.get("API_KEY"), Some(&String::new()));
        assert_eq!(backed_up.get("DEBUG"), Some(&String::new()));
        assert!(!backed_up.values().any(|v| v == "actual-secret-123"));
    }
}
