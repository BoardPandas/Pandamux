//! Data-driven keyboard shortcut map (spec 2.6).
//!
//! One table drives BOTH the event decoder (pandamux-app) and every display
//! surface (settings Keyboard tab, cheat sheet, palette hints), so the label
//! list can never drift from the decode table again. Framework-agnostic:
//! nothing here knows about Iced; the runtime converts its key events into
//! [`KeyInput`] and asks the map to resolve them.
//!
//! User overrides come from `keyboard.overrides` (action id to chord string,
//! or null to unbind) and `keyboard.passThrough` (chords always forwarded to
//! the terminal even when bound): reserved app-level keybindings with a
//! settings-based escape hatch, the Windows Terminal model.

use crate::settings::KeyboardSettings;
use std::fmt;
use std::str::FromStr;

/// Modifier state for a chord or input. Alt participates in matching so an
/// Alt-modified press never false-positives a Ctrl binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    pub const CTRL: Mods = Mods {
        ctrl: true,
        shift: false,
        alt: false,
    };
    pub const CTRL_SHIFT: Mods = Mods {
        ctrl: true,
        shift: true,
        alt: false,
    };
    pub const SHIFT: Mods = Mods {
        ctrl: false,
        shift: true,
        alt: false,
    };
    pub const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
    };
}

/// Named (non-character) keys the keymap and decoder care about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Tab,
    Enter,
    Escape,
    Space,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    F1,
}

impl NamedKey {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::Enter => "enter",
            Self::Escape => "escape",
            Self::Space => "space",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Insert => "insert",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "pageup",
            Self::PageDown => "pagedown",
            Self::ArrowUp => "up",
            Self::ArrowDown => "down",
            Self::ArrowLeft => "left",
            Self::ArrowRight => "right",
            Self::F1 => "f1",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "tab" => Self::Tab,
            "enter" | "return" => Self::Enter,
            "escape" | "esc" => Self::Escape,
            "space" => Self::Space,
            "backspace" => Self::Backspace,
            "delete" | "del" => Self::Delete,
            "insert" | "ins" => Self::Insert,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" => Self::PageUp,
            "pagedown" => Self::PageDown,
            "up" => Self::ArrowUp,
            "down" => Self::ArrowDown,
            "left" => Self::ArrowLeft,
            "right" => Self::ArrowRight,
            "f1" => Self::F1,
            _ => return None,
        })
    }
}

/// The key half of a chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeySpec {
    /// A character key, matched against the lowercased base character.
    Char(char),
    /// A PHYSICAL digit-row key (0-9), matched by scan position so Ctrl+1..9
    /// works on layouts where digits are shifted (AZERTY).
    Digit(u8),
    Named(NamedKey),
}

/// A parsed chord like "ctrl+shift+c", "ctrl+1", "shift+pageup", or "f1".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub mods: Mods,
    pub key: KeySpec,
}

impl KeyChord {
    pub const fn ctrl(key: char) -> Self {
        Self {
            mods: Mods::CTRL,
            key: KeySpec::Char(key),
        }
    }

    pub const fn ctrl_shift(key: char) -> Self {
        Self {
            mods: Mods::CTRL_SHIFT,
            key: KeySpec::Char(key),
        }
    }

    /// Human-facing display like "Ctrl+Shift+C" (the serialized form stays
    /// lowercase; see `Display`).
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.mods.alt {
            parts.push("Alt".to_string());
        }
        if self.mods.shift {
            parts.push("Shift".to_string());
        }
        parts.push(match self.key {
            KeySpec::Char(c) => c.to_uppercase().to_string(),
            KeySpec::Digit(d) => d.to_string(),
            KeySpec::Named(named) => {
                let name = named.as_str();
                let mut chars = name.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                    None => String::new(),
                }
            }
        });
        parts.join("+")
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            write!(f, "ctrl+")?;
        }
        if self.mods.alt {
            write!(f, "alt+")?;
        }
        if self.mods.shift {
            write!(f, "shift+")?;
        }
        match self.key {
            KeySpec::Char(c) => write!(f, "{c}"),
            KeySpec::Digit(d) => write!(f, "{d}"),
            KeySpec::Named(named) => write!(f, "{}", named.as_str()),
        }
    }
}

