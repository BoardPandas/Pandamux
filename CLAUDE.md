# PandaMUX Everywhere — Development Guide

Electron-based Windows terminal multiplexer for AI agents. TypeScript, React 19, Zustand, xterm.js, node-pty.

**Owner**: BoardPandas (github.com/BoardPandas). Prefers fast, pragmatic solutions; tests live.
**Repo**: github.com/BoardPandas/Pandamux | **Site**: pandamux.boardpandas.ai (Netlify, static from `site/`)
**Version**: see `package.json` / `CHANGELOG.md` (currently 0.15.x)

> **Direction**: PandaMUX Everywhere is being rebuilt as a fully native Rust app (Iced + alacritty_terminal + portable-pty; the browser pane is dropped). The Electron app in this repo is frozen to bug fixes and now uses pnpm (migrated from npm). The master plan is `tasks/plan-repo.md`. **This guide documents the current Electron build.** The Rust workspace gets its own CLAUDE.md hierarchy once its crates exist.
>
> The `.claude/` folder is the source of truth for how this repo runs (commits, changelog, knowledge-base checks, agents). See Conventions at the bottom.

---

## Build & Dev

This repo uses **pnpm** (not npm). Toolchain is pinned: pnpm 11.10.0 + Node 24.18.0 (24 LTS), both enforced via `engines` and the `packageManager` field. Use corepack (`corepack enable pnpm`) so the pinned pnpm is used automatically; a globally installed pnpm is shadowed by the corepack shim. pnpm settings live in `pnpm-workspace.yaml` (`nodeLinker: hoisted` is mandatory for node-pty + ASAR; `allowBuilds` approves the native/binary packages, since pnpm 11 blocks dependency build scripts by default).

```bash
pnpm install           # Install deps (hoisted node_modules; runs allowBuilds + postinstall)
pnpm run dev           # Vite (port 5199) + Electron hot-reload
pnpm run build:main    # tsc main/preload/cli only (fast iteration)
pnpm run build:renderer # Vite production build (renderer only)
pnpm run build         # Full: tsc + vite + electron-builder
pnpm test              # Vitest unit tests
pnpm run test:watch    # Vitest watch mode
pnpm run lint          # ESLint src/
```

node-pty is the only native dependency and ships **N-API prebuilds** (ABI-stable across Node and Electron; verified loading under both Node 24 and Electron 33 / ABI 130 / N-API 9). We deliberately do NOT rebuild it from source: there is no `install-app-deps` postinstall, and `electron-builder.json` sets `"npmRebuild": false`. This trusts the prebuilds (unpacked from the asar via `asarUnpack`), avoids node-pty's flaky legacy winpty gyp build, and means a normal `pnpm install` needs no Python/VS Build Tools toolchain. If you ever add a non-N-API native dependency you must reintroduce a rebuild step, and on Python 3.12+ run `pip install setuptools` first so node-gyp finds the removed `distutils` module.

### Known Build Gotcha

