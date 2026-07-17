<!-- PAGE_ID: pandamux_08_configuration -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [crates/pandamux-core/src/config.rs:1-361](crates/pandamux-core/src/config.rs#L1-L361)
- [crates/pandamux-core/src/settings.rs:1-241](crates/pandamux-core/src/settings.rs#L1-L241)
- [crates/pandamux-core/src/keymap.rs:1-838](crates/pandamux-core/src/keymap.rs#L1-L838)
- [crates/pandamux-core/src/home.rs:1-188](crates/pandamux-core/src/home.rs#L1-L188)
- [crates/pandamux-core/src/i18n.rs:1-135](crates/pandamux-core/src/i18n.rs#L1-L135)
- [crates/pandamux-core/src/state.rs:1-40](crates/pandamux-core/src/state.rs#L1-L40)
- [crates/pandamux-ui/src/theme.rs:1-480](crates/pandamux-ui/src/theme.rs#L1-L480)
- [crates/pandamux-app/src/persistence.rs:1-806](crates/pandamux-app/src/persistence.rs#L1-L806)
- [crates/pandamux-app/src/iced_runtime.rs:383-403](crates/pandamux-app/src/iced_runtime.rs#L383-L403)
- [crates/pandamux-app/src/iced_runtime.rs:4436-4461](crates/pandamux-app/src/iced_runtime.rs#L4436-L4461)
- [crates/pandamux-app/src/backend.rs:826-842](crates/pandamux-app/src/backend.rs#L826-L842)
- [resources/shell-integration/pandamux-bash-integration.sh:1-67](resources/shell-integration/pandamux-bash-integration.sh#L1-L67)
- [docs/config.md:1-81](docs/config.md#L1-L81)

</details>

# Configuration

> **Related Pages**: [Core Domain and State](CORE_DOMAIN.md), [Shell Integration and Status](../features/SHELL_INTEGRATION.md)

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_layers -->
## Configuration Sources

PandaMUX's runtime configuration is JSON, not the TOML file described by the older `docs/config.md` note (docs/config.md:1-16); the currently shipping settings pipeline is [`UserSettings`](crates/pandamux-core/src/settings.rs#L22-L27), persisted as `config/settings.json` under the app's per-user data directory. That directory is resolved by [`SessionStore::default_dir`](crates/pandamux-app/src/persistence.rs#L397-L405): `%APPDATA%/pandamux` when `APPDATA` is set (Windows), else `$HOME/.pandamux`, else the OS temp directory as a last resort (crates/pandamux-app/src/persistence.rs:397-405). `SettingsStore`, `SshProfileStore`, and `LauncherPrefsStore` all place their file under a `config/` subfolder of that same base directory (their `default_dir()` methods delegate to `SessionStore::default_dir().join("config")`) (crates/pandamux-app/src/persistence.rs:127-129,264-266,341-343).

`SettingsStore::load` returns `UserSettings::default()` when `config/settings.json` is missing, so a fresh install has no file to create until the first explicit save (crates/pandamux-app/src/persistence.rs:360-368). Every field on `UserSettings` and its nested structs carries `#[serde(default)]`, so a file written by an older build (or a hand-edited partial file) still loads: missing keys silently take their compiled-in default (crates/pandamux-core/src/settings.rs:20-27).

Settings persistence is versioned and defensive: on load, if the on-disk `version` is older than `SETTINGS_SCHEMA_VERSION`, the store copies the original file to `settings.v<old>.bak.json` before rewriting it at the current version; a *newer* on-disk version is rejected outright as `SettingsStoreError::UnsupportedVersion`; and a file that fails to parse is reported as `SettingsStoreError::Corrupt` and left untouched on disk rather than being clobbered (crates/pandamux-app/src/persistence.rs:358-388). The `SshProfileStore` (`ssh-profiles.json`) and the session store follow the identical backup-then-migrate policy (crates/pandamux-app/src/persistence.rs:144-173).

Configuration is distinct from session **state**: the Home dashboard layout ([`HomeLayout`](crates/pandamux-core/src/home.rs#L18-L23), an ordered arrangement of pinned live-session panes) is a field of `AppState` (crates/pandamux-core/src/state.rs:30) and round-trips through `session.json` / named-session files, not through `config/settings.json` — it is user *state*, not a settings knob.

| Location | Contents | Resolved by |
|---|---|---|
| `<default_dir>/config/settings.json` | `UserSettings` (UI, terminal, keyboard) | [`SettingsStore`](crates/pandamux-app/src/persistence.rs#L332-L389) |
| `<default_dir>/config/ssh-profiles.json` | Saved SSH host profiles (secretless) | [`SshProfileStore`](crates/pandamux-app/src/persistence.rs#L118-L174) |
| `<default_dir>/config/launcher.json` | Pinned favorites + recent launches | [`LauncherPrefsStore`](crates/pandamux-app/src/persistence.rs#L255-L292) |
| `<default_dir>/sessions/session.json` | Auto-restored workspace/pane tree + `HomeLayout` | [`SessionStore`](crates/pandamux-app/src/persistence.rs#L391-L531) |
| `resources/themes/*.theme` | Bundled Ghostty-style terminal color schemes | [`themes_dir`](crates/pandamux-app/src/iced_runtime.rs#L4439-L4461) |

Sources: [persistence.rs:1-19](crates/pandamux-app/src/persistence.rs#L1-L19), [persistence.rs:391-405](crates/pandamux-app/src/persistence.rs#L391-L405), [settings.rs:1-27](crates/pandamux-core/src/settings.rs#L1-L27), [home.rs:1-23](crates/pandamux-core/src/home.rs#L1-L23), [state.rs:30](crates/pandamux-core/src/state.rs#L30)
<!-- END:AUTOGEN pandamux_08_configuration_layers -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_config-toml -->
## config.toml Schema

_TBD_ — no TOML config parser exists in the current codebase. `crates/pandamux-core/src/config.rs` (the file the older `docs/config.md` note implies holds a TOML schema) actually implements the [`Theme`](crates/pandamux-core/src/config.rs#L20-L30)/[`ThemeStore`](crates/pandamux-core/src/config.rs#L196-L248) model and Ghostty/`.theme`-file parsing (crates/pandamux-core/src/config.rs:64-114), documented under [Themes and Localization](#themes-and-localization) below. A repo-wide search for `config.toml`, `toml::`, or a `TerminalConfig`-style struct with `font-family`/`cursor-style`/`scrollback-lines` fields turns up nothing outside `docs/config.md` itself; the persisted, versioned schema that actually exists is [`UserSettings`](crates/pandamux-core/src/settings.rs#L22-L27) (JSON), covered in [Settings Model](#settings-model). Treat `docs/config.md`'s TOML example as aspirational/legacy documentation rather than a description of shipping behavior.
<!-- END:AUTOGEN pandamux_08_configuration_config-toml -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_settings -->
## Settings Model

[`UserSettings`](crates/pandamux-core/src/settings.rs#L22-L27) is the single schema behind `config/settings.json`, the Settings UI, and the `config.get` / `config.set` pipe RPC methods (crates/pandamux-core/src/settings.rs:1-8). It is versioned (`SETTINGS_SCHEMA_VERSION = 1`) and every field defaults via `#[serde(default)]` so older files keep loading (crates/pandamux-core/src/settings.rs:14,20-27). `UserSettings::normalize` clamps `terminal.scrollbackLines` into `[SCROLLBACK_LINES_MIN, SCROLLBACK_LINES_MAX]` (1,000-200,000) after every load and every `settings_set` call (crates/pandamux-core/src/settings.rs:16-18,41-47,152-157).

| Field (dotted, camelCase) | Type | Default | Meaning |
|---|---|---|---|
| `version` | `u32` | `1` | Schema version; not settable via `config.set` (crates/pandamux-core/src/settings.rs#L23,L130-L132) |
| `ui.theme` | `String` | `"dark"` | Chrome theme name (`"dark"` \| `"light"`); unknown values fall back at the mapping layer (crates/pandamux-core/src/settings.rs#L56-L67) |
| `ui.accent` | `String` | `"teal"` | Accent color name (`"teal"` \| `"gold"` \| `"blue"` \| `"mauve"`) (crates/pandamux-core/src/settings.rs#L58,L66) |
| `ui.showStatusBar` | `bool` | `true` | Show the status bar (crates/pandamux-core/src/settings.rs#L60,L68) |
| `terminal.scrollbackLines` | `u32` | `10,000` | Scrollback history per session, clamped to 1,000-200,000 (crates/pandamux-core/src/settings.rs#L77,L90,L16-L18) |
| `terminal.welcomePromptEnabled` | `bool` | `true` | Show the tool chooser on fresh bare terminals (spec 2.7) (crates/pandamux-core/src/settings.rs#L78-L79,L91) |
| `terminal.rightClickPasteOptin` | `bool` | `false` | Opt-in classic right-click paste; off by default so right-click opens the context menu (crates/pandamux-core/src/settings.rs#L80-L82,L92) |
| `terminal.confirmCloseOnRunning` | `bool` | `true` | Confirm closing a tab whose shell is still running (crates/pandamux-core/src/settings.rs#L83-L84,L93) |
| `keyboard.passThrough` | `Vec<String>` | `[]` | Chords always forwarded to the terminal even when bound (crates/pandamux-core/src/settings.rs#L101-L103) |
| `keyboard.overrides` | `BTreeMap<String, Option<String>>` | `{}` | Action id → chord string, or `null` to unbind; unknown ids/unparseable chords warn and keep the default (crates/pandamux-core/src/settings.rs#L104-L106) |

`settings_get`/`settings_set` (crates/pandamux-core/src/settings.rs:111-158) operate on dotted camelCase paths against the serialized JSON tree: `settings_get` walks `serde_json::Value::get` segment by segment and errors on an unknown key (crates/pandamux-core/src/settings.rs:111-123); `settings_set` rejects an empty path or `"version"` outright, requires every path segment to already exist as an object key (except under `keyboard.overrides`, where insertion is allowed), and re-deserializes the whole tree back into `UserSettings` so a type mismatch is rejected with the serde error rather than silently corrupting the struct (crates/pandamux-core/src/settings.rs:129-158).

```rust
// crates/pandamux-core/src/settings.rs:109-123
pub fn settings_get(settings: &UserSettings, key: &str) -> Result<Value, String> {
    let root = serde_json::to_value(settings).map_err(|error| error.to_string())?;
    if key.is_empty() {
        return Ok(root);
    }
    let mut node = &root;
    for segment in key.split('.') {
        node = node
            .get(segment)
            .ok_or_else(|| format!("unknown settings key: {key}"))?;
    }
    Ok(node.clone())
}
```

Sources: [settings.rs:1-241](crates/pandamux-core/src/settings.rs#L1-L241), [persistence.rs:294-389](crates/pandamux-app/src/persistence.rs#L294-L389)
<!-- END:AUTOGEN pandamux_08_configuration_settings -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_keymap -->
## Keymap

[`Keymap`](crates/pandamux-core/src/keymap.rs#L420-L423) is a single, framework-agnostic table of `(KeyChord, Action)` bindings that drives both the Iced key-event decoder and every display surface (Settings Keyboard tab, cheat sheet, palette hints), so the label list can never drift from the decode table (crates/pandamux-core/src/keymap.rs:1-12). `Keymap::defaults()` builds the built-in table (crates/pandamux-core/src/keymap.rs:434-481); `Keymap::with_settings` layers the user's `keyboard.overrides` and `keyboard.passThrough` on top, warning (and keeping the default) on an unknown action id or an unparseable chord string (crates/pandamux-core/src/keymap.rs:483-519). An override with a `null` chord unbinds the action entirely; a chord override both replaces every default chord for that action and steals the chord away from any other action bound to it (crates/pandamux-core/src/keymap.rs:493-509).

Each `Action` variant carries a stable `id()` string (the key used in `keyboard.overrides`), a display `label()`, and a `category()` for cheat-sheet grouping (crates/pandamux-core/src/keymap.rs:258-363,375-408). `allowed_with_overlay()` marks the handful of actions (command palette, settings, cheat sheet) that still fire while a centered overlay is open; every other action is swallowed instead of acting behind the overlay's back, e.g. so Ctrl+W typed into the palette does not close a pane (crates/pandamux-core/src/keymap.rs:365-373).

| Chord | Action | Category |
|---|---|---|
| `Ctrl+K` / `Ctrl+Shift+P` | `commandPalette` | General (crates/pandamux-core/src/keymap.rs#L445-L446) |
| `Ctrl+T` | `newSession` | General (crates/pandamux-core/src/keymap.rs#L447) |
| `Ctrl+,` | `openSettings` | General (crates/pandamux-core/src/keymap.rs#L448) |
| `Ctrl+F` | `find` | General (crates/pandamux-core/src/keymap.rs#L449) |
| `Ctrl+/` / `F1` | `cheatSheet` | General (crates/pandamux-core/src/keymap.rs#L454-L455) |
| `Ctrl+D` | `splitRight` | Panes & tabs (crates/pandamux-core/src/keymap.rs#L456) |
| `Ctrl+Shift+D` | `splitDown` | Panes & tabs (crates/pandamux-core/src/keymap.rs#L457) |
| `Ctrl+W` | `closeTab` | Panes & tabs (crates/pandamux-core/src/keymap.rs#L458) |
| `Ctrl+Enter` | `zoomPane` | Panes & tabs (crates/pandamux-core/src/keymap.rs#L459) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | `nextTab` / `prevTab` | Panes & tabs (crates/pandamux-core/src/keymap.rs#L460-L461) |
| `Ctrl+0` / `Ctrl+Home` | `goHome` | Projects (crates/pandamux-core/src/keymap.rs#L462-L463) |
| `Ctrl+1`..`Ctrl+9` | `focusProject{n}` | Projects (crates/pandamux-core/src/keymap.rs#L474-L476) |
| `Ctrl+C` | `copyOrInterrupt` | Terminal (crates/pandamux-core/src/keymap.rs#L464) |
| `Ctrl+Shift+C` | `copy` | Terminal (crates/pandamux-core/src/keymap.rs#L465) |
| `Ctrl+V` / `Ctrl+Shift+V` | `paste` | Terminal (crates/pandamux-core/src/keymap.rs#L466-L467) |
| `Shift+PageUp` / `Shift+PageDown` | `scrollPageUp` / `scrollPageDown` | Terminal (crates/pandamux-core/src/keymap.rs#L468-L472) |

Digit chords match by *physical* scan position (`KeySpec::Digit`), not the shifted character, so `Ctrl+1`..`Ctrl+9` resolve correctly on layouts (e.g. AZERTY) where the digit row requires Shift (crates/pandamux-core/src/keymap.rs:121-124,521-536). `Keymap::resolve` checks pass-through chords first (returning `None` so the press reaches the terminal), then the physical digit, then the lowercased character, then the named key, in that order (crates/pandamux-core/src/keymap.rs:524-571).

Sources: [keymap.rs:1-838](crates/pandamux-core/src/keymap.rs#L1-L838)
<!-- END:AUTOGEN pandamux_08_configuration_keymap -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_themes -->
## Themes and Localization

Terminal color themes are modeled by [`Theme`](crates/pandamux-core/src/config.rs#L20-L30) in `pandamux-core::config` (background, foreground, cursor, selection background, and up to a 16-entry ANSI palette) and loaded either from a bundled Ghostty-style `.theme` file via `parse_ghostty_theme` (line-oriented `key = value`, `#` comments, unknown keys ignored) or imported from a Windows Terminal `settings.json` via `import_windows_terminal` (crates/pandamux-core/src/config.rs#L61-L114,L137-L192). [`ThemeStore`](crates/pandamux-core/src/config.rs#L196-L248) holds the loaded set plus which theme is active, inserting by name (replacing any existing theme of the same name) and refusing to activate an unknown name (crates/pandamux-core/src/config.rs#L206-L239). 29 bundled schemes ship under `resources/themes/*.theme` (e.g. `Dracula.theme`, `Nord.theme`, `Tokyo Night.theme`, `Catppuccin Mocha.theme`).

At startup, `PandaMuxRuntime::load_bundled_themes` resolves the themes directory via `themes_dir()` and inserts one `Theme` per `.theme` file it finds, keyed by file stem (crates/pandamux-app/src/iced_runtime.rs#L383-L403). `themes_dir()` checks, in order: the `PANDAMUX_THEMES_DIR` environment variable, `<exe dir>/resources/themes`, then walks up from the current working directory looking for a `resources/themes` folder (the dev-checkout case); it returns `None` if none is found (crates/pandamux-app/src/iced_runtime.rs#L4436-L4461).

```rust
// crates/pandamux-app/src/iced_runtime.rs:4436-4461
fn themes_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("PANDAMUX_THEMES_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join("resources").join("themes");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("resources").join("themes");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}
```

The Iced UI layer's own `pandamux-ui::theme` module is a *different* concept: chrome design tokens ([`UiTheme`](crates/pandamux-ui/src/theme.rs#L19-L23) dark/light, [`Accent`](crates/pandamux-ui/src/theme.rs#L37-L43) teal/gold/blue/mauve, layout/typography constants, and the [`Palette`](crates/pandamux-ui/src/theme.rs#L281-L304) of chrome colors), independent of the terminal color scheme (crates/pandamux-ui/src/theme.rs#L1-L23). [`TermScheme::from_theme`](crates/pandamux-ui/src/theme.rs#L246-L267) bridges the two: it maps a loaded core `Theme`'s background/foreground/cursor/palette hex strings onto the fixed-dark terminal scheme's color slots, falling back to the built-in default for any missing or unparseable color (crates/pandamux-ui/src/theme.rs#L242-L267).

Localization is handled by [`Locale`](crates/pandamux-core/src/i18n.rs#L11-L17) (`En`/`Fr`/`Ar`/`Ja`) and [`Localizer`](crates/pandamux-core/src/i18n.rs#L71-L102), a minimal in-binary catalog ported from the Electron `site/i18n.js` language switching (crates/pandamux-core/src/i18n.rs#L1-L4). `Localizer::t` looks up a key in the active locale's catalog, falls back to English, and finally falls back to the key itself when no translation exists at all (crates/pandamux-core/src/i18n.rs#L94-L102). Coverage is intentionally partial today: English and French have entries for all six catalog keys (`new_session`, `settings`, `notifications`, `find`, `sessions`, `command_palette`), while Arabic and Japanese only cover `settings` and `notifications` (crates/pandamux-core/src/i18n.rs#L44-L67).

| Locale | Code | Catalog coverage |
|---|---|---|
| English | `en` | All 6 keys (default) (crates/pandamux-core/src/i18n.rs#L45-L50) |
| French | `fr` | All 6 keys (crates/pandamux-core/src/i18n.rs#L52-L57) |
| Arabic | `ar` | `settings`, `notifications` only; rest fall back to English (crates/pandamux-core/src/i18n.rs#L59-L61) |
| Japanese | `ja` | `settings`, `notifications` only; rest fall back to English (crates/pandamux-core/src/i18n.rs#L62-L63) |

Sources: [config.rs:1-248](crates/pandamux-core/src/config.rs#L1-L248), [i18n.rs:1-135](crates/pandamux-core/src/i18n.rs#L1-L135), [theme.rs (ui):1-267](crates/pandamux-ui/src/theme.rs#L1-L267), [iced_runtime.rs:383-403,4436-4461](crates/pandamux-app/src/iced_runtime.rs#L383-L403)
<!-- END:AUTOGEN pandamux_08_configuration_themes -->

---

<!-- BEGIN:AUTOGEN pandamux_08_configuration_env -->
## Environment Variables

`pandamux_env` builds the `PANDAMUX_*` environment injected into every spawned shell/agent PTY so shell-integration scripts, the CLI, and orchestrator hooks can find the pipe and identify their surface/agent; it is a direct port of the env vars the Electron build set on spawned shells (crates/pandamux-app/src/backend.rs#L826-L842).

```rust
// crates/pandamux-app/src/backend.rs:831-842
pub(crate) fn pandamux_env(surface_id: &str, agent_id: Option<&str>) -> Vec<(String, String)> {
    let pipe = std::env::var("PANDAMUX_PIPE").unwrap_or_else(|_| r"\\.\pipe\pandamux".to_string());
    let mut env = vec![
        ("PANDAMUX".to_string(), "1".to_string()),
        ("PANDAMUX_SURFACE_ID".to_string(), surface_id.to_string()),
        ("PANDAMUX_PIPE".to_string(), pipe),
    ];
    if let Some(agent_id) = agent_id {
        env.push(("PANDAMUX_AGENT_ID".to_string(), agent_id.to_string()));
    }
    env
}
```

| Variable | Meaning | Source |
|---|---|---|
| `PANDAMUX` | Set to `"1"` on every spawned shell/agent so scripts can detect they are running inside PandaMUX (crates/pandamux-app/src/backend.rs#L834); the bash integration script also exports it unconditionally on load ([pandamux-bash-integration.sh:5](resources/shell-integration/pandamux-bash-integration.sh#L5)) |
| `PANDAMUX_SURFACE_ID` | The id of the terminal surface the shell is running in; read by shell-integration hooks to tag `report_pwd`/`report_git_branch`/`report_shell_state`/`ports_kick` messages (crates/pandamux-app/src/backend.rs#L835; [pandamux-bash-integration.sh:16,22,41,43,45,50](resources/shell-integration/pandamux-bash-integration.sh#L16-L50)) |
| `PANDAMUX_PIPE` | Named-pipe path the CLI/hooks connect to; defaults to `\\.\pipe\pandamux` if the parent process did not already set `PANDAMUX_PIPE` (crates/pandamux-app/src/backend.rs#L832,L836) |
| `PANDAMUX_AGENT_ID` | Present only for agent-spawned surfaces; minted before the PTY spawns so the orchestrator's on-agent-stop / on-tool-use hooks can key per-agent state on it (crates/pandamux-app/src/backend.rs#L802-L804,L838-L839) |
| `PANDAMUX_CLI` | _TBD_ — no `pandamux-app` or shell-integration code sets this variable; it is only read (as an already-present env var) by the third-party `resources/opencode-plugin/pandamux.js` integration, not written by PandaMUX itself. The Shell Integration section of `CLAUDE.md` lists it as app-set, but that does not match current code. |

Sources: [backend.rs:826-842](crates/pandamux-app/src/backend.rs#L826-L842), [pandamux-bash-integration.sh:1-67](resources/shell-integration/pandamux-bash-integration.sh#L1-L67)
<!-- END:AUTOGEN pandamux_08_configuration_env -->

---