impl FromStr for KeyChord {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut mods = Mods::default();
        let mut key = None;
        for part in value.split('+') {
            let part = part.trim().to_lowercase();
            match part.as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "shift" => mods.shift = true,
                "alt" => mods.alt = true,
                "" => return Err(format!("empty key in chord: {value}")),
                _ => {
                    if key.is_some() {
                        return Err(format!("multiple keys in chord: {value}"));
                    }
                    key = Some(parse_key(&part).ok_or_else(|| format!("unknown key: {part}"))?);
                }
            }
        }
        let key = key.ok_or_else(|| format!("chord has no key: {value}"))?;
        Ok(Self { mods, key })
    }
}

fn parse_key(part: &str) -> Option<KeySpec> {
    if let Some(named) = NamedKey::parse(part) {
        return Some(KeySpec::Named(named));
    }
    let mut chars = part.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if let Some(digit) = first.to_digit(10) {
        return Some(KeySpec::Digit(digit as u8));
    }
    Some(KeySpec::Char(first))
}

/// Everything the keymap needs to know about one key press. The runtime
/// builds this from its framework event; `resolve` matches it against chords.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyInput {
    pub mods: Mods,
    /// The logical base character, when the press was a character key.
    pub character: Option<String>,
    /// The named key, when the press was one.
    pub named: Option<NamedKey>,
    /// The physical digit-row key (0-9) regardless of layout, when known.
    pub physical_digit: Option<u8>,
    /// The composed text the OS produced (honours shift + layout); preferred
    /// for plain typing.
    pub text: Option<String>,
}

/// Every action a chord can trigger. Ids are stable strings used by
/// `keyboard.overrides`; labels/categories drive the cheat sheet and the
/// settings Keyboard tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    CommandPalette,
    NewSession,
    OpenSettings,
    Find,
    Notifications,
    ToggleStatusBar,
    ToggleTheme,
    CycleAccent,
    CheatSheet,
    SplitRight,
    SplitDown,
    CloseTab,
    ZoomPane,
    NextTab,
    PrevTab,
    /// Ctrl+1..9: focus the Nth project; repeated presses cycle its tabs.
    FocusProject(u8),
    GoHome,
    CopyOrInterrupt,
    Copy,
    Paste,
    ScrollPageUp,
    ScrollPageDown,
}

impl Action {
    pub fn id(self) -> String {
        match self {
            Self::CommandPalette => "commandPalette".to_string(),
            Self::NewSession => "newSession".to_string(),
            Self::OpenSettings => "openSettings".to_string(),
            Self::Find => "find".to_string(),
            Self::Notifications => "notifications".to_string(),
            Self::ToggleStatusBar => "toggleStatusBar".to_string(),
            Self::ToggleTheme => "toggleTheme".to_string(),
            Self::CycleAccent => "cycleAccent".to_string(),
            Self::CheatSheet => "cheatSheet".to_string(),
            Self::SplitRight => "splitRight".to_string(),
            Self::SplitDown => "splitDown".to_string(),
            Self::CloseTab => "closeTab".to_string(),
            Self::ZoomPane => "zoomPane".to_string(),
            Self::NextTab => "nextTab".to_string(),
            Self::PrevTab => "prevTab".to_string(),
            Self::FocusProject(n) => format!("focusProject{n}"),
            Self::GoHome => "goHome".to_string(),
            Self::CopyOrInterrupt => "copyOrInterrupt".to_string(),
            Self::Copy => "copy".to_string(),
            Self::Paste => "paste".to_string(),
            Self::ScrollPageUp => "scrollPageUp".to_string(),
            Self::ScrollPageDown => "scrollPageDown".to_string(),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::CommandPalette => "Command palette".to_string(),
            Self::NewSession => "New session".to_string(),
            Self::OpenSettings => "Open settings".to_string(),
            Self::Find => "Find in terminal".to_string(),
            Self::Notifications => "Toggle notifications".to_string(),
            Self::ToggleStatusBar => "Toggle status bar".to_string(),
            Self::ToggleTheme => "Toggle theme".to_string(),
            Self::CycleAccent => "Cycle accent color".to_string(),
            Self::CheatSheet => "Keyboard shortcuts".to_string(),
            Self::SplitRight => "Split pane right".to_string(),
            Self::SplitDown => "Split pane down".to_string(),
            Self::CloseTab => "Close tab".to_string(),
            Self::ZoomPane => "Zoom pane".to_string(),
            Self::NextTab => "Next tab".to_string(),
            Self::PrevTab => "Previous tab".to_string(),
            Self::FocusProject(n) => format!("Focus project {n} / cycle its tabs"),
            Self::GoHome => "Go to Home".to_string(),
            Self::CopyOrInterrupt => "Copy selection (else interrupt)".to_string(),
            Self::Copy => "Copy selection".to_string(),
            Self::Paste => "Paste".to_string(),
            Self::ScrollPageUp => "Scroll page up".to_string(),
            Self::ScrollPageDown => "Scroll page down".to_string(),
        }
    }

