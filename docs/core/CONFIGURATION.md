<!-- PAGE_ID: pandamux_06_configuration -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [config-loader.ts:1-376](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L1-L376)
- [settings-store.ts:1-48](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L1-L48)
- [user-config.ts:1-232](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L1-L232)
- [theme-loader.ts:1-184](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L1-L184)
- [toml-parser.ts:1-243](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/toml-parser.ts#L1-L243)
- [settings-slice.ts:1-509](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L1-L509)
- [ipc-handlers.ts:116-282](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L116-L282)
- [index.ts (preload):45-125](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/preload/index.ts#L45-L125)
- [types.ts:38-113](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/types.ts#L38-L113)
- [pty-manager.ts:100-270](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L100-L270)
- [instance.ts:1-93](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/instance.ts#L1-L93)

</details>

# Configuration

> **Related Pages**: [Renderer and State](RENDERER_AND_STATE.md), [Shell Integration and Status](../features/SHELL_INTEGRATION.md)

---

<!-- BEGIN:AUTOGEN pandamux_06_configuration_layers -->
## Configuration Layers

PandaMUX has no single config file; instead it layers three independent stores plus one file that is entirely optional, and each layer's job is to survive a specific failure mode (a fresh install, a portable-zip update, or a per-pane override).

| Layer | Source | Applied |
|---|---|---|
| Built-in defaults | Code constants: `DEFAULT_TERMINAL_PREFS`, `DEFAULT_SIDEBAR_PREFS`, etc. ([settings-slice.ts:311-319](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L311-L319)) and `getDefaultTheme()` (Monokai) ([theme-loader.ts:152-183](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L152-L183)) | Lowest; used when no other layer supplies a value |
| Persisted app settings | `%APPDATA%\pandamux\settings.json`, written by `saveSetting()` and hydrated into the Zustand `settings-slice` on module load ([settings-store.ts:14-43](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L14-L43)) | Overrides defaults at runtime; edited via the Settings UI |
| User config file | `~/.pandamux/config.toml`, parsed by `loadUserConfig()` ([user-config.ts:76-97](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L76-L97)) | Overrides persisted settings at startup and whenever `pandamux reload-config` runs |
| Per-pane / CLI override | e.g. `pandamux split --color-scheme NAME` | Highest; wins for that one surface only |

The `user-config.ts` header comment states the intended contract directly: "File-wins-at-startup, app-wins-at-runtime: this data seeds the store on boot; users can still tweak via the Settings UI afterwards" ([user-config.ts:30-32](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L30-L32)). In practice this means `config.toml` is read once at boot (and again on an explicit reload), while any change made through the Settings UI during that session takes effect immediately and is not clobbered until the next reload.

Two of the three persistent layers exist specifically because of an app-packaging quirk: pandamux ships as a portable zip extracted to a version-numbered folder, so the Chromium `file://` origin (and therefore `localStorage`) changes on every update, silently discarding font/theme/shortcut customizations. Both `settings-store.ts` and `settings-slice.ts` carry this as their motivating comment ([settings-store.ts:5-13](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L5-L13), [settings-slice.ts:5-16](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L5-L16)).

Sources: [settings-store.ts:1-48](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L1-L48), [user-config.ts:1-97](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L1-L97), [settings-slice.ts:1-16](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L1-L16)
<!-- END:AUTOGEN pandamux_06_configuration_layers -->

---

<!-- BEGIN:AUTOGEN pandamux_06_configuration_settings-store -->
## Settings Store

The settings store is split across two files: `settings-store.ts` in the main process persists an arbitrary key/value map to disk, and `settings-slice.ts` in the renderer defines the typed preference shapes and exposes them to React components through Zustand.

`settings-store.ts` reads and writes a single JSON file at `%APPDATA%\pandamux\settings.json` (the same stable directory `session.json` lives in), using an atomic temp-file-then-rename write to avoid partial writes ([settings-store.ts:14-43](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L14-L43)):

```typescript
const SETTINGS_FILE = path.join(getAppDataDir(), 'settings.json');

type SettingsMap = Record<string, unknown>;

export function loadSettings(): SettingsMap {
  try {
    if (!fs.existsSync(SETTINGS_FILE)) return {};
    const raw = fs.readFileSync(SETTINGS_FILE, 'utf-8');
    const parsed = JSON.parse(raw);
    return parsed && typeof parsed === 'object' ? (parsed as SettingsMap) : {};
  } catch {
    return {};
  }
}

export function saveSetting(key: string, value: unknown): void {
  try {
    const dir = getAppDataDir();
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    const current = loadSettings();
    current[key] = value;
    // Atomic write: temp file then rename (mirrors session-persistence.ts).
    const tmp = SETTINGS_FILE + '.tmp';
    fs.writeFileSync(tmp, JSON.stringify(current, null, 2), 'utf-8');
    if (fs.existsSync(SETTINGS_FILE)) fs.unlinkSync(SETTINGS_FILE);
    fs.renameSync(tmp, SETTINGS_FILE);
  } catch (err) {
    console.error('Failed to save setting:', err);
  }
}
```

Two IPC endpoints expose this store: a synchronous `settings:get-all-sync` so the renderer can hydrate state at module-load time with no async flash, and a fire-and-forget `settings:set` ([ipc-handlers.ts:277-282](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L277-L282)), reached from the renderer via `window.pandamux.settings.getAllSync()` / `.set()` ([index.ts (preload):115-125](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/preload/index.ts#L115-L125)).

`settings-slice.ts` defines one storage key per preference group and loads each with a merge-over-defaults pattern, falling back to (and migrating forward from) legacy `localStorage` values when the file store has nothing yet ([settings-slice.ts:18-61](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L18-L61)):

| Storage key | Preference group | Default constant |
|---|---|---|
| `pandamux-workspace-prefs` | `WorkspacePrefs` (placement, default shell, welcome screen, auto-diff tab) | `DEFAULT_WORKSPACE_PREFS` ([settings-slice.ts:274-280](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L274-L280)) |
| `pandamux-terminal-prefs` | `TerminalPrefs` (font, theme, cursor, scrollback, user color schemes) | `DEFAULT_TERMINAL_PREFS` ([settings-slice.ts:311-319](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L311-L319)) |
| `pandamux-sidebar-prefs` | `SidebarPrefs` (git branch/PR/ports visibility, active-tab indicator) | `DEFAULT_SIDEBAR_PREFS` ([settings-slice.ts:246-255](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L246-L255)) |
| `pandamux-notification-prefs` | `NotificationPrefs` (toast, taskbar flash, sound, agent notify) | `DEFAULT_NOTIFICATION_PREFS` ([settings-slice.ts:335-343](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L335-L343)) |
| `pandamux-browser-prefs` | `BrowserPrefs` (search engine, devtools icon, open-on-startup) | `DEFAULT_BROWSER_PREFS` ([settings-slice.ts:354-358](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L354-L358)) |
| `pandamux-appearance-prefs` | `AppearancePrefs` (app UI theme: system/dark/light) | `DEFAULT_APPEARANCE_PREFS` ([settings-slice.ts:370-375](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L370-L375)) |
| `pandamux-shortcuts` | `Record<ShortcutAction, ShortcutBinding>` (51+ keybindings) | `DEFAULT_SHORTCUTS` ([settings-slice.ts:176-231](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L176-L231)) |
| `pandamux-quick-launch-profiles` | `QuickLaunchProfile[]` (global `+` dropdown presets) | `[]` (via `loadPersistedArray`) ([settings-slice.ts:65-79](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L65-L79)) |
| `pandamux-language` | UI language (`en`/`fr`/`zh`) | OS/browser locale, else English (via `loadPersistedLanguage`) ([settings-slice.ts:84-98](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L84-L98)) |

One state field, `broadcastInputActive`, is deliberately excluded from this table: it is runtime-only and never persisted, because silently restoring "type into every pane at once" mode on next launch was judged too dangerous ([settings-slice.ts:393-399](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L393-L399)).

Sources: [settings-store.ts:1-48](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L1-L48), [settings-slice.ts:1-509](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/renderer/store/settings-slice.ts#L1-L509), [ipc-handlers.ts:274-282](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L274-L282)
<!-- END:AUTOGEN pandamux_06_configuration_settings-store -->

---

<!-- BEGIN:AUTOGEN pandamux_06_configuration_user-config -->
## User Config

`user-config.ts` reads an optional TOML file at `~/.pandamux/config.toml` (`getConfigPath()` resolves it via `os.homedir()`, so on Windows it is `%USERPROFILE%\.pandamux\config.toml`) and maps it onto a partial `TerminalPrefs`-like shape plus an app-appearance setting ([user-config.ts:71-97](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L71-L97)).

```typescript
export function getConfigPath(): string {
  const home = os.homedir();
  return path.join(home, '.pandamux', 'config.toml');
}

export function loadUserConfig(filePath: string = getConfigPath()): UserConfig {
  const errors: string[] = [];
  if (!fs.existsSync(filePath)) {
    return { path: filePath, errors };
  }

  let text: string;
  try {
    text = fs.readFileSync(filePath, 'utf-8');
  } catch (e: any) {
    return { path: filePath, errors: [`read failed: ${e?.message || e}`] };
  }

  let parsed: TomlTable;
  try {
    parsed = parseToml(text);
  } catch (e: any) {
    return { path: filePath, errors: [`parse failed: ${e?.message || e}`] };
  }

  return { ...mapToConfig(parsed, errors), path: filePath, errors };
}
```

Loading is entirely defensive: a missing file returns an empty config with no error, and a bad key is skipped with a message pushed onto `errors[]` rather than thrown ([user-config.ts:99-102](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L99-L102)). Both `kebab-case` and `camelCase` keys are accepted for every field, e.g. `terminal['font-family'] ?? terminal.fontFamily` ([user-config.ts:179-198](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L179-L198)).

| Key | Type | Notes |
|---|---|---|
| `[terminal] font-family` | string | Overrides `TerminalPrefs.fontFamily` ([user-config.ts:179-180](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L179-L180)) |
| `[terminal] font-size` | number | ([user-config.ts:182-183](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L182-L183)) |
| `[terminal] cursor-style` | `"block" \| "underline" \| "bar"` | Invalid value pushes an error, key is skipped ([user-config.ts:185-192](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L185-L192)) |
| `[terminal] cursor-blink` | boolean | ([user-config.ts:194-195](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L194-L195)) |
| `[terminal] scrollback-lines` | number | ([user-config.ts:197-198](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L197-L198)) |
| `[terminal.colors] default` (or `theme`) | string | Global default color-scheme name; bundled theme or a key under `schemes` ([user-config.ts:164-165](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L164-L165)) |
| `[terminal.colors.schemes.<name>]` | table | `background`, `foreground`, `cursor`/`cursor-color`, `cursor-text`, `selection-background`, `selection-foreground`, `palette` (up to 16 entries) ([user-config.ts:130-147](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L130-L147)) |
| `[appearance] ui-theme` | `"light" \| "dark" \| "system"` | App chrome theme, independent of terminal color scheme (issue #67) ([user-config.ts:207-219](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L207-L219)) |

The file is parsed by a small hand-rolled TOML implementation (`toml-parser.ts`) that intentionally supports only the subset `config.toml` needs (tables, dotted table paths, quoted keys, strings, numbers, booleans, and possibly multi-line arrays) and explicitly does not support inline tables, datetimes, multi-line strings, or non-decimal numbers ([toml-parser.ts:1-17](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/toml-parser.ts#L1-L17)). It throws `Error` on malformed input, which `loadUserConfig()` catches and turns into a non-fatal `errors[]` entry.

The parsed config is surfaced to the renderer as `window.pandamux.config.getUserConfig()` / `.reloadUserConfig()` / `.getUserConfigPath()`, backed by `IPC_CHANNELS.CONFIG_GET_USER_CONFIG`, `CONFIG_RELOAD_USER_CONFIG`, and the ad-hoc `'config:getUserConfigPath'` channel ([ipc-handlers.ts:144-161](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L144-L161)); a reload broadcasts `CONFIG_USER_CONFIG_UPDATED` to every open window so all surfaces live-apply the new prefs ([ipc-handlers.ts:149-158](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L149-L158)). This is what the CLI's `pandamux reload-config` (an alias for `config reload`) ultimately triggers.

Sources: [user-config.ts:1-232](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/user-config.ts#L1-L232), [toml-parser.ts:1-243](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/toml-parser.ts#L1-L243), [ipc-handlers.ts:144-161](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L144-L161)
<!-- END:AUTOGEN pandamux_06_configuration_user-config -->

---

<!-- BEGIN:AUTOGEN pandamux_06_configuration_themes -->
## Theme Loading

`theme-loader.ts` owns the Ghostty-style `.theme` file format (`key = value` pairs, `#` comments, and `palette = N=RRGGBB` lines for the 16 ANSI colors) used both for bundled themes and for parsing arbitrary theme text ([theme-loader.ts:9-65](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L9-L65)).

Bundled themes live in `resources/themes/`; at this commit the directory contains 29 tracked `.theme` files (e.g. `Dracula.theme`, `Nord.theme`, `Gruvbox Dark.theme`) ([resources/themes/](https://github.com/BoardPandas/Pandamux/tree/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/resources/themes)). `getThemesDir()` resolves the directory relative to `process.resourcesPath` when packaged, or `../../resources/themes` in dev, guarding the `electron` import in a `try/catch` so the same module also runs under Vitest ([theme-loader.ts:80-91](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L80-L91)). `scanThemesDir()` reads every file in that directory, derives the theme name from the filename (`path.parse(entry).name`), and skips any file that fails to parse ([theme-loader.ts:93-120](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L93-L120)).

`getThemeByName()` resolves a theme by exact match, then case-insensitive match, falling back to the built-in default when nothing matches ([theme-loader.ts:135-147](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L135-L147)):

```typescript
export function getThemeByName(name: string | undefined | null): ThemeConfig {
  if (!name) return getDefaultTheme();
  const bundled = loadBundledThemes();
  // Exact match
  const direct = bundled.get(name);
  if (direct) return direct;
  // Case-insensitive match
  const target = name.toLowerCase();
  for (const [key, theme] of bundled) {
    if (key.toLowerCase() === target) return theme;
  }
  return getDefaultTheme();
}
```

The built-in default is a hard-coded Monokai palette ([theme-loader.ts:152-183](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L152-L183)), returned whenever no name is given or no match is found. `ThemeConfig` itself is a fixed shape: `name`, `background`, `foreground`, `cursor`, `cursorText`, `selectionBackground`, `selectionForeground`, a 16-entry ANSI `palette`, `fontFamily`, `fontSize`, and `backgroundOpacity` ([types.ts:101-113](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/types.ts#L101-L113)).

Themes reach the renderer through `window.pandamux.config.getTheme(name?)` and `.getThemeList()`, both wired to `IPC_CHANNELS.CONFIG_GET_THEME` / `CONFIG_GET_THEME_LIST` ([index.ts (preload):45-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/preload/index.ts#L45-L47)). The list handler always prepends `'Monokai'` before the bundled names and de-duplicates/sorts the result ([ipc-handlers.ts:122-126](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L122-L126)), so `'Monokai'` is always a valid selection even though it has no corresponding `.theme` file on disk.

Sources: [theme-loader.ts:1-184](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/theme-loader.ts#L1-L184), [ipc-handlers.ts:116-126](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L116-L126), [types.ts:101-113](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/types.ts#L101-L113)
<!-- END:AUTOGEN pandamux_06_configuration_themes -->

---

<!-- BEGIN:AUTOGEN pandamux_06_configuration_import -->
## Terminal Config Import

`config-loader.ts` imports settings from two third-party terminal emulators (Windows Terminal and Ghostty) and from a per-project `.pandamux.json` file, all exposed under `window.pandamux.config.*`.

**Windows Terminal.** `parseWindowsTerminalConfig()` reads `%LOCALAPPDATA%\Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json` and delegates to `parseWindowsTerminalSettingsJson()`, which is exported separately so tests can call it without touching the filesystem ([config-loader.ts:112-182](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L112-L182)). It resolves the default profile (by `defaultProfile` GUID, else the first entry) and its color scheme (by name, else the first scheme), then maps the WT scheme's named ANSI colors (or numbered `color0`..`color15` fallbacks) onto a `ThemeConfig` palette via `schemeToTheme()` ([config-loader.ts:68-110](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L68-L110)):

```typescript
function schemeToTheme(profile: WTProfile, scheme: WTColorScheme): ThemeConfig {
  const palette: string[] = [
    normalizeColor(scheme.black || scheme['color0'] || ''),
    normalizeColor(scheme.red || scheme['color1'] || ''),
    // ... 14 more entries, named-or-numbered fallback per slot
  ];

  const fontFace = (profile.font?.face) || profile.fontFace || 'Cascadia Mono';
  const fontSize = profile.font?.size || profile.fontSize || 13;

  return {
    name: scheme.name || 'Windows Terminal',
    background: normalizeColor(scheme.background || ''),
    foreground: normalizeColor(scheme.foreground || ''),
    cursor: normalizeColor(scheme.cursorColor || ''),
    // ...
    palette,
    fontFamily: fontFace,
    fontSize,
    backgroundOpacity: 1.0,
  };
}
```

A separate function, `importWindowsTerminalProfiles()`, turns every non-hidden WT profile into a `QuickLaunchProfile`, mapping `commandline` to `shell` and expanding `%ENV%`-style `startingDirectory` tokens against `process.env` ([config-loader.ts:238-273](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L238-L273)).

**Ghostty.** `parseGhosttyConfig()` reads `~/.config/ghostty/config` (via `USERPROFILE`/`HOME`) as plain `key = value` text, optionally resolving a `theme = <name>` directive against the same bundled-theme map used for the in-app theme picker so config values layer over (rather than replace) the named theme ([config-loader.ts:283-369](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L283-L369)).

**Project quick-launch profiles.** `loadProjectProfiles(cwd)` reads `<cwd>/.pandamux.json` (mirroring cmux's `cmux.json`), accepting either a bare array or `{ "profiles": [...] }`, and never throws; a missing or malformed file just yields `[]` ([config-loader.ts:212-231](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L212-L231)). Each raw entry is validated by `sanitizeProfile()`, which requires a non-empty `name`, restricts `type` to `terminal | browser | markdown`, and drops any field with the wrong JS type ([config-loader.ts:188-210](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L188-L210)).

| Preload method | IPC channel | Loader function |
|---|---|---|
| `config.importWindowsTerminal()` | `CONFIG_IMPORT_WT` | `parseWindowsTerminalConfig()` ([ipc-handlers.ts:128-130](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L128-L130)) |
| `config.importGhostty()` | `CONFIG_IMPORT_GHOSTTY` | `parseGhosttyConfig()` ([ipc-handlers.ts:132-134](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L132-L134)) |
| `config.getProjectProfiles(cwd)` | `'config:getProjectProfiles'` | `loadProjectProfiles(cwd)` ([ipc-handlers.ts:136-139](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L136-L139)) |
| `config.importWindowsTerminalProfiles()` | `'config:importWindowsTerminalProfiles'` | `importWindowsTerminalProfiles()` ([ipc-handlers.ts:140-142](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L140-L142)) |

All four parsers are defensive: every top-level function is wrapped in `try { ... } catch { return null / [] }`, so a malformed or absent third-party config file never crashes pandamux; it just yields no import.

Sources: [config-loader.ts:1-376](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/config-loader.ts#L1-L376), [ipc-handlers.ts:128-142](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L128-L142)
<!-- END:AUTOGEN pandamux_06_configuration_import -->

---

<!-- BEGIN:AUTOGEN pandamux_06_configuration_env -->
## Environment Variables

pandamux does not read environment variables as *input* configuration; instead it *sets* a fixed set of `PANDAMUX_*` variables into every PTY it spawns so that shell-integration scripts, the CLI, and hooks running inside that shell can identify and reach the running instance. `pty-manager.ts` builds this env block once per PTY create ([pty-manager.ts:227-239](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L227-L239)):

```typescript
const processEnvClean = Object.fromEntries(
  Object.entries(process.env).filter((entry): entry is [string, string] => entry[1] !== undefined)
);
const env: { [key: string]: string } = {
  ...processEnvClean,
  ...options.env,
  PANDAMUX: '1',
  PANDAMUX_SURFACE_ID: id,
  PANDAMUX_PIPE: getPipePath(),
  PANDAMUX_PIPE_TOKEN: readPipeToken(),
  PANDAMUX_CLI: cliPath,
};
```

| Variable | Set by | Purpose |
|---|---|---|
| `PANDAMUX` | `pty-manager.ts` ([pty-manager.ts:234](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L234)); also hard-set to `1` in the cmd integration script ([pandamux-cmd-integration.cmd:6](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shell-integration/pandamux-cmd-integration.cmd#L6)) | Presence check: "this shell is running inside pandamux" |
| `PANDAMUX_SURFACE_ID` | `pty-manager.ts` ([pty-manager.ts:235](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L235)) | The surface (tab)'s ID; since PTY IDs = Surface IDs, this ties shell-reported events (shell state, git/PR/port polling) back to the exact pane |
| `PANDAMUX_PIPE` | `pty-manager.ts`, from `getPipePath()` ([pty-manager.ts:236](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L236), [instance.ts:19-21](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/instance.ts#L19-L21)) | The named pipe path (`\\.\pipe\pandamux`, or `\\.\pipe\pandamux-<name>` for a named instance) the CLI and shell integration write to |
| `PANDAMUX_CLI` | `pty-manager.ts` ([pty-manager.ts:238](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L238)) | Filesystem path to the bundled `pandamux.js` CLI entry point, invoked by shell wrapper functions (`pandamux() { node "$PANDAMUX_CLI" "$@"; }` in bash, `function pandamux { node "$env:PANDAMUX_CLI" @args }` in PowerShell) |

Additional `PANDAMUX_*` variables exist for narrower purposes and are also worth knowing about:

| Variable | Set by | Purpose |
|---|---|---|
| `PANDAMUX_PIPE_TOKEN` | `pty-manager.ts` via `readPipeToken()` ([pty-manager.ts:237](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L237)); generated once per instance by `ensurePipeToken()` in main ([index.ts:80-84](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L80-L84)) | Auth token required to authenticate privileged V2 pipe requests (`agent.spawn`, `browser.eval`, `markdown.load_file`, etc.) |
| `PANDAMUX_INTEGRATION` | `pty-manager.ts`, WSL branch only ([pty-manager.ts:119](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L119)) | Marks that shell integration is active inside a WSL distro |
| `PANDAMUX_INSTANCE` | User-set before launch; read by `instance.ts` ([instance.ts:15-16](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/instance.ts#L15-L16)) and `index.ts` ([index.ts:214](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L214)) | Runs pandamux as a side-by-side named instance: suffixes the pipe path and `%APPDATA%` directory so a dev build and an installed build don't collide |
| `PANDAMUX_PS1_SCRIPT` | `pty-manager.ts`, PowerShell branch only ([pty-manager.ts:107-109](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L107-L109)) | Absolute path to `pandamux-powershell-integration.ps1`, dot-sourced by the PowerShell launch args |
| `PANDAMUX_STARTUP_COMMANDS` | `pty-manager.ts`, only when a quick-launch profile supplies `startupCommands` on a PowerShell shell ([pty-manager.ts:260-264](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L260-L264)) | Newline-joined commands the integration script runs via `Invoke-Expression` during shell init, before the first prompt renders |

`WSLENV` is also mutated (not created) to forward the `PANDAMUX*` variables into a WSL distro, since WSL otherwise strips all Windows environment variables from the child shell ([pty-manager.ts:118-126](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L118-L126)).

Sources: [pty-manager.ts:100-270](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L100-L270), [instance.ts:1-93](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/instance.ts#L1-L93), [index.ts:80-84,214-216](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L80-L84)
<!-- END:AUTOGEN pandamux_06_configuration_env -->

---
