/// Actions dispatched from system tray interactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ToggleMain,
    ToggleScratchpad,
    OpenSettings,
    Quit,
}

/// An entry in the system tray context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenuItem {
    pub id: &'static str,
    pub label: &'static str,
    pub action: TrayAction,
    pub shortcut: Option<&'static str>,
}

/// System tray icon controller and global hotkey manager.
///
/// Coordinates the persistent system tray presence, global quick-summon hotkey
/// (`CmdOrCtrl+Shift+Panda` / `Win+Alt+P`), and popup context menu actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemTrayState {
    pub is_visible: bool,
    pub tooltip: String,
    pub hotkey_display: &'static str,
    pub menu_items: Vec<TrayMenuItem>,
}

impl Default for SystemTrayState {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemTrayState {
    /// Creates a new system tray state configured with the platform-appropriate hotkey.
    pub fn new() -> Self {
        let hotkey_display = if cfg!(target_os = "windows") {
            "Win+Alt+P"
        } else if cfg!(target_os = "macos") {
            "⌘⇧P"
        } else {
            "Ctrl+Shift+P"
        };

        Self {
            is_visible: true,
            tooltip: "PandaMUX: AI Agent Terminal Multiplexer".to_string(),
            hotkey_display,
            menu_items: Self::default_menu_items(hotkey_display),
        }
    }

    /// Constructs the default system tray context menu catalog.
    pub fn default_menu_items(hotkey: &'static str) -> Vec<TrayMenuItem> {
        vec![
            TrayMenuItem {
                id: "show_main",
                label: "Show PandaMUX",
                action: TrayAction::ToggleMain,
                shortcut: None,
            },
            TrayMenuItem {
                id: "scratchpad",
                label: "Quick Scratchpad",
                action: TrayAction::ToggleScratchpad,
                shortcut: Some(hotkey),
            },
            TrayMenuItem {
                id: "settings",
                label: "Settings...",
                action: TrayAction::OpenSettings,
                shortcut: Some("Ctrl+,"),
            },
            TrayMenuItem {
                id: "quit",
                label: "Quit PandaMUX",
                action: TrayAction::Quit,
                shortcut: None,
            },
        ]
    }

    /// Updates tooltip text with live fleet summary (e.g. running agent count).
    pub fn set_tooltip(&mut self, active_agents: usize) {
        if active_agents > 0 {
            self.tooltip = format!("PandaMUX ({active_agents} active agents)");
        } else {
            self.tooltip = "PandaMUX: Idle".to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn test_system_tray_state_defaults() {
        let tray = SystemTrayState::new();
        assert!(tray.is_visible);
        assert!(!tray.hotkey_display.is_empty());
        assert_eq!(tray.menu_items.len(), 4);

        let scratchpad_item = tray
            .menu_items
            .iter()
            .find(|i| i.id == "scratchpad")
            .unwrap();
        assert_eq!(scratchpad_item.action, TrayAction::ToggleScratchpad);
        assert_eq!(scratchpad_item.shortcut, Some(tray.hotkey_display));
    }

    #[test]
    fn test_system_tray_tooltip_updates() {
        let mut tray = SystemTrayState::new();
        tray.set_tooltip(3);
        assert_eq!(tray.tooltip, "PandaMUX (3 active agents)");

        tray.set_tooltip(0);
        assert_eq!(tray.tooltip, "PandaMUX: Idle");
    }
}
