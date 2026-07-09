//! Central design-token module for the native Iced shell.
//!
//! Every chrome color, size, radius, shadow, and typography choice from the
//! Phase 3 UI design handoff (see `tasks/plan-repo.md` Section 12 and
//! `design_handoff_pandamux_ui/README.md`) is encoded here. Widgets must consume
//! these tokens rather than hardcoding colors, so a theme or accent change is a
//! single-source edit.
//!
//! Terminal panes stay dark in both chrome themes: the terminal color scheme is
//! independent of the chrome theme, as in the Electron app. Those tokens live in
//! the `term` submodule and never vary with [`UiTheme`].

use iced::border::Radius;
use iced::font::Weight;
use iced::{Border, Color, Font, Shadow, Vector};

/// Chrome theme variant. Terminal panes ignore this (see module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiTheme {
    #[default]
    Dark,
    Light,
}

impl UiTheme {
    pub fn toggled(self) -> Self {
        match self {
            UiTheme::Dark => UiTheme::Light,
            UiTheme::Light => UiTheme::Dark,
        }
    }
}

/// User-configurable accent. Teal is the default (from the panda logo); the
/// alternates are the ones offered in the design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Accent {
    #[default]
    Teal,
    Gold,
    Blue,
    Mauve,
}

impl Accent {
    pub const fn color(self) -> Color {
        match self {
            Accent::Teal => Color::from_rgb8(0x43, 0xd9, 0xc9),
            Accent::Gold => Color::from_rgb8(0xd8, 0xb4, 0x5e),
            Accent::Blue => Color::from_rgb8(0x4d, 0x9f, 0xff),
            Accent::Mauve => Color::from_rgb8(0xb4, 0x8e, 0xad),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Accent::Teal => Accent::Gold,
            Accent::Gold => Accent::Blue,
            Accent::Blue => Accent::Mauve,
            Accent::Mauve => Accent::Teal,
        }
    }
}

/// The four shell families the design tints badges and glyphs by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    PowerShell,
    Ssh,
    Wsl,
    Cmd,
}

impl ShellKind {
    /// Best-effort classification of a shell command string (`pwsh`, `ssh ...`,
    /// `wsl.exe`, `cmd`) into a design shell family.
    pub fn classify(shell: &str) -> Self {
        let shell = shell.to_ascii_lowercase();
        if shell.contains("ssh") {
            ShellKind::Ssh
        } else if shell.contains("wsl") {
            ShellKind::Wsl
        } else if shell.contains("cmd") {
            ShellKind::Cmd
        } else {
            ShellKind::PowerShell
        }
    }

    /// Short badge abbreviation (PS / SSH / WSL / CMD).
    pub fn abbreviation(self) -> &'static str {
        match self {
            ShellKind::PowerShell => "PS",
            ShellKind::Ssh => "SSH",
            ShellKind::Wsl => "WSL",
            ShellKind::Cmd => "CMD",
        }
    }

    /// Single-glyph prompt marker used on tabs.
    pub fn glyph(self) -> &'static str {
        match self {
            ShellKind::PowerShell => ">",
            ShellKind::Ssh => "\u{2192}", // →
            ShellKind::Wsl => "\u{03bb}", // λ
            ShellKind::Cmd => "$",
        }
    }
}

// ---------------------------------------------------------------------------
// Layout tokens (identical across themes)
// ---------------------------------------------------------------------------

pub const TITLEBAR_HEIGHT: f32 = 40.0;
pub const RAIL_WIDTH: f32 = 52.0;
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
pub const TAB_BAR_HEIGHT: f32 = 36.0;
pub const SESSION_PANEL_WIDTH: f32 = 264.0;
pub const SESSION_PANEL_COMPACT_WIDTH: f32 = 216.0;

pub const WORKSPACE_PADDING: f32 = 10.0;
pub const PANE_GAP: f32 = 8.0;
pub const TERMINAL_PADDING: f32 = 12.0;

pub const RADIUS_PANE: f32 = 12.0;
pub const RADIUS_OVERLAY: f32 = 14.0;
pub const RADIUS_ROW: f32 = 8.0;
pub const RADIUS_CHIP: f32 = 5.0;
pub const RADIUS_RAIL_BUTTON: f32 = 10.0;

// ---------------------------------------------------------------------------
// Typography tokens
// ---------------------------------------------------------------------------

