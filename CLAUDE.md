# PandaMUX Everywhere — Development Guide

Native Windows terminal multiplexer for AI agents. Rust workspace: Iced (GPU UI) + alacritty_terminal (grid) + portable-pty (local PTY) + russh (remote PTY/SFTP).

**Owner**: BoardPandas (github.com/BoardPandas). Prefers fast, pragmatic solutions; tests live.
**Repo**: github.com/BoardPandas/Pandamux | **Site**: pandamux.boardpandas.ai (Netlify, static from `site/`)
**Version**: single source is `[workspace.package] version` in the root `Cargo.toml` (currently 0.35.x). See `CHANGELOG.md`.

> **History**: this app was rewritten from an Electron/TypeScript prototype into a fully native Rust application (plan: `tasks/plan-repo.md`). The Electron build was a never-shipped fork pull and has been deleted (Phase 7). The browser/CDP pane was intentionally dropped; agents use Claude Code's own browser tooling instead.
>
> Phases 1-6 are complete (native terminal engine, Iced shell, pipe/CLI parity, peripheral UI, SSH remote surfaces + OSC 52 + SFTP image paste). Phase 7 (ship: signed installer + release CI) is landing now. The `.claude/` folder is the source of truth for how this repo runs (commits, changelog, knowledge-base checks, agents). See Conventions at the bottom.

---

## Build & Dev

Rust stable toolchain (rustup) + MSVC build tools (Windows target). No Node/pnpm anymore.

```bash
# GUI app (the Iced shell needs the iced-runtime feature)
cargo run -p pandamux-app --features iced-runtime -- --iced-shell        # interactive window
cargo run -p pandamux-app --features iced-runtime -- --iced-shell-smoke  # noninteractive CI smoke
cargo build --release -p pandamux-app --features iced-runtime            # release GUI (pandamux.exe)

# CLI (pandamux-cli.exe) + headless pipe server
cargo build --release -p pandamux-cli
cargo run  -p pandamux-app                                               # headless pipe server (no GUI)

# Checks
cargo fmt --all --check
.\scripts\check-rust-boundaries.ps1     # enforces the crate-isolation invariant (Section 6.1 of the plan)
cargo test --workspace
cargo test -p pandamux-ui  --features iced-runtime --lib
cargo test -p pandamux-app --features iced-runtime --bin pandamux
```

The GUI binary is declared as `[[bin]] name = "pandamux"` in `crates/pandamux-app/Cargo.toml`, so the release artifact is `target/release/pandamux.exe` while the cargo package stays `pandamux-app` (so `-p pandamux-app` and the CI smokes are unchanged).

### Native dependency notes

- Native deps of note: `alacritty_terminal`, `portable-pty`, `russh` + `russh-sftp`, `iced` (+ wgpu/glyphon/cosmic-text via Iced), `arboard`. All are pinned to exact versions (`=x.y.z`); see the crate manifests and the Phase 2 spike report (`spikes/phase2-native-terminal/PHASE2_REPORT.md`) for why.
- `winresource` (build-dependency of `pandamux-app`, Windows only) embeds the icon + version metadata into `pandamux.exe` via `build.rs`. A missing resource compiler is treated as a warning so a dev box without the Windows SDK still builds; CI (windows-latest) has `rc.exe` and embeds for real.
- Windows Application Control can intermittently block freshly built Cargo test/build-script executables with `os error 4551`. It is host-policy noise: rerun, or `cargo clean -p <pkg>` then rerun.

---

## Architecture (Rust workspace)

