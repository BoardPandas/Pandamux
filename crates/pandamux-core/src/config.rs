//! Theme model and parsers, ported from the Electron theme-loader/config-loader.
//!
//! A [`Theme`] holds the small set of colors the terminal UI needs: background,
//! foreground, cursor, selection background, and a 16 (or fewer) entry ANSI
//! palette. Themes are parsed either from the bundled Ghostty-style `.theme`
//! files or imported from a Windows Terminal `settings.json`. The [`ThemeStore`]
//! keeps the loaded set plus which one is active.

use serde::{Deserialize, Serialize};

/// Whether a theme reads as dark or light overall (used to pick UI chrome).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub cursor: Option<String>,
    pub selection_background: Option<String>,
    pub palette: Vec<String>,
}

/// Parse "#rrggbb" or "rrggbb" into (r, g, b). Returns None unless the string is
/// exactly 6 hex digits (with an optional leading '#').
pub fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Relative luminance (0..1) of a hex color; None if `hex` doesn't parse.
fn relative_luminance(hex: &str) -> Option<f64> {
    let (r, g, b) = parse_hex(hex)?;
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    Some(0.2126 * r + 0.7152 * g + 0.0722 * b)
}

/// Grow `palette` (filling gaps with an empty string) so index `index` is valid,
/// then set it.
fn set_palette_index(palette: &mut Vec<String>, index: usize, value: String) {
    if palette.len() <= index {
        palette.resize(index + 1, String::new());
    }
    palette[index] = value;
}

/// Parse a Ghostty-style theme file. Lines are `key = value`, blank lines and
/// `#` comments are ignored, and unknown keys are ignored. `palette` lines carry
/// their own `index=#hex` value (e.g. `palette = 0=#21222c`).
pub fn parse_ghostty_theme(name: impl Into<String>, content: &str) -> Theme {
    let mut background = None;
    let mut foreground = None;
    let mut cursor = None;
    let mut selection_background = None;
    let mut palette = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().to_string();

        match key {
            "background" => background = Some(value),
            "foreground" => foreground = Some(value),
            "cursor-color" => cursor = Some(value),
            "selection-background" => selection_background = Some(value),
            "palette" => {
                let Some((index, color)) = value.split_once('=') else {
                    continue;
                };
                let Ok(index) = index.trim().parse::<usize>() else {
                    continue;
                };
                set_palette_index(&mut palette, index, color.trim().to_string());
            }
            _ => {}
        }
    }

    let appearance = match background.as_deref().and_then(relative_luminance) {
        Some(luminance) if luminance >= 0.5 => Appearance::Light,
        _ => Appearance::Dark,
    };

    Theme {
        name: name.into(),
        appearance,
        background,
        foreground,
        cursor,
        selection_background,
        palette,
    }
}

/// ANSI color names in Windows Terminal's `settings.json` scheme objects, in
/// palette index order (0..15).
const WINDOWS_TERMINAL_ANSI_KEYS: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "purple",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightPurple",
    "brightCyan",
    "brightWhite",
];

/// Parse a Windows Terminal `settings.json`, returning one [`Theme`] per entry in
/// the top-level `schemes` array.
pub fn import_windows_terminal(content: &str) -> Result<Vec<Theme>, String> {
    let root: serde_json::Value =
        serde_json::from_str(content).map_err(|err| format!("invalid JSON: {err}"))?;

    let schemes = root
        .get("schemes")
        .and_then(|schemes| schemes.as_array())
        .ok_or_else(|| "missing top-level \"schemes\" array".to_string())?;

    let mut themes = Vec::with_capacity(schemes.len());
    for scheme in schemes {
        let name = scheme
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("Unnamed")
            .to_string();
        let background = scheme
            .get("background")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let foreground = scheme
            .get("foreground")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let cursor = scheme
            .get("cursorColor")
            .and_then(|value| value.as_str())
            .map(str::to_string);

        let mut palette = Vec::new();
        for (index, key) in WINDOWS_TERMINAL_ANSI_KEYS.iter().enumerate() {
            if let Some(value) = scheme.get(*key).and_then(|value| value.as_str()) {
                set_palette_index(&mut palette, index, value.to_string());
            }
        }

        let appearance = match background.as_deref().and_then(relative_luminance) {
            Some(luminance) if luminance >= 0.5 => Appearance::Light,
            _ => Appearance::Dark,
        };

        themes.push(Theme {
            name,
            appearance,
            background,
            foreground,
            cursor,
            selection_background: None,
            palette,
        });
    }

    Ok(themes)
}

