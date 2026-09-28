use pandamux_protocol::hark_rpc::{
    HarkDictatePromptParams, HarkDictatePromptResult, HarkSpellbookEntry, HarkSpellbookSyncResult,
    HarkStatusResult,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Default IPC endpoint for communication with the Hark voice assistant daemon.
#[cfg(windows)]
pub const DEFAULT_HARK_PIPE_PATH: &str = r"\\.\pipe\hark-pandamux";
#[cfg(not(windows))]
pub const DEFAULT_HARK_PIPE_PATH: &str = "/tmp/hark-pandamux.sock";

/// Bridge maintaining connection state, symbol vocabulary, and prompt streaming with Hark.
#[derive(Clone, Debug)]
pub struct HarkBridge {
    pipe_path: String,
    spellbook: Arc<RwLock<Vec<HarkSpellbookEntry>>>,
    connected: Arc<AtomicBool>,
}

impl Default for HarkBridge {
    fn default() -> Self {
        Self::new(DEFAULT_HARK_PIPE_PATH)
    }
}

impl HarkBridge {
    pub fn new(pipe_path: impl Into<String>) -> Self {
        Self {
            pipe_path: pipe_path.into(),
            spellbook: Arc::new(RwLock::new(Vec::new())),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn pipe_path(&self) -> &str {
        &self.pipe_path
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::SeqCst);
    }

    /// Synchronizes a set of symbols into the Hark spellbook, deduplicating existing entries.
    pub fn sync_spellbook(&self, symbols: Vec<HarkSpellbookEntry>) -> HarkSpellbookSyncResult {
        let mut book = self.spellbook.write().unwrap();
        let mut existing_symbols: HashSet<String> =
            book.iter().map(|s| s.symbol.to_lowercase()).collect();
        let mut added_count = 0;

        for entry in symbols {
            let key = entry.symbol.to_lowercase();
            if !existing_symbols.contains(&key) && !entry.symbol.trim().is_empty() {
                existing_symbols.insert(key);
                book.push(entry);
                added_count += 1;
            }
        }

        let status = if self.is_connected() {
            "synchronized_live"
        } else {
            "cached_for_daemon_connect"
        };

        HarkSpellbookSyncResult {
            synchronized_count: added_count,
            pipe_path: self.pipe_path.clone(),
            status: status.to_string(),
        }
    }

    /// Returns current status of the Hark native voice bridge.
    pub fn get_status(&self) -> HarkStatusResult {
        let count = self.spellbook.read().unwrap().len();
        let connected = self.is_connected();

        #[cfg(windows)]
        let daemon_available = connected || std::path::Path::new(&self.pipe_path).exists();
        #[cfg(not(windows))]
        let daemon_available = connected || std::path::Path::new(&self.pipe_path).exists();

        HarkStatusResult {
            connected,
            pipe_path: self.pipe_path.clone(),
            spellbook_count: count,
            daemon_available,
        }
    }

    /// Handles a transcribed prompt from Hark push-to-talk or voice trigger.
    pub fn handle_dictate_prompt(
        &self,
        params: HarkDictatePromptParams,
    ) -> HarkDictatePromptResult {
        let trimmed = params.text.trim();
        if trimmed.is_empty() {
            return HarkDictatePromptResult {
                accepted: false,
                text: String::new(),
            };
        }

        HarkDictatePromptResult {
            accepted: true,
            text: trimmed.to_string(),
        }
    }
}

/// Helper that extracts identifiers and keywords from source code for spellbook tuning.
pub fn extract_symbols_from_source(source: &str, category: &str) -> Vec<HarkSpellbookEntry> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();

    for token in source.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let token = token.trim();
        if token.len() >= 3
            && !seen.contains(token)
            && token.chars().next().is_some_and(|c| c.is_alphabetic())
        {
            seen.insert(token.to_string());
            let weight = if token.contains('_') || token.chars().any(|c| c.is_uppercase()) {
                1.5
            } else {
                1.0
            };
            symbols.push(HarkSpellbookEntry {
                symbol: token.to_string(),
                category: category.to_string(),
                weight,
            });
        }
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_symbols_from_source() {
        let code =
            "struct TerminalViewState { fn handle_resize(&mut self, cols: u16, rows: u16) {} }";
        let symbols = extract_symbols_from_source(code, "type");
        assert!(
            symbols
                .iter()
                .any(|s| s.symbol == "TerminalViewState" && s.weight > 1.0)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.symbol == "handle_resize" && s.weight > 1.0)
        );
    }

    #[test]
    fn test_hark_bridge_spellbook_sync_and_status() {
        let bridge = HarkBridge::new("/tmp/test-hark.sock");
        assert!(!bridge.is_connected());

        let initial_status = bridge.get_status();
        assert_eq!(initial_status.spellbook_count, 0);

        let entries = vec![
            HarkSpellbookEntry {
                symbol: "Galahad".to_string(),
                category: "environment".to_string(),
                weight: 2.0,
            },
            HarkSpellbookEntry {
                symbol: "PandaMUX".to_string(),
                category: "project".to_string(),
                weight: 2.5,
            },
            HarkSpellbookEntry {
                symbol: "galahad".to_string(), // duplicate lowercase
                category: "environment".to_string(),
                weight: 1.0,
            },
        ];

        let sync_result = bridge.sync_spellbook(entries);
        assert_eq!(sync_result.synchronized_count, 2);
        assert_eq!(sync_result.status, "cached_for_daemon_connect");

        let status_after = bridge.get_status();
        assert_eq!(status_after.spellbook_count, 2);

        // Dictate prompt handling
        let prompt_result = bridge.handle_dictate_prompt(HarkDictatePromptParams {
            text: "build the galahad release with cargo".to_string(),
            is_final: true,
            confidence: 0.98,
        });
        assert!(prompt_result.accepted);
        assert_eq!(prompt_result.text, "build the galahad release with cargo");
    }
}