```
crates/
  pandamux-core/   Shared domain types, split tree, session model, pipe-protocol (JSON-RPC) types,
                   agent + sidebar + notification + ssh models. CANONICAL state lives here. Zero Iced.
  pandamux-term/   Terminal engine: alacritty_terminal grid, portable-pty local PTY, russh remote PTY +
                   SFTP, OSC 52 clipboard policy, hand-built search/serialize/link detection, shell
                   lifecycle (resolution/chunking/DA1-CPR/tree-kill). Exposes pandamux's OWN grid types;
                   alacritty_terminal never leaks out.
  pandamux-ui/     Iced app: canvas terminal viewport (wgpu/cosmic-text/glyphon), panes/splits/tabs,
                   chrome (titlebar/rail/status bar), session panel, command palette, settings,
                   markdown/diff surfaces, theming, icons. The ONLY crate that imports Iced.
  pandamux-app/    Binary (pandamux.exe): composition root + tokio runtime; owns authoritative mutable
                   state; named-pipe server, agent manager, git/port pollers, session persistence,
                   in-app updater, Claude-context startup integration, OS clipboard bridge.
  pandamux-cli/    Binary (pandamux-cli.exe): the `pandamux` CLI, pipe client (wire-compatible with the
                   V2 JSON-RPC protocol).
resources/         Runtime assets loaded from <exe dir>/resources: themes, sounds, shell-integration,
                   icons, claude-instructions.md, pandamux-orchestrator plugin.
site/              Landing page (static HTML, Netlify) — describes the public download.
tasks/plan-repo.md The master plan (phases, decisions, gotchas, UI design spec in Section 12).
```

### Key design decisions

- **Crate-isolation invariant (hard rule, CI-enforced by `scripts/check-rust-boundaries.ps1`)**: `pandamux-core`/`pandamux-term` have ZERO Iced dependency; `pandamux-ui` is the only crate that imports Iced; `alacritty_terminal` types never appear outside `pandamux-term`. An engine or framework swap then touches only one crate.
- **Backend-owned state (intent-in, delta-out, single writer)**: `pandamux-app` owns the canonical workspace/pane/surface split tree; the Iced UI holds a read-projection and submits intents. The named-pipe server (CLI/agents/orchestrator) and the UI both submit the SAME intents to the SAME sync dispatcher (`pandamux-app::backend::handle_line`), so a CLI-driven and a UI-driven mutation are indistinguishable at the state layer. The pipe server runs as an Iced subscription in the live runtime.
- **PTY ID = Surface ID**: each terminal surface keeps its `Term`/PTY alive in memory keyed by surface id; switching tabs never reconstructs grid state (native equivalent of the old keep-alive tabs).
- **Immutable split tree**: layouts are a binary `SplitNode` tree in `pandamux-core::split`; mutations produce new trees. The UI renders a 2-level column projection over it (design decision 12.1); UI-initiated splits stay 2-level so the projection round-trips, while arbitrary-depth CLI/orchestrator trees still render via a graceful fallback.
- **No MCP**: all Claude Code integration is via the `pandamux` CLI over the named pipe. Do NOT build MCP servers.

---

## Release Process (Phase 7)

The whole release runs in GitHub Actions on a `v*` tag (`.github/workflows/release.yml`, windows-latest). A person never builds or signs locally. The GitHub Release carries exactly ONE asset: the signed NSIS `Setup.exe`. The in-app updater (`pandamux-app::updater`) discovers it via the GitHub Releases API, so there is no Velopack feed / `latest.yml`.

Pipeline: `cargo build --release` (winresource embeds icon + version into `pandamux.exe`) → Azure Trusted Signing on `pandamux.exe` + `pandamux-cli.exe` → `cargo packager` bundles the already-signed exe into `Setup.exe` (NSIS; cargo-packager does not rebuild, so the signature stays valid) → sign the installer → `gh release` with the installer as the only asset.

Signing secrets come from Doppler (`pandamux`/`prd`, six `AZURE_*` vars) via the single `DOPPLER_TOKEN` repo secret and `dopplerhq/secrets-fetch-action`; the `pandamux-ci-signing` service principal holds the **Artifact Signing Certificate Profile Signer** role on the `SupportForge` profile (Wellforce `HDBtrustedsigning` account). Signing MUST be the last mutation of a binary before it is packaged/uploaded.

### To cut a release

1. Update `CHANGELOG.md` and bump `[workspace.package] version` in the root `Cargo.toml` (see `.claude/rules/commit-changelog.md`).
2. Commit, then tag: `git tag -a v<VERSION> -m "PandaMUX Everywhere v<VERSION>"` and `git push origin v<VERSION>`.
3. The workflow builds, signs, packages, signs the installer, and publishes the Release.

### Local packaging validation (optional, no signing)

```bash
cargo build --release -p pandamux-cli
cargo build --release -p pandamux-app --features iced-runtime
cargo packager --release --formats nsis --out-dir dist   # -> dist/pandamux_<ver>_x64-setup.exe
```