/// Named monospace face. The JetBrains Mono TTF is intended to be bundled and
/// registered with the application; until the font bytes ship, this named font
/// resolves to JetBrains Mono when installed and falls back to the system
/// monospace otherwise. Keep this the single reference point for the mono face.
pub const MONO_FONT: Font = Font::with_name("JetBrains Mono");

/// UI face: system-ui / Segoe UI on Windows.
pub const UI_FONT: Font = Font::DEFAULT;

pub fn mono(weight: Weight) -> Font {
    Font {
        weight,
        ..MONO_FONT
    }
}

pub fn ui(weight: Weight) -> Font {
    Font {
        weight,
        ..Font::DEFAULT
    }
}

// UI text sizes
pub const SIZE_TITLE: f32 = 13.0;
pub const SIZE_BODY: f32 = 12.5;
pub const SIZE_SECONDARY: f32 = 11.0;
pub const SIZE_GROUP_HEADER: f32 = 10.5;
// Mono text sizes
pub const SIZE_TERMINAL: f32 = 12.5;
pub const SIZE_METADATA: f32 = 10.0;
pub const SIZE_KBD: f32 = 10.5;
pub const SIZE_STATUS_BAR: f32 = 10.5;

// ---------------------------------------------------------------------------
// Terminal pane scheme (fixed, both themes)
// ---------------------------------------------------------------------------

pub mod term {
    use iced::Color;

    /// Terminal surface fill (~#10171b at 0.8 alpha).
    pub const SURFACE: Color = Color::from_rgba8(13, 19, 22, 0.8);
    pub const TEXT: Color = Color::from_rgb8(0xb7, 0xc6, 0xc6);
    pub const DIM: Color = Color::from_rgb8(0x6b, 0x7c, 0x80);
    pub const SUCCESS: Color = Color::from_rgb8(0x7f, 0xd8, 0x8f);
    pub const GOLD: Color = Color::from_rgb8(0xd8, 0xb4, 0x5e);
    /// Opaque approximation of the surface for the solid pane backer.
    pub const SURFACE_OPAQUE: Color = Color::from_rgb8(0x10, 0x17, 0x1b);

    /// Cursor block dimensions (px).
    pub const CURSOR_WIDTH: f32 = 7.0;
    pub const CURSOR_HEIGHT: f32 = 15.0;
    /// Cell metrics used by the canvas viewport.
    pub const CELL_WIDTH: f32 = 8.4;
    pub const CELL_HEIGHT: f32 = 21.0;
}

/// The terminal color scheme applied to the canvas viewport. Defaults to the
/// fixed-dark scheme above; a selected theme (loaded from a `.theme` file or an
/// imported config) overrides it. Independent of the chrome [`UiTheme`], as in
/// the Electron app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TermScheme {
    pub background: Color,
    pub text: Color,
    pub dim: Color,
    pub success: Color,
    pub gold: Color,
    pub cursor: Color,
}

impl Default for TermScheme {
    fn default() -> Self {
        Self {
            background: term::SURFACE_OPAQUE,
            text: term::TEXT,
            dim: term::DIM,
            success: term::SUCCESS,
            gold: term::GOLD,
            cursor: Accent::Teal.color(),
        }
    }
}

impl TermScheme {
    /// Derive a scheme from a loaded core theme, falling back to the default for
    /// any missing or invalid color. Palette indices follow ANSI: 2 = green
    /// (success), 3 = yellow (gold), 8 = bright black (dim).
    pub fn from_theme(theme: &pandamux_core::Theme) -> Self {
        let base = Self::default();
        let hex = |value: &Option<String>, fallback: Color| {
            value.as_deref().and_then(hex_to_color).unwrap_or(fallback)
        };
        let palette = |index: usize, fallback: Color| {
            theme
                .palette
                .get(index)
                .and_then(|value| hex_to_color(value))
                .unwrap_or(fallback)
        };
        Self {
            background: hex(&theme.background, base.background),
            text: hex(&theme.foreground, base.text),
            dim: palette(8, base.dim),
            success: palette(2, base.success),
            gold: palette(3, base.gold),
            cursor: hex(&theme.cursor, hex(&theme.foreground, base.text)),
        }
    }
}

fn hex_to_color(hex: &str) -> Option<Color> {
    pandamux_core::parse_hex(hex).map(|(r, g, b)| Color::from_rgb8(r, g, b))
}