The original checkout lived under a OneDrive path with spaces, which broke:
- `npm link` / `node-gyp` (can't build node-pty)
- `electron-builder` winCodeSign (symlink errors)

The current checkout (`D:\Dev\Repos\Pandamux`) has no spaces, so these may not bite. Either way, the release flow uses ASAR-based manual packaging (see Release Process below) rather than `electron-builder` for the final package.

---

## Architecture

```
src/
  main/           Electron main process
  renderer/       React UI (Vite)
  preload/        contextBridge (window.pandamux)
  cli/            CLI → named pipe (\\.\pipe\pandamux)
  shared/         Shared types (IPC channels, branded IDs)
  shell-integration/  Shell hooks (bash/zsh/PowerShell/cmd)

resources/        Runtime assets (icons, themes, sounds, shell-integration, CLI)
  pandamux-orchestrator/  Claude Code plugin (auto-installed on startup)
site/             Landing page (static HTML, Netlify)
tests/            Unit + e2e (Vitest)
docs/             Planning docs
```

### Main Process (`src/main/`)

| File | Role |
|------|------|
| `index.ts` | Entry point, AppUserModelId, auto-save (30s), pipe server startup, V2 pipe handlers (workspace/pane/surface/markdown/sidebar/notification) |
| `pty-manager.ts` | PTY lifecycle (create with surfaceId, write, resize, kill) |
| `pipe-server.ts` | Named pipe `\\.\pipe\pandamux` — V1 text (shell hooks), V2 JSON-RPC (CLI/agents) |
| `cdp-bridge.ts` | Browser webview control via Chrome DevTools Protocol |
| `cdp-proxy.ts` | CDP WebSocket proxy |
| `agent-manager.ts` | Agent PTY spawning, round-robin distribution across panes |
| `window-manager.ts` | Electron BrowserWindow creation/management |
| `ipc-handlers.ts` | All IPC channel handlers |
| `claude-context.ts` | Auto-injects PandaMUX Everywhere instructions into `~/.claude/CLAUDE.md`, configures hooks, installs pandamux-orchestrator plugin |
| `claude-observer.ts` | Monitors Claude Code activity for sidebar display |
| `session-persistence.ts` | Auto-save/restore window state |
| `git-poller.ts` | Git branch/dirty status polling |
| `pr-poller.ts` | GitHub PR status polling |
| `port-scanner.ts` | Active port detection for running dev servers |
| `theme-loader.ts` | Theme loading |
| `config-loader.ts` | WT/Ghostty config import |
| `shell-detector.ts` | Available shells detection |
| `updater.ts` | Auto-update (electron-updater) |

### Renderer (`src/renderer/`)

**Components** (in `components/`):
- `SplitPane/` — PaneWrapper, SplitContainer, SplitDivider, SurfaceTabBar
- `Terminal/` — TerminalPane, FindBar, CopyMode, NotificationRing
- `Browser/` — BrowserPane, AddressBar
- `Sidebar/` — Sidebar, WorkspaceRow, SessionMenu, SidebarResizeHandle
- `Titlebar/` — Titlebar, NotificationBell, NotificationPanel
- `Settings/` — SettingsWindow + per-category panels
- `CommandPalette/` — CommandPalette
- `Markdown/` — MarkdownPane
- `Tutorial/` — Tutorial

**Hooks** (in `hooks/`):
- `useTerminal.ts` — xterm.js lifecycle, PTY connection, OSC notifications, WebGL renderer
- `useKeyboardShortcuts.ts` — 51+ shortcut actions, safe interception

**Pipe Bridge** (`pipe-bridge.ts`):
- Exposes Zustand store operations as `window.__pandamux_*` globals
- Called by main process via `executeJavaScript` to bridge V2 pipe commands to renderer
- Covers: workspace CRUD, pane split/close/list, surface CRUD, markdown content, notifications

**Store** (Zustand, in `store/`):
- `workspace-slice.ts` — Workspace CRUD, split tree updates
- `surface-slice.ts` — Surface/tab add/close/move/navigate
- `settings-slice.ts` — Shortcuts, sidebar prefs, theme
- `notification-slice.ts` — Notification lifecycle (max 200)
- `agent-slice.ts` — Agent metadata tracking
- `split-utils.ts` — Immutable split tree helpers

### Preload API (`window.pandamux`)

```
pty:      create, write, resize, kill, has, onData, onExit
system:   platform, getShells, openExternal, toggleDevTools
config:   getTheme, getThemeList, importWindowsTerminal, importGhostty
metadata: onUpdate
notification: fire, onFocusSurface
browser:  navigate
agent:    list, status, onUpdate
clipboard: pasteImage
hook:     onEvent
claudeActivity: onUpdate
session:  save, load, list, delete
cdp:      attach, detach
window:   create, close, focus, list, minimize, maximize, isMaximized
```

---

## Key Design Decisions

### No MCP — CLI Only
Do NOT build MCP servers. Use the pandamux CLI (`pandamux <command>`) via Bash instead.
The CLI talks to the named pipe, which is simpler and more reliable.
For new Claude Code integrations, add CLI commands in `src/cli/pandamux.ts`.

### Branded ID Types
`WorkspaceId`, `PaneId`, `SurfaceId`, `WindowId` — branded string types in `src/shared/types.ts`.
Pattern: `surf-{uuid}`, `pane-{uuid}`, `ws-{uuid}`, `win-{uuid}`.

### Keep-Alive Tabs
Terminal tabs in a pane are ALL rendered simultaneously (hidden with `visibility: hidden`).
When switching tabs, only CSS changes — the xterm instance stays alive, no PTY reconnection needed.
The `surfaceId` is passed to `pty.create()` so PTY ID = Surface ID (enables reliable re-attachment).

### Split Tree
Pane layouts use an immutable binary tree (`SplitNode`). Each leaf = one pane with N surfaces (tabs).
Mutations go through `splitNode()`, `removeLeaf()`, `findLeaf()`, `getAllPaneIds()` in `split-utils.ts`.

---

## Release Process (CRITICAL)

PandaMUX Everywhere is distributed as a **portable zip** (not NSIS installer) because without code-signing, Windows SmartScreen flags installers more aggressively than zip extractions.

### Step-by-step

```bash
# 1. Build everything
pnpm run build:main       # Compile TS → dist/main/, dist/preload/, dist/cli/
pnpm exec vite build      # Build renderer → dist/renderer/

# 2. Verify compiled code
# Check that fixes are in the compiled output:
python -c "import re; f=open('dist/renderer/assets/index-*.js').read(); print('OK' if 'your_fix_marker' in f else 'MISSING')"
grep -c 'your_fix_string' dist/main/index.js

# 3. Create ASAR staging
# IMPORTANT: always run from the project root (use absolute paths or cd back
# after any `cd .asar-staging`). If cwd drifts into .asar-staging during this
# section, subsequent `mkdir build-out` lands INSIDE the staging dir and the
# next asar pack will recursively include its own previous output → 188M asar.
rm -rf .asar-staging build-out
mkdir -p .asar-staging build-out
cp -r dist .asar-staging/dist          # explicit dest path — trailing-slash form is flaky on Git Bash
cp package.json .asar-staging/package.json
( cd .asar-staging && pnpm install --prod --ignore-scripts --config.node-linker=hoisted )   # subshell (cwd doesn't leak); the hoisted flag keeps staging node_modules junction-free (pnpm-workspace.yaml is not copied into staging, so pass it explicitly), avoiding the pnpm reparse-point deletion hazard on the later rm -rf
rm -rf .asar-staging/node_modules/node-pty/build   # force prebuilds load path: conpty.dll (useConptyDll) resolves relative to the LOADED conpty.node, and only prebuilds/win32-x64/ has the conpty/ dir next to it

# 4. Pack ASAR (with native module unpacking)
# Use --unpack-dir (path-based), NOT --unpack "**/*.node" — the glob form
# silently fails on Git Bash for Windows (shell eats the pattern, asar produces
# the asar but creates no .unpacked dir, no error). Output to build-out/ so we
# never touch the live resources/app.asar while pandamux may be running.
npx asar pack .asar-staging build-out/app.asar --unpack-dir "node_modules/node-pty/prebuilds"

# 5. Verify native modules are unpacked
ls build-out/app.asar.unpacked/node_modules/node-pty/prebuilds/win32-x64/
# Must contain: conpty.node, conpty_console_list.node, pty.node
# Sanity: ASAR should be ~24M (natives unpacked). 80M+ means natives weren't
# moved out; 180M+ means staging got polluted (see step 3 warning).

# 5b. Verify the PRs/fixes you intended to ship are actually inside the ASAR.
# extract-file's stdout piping is unreliable on Windows — extract to /tmp instead.
rm -rf /tmp/asar-verify && mkdir -p /tmp/asar-verify
( cd /tmp/asar-verify && npx --prefix "$(pwd)" asar extract "$(pwd)/build-out/app.asar" . )
grep -c 'your_fix_marker' /tmp/asar-verify/dist/renderer/assets/index-*.js
grep -c 'your_fix_string' /tmp/asar-verify/dist/main/index.js

# 6. Create release staging
# Easiest base: the previous release zip. Avoids needing a separate
# pandamux_v_extracted/ dir and avoids picking up stray files from the project root.
rm -rf ../pandamux-release-staging
mkdir -p ../pandamux-release-staging
( cd ../pandamux-release-staging && unzip -q ../pandamux/pandamux-<PREV_VERSION>-win-x64.zip )

# 7. Copy ASAR + resources into release staging
cp build-out/app.asar ../pandamux-release-staging/resources/app.asar
rm -rf ../pandamux-release-staging/resources/app.asar.unpacked
cp -r build-out/app.asar.unpacked ../pandamux-release-staging/resources/app.asar.unpacked
cp resources/icon.png ../pandamux-release-staging/resources/
rm -rf ../pandamux-release-staging/resources/themes && cp -r resources/themes ../pandamux-release-staging/resources/themes
rm -rf ../pandamux-release-staging/resources/sounds && cp -r resources/sounds ../pandamux-release-staging/resources/sounds
mkdir -p ../pandamux-release-staging/resources/cli && cp dist/cli/pandamux.js ../pandamux-release-staging/resources/cli/pandamux.js
rm -rf ../pandamux-release-staging/resources/shell-integration && mkdir -p ../pandamux-release-staging/resources/shell-integration
cp -r src/shell-integration/* ../pandamux-release-staging/resources/shell-integration/
rm -rf ../pandamux-release-staging/resources/pandamux-orchestrator && cp -r resources/pandamux-orchestrator ../pandamux-release-staging/resources/pandamux-orchestrator

# 8. Embed icon + metadata in exe (rcedit)
# CRITICAL: rcedit exports `{ rcedit }` (named export). `const rcedit =
# require('rcedit')` followed by `rcedit(...)` throws "rcedit is not a function".
# Always destructure: `const { rcedit } = require('rcedit')`.
node -e "
  const { rcedit } = require('rcedit');
  rcedit('../pandamux-release-staging/pandamux.exe', {
    icon: 'resources/icons/icon.ico',
    'version-string': {
      ProductName: 'PandaMUX Everywhere',
      FileDescription: 'PandaMUX Everywhere',
      CompanyName: 'BoardPandas',
      InternalName: 'pandamux',
      OriginalFilename: 'pandamux.exe',
      LegalCopyright: 'Copyright (c) 2026 PandaMUX Everywhere'
    },
    'file-version': '0.7.20',
    'product-version': '0.7.20'
  }).then(() => console.log('rcedit done'), e => { console.error(e); process.exit(1); });
"
# NOTE: rcedit CANNOT modify a running exe. The staging copy is fine; never
# point rcedit at the pandamux.exe living in the project root if it's running.

# 9. Create zip
powershell -NoProfile -Command "Compress-Archive -Path '..\pandamux-release-staging\*' -DestinationPath '..\pandamux-<VERSION>-win-x64.zip' -CompressionLevel Optimal"

# 9b. Generate latest.yml (REQUIRED — electron-updater 404s on every launch
# without it; issue #68. The CI workflow does this automatically, but manual
# releases MUST do it too.)
node -e "
  const crypto = require('crypto'); const fs = require('fs');
  const version = '<VERSION>';
  const zip = '../pandamux-' + version + '-win-x64.zip';
  const data = fs.readFileSync(zip);
  const sha512 = crypto.createHash('sha512').update(data).digest('base64');
  const yaml = ['version: ' + version, 'files:', '  - url: pandamux-' + version + '-win-x64.zip',
    '    sha512: ' + sha512, '    size: ' + data.length, 'path: pandamux-' + version + '-win-x64.zip',
    'sha512: ' + sha512, 'releaseDate: ' + JSON.stringify(new Date().toISOString()), ''].join('\n');
  fs.writeFileSync('../latest.yml', yaml);
  console.log('latest.yml written:', data.length, 'bytes,', sha512.slice(0, 16) + '...');
"

# 10. Tag, push, publish (zip AND latest.yml — both assets are required)
git add package.json package-lock.json && git commit -m "chore(release): bump to <VERSION>"
git push origin master
git tag -a v<VERSION> -m "PandaMUX Everywhere <VERSION>" && git push origin v<VERSION>
gh release create v<VERSION> ../pandamux-<VERSION>-win-x64.zip ../latest.yml --repo BoardPandas/Pandamux --title "v<VERSION>" --notes "..."

# 11. (Optional) Hot-swap into the locally running pandamux for immediate testing
cp build-out/app.asar resources/app.asar
rm -rf resources/app.asar.unpacked && cp -r build-out/app.asar.unpacked resources/app.asar.unpacked
# Then restart pandamux to pick up changes

# 12. Cleanup
rm -rf .asar-staging build-out /tmp/asar-verify ../pandamux-release-staging
```

### Release Checklist

- [ ] `pnpm run build:main` succeeds
- [ ] `pnpm exec vite build` succeeds
- [ ] Compiled code verified (grep for key changes in dist/)
- [ ] ASAR packed with `--unpack-dir node_modules/node-pty/prebuilds` (NOT `--unpack` glob)
- [ ] ASAR size is ~24M (natives unpacked). 80M+ ⇒ unpack didn't take. 180M+ ⇒ staging polluted.
- [ ] node-pty native modules present in `app.asar.unpacked/node_modules/node-pty/prebuilds/win32-x64/`
- [ ] PR-specific markers grep-confirmed inside the packed ASAR (extracted to /tmp)
- [ ] pandamux-orchestrator plugin copied to release staging
- [ ] rcedit applied (icon + version metadata) — `{ rcedit }` destructured
- [ ] `latest.yml` generated (sha512 + size of the final zip) and uploaded as a release asset — electron-updater 404s without it (issue #68)
- [ ] Zip created and uploaded to GitHub release
- [ ] Mark of the Web: remind user to right-click > Unblock after download

### Important Notes

- **rcedit can't modify a running exe** — always work on a copy
- **rcedit named export**: `const { rcedit } = require('rcedit')`. Non-destructured `const rcedit = require('rcedit')` throws "rcedit is not a function" (different from older docs).
- **asar `--unpack` glob silently fails on Git Bash for Windows**: pattern like `"**/*.node"` gets shell-eaten and asar emits no `.unpacked/` dir, no error. Use `--unpack-dir node_modules/node-pty/prebuilds` (path-based) instead.
- **Bash cwd drift can recursively pollute staging**: if you `cd .asar-staging` and forget to come back, the next `mkdir build-out && asar pack` creates `.asar-staging/build-out/app.asar`, and a re-pack will swallow its own output into the new asar (188M). Always use subshells `( cd dir && cmd )` or absolute paths.
- **Don't pack ASAR directly to `resources/app.asar`** if pandamux may be running — pack to `build-out/` and copy at step 7.
- **MOTW (Mark of the Web)**: Downloaded zips get `Zone.Identifier` NTFS stream. Fix: `powershell "Get-ChildItem -Recurse | Unblock-File"`
- **Windows taskbar pinning** uses PE `FileDescription` for the shortcut name — ensure rcedit sets it to "PandaMUX Everywhere"
- **AppUserModelId** is set to `com.pandamux.app` in `src/main/index.ts` for proper taskbar grouping

---

## Named Pipe V2 Handlers

The pipe server in `index.ts` handles V2 JSON-RPC methods. Most delegate to the renderer via `executeJavaScript('window.__pandamux_*(...)')`. The renderer's `pipe-bridge.ts` exposes Zustand store operations as these globals.

**Fully implemented V2 methods:**
- `system.identify`, `system.capabilities`, `system.tree`
- `workspace.create`, `workspace.close`, `workspace.select`, `workspace.rename`, `workspace.list`
- `pane.split`, `pane.close`, `pane.focus`, `pane.zoom`, `pane.list`
- `surface.create`, `surface.close`, `surface.focus`, `surface.list`
- `surface.send_text`, `surface.send_key`, `surface.trigger_flash`
- `markdown.set_content`, `markdown.load_file`
- `notification.list`, `notification.clear`
- `sidebar.set_status`, `sidebar.set_progress`, `sidebar.log`, `sidebar.get_state`
- `browser.*` (via CDP bridge)
- `agent.spawn`, `agent.spawn_batch`, `agent.status`, `agent.list`, `agent.kill`
- `hook.event`, `diff.refresh`

**Partially implemented:** `surface.read_text` (stub — needs xterm serializer addon)

---

## pandamux-orchestrator Plugin

Claude Code plugin bundled in `resources/pandamux-orchestrator/`. Auto-installed into `~/.claude/plugins/cache/` on startup by `ensureOrchestratorPlugin()` in `claude-context.ts`. Also published standalone: `github.com/amirlehmam/wmux-orchestrator`.

**What it does:** Decomposes complex dev tasks into parallel Claude Code agents coordinated through dependency-aware waves with automated review. With PandaMUX Everywhere: each agent in its own visible terminal pane. Without PandaMUX Everywhere: falls back to native subagents.

**Plugin structure:**
```
resources/pandamux-orchestrator/
  .claude-plugin/plugin.json    Manifest (name, version, author)
  commands/orchestrate.md       /pandamux:orchestrate slash command
  skills/orchestrate/SKILL.md   Core: codebase analysis, wave planning, agent spawning
  skills/reviewer/SKILL.md      Post-orchestration review and auto-fix
  skills/pandamux-detect/SKILL.md   Detects pandamux availability for degraded mode
  agents/pandamux-worker.md         Worker template with file zone enforcement
  hooks/hooks.json              PostToolUse, SubagentStop, Stop, SessionStart
  scripts/json-tool.js          Node.js JSON helper (replaces jq)
  scripts/orchestration-state.sh  State file management library
  scripts/spawn-agents.sh       Creates panes + launches Claude Code agents
  scripts/on-agent-stop.sh      Wave transition driver (core orchestration)
  scripts/check-status.sh       Markdown dashboard generator
  scripts/*.sh                  Other utilities (cleanup, collect-results, etc.)
```

**Key design:** Skills handle intelligence (prompts), hooks handle reactivity (events), scripts handle pandamux operations (CLI). State shared via JSON file in TMPDIR. No daemon.

---

## CLI Reference

```bash
# System
pandamux ping | identify | capabilities

# Workspaces
pandamux new-workspace [--title T] [--shell S] [--cwd D]
pandamux close-workspace | select-workspace | rename-workspace | list-workspaces

# Surfaces (tabs within a pane)
pandamux new-surface [--type terminal|browser|markdown]
pandamux close-surface | focus-surface | list-surfaces

# Panes
pandamux split [--down] [--type T] | close-pane | focus-pane | zoom-pane | list-panes | tree

# Terminal I/O
pandamux send <text> | send-key <key> [--ctrl] [--shift] [--alt]
pandamux read-screen [--lines N] | trigger-flash

# Browser (CDP)
pandamux browser open <url> | snapshot | click @eN | type @eN <text>
pandamux browser fill @eN <value> | get-text | screenshot | eval <js>
pandamux browser back | forward | reload

# Agents
pandamux agent spawn [--cmd C] [--label L] [--cwd D] [--pane P]
pandamux agent spawn-batch --json '[...]' [--strategy distribute|stack|split]
pandamux agent status <id> | list | kill <id>

# Notifications & Sidebar
pandamux notify <text> | list-notifications | clear-notifications
pandamux set-status <key> <value> | set-progress <val> [--label L]
pandamux log <level> <message> | sidebar-state

# Hooks
pandamux hook --event <type> --tool <name> [--agent <id>]
```

---

## IPC Channels

All defined in `src/shared/types.ts` → `IPC_CHANNELS`:

```
PTY:     pty:create, pty:write, pty:resize, pty:kill, pty:has, pty:data, pty:exit
Window:  window:create/close/focus/list/minimize/maximize/isMaximized
Config:  config:getTheme/getThemeList/importWindowsTerminal/importGhostty
System:  system:getShells/openExternal
Notify:  notification:fire/list/clear/jump
Agent:   agent:spawn/spawn-batch/status/list/kill/update
CDP:     cdp:attach/detach
Session: session:save-named/load-named/list-named/delete-named
Meta:    metadata:update, hook:event, claude:activity
```

---

## Shell Integration

Scripts in `src/shell-integration/` (deployed to `resources/shell-integration/`):

| Script | Reports |
|--------|---------|
| `pandamux-powershell-integration.ps1` | cwd, git branch/dirty, shell state, PR polling (45s) |
| `pandamux-bash-integration.sh` | cwd, git branch/dirty, shell state, ports |
| `pandamux-cmd-integration.cmd` | Basic OSC 9 escape sequences |

Env vars set by pandamux in spawned shells: `PANDAMUX=1`, `PANDAMUX_SURFACE_ID`, `PANDAMUX_PIPE`, `PANDAMUX_CLI`.

---

## Website (pandamux.boardpandas.ai)

Static site in `site/`. Deployed to Netlify (`netlify.toml` at repo root).

```bash
# Deploy
npx netlify deploy --prod --dir site
```

`site/index.html` — Landing page with i18n (English, French, Arabic, Japanese).
`site/i18n.js` — Language switching via URL hash (`#ar`, `#fr`, `#ja`).

---

## Testing

```bash
pnpm test                   # Run all unit tests
pnpm run test:watch         # Watch mode
pnpm exec vitest run tests/unit/pty-manager.test.ts  # Single file
```

Test files in `tests/unit/`: agent-manager, cdp-bridge, config-loader, notification-slice, pipe-server, port-scanner, pty-manager, session-persistence, shell-detector, split-tree.

---

## Conventions

- **State**: Zustand slices in `src/renderer/store/`, composed in `index.ts`
- **IPC**: Channels defined in `src/shared/types.ts`, never use magic strings
- **CSS**: `src/renderer/styles/`, class prefix per component (`.pane-wrapper__*`, `.surface-tab__*`)
- **Immutable trees**: Split tree mutations always produce new objects via `patchLeaf()`
- **PTY IDs = Surface IDs**: Always pass `surfaceId` when creating PTYs for reliable re-attachment
- **No MCP**: All Claude Code integration via CLI commands
- **Workflow source of truth (`.claude/`)**: before committing, update `CHANGELOG.md` and bump the version per `.claude/rules/commit-changelog.md`; write commit messages to a file and use `git commit -F` (never inline `-m`). Consult the BP and LL-G knowledge bases before config/code work (`.claude/rules/bp-check.md`, `.claude/rules/llg-check.md`). Use the custom agents in `.claude/agents/`, not built-in subagent types.
- **Writing style**: no em dashes or double dashes in files, code, or comments; use commas, colons, parentheses, or semicolons instead.