/// The loaded set of themes plus which one is active.
#[derive(Clone, Debug, Default)]
pub struct ThemeStore {
    themes: Vec<Theme>,
    active: Option<String>,
}

impl ThemeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a theme, replacing any existing one with the same name.
    pub fn insert(&mut self, theme: Theme) {
        self.themes.retain(|existing| existing.name != theme.name);
        self.themes.push(theme);
    }

    /// Names of all loaded themes, sorted ascending.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.themes.iter().map(|theme| theme.name.clone()).collect();
        names.sort();
        names
    }

    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.iter().find(|theme| theme.name == name)
    }

    pub fn active(&self) -> Option<&Theme> {
        self.active.as_deref().and_then(|name| self.get(name))
    }

    pub fn active_name(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Set the active theme by name; returns false (and leaves `active`
    /// unchanged) if no theme with that name is loaded.
    pub fn set_active(&mut self, name: &str) -> bool {
        if self.get(name).is_none() {
            return false;
        }
        self.active = Some(name.to_string());
        true
    }

    pub fn len(&self) -> usize {
        self.themes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRACULA_LIKE: &str = "\
# A Dracula-like theme
background = #282a36
foreground = #f8f8f2
cursor-color = #f8f8f2
selection-background = #44475a
palette = 0=#000000
palette = 1=#ff5555
palette = 2=#50fa7b
";

    #[test]
    fn parses_ghostty_theme() {
        let theme = parse_ghostty_theme("dracula-like", DRACULA_LIKE);
        assert_eq!(theme.name, "dracula-like");
        assert_eq!(theme.background, Some("#282a36".to_string()));
        assert_eq!(theme.foreground, Some("#f8f8f2".to_string()));
        assert_eq!(theme.cursor, Some("#f8f8f2".to_string()));
        assert_eq!(theme.selection_background, Some("#44475a".to_string()));
        assert_eq!(theme.palette[1], "#ff5555");
        assert_eq!(theme.appearance, Appearance::Dark);
    }

    #[test]
    fn imports_windows_terminal_scheme() {
        let content = r##"{"schemes":[{"name":"X","background":"#000000","foreground":"#ffffff","red":"#ff0000"}]}"##;
        let themes = import_windows_terminal(content).unwrap();
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "X");
        assert_eq!(themes[0].palette[1], "#ff0000");
    }

    #[test]
    fn import_windows_terminal_rejects_invalid_json() {
        assert!(import_windows_terminal("not json").is_err());
    }

    #[test]
    fn parses_hex() {
        assert_eq!(parse_hex("#ff5555"), Some((0xff, 0x55, 0x55)));
        assert_eq!(parse_hex("ff5555"), Some((0xff, 0x55, 0x55)));
        assert_eq!(parse_hex("#ff55"), None);
        assert_eq!(parse_hex("nothex1"), None);
    }

    #[test]
    fn theme_store_insert_replaces_by_name() {
        let mut store = ThemeStore::new();
        store.insert(Theme {
            name: "dark".to_string(),
            appearance: Appearance::Dark,
            background: Some("#000000".to_string()),
            foreground: None,
            cursor: None,
            selection_background: None,
            palette: Vec::new(),
        });
        store.insert(Theme {
            name: "dark".to_string(),
            appearance: Appearance::Dark,
            background: Some("#111111".to_string()),
            foreground: None,
            cursor: None,
            selection_background: None,
            palette: Vec::new(),
        });
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.get("dark").unwrap().background,
            Some("#111111".to_string())
        );
    }

    #[test]
    fn theme_store_set_active_rejects_unknown() {
        let mut store = ThemeStore::new();
        store.insert(Theme {
            name: "dark".to_string(),
            appearance: Appearance::Dark,
            background: None,
            foreground: None,
            cursor: None,
            selection_background: None,
            palette: Vec::new(),
        });
        assert!(!store.set_active("missing"));
        assert!(store.active().is_none());
        assert!(store.set_active("dark"));
        assert_eq!(store.active_name(), Some("dark"));
    }

    #[test]
    fn theme_store_names_are_sorted() {
        let mut store = ThemeStore::new();
        for name in ["zebra", "apple", "mango"] {
            store.insert(Theme {
                name: name.to_string(),
                appearance: Appearance::Dark,
                background: None,
                foreground: None,
                cursor: None,
                selection_background: None,
                palette: Vec::new(),
            });
        }
        assert_eq!(store.names(), vec!["apple", "mango", "zebra"]);
    }
}