// ---------------------------------------------------------------------------
// Chrome palette
// ---------------------------------------------------------------------------

/// The full set of chrome colors for one [`UiTheme`] + [`Accent`] combination.
/// `Copy` so it can be captured by value into Iced style closures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub theme: UiTheme,
    pub accent: Color,
    /// `true` when the overlay tint base is white (dark theme), `false` for
    /// black (light theme). Drives [`Palette::ov`].
    overlay_white: bool,
    pub bg_base: Color,
    /// The window background vertical-gradient endpoints (top -> bottom).
    pub bg_top: Color,
    pub bg_bottom: Color,
    pub t1: Color,
    pub t2: Color,
    pub t3: Color,
    pub t4: Color,
    pub bgc: Color,
    pub inset: Color,
    pub panel: Color,
    pub panel2: Color,
    pub scrim: Color,
    pub shell_powershell: Color,
    pub shell_ssh: Color,
    pub shell_wsl: Color,
    pub shell_cmd: Color,
}

impl Palette {
    pub fn new(theme: UiTheme, accent: Accent) -> Self {
        match theme {
            UiTheme::Dark => Self::dark(accent),
            UiTheme::Light => Self::light(accent),
        }
    }

    fn dark(accent: Accent) -> Self {
        Palette {
            theme: UiTheme::Dark,
            accent: accent.color(),
            overlay_white: true,
            bg_base: Color::from_rgb8(0x0b, 0x0f, 0x12),
            bg_top: Color::from_rgb8(0x0c, 0x11, 0x14),
            bg_bottom: Color::from_rgb8(0x0a, 0x0e, 0x11),
            t1: Color::from_rgb8(0xdb, 0xe6, 0xe6),
            t2: Color::from_rgb8(0x8f, 0xa0, 0xa3),
            t3: Color::from_rgb8(0x7d, 0x8d, 0x90),
            t4: Color::from_rgb8(0x55, 0x66, 0x6a),
            bgc: Color::from_rgb8(0x0d, 0x12, 0x15),
            inset: Color::from_rgba8(0, 0, 0, 0.25),
            panel: Color::from_rgba8(20, 27, 31, 0.92),
            panel2: Color::from_rgba8(18, 25, 29, 0.95),
            scrim: Color::from_rgba8(5, 8, 10, 0.5),
            shell_powershell: Color::from_rgb8(0x43, 0xd9, 0xc9),
            shell_ssh: Color::from_rgb8(0xd8, 0xb4, 0x5e),
            shell_wsl: Color::from_rgb8(0x7f, 0xd8, 0x8f),
            shell_cmd: Color::from_rgb8(0x9a, 0xa7, 0xb0),
        }
    }

    fn light(accent: Accent) -> Self {
        Palette {
            theme: UiTheme::Light,
            accent: accent.color(),
            overlay_white: false,
            bg_base: Color::from_rgb8(0xed, 0xf1, 0xf1),
            bg_top: Color::from_rgb8(0xf2, 0xf5, 0xf5),
            bg_bottom: Color::from_rgb8(0xe7, 0xec, 0xec),
            t1: Color::from_rgb8(0x1c, 0x25, 0x27),
            t2: Color::from_rgb8(0x3f, 0x50, 0x54),
            t3: Color::from_rgb8(0x5c, 0x6c, 0x70),
            t4: Color::from_rgb8(0x8a, 0x9a, 0x9e),
            bgc: Color::from_rgb8(0xee, 0xf2, 0xf2),
            inset: Color::from_rgba8(0, 0, 0, 0.07),
            panel: Color::from_rgba8(250, 252, 252, 0.94),
            panel2: Color::from_rgba8(252, 253, 253, 0.97),
            scrim: Color::from_rgba8(90, 102, 106, 0.35),
            shell_powershell: Color::from_rgb8(0x0e, 0x9a, 0x8c),
            shell_ssh: Color::from_rgb8(0xa1, 0x7e, 0x22),
            shell_wsl: Color::from_rgb8(0x3d, 0x9a, 0x50),
            shell_cmd: Color::from_rgb8(0x5c, 0x6c, 0x70),
        }
    }