    pub fn category(self) -> &'static str {
        match self {
            Self::CommandPalette
            | Self::NewSession
            | Self::OpenSettings
            | Self::Find
            | Self::Notifications
            | Self::ToggleStatusBar
            | Self::ToggleTheme
            | Self::CycleAccent
            | Self::CheatSheet => "General",
            Self::SplitRight
            | Self::SplitDown
            | Self::CloseTab
            | Self::ZoomPane
            | Self::NextTab
            | Self::PrevTab => "Panes & tabs",
            Self::FocusProject(_) | Self::GoHome => "Projects",
            Self::CopyOrInterrupt
            | Self::Copy
            | Self::Paste
            | Self::ScrollPageUp
            | Self::ScrollPageDown => "Terminal",
        }
    }

    /// Whether this action still fires while a centered overlay is open.
    /// Everything else is swallowed instead of acting behind the overlay's
    /// back (fixes Ctrl+W typed into the palette closing a pane).
    pub fn allowed_with_overlay(self) -> bool {
        matches!(
            self,
            Self::CommandPalette | Self::OpenSettings | Self::CheatSheet
        )
    }

    fn parse_id(id: &str) -> Option<Self> {
        Some(match id {
            "commandPalette" => Self::CommandPalette,
            "newSession" => Self::NewSession,
            "openSettings" => Self::OpenSettings,
            "find" => Self::Find,
            "notifications" => Self::Notifications,
            "toggleStatusBar" => Self::ToggleStatusBar,
            "toggleTheme" => Self::ToggleTheme,
            "cycleAccent" => Self::CycleAccent,
            "cheatSheet" => Self::CheatSheet,
            "splitRight" => Self::SplitRight,
            "splitDown" => Self::SplitDown,
            "closeTab" => Self::CloseTab,
            "zoomPane" => Self::ZoomPane,
            "nextTab" => Self::NextTab,
            "prevTab" => Self::PrevTab,
            "goHome" => Self::GoHome,
            "copyOrInterrupt" => Self::CopyOrInterrupt,
            "copy" => Self::Copy,
            "paste" => Self::Paste,
            "scrollPageUp" => Self::ScrollPageUp,
            "scrollPageDown" => Self::ScrollPageDown,
            _ => {
                let n = id.strip_prefix("focusProject")?.parse::<u8>().ok()?;
                if (1..=9).contains(&n) {
                    Self::FocusProject(n)
                } else {
                    return None;
                }
            }
        })
    }
}

/// A cheat-sheet / settings section: category title plus (label, chords).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapSection {
    pub title: String,
    pub entries: Vec<(String, String)>,
}

