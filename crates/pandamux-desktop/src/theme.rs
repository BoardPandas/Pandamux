use gpui_kit::gpui::{px, rgb, rgba, Pixels, Rgba};

/// Section 12 UI Theme Specification for PandaMUX.
///
/// Encodes the authoritative visual tokens delivered in Section 12 of the PandaMUX spec:
/// dark and light chrome palettes, user-configurable accent colors, shell badge tints,
/// fixed-dark terminal scheme, exact typography scale, radii, and spatial dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AccentColor {
    #[default]
    Teal,
    Gold,
    Blue,
    Purple,
}

impl AccentColor {
    pub fn hex(&self) -> u32 {
        match self {
            Self::Teal => 0x43d9c9,
            Self::Gold => 0xd8b45e,
            Self::Blue => 0x4d9fff,
            Self::Purple => 0xb48ead,
        }
    }

    pub fn color(&self) -> Rgba {
        rgb(self.hex())
    }
}

/// Palette tokens for UI chrome rendering.
#[derive(Clone, Debug)]
pub struct ChromePalette {
    pub bg_base_start: Rgba,
    pub bg_base_end: Rgba,
    pub bgc_knockout: Rgba,
    pub text_t1: Rgba,
    pub text_t2: Rgba,
    pub text_t3: Rgba,
    pub text_t4: Rgba,
    pub inset: Rgba,
    pub panel: Rgba,
    pub panel2: Rgba,
    pub scrim: Rgba,
    pub glow_teal: Rgba,
    pub glow_gold: Rgba,
}

/// Fixed terminal viewport palette, independent of chrome theme.
#[derive(Clone, Debug)]
pub struct TerminalPalette {
    pub surface: Rgba,
    pub text: Rgba,
    pub dim: Rgba,
    pub success: Rgba,
    pub warn: Rgba,
    pub prompt: Rgba,
}

/// Shell tint colors for badges and metadata.
#[derive(Clone, Debug)]
pub struct ShellColors {
    pub powershell: Rgba,
    pub ssh: Rgba,
    pub wsl: Rgba,
    pub cmd: Rgba,
}

/// Dimensions and spacing tokens.
pub struct Spacing;

impl Spacing {
    pub const TITLEBAR_HEIGHT: Pixels = px(40.0);
    pub const STATUSBAR_HEIGHT: Pixels = px(26.0);
    pub const WORKSPACE_PADDING: Pixels = px(10.0);
    pub const PANE_GAP: Pixels = px(8.0);
    pub const TAB_BAR_HEIGHT: Pixels = px(34.0);
    pub const RAIL_WIDTH: Pixels = px(52.0);
    pub const SIDEBAR_WIDTH: Pixels = px(264.0);
    pub const SIDEBAR_COMPACT_WIDTH: Pixels = px(216.0);
}

/// Corner radii tokens.
pub struct Radii;

impl Radii {
    pub const PANE: Pixels = px(12.0);
    pub const OVERLAY: Pixels = px(16.0);
    pub const ROW: Pixels = px(8.0);
    pub const CHIP: Pixels = px(6.0);
    pub const RAIL_BUTTON: Pixels = px(10.0);
}

/// Typography token definitions.
pub struct Typography;

impl Typography {
    pub const UI_FAMILY: &'static str = "Segoe UI";
    pub const MONO_FAMILY: &'static str = "JetBrains Mono";

    pub const TITLE_SIZE: Pixels = px(13.0);
    pub const BODY_SIZE: Pixels = px(12.5);
    pub const SECONDARY_SIZE: Pixels = px(11.0);
    pub const GROUP_HEADER_SIZE: Pixels = px(10.5);
    pub const META_SIZE: Pixels = px(10.0);
    pub const STATUS_SIZE: Pixels = px(10.5);
}

/// Complete theme bundle holding active mode and tokens.
#[derive(Clone, Debug)]
pub struct Theme {
    pub mode: ThemeMode,
    pub accent: AccentColor,
    pub chrome: ChromePalette,
    pub terminal: TerminalPalette,
    pub shells: ShellColors,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark(AccentColor::Teal)
    }
}

impl Theme {
    /// Constructs the Section 12 dark chrome palette (default).
    pub fn dark(accent: AccentColor) -> Self {
        let accent_color = accent.color();

        let chrome = ChromePalette {
            bg_base_start: rgb(0x0c1114),
            bg_base_end: rgb(0x0a0e11),
            bgc_knockout: rgb(0x0d1215),
            text_t1: rgb(0xdbe6e6),
            text_t2: rgb(0x8fa0a3),
            text_t3: rgb(0x7d8d90),
            text_t4: rgb(0x55666a),
            inset: rgba(0x00000040),
            panel: rgba(0x141b1feb),
            panel2: rgba(0x12191df2),
            scrim: rgba(0x05080a80),
            glow_teal: rgba(0x43d9c912),
            glow_gold: rgba(0xd8b45e0d),
        };

        let terminal = TerminalPalette {
            surface: rgba(0x0d1316cc),
            text: rgb(0xb7c6c6),
            dim: rgb(0x6b7c80),
            success: rgb(0x7fd88f),
            warn: rgb(0xd8b45e),
            prompt: accent_color,
        };

        let shells = ShellColors {
            powershell: rgb(0x43d9c9),
            ssh: rgb(0xd8b45e),
            wsl: rgb(0x7fd88f),
            cmd: rgb(0x9aa7b0),
        };

        Self {
            mode: ThemeMode::Dark,
            accent,
            chrome,
            terminal,
            shells,
        }
    }

    /// Constructs the Section 12 light chrome palette.
    pub fn light(accent: AccentColor) -> Self {
        let accent_color = accent.color();

        let chrome = ChromePalette {
            bg_base_start: rgb(0xf2f5f5),
            bg_base_end: rgb(0xe7ecec),
            bgc_knockout: rgb(0xeef2f2),
            text_t1: rgb(0x1c2527),
            text_t2: rgb(0x3f5054),
            text_t3: rgb(0x5c6c70),
            text_t4: rgb(0x8a9a9e),
            inset: rgba(0x00000012),
            panel: rgba(0xfafcfcf0),
            panel2: rgba(0xfcfcfcf7),
            scrim: rgba(0x5a666a59),
            glow_teal: rgba(0x43d9c91f),
            glow_gold: rgba(0xd8b45e14),
        };

        let terminal = TerminalPalette {
            surface: rgba(0x0d1316cc),
            text: rgb(0xb7c6c6),
            dim: rgb(0x6b7c80),
            success: rgb(0x7fd88f),
            warn: rgb(0xd8b45e),
            prompt: accent_color,
        };

        let shells = ShellColors {
            powershell: rgb(0x0e9a8c),
            ssh: rgb(0xa17e22),
            wsl: rgb(0x3d9a50),
            cmd: rgb(0x5c6c70),
        };

        Self {
            mode: ThemeMode::Light,
            accent,
            chrome,
            terminal,
            shells,
        }
    }
}