The packager config is `[package.metadata.packager]` in `crates/pandamux-app/Cargo.toml`. Resources land at `<install>/resources/...`, exactly where the runtime's `resources_dir()` looks. `cargo-packager` downloads its own NSIS (no choco install needed). Keys are kebab-case, but note `installer-mode` (not `install-mode`).

### Distribution

- winget auto-publish (`.github/workflows/winget.yml`) runs on release published. The installer type changed from the old portable zip to NSIS, so the winget-pkgs package needs a one-time manual re-bootstrap PR from `winget/*.yaml` (now `InstallerType: nsis`) before winget-releaser can auto-update it.
- macOS/Linux packaging is deferred (rcodesign + AppImage/Flatpak noted for later).

---

## Named Pipe + CLI (parity contract)

`\\.\pipe\pandamux`: V1 text protocol (shell-integration hooks, e.g. `report_pwd`) + V2 JSON-RPC (CLI/agents/orchestrator, token-authenticated). The V2 protocol is wire-compatible with the historical prototype and is preserved as-is (browser/CDP methods excepted — they reject with a "use Claude Code's browser tooling" message; `system.capabilities` reports `browser: false`).

V2 methods (all route through `pandamux-app::backend::handle_line`): `system.*`, `workspace.*`, `pane.*`, `layout.grid`, `surface.*` (incl. `send_text`/`send_key`/`read_text`/`paste`/`paste_image`/`set_color_scheme`), `markdown.*`, `diff.*`, `notification.*`, `sidebar.*`, `agent.*`, `clipboard.*`, `ssh.*`, `window.*`, `config.*`, `theme.*`, `hook.event`. The authoritative CLI command list is `crates/pandamux-cli` (`pandamux <command>`); `pandamux browser *` does NOT exist.

---

## pandamux-orchestrator Plugin

Claude Code plugin bundled in `resources/pandamux-orchestrator/`. Auto-installed into the Claude plugin cache on GUI launch by `pandamux-app::claude_context`. Decomposes complex dev tasks into parallel Claude Code agents in visible panes, coordinated through the pipe protocol (state in a JSON file in TMPDIR; no daemon). It talks the CLI/pipe only, so it works unchanged against the Rust pipe server. See `resources/pandamux-orchestrator/` for its skills/hooks/scripts.

---

## Shell Integration

Scripts in `resources/shell-integration/` (bash/zsh/PowerShell/cmd) report cwd, git branch/dirty, and shell state over the pipe. Env vars set by pandamux in spawned shells: `PANDAMUX=1`, `PANDAMUX_SURFACE_ID`, `PANDAMUX_PIPE`, `PANDAMUX_CLI`, `PANDAMUX_AGENT_ID` (for orchestrator hooks). Per-session cwd tracking uses OSC 9;9 / OSC 7 plus the V1 `report_pwd`.

---

## Website (pandamux.boardpandas.ai)

Static site in `site/`, deployed to Netlify (`netlify.toml`). `site/index.html` is the landing page with i18n (en/fr/ar/ja via `site/i18n.js`, URL-hash switching). The public site still describes the previous download; it is updated to the native app when the first native release ships (`npx netlify deploy --prod --dir site`).

---

## Conventions

- **State**: canonical in `pandamux-app` (`pandamux-core` types); the Iced UI is a read-projection (intent-in, delta-out).
- **Crate isolation**: never import Iced outside `pandamux-ui`; never leak `alacritty_terminal` types outside `pandamux-term`. CI fails the build otherwise.
- **Pins**: dependencies are pinned exactly (`=x.y.z`). Re-verify + bump deliberately at dependency refreshes.
- **Version**: single source is the root `Cargo.toml` `[workspace.package] version`; crates inherit via `version.workspace = true`. It drives `CARGO_PKG_VERSION`, which the updater compares to GitHub releases.
- **Workflow source of truth (`.claude/`)**: before committing, update `CHANGELOG.md` and bump the version per `.claude/rules/commit-changelog.md`; write commit messages to a file and use `git commit -F` (never inline `-m`). Consult the LL-G and BP knowledge bases before code/config work (`.claude/rules/llg-check.md`, `.claude/rules/bp-check.md`). Use the custom agents in `.claude/agents/`, not built-in subagent types.
- **Writing style**: no em dashes or double dashes in files, code, or comments; use commas, colons, parentheses, or semicolons instead.