/// The resolved shortcut table: ordered (chord, action) bindings plus the
/// pass-through list.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap {
    bindings: Vec<(KeyChord, Action)>,
    pass_through: Vec<KeyChord>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Keymap {
    /// The built-in table. Multiple chords may map to one action (the first
    /// listed is the primary shown in hints).
    pub fn defaults() -> Self {
        use KeySpec::{Digit, Named};
        let named = |mods: Mods, key: NamedKey| KeyChord {
            mods,
            key: Named(key),
        };
        let digit = |d: u8| KeyChord {
            mods: Mods::CTRL,
            key: Digit(d),
        };
        let mut bindings = vec![
            (KeyChord::ctrl('k'), Action::CommandPalette),
            (KeyChord::ctrl_shift('p'), Action::CommandPalette),
            (KeyChord::ctrl('t'), Action::NewSession),
            (KeyChord::ctrl(','), Action::OpenSettings),
            (KeyChord::ctrl('f'), Action::Find),
            (KeyChord::ctrl('n'), Action::Notifications),
            (KeyChord::ctrl('b'), Action::ToggleStatusBar),
            (KeyChord::ctrl_shift('t'), Action::ToggleTheme),
            (KeyChord::ctrl_shift('a'), Action::CycleAccent),
            (KeyChord::ctrl('/'), Action::CheatSheet),
            (named(Mods::NONE, NamedKey::F1), Action::CheatSheet),
            (KeyChord::ctrl('d'), Action::SplitRight),
            (KeyChord::ctrl_shift('d'), Action::SplitDown),
            (KeyChord::ctrl('w'), Action::CloseTab),
            (named(Mods::CTRL, NamedKey::Enter), Action::ZoomPane),
            (named(Mods::CTRL, NamedKey::Tab), Action::NextTab),
            (named(Mods::CTRL_SHIFT, NamedKey::Tab), Action::PrevTab),
            (digit(0), Action::GoHome),
            (named(Mods::CTRL, NamedKey::Home), Action::GoHome),
            (KeyChord::ctrl('c'), Action::CopyOrInterrupt),
            (KeyChord::ctrl_shift('c'), Action::Copy),
            (KeyChord::ctrl('v'), Action::Paste),
            (KeyChord::ctrl_shift('v'), Action::Paste),
            (named(Mods::SHIFT, NamedKey::PageUp), Action::ScrollPageUp),
            (
                named(Mods::SHIFT, NamedKey::PageDown),
                Action::ScrollPageDown,
            ),
        ];
        for n in 1..=9 {
            bindings.push((digit(n), Action::FocusProject(n)));
        }
        Self {
            bindings,
            pass_through: Vec::new(),
        }
    }

    /// Defaults plus the user's `keyboard.*` settings. Bad entries are
    /// reported as warnings and leave the default in place.
    pub fn with_settings(settings: &KeyboardSettings) -> (Self, Vec<String>) {
        let mut map = Self::defaults();
        let mut warnings = Vec::new();
        for (id, chord) in &settings.overrides {
            let Some(action) = Action::parse_id(id) else {
                warnings.push(format!("keyboard.overrides: unknown action id: {id}"));
                continue;
            };
            match chord {
                None => {
                    // Explicit null unbinds the action entirely.
                    map.bindings.retain(|(_, bound)| *bound != action);
                }
                Some(chord) => match chord.parse::<KeyChord>() {
                    Ok(chord) => {
                        // The override replaces every default chord for the
                        // action and steals the chord from any other action.
                        map.bindings
                            .retain(|(existing, bound)| *bound != action && *existing != chord);
                        map.bindings.push((chord, action));
                    }
                    Err(error) => {
                        warnings.push(format!("keyboard.overrides[{id}]: {error}"));
                    }
                },
            }
        }
        for value in &settings.pass_through {
            match value.parse::<KeyChord>() {
                Ok(chord) => map.pass_through.push(chord),
                Err(error) => warnings.push(format!("keyboard.passThrough: {error}")),
            }
        }
        (map, warnings)
    }

    /// The chords a key press could be: the physical digit first (so Ctrl+1
    /// wins on AZERTY where the base character needs shift), then the
    /// lowercased character, then the named key.
    fn candidates(input: &KeyInput) -> Vec<KeyChord> {
        let mut chords = Vec::new();
        if let Some(digit) = input.physical_digit {
            // Digit chords are declared without shift; drop it so a layout
            // that shifts its digit row still matches.
            chords.push(KeyChord {
                mods: Mods {
                    shift: false,
                    ..input.mods
                },
                key: KeySpec::Digit(digit),
            });
        }
        if let Some(character) = &input.character {
            let mut chars = character.chars();
            if let (Some(first), None) = (chars.next(), chars.next()) {
                chords.push(KeyChord {
                    mods: input.mods,
                    key: KeySpec::Char(first.to_ascii_lowercase()),
                });
            }
        }
        if let Some(named) = input.named {
            chords.push(KeyChord {
                mods: input.mods,
                key: KeySpec::Named(named),
            });
        }
        chords
    }

    /// Resolve a key press to an action. Pass-through chords resolve to
    /// nothing (the press reaches the terminal instead).
    pub fn resolve(&self, input: &KeyInput) -> Option<Action> {
        for chord in Self::candidates(input) {
            if self.pass_through.contains(&chord) {
                return None;
            }
            if let Some((_, action)) = self
                .bindings
                .iter()
                .find(|(candidate, _)| *candidate == chord)
            {
                return Some(*action);
            }
        }
        None
    }

    /// The primary chord bound to an action, if any.
    pub fn chord_for(&self, action: Action) -> Option<KeyChord> {
        self.bindings
            .iter()
            .find(|(_, bound)| *bound == action)
            .map(|(chord, _)| *chord)
    }

    /// Display string for an action's primary chord ("Ctrl+K"), for hints.
    pub fn display_for(&self, action: Action) -> Option<String> {
        self.chord_for(action).map(|chord| chord.label())
    }

    /// Categorized sections for the cheat sheet and settings Keyboard tab.
    /// Every bound chord shows, joined with " / "; Ctrl+1..9 collapses into
    /// one row.
    pub fn sections(&self) -> Vec<KeymapSection> {
        let mut order: Vec<Action> = Vec::new();
        for (_, action) in &self.bindings {
            let action = match action {
                // Collapse the nine project rows into the first.
                Action::FocusProject(_) => Action::FocusProject(1),
                other => *other,
            };
            if !order.contains(&action) {
                order.push(action);
            }
        }
        let mut sections: Vec<KeymapSection> = Vec::new();
        for category in ["General", "Panes & tabs", "Projects", "Terminal"] {
            let entries: Vec<(String, String)> = order
                .iter()
                .filter(|action| action.category() == category)
                .map(|action| {
                    let chords: Vec<String> = match action {
                        Action::FocusProject(_) => vec!["Ctrl+1..9".to_string()],
                        action => self
                            .bindings
                            .iter()
                            .filter(|(_, bound)| bound == action)
                            .map(|(chord, _)| chord.label())
                            .collect(),
                    };
                    let label = match action {
                        Action::FocusProject(_) => {
                            "Focus the Nth project / cycle its tabs".to_string()
                        }
                        action => action.label(),
                    };
                    (label, chords.join(" / "))
                })
                .collect();
            if !entries.is_empty() {
                sections.push(KeymapSection {
                    title: category.to_string(),
                    entries,
                });
            }
        }
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn ctrl_char(c: char) -> KeyInput {
        KeyInput {
            mods: Mods::CTRL,
            character: Some(c.to_string()),
            ..KeyInput::default()
        }
    }

    #[test]
    fn chords_round_trip_through_display_and_parse() {
        for raw in [
            "ctrl+shift+c",
            "ctrl+1",
            "shift+pageup",
            "f1",
            "ctrl+home",
            "ctrl+,",
            "ctrl+/",
        ] {
            let chord: KeyChord = raw.parse().expect(raw);
            assert_eq!(chord.to_string(), raw);
        }
        assert!("ctrl+".parse::<KeyChord>().is_err());
        assert!("ctrl+q+w".parse::<KeyChord>().is_err());
        assert!("ctrl+banana".parse::<KeyChord>().is_err());
    }

    #[test]
    fn default_table_has_no_duplicate_chords() {
        let map = Keymap::defaults();
        let mut seen = std::collections::HashSet::new();
        for (chord, action) in &map.bindings {
            assert!(
                seen.insert(*chord),
                "duplicate chord {chord} (on {})",
                action.id()
            );
        }
    }

    #[test]
    fn resolves_characters_digits_and_named_keys() {
        let map = Keymap::defaults();
        assert_eq!(map.resolve(&ctrl_char('k')), Some(Action::CommandPalette));
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::CTRL_SHIFT,
                character: Some("c".to_string()),
                ..KeyInput::default()
            }),
            Some(Action::Copy)
        );
        // Physical digits match regardless of what character the layout
        // produces (AZERTY Ctrl+1 emits "&").
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::CTRL,
                character: Some("&".to_string()),
                physical_digit: Some(1),
                ..KeyInput::default()
            }),
            Some(Action::FocusProject(1))
        );
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::CTRL,
                physical_digit: Some(0),
                ..KeyInput::default()
            }),
            Some(Action::GoHome)
        );
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::SHIFT,
                named: Some(NamedKey::PageUp),
                ..KeyInput::default()
            }),
            Some(Action::ScrollPageUp)
        );
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::CTRL_SHIFT,
                named: Some(NamedKey::Tab),
                ..KeyInput::default()
            }),
            Some(Action::PrevTab)
        );
        // Unbound: plain typing and unmapped Ctrl+letters.
        assert_eq!(map.resolve(&ctrl_char('l')), None);
        assert_eq!(
            map.resolve(&KeyInput {
                character: Some("x".to_string()),
                ..KeyInput::default()
            }),
            None
        );
    }

    #[test]
    fn overrides_rebind_unbind_and_warn() {
        let mut settings = KeyboardSettings::default();
        settings
            .overrides
            .insert("find".to_string(), Some("ctrl+shift+f".to_string()));
        settings.overrides.insert("closeTab".to_string(), None);
        settings
            .overrides
            .insert("notARealAction".to_string(), Some("ctrl+x".to_string()));
        settings
            .overrides
            .insert("paste".to_string(), Some("ctrl+banana".to_string()));
        let (map, warnings) = Keymap::with_settings(&settings);

        // Rebound: the old chord is free, the new one resolves.
        assert_eq!(map.resolve(&ctrl_char('f')), None);
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::CTRL_SHIFT,
                character: Some("f".to_string()),
                ..KeyInput::default()
            }),
            Some(Action::Find)
        );
        // Unbound entirely.
        assert_eq!(map.resolve(&ctrl_char('w')), None);
        // Bad entries warned and left defaults alone.
        assert_eq!(warnings.len(), 2);
        assert_eq!(map.resolve(&ctrl_char('v')), Some(Action::Paste));
    }

    #[test]
    fn pass_through_forwards_bound_chords_to_the_terminal() {
        let settings = KeyboardSettings {
            pass_through: vec!["ctrl+1".to_string(), "ctrl+v".to_string()],
            overrides: BTreeMap::new(),
        };
        let (map, warnings) = Keymap::with_settings(&settings);
        assert!(warnings.is_empty());
        assert_eq!(
            map.resolve(&KeyInput {
                mods: Mods::CTRL,
                physical_digit: Some(1),
                ..KeyInput::default()
            }),
            None
        );
        assert_eq!(map.resolve(&ctrl_char('v')), None);
        // Untouched bindings still resolve.
        assert_eq!(map.resolve(&ctrl_char('k')), Some(Action::CommandPalette));
    }

    #[test]
    fn sections_cover_every_bound_action() {
        let map = Keymap::defaults();
        let sections = map.sections();
        let titles: Vec<&str> = sections
            .iter()
            .map(|section| section.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec!["General", "Panes & tabs", "Projects", "Terminal"]
        );
        let all: Vec<&(String, String)> = sections
            .iter()
            .flat_map(|section| section.entries.iter())
            .collect();
        // Every distinct action shows exactly once (projects collapsed).
        assert!(all.iter().any(|(label, chords)| label
            == "Focus the Nth project / cycle its tabs"
            && chords == "Ctrl+1..9"));
        assert!(
            all.iter()
                .any(|(label, chords)| label == "Command palette" && chords.contains("Ctrl+K"))
        );
        assert!(
            all.iter()
                .any(|(label, chords)| label == "Paste" && chords == "Ctrl+V / Ctrl+Shift+V")
        );
    }

    #[test]
    fn display_for_reports_the_primary_chord() {
        let map = Keymap::defaults();
        assert_eq!(
            map.display_for(Action::CommandPalette),
            Some("Ctrl+K".to_string())
        );
        assert_eq!(
            map.display_for(Action::NewSession),
            Some("Ctrl+T".to_string())
        );
        assert_eq!(
            map.display_for(Action::ZoomPane),
            Some("Ctrl+Enter".to_string())
        );
    }
}