    /// Overlay tint at the given alpha: `rgba(overlay, a)`. White in dark theme,
    /// black in light theme. Used for every hover/border fill.
    pub fn ov(&self, alpha: f32) -> Color {
        if self.overlay_white {
            Color::from_rgba(1.0, 1.0, 1.0, alpha)
        } else {
            Color::from_rgba(0.0, 0.0, 0.0, alpha)
        }
    }

    /// Accent at a reduced alpha (for tinted fills / rings).
    pub fn accent_alpha(&self, alpha: f32) -> Color {
        with_alpha(self.accent, alpha)
    }

    /// The window background as the design's vertical gradient (top -> bottom).
    /// The radial teal/gold ambience glows have no Iced primitive and remain a
    /// tracked approximation; this covers the base gradient.
    pub fn bg_gradient(&self) -> iced::Background {
        iced::Background::Gradient(iced::Gradient::Linear(
            iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
                .add_stop(0.0, self.bg_top)
                .add_stop(1.0, self.bg_bottom),
        ))
    }

    pub fn shell_color(&self, kind: ShellKind) -> Color {
        match kind {
            ShellKind::PowerShell => self.shell_powershell,
            ShellKind::Ssh => self.shell_ssh,
            ShellKind::Wsl => self.shell_wsl,
            ShellKind::Cmd => self.shell_cmd,
        }
    }
}

/// Return `color` with its alpha replaced.
pub fn with_alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

/// Uniform-radius border helper.
pub fn border(color: Color, width: f32, radius: f32) -> Border {
    Border {
        color,
        width,
        radius: Radius::from(radius),
    }
}

/// The pane drop shadow: `0 8px 30px rgba(0,0,0,0.25)`.
pub fn pane_shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba8(0, 0, 0, 0.25),
        offset: Vector::new(0.0, 8.0),
        blur_radius: 30.0,
    }
}

/// The overlay drop shadow: `0 24px 70px rgba(0,0,0,0.5)`.
pub fn overlay_shadow() -> Shadow {
    Shadow {
        color: Color::from_rgba8(0, 0, 0, 0.5),
        offset: Vector::new(0.0, 24.0),
        blur_radius: 70.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_tint_flips_with_theme() {
        let dark = Palette::new(UiTheme::Dark, Accent::Teal);
        let light = Palette::new(UiTheme::Light, Accent::Teal);
        assert_eq!(dark.ov(0.5), Color::from_rgba(1.0, 1.0, 1.0, 0.5));
        assert_eq!(light.ov(0.5), Color::from_rgba(0.0, 0.0, 0.0, 0.5));
    }

    #[test]
    fn accent_is_configurable() {
        let teal = Palette::new(UiTheme::Dark, Accent::Teal);
        let gold = Palette::new(UiTheme::Dark, Accent::Gold);
        assert_eq!(teal.accent, Accent::Teal.color());
        assert_ne!(teal.accent, gold.accent);
    }

    #[test]
    fn shell_classification_matches_families() {
        assert_eq!(ShellKind::classify("pwsh"), ShellKind::PowerShell);
        assert_eq!(ShellKind::classify("ssh chaz@galahad"), ShellKind::Ssh);
        assert_eq!(ShellKind::classify("wsl.exe -d Ubuntu"), ShellKind::Wsl);
        assert_eq!(ShellKind::classify("cmd.exe"), ShellKind::Cmd);
    }

    #[test]
    fn accent_cycles_through_all_variants() {
        assert_eq!(Accent::Teal.next(), Accent::Gold);
        assert_eq!(Accent::Gold.next(), Accent::Blue);
        assert_eq!(Accent::Blue.next(), Accent::Mauve);
        assert_eq!(Accent::Mauve.next(), Accent::Teal);
    }

    #[test]
    fn term_scheme_maps_theme_colors_with_fallback() {
        let theme = pandamux_core::parse_ghostty_theme(
            "t",
            "background = #101010\nforeground = #eeeeee\npalette = 2=#00ff00\n",
        );
        let scheme = TermScheme::from_theme(&theme);
        assert_eq!(scheme.background, Color::from_rgb8(0x10, 0x10, 0x10));
        assert_eq!(scheme.text, Color::from_rgb8(0xee, 0xee, 0xee));
        assert_eq!(scheme.success, Color::from_rgb8(0x00, 0xff, 0x00));
        // palette[3] (gold) absent -> falls back to the default scheme's gold.
        assert_eq!(scheme.gold, TermScheme::default().gold);
    }
}
