<!-- PAGE_ID: pandamux_02_getting-started -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [CLAUDE.md:1-151](../CLAUDE.md#L1-L151)
- [Cargo.toml:1-24](../Cargo.toml#L1-L24)
- [README.md:38-55](../README.md#L38-L55)
- [scripts/check-rust-boundaries.ps1:1-33](../scripts/check-rust-boundaries.ps1#L1-L33)
- [.github/workflows/rust.yml:1-56](../.github/workflows/rust.yml#L1-L56)
- [crates/pandamux-app/Cargo.toml:1-73](../crates/pandamux-app/Cargo.toml#L1-L73)
- [.claude/rules/commit-changelog.md:1-45](../.claude/rules/commit-changelog.md#L1-L45)

</details>

# Getting Started

> **Related Pages**: [Overview](OVERVIEW.md), [Release and Packaging](operations/RELEASE.md)

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_prerequisites -->
## Prerequisites

PandaMUX is a native Windows application built with a Rust stable toolchain; there is no Node/pnpm toolchain to install anymore ([CLAUDE.md:15-17](../CLAUDE.md#L15-L17)).

| Requirement | Purpose |
|---|---|
| Rust stable toolchain (rustup) | Compiles the workspace; `rust-version = "1.88"` is the minimum declared in the workspace manifest ([Cargo.toml:17](../Cargo.toml#L17)). |
| MSVC build tools | Required for the Windows target, since PandaMUX links against native Windows APIs (ConPTY via `portable-pty`, etc.) ([CLAUDE.md:15-17](../CLAUDE.md#L15-L17)). |

The repository previously shipped an Electron/TypeScript prototype; that build has been deleted, so no `npm`/`pnpm install` step exists in this workspace ([CLAUDE.md:9](../CLAUDE.md#L9), [CLAUDE.md:17](../CLAUDE.md#L17)).

Sources: [CLAUDE.md:9-17](../CLAUDE.md#L9-L17), [Cargo.toml:12-17](../Cargo.toml#L12-L17)
<!-- END:AUTOGEN pandamux_02_getting-started_prerequisites -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_build -->
## Building and Running

Clone the repository, then use the exact `cargo` invocations below; they are the authoritative commands for this workspace ([CLAUDE.md:19-40](../CLAUDE.md#L19-L40)).

```bash
git clone https://github.com/BoardPandas/Pandamux.git
cd Pandamux
```

The GUI shell needs the `iced-runtime` feature. With that feature enabled, running the binary with **no arguments opens the window by default**: the installed Start Menu shortcut runs `pandamux.exe` with no args, so the argument-less path must be the GUI, and the feature build is a Windows GUI-subsystem binary (no console window appears) ([CLAUDE.md:20-23](../CLAUDE.md#L20-L23)).

```bash
# GUI app (interactive window, default when iced-runtime is enabled)
cargo run -p pandamux-app --features iced-runtime

# Same, via the back-compat flag
cargo run -p pandamux-app --features iced-runtime -- --iced-shell

# Noninteractive CI smoke test of the Iced shell view
cargo run -p pandamux-app --features iced-runtime -- --iced-shell-smoke

# Release GUI build (pandamux.exe)
cargo build --release -p pandamux-app --features iced-runtime
```

Without the `iced-runtime` feature, `pandamux-app` runs as a headless named-pipe server instead of a GUI; the CLI binary is built separately.

```bash
# CLI (pandamux-cli.exe)
cargo build --release -p pandamux-cli

# Headless pipe server (no GUI)
cargo run -p pandamux-app

# Force the pipe server even in a GUI build
cargo run -p pandamux-app --features iced-runtime -- --headless
```

(all commands quoted verbatim from [CLAUDE.md:19-40](../CLAUDE.md#L19-L40)).

| Goal | Command |
|---|---|
| Run GUI (default) | `cargo run -p pandamux-app --features iced-runtime` |
| Run GUI (explicit flag) | `cargo run -p pandamux-app --features iced-runtime -- --iced-shell` |
| CI smoke of Iced shell | `cargo run -p pandamux-app --features iced-runtime -- --iced-shell-smoke` |
| Build release GUI | `cargo build --release -p pandamux-app --features iced-runtime` |
| Build release CLI | `cargo build --release -p pandamux-cli` |
| Run headless pipe server | `cargo run -p pandamux-app` |
| Force headless in GUI build | `cargo run -p pandamux-app --features iced-runtime -- --headless` |

Sources: [CLAUDE.md:19-40](../CLAUDE.md#L19-L40)
<!-- END:AUTOGEN pandamux_02_getting-started_build -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_features -->
## Cargo Features and Binaries

`pandamux-app`'s `Cargo.toml` declares one opt-in feature, `iced-runtime`, which pulls in the `iced` GUI crate, the `reqwest` update-check client, and forwards to `pandamux-ui/iced-runtime` ([crates/pandamux-app/Cargo.toml:33-35](../crates/pandamux-app/Cargo.toml#L33-L35)).

```toml
[features]
default = []
iced-runtime = ["dep:iced", "dep:reqwest", "pandamux-ui/iced-runtime"]
```

([crates/pandamux-app/Cargo.toml:33-35](../crates/pandamux-app/Cargo.toml#L33-L35))

The default feature set is empty, which is why `cargo run -p pandamux-app` with no `--features` flag builds the headless pipe server: `iced` and `reqwest` are both `optional = true` and only compiled in behind `iced-runtime` ([crates/pandamux-app/Cargo.toml:17-18](../crates/pandamux-app/Cargo.toml#L17-L18), [crates/pandamux-app/Cargo.toml:26-28](../crates/pandamux-app/Cargo.toml#L26-L28), [crates/pandamux-app/Cargo.toml:34](../crates/pandamux-app/Cargo.toml#L34)).

The GUI binary is declared with an explicit `[[bin]]` table so the produced executable is named `pandamux` (`pandamux.exe` on Windows) even though the Cargo package remains `pandamux-app`; this keeps `-p pandamux-app` invocations and the CI smoke commands unchanged while matching the historical Electron exe name and the winresource metadata embedded in `build.rs` ([crates/pandamux-app/Cargo.toml:9-14](../crates/pandamux-app/Cargo.toml#L9-L14), [CLAUDE.md:42](../CLAUDE.md#L42)).

```toml
[[bin]]
name = "pandamux"
path = "src/main.rs"
```

([crates/pandamux-app/Cargo.toml:12-14](../crates/pandamux-app/Cargo.toml#L12-L14))

| Package | Binary name | Notes |
|---|---|---|
| `pandamux-app` | `pandamux` (`pandamux.exe`) | GUI/headless composition root; `[[bin]] name = "pandamux"` overrides the default package-name binary ([crates/pandamux-app/Cargo.toml:12-14](../crates/pandamux-app/Cargo.toml#L12-L14)). |
| `pandamux-cli` | `pandamux-cli` (`pandamux-cli.exe`) | The `pandamux` CLI, wire-compatible pipe client ([CLAUDE.md:68-69](../CLAUDE.md#L68-L69)). |

`[package.metadata.packager]` also configures the release packaging: it targets the `pandamux` binary as `main = true`, bundles `resources/` alongside the exe, and separately copies the already-built `pandamux-cli.exe` in as a sibling binary of the installed app ([crates/pandamux-app/Cargo.toml:50-68](../crates/pandamux-app/Cargo.toml#L50-L68)).

Sources: [crates/pandamux-app/Cargo.toml:1-73](../crates/pandamux-app/Cargo.toml#L1-L73), [CLAUDE.md:42](../CLAUDE.md#L42)
<!-- END:AUTOGEN pandamux_02_getting-started_features -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_testing -->
## Checks and Tests

Run these locally before committing; they mirror what CI enforces on every PR and push to `master`/`main` ([.github/workflows/rust.yml:1-19](../.github/workflows/rust.yml#L1-L19)).

```bash
cargo fmt --all --check
.\scripts\check-rust-boundaries.ps1     # enforces the crate-isolation invariant (Section 6.1 of the plan)
cargo test --workspace
cargo test -p pandamux-ui  --features iced-runtime --lib
cargo test -p pandamux-app --features iced-runtime --bin pandamux
```

([CLAUDE.md:34-39](../CLAUDE.md#L34-L39))

`check-rust-boundaries.ps1` enforces the crate-isolation invariant by reading each guarded crate's `Cargo.toml` and failing the build if a forbidden dependency line is present: `pandamux-core` must not depend on `iced` or `alacritty_terminal`, and `pandamux-term` must not depend on `iced` ([scripts/check-rust-boundaries.ps1:7-16](../scripts/check-rust-boundaries.ps1#L7-L16)).

```powershell
$rules = @(
  @{
    Name = 'pandamux-core'
    Forbidden = @('iced', 'alacritty_terminal')
  },
  @{
    Name = 'pandamux-term'
    Forbidden = @('iced')
  }
)
```

([scripts/check-rust-boundaries.ps1:7-16](../scripts/check-rust-boundaries.ps1#L7-L16))

The CI workflow (`.github/workflows/rust.yml`, `windows-latest`) runs on pull requests and pushes touching `Cargo.toml`, `Cargo.lock`, `crates/**`, the boundary script, or the workflow file itself, plus manual `workflow_dispatch` ([.github/workflows/rust.yml:3-19](../.github/workflows/rust.yml#L3-L19)).

| CI step | Command |
|---|---|
| Check formatting | `cargo fmt --all --check` ([.github/workflows/rust.yml:36](../.github/workflows/rust.yml#L36)) |
| Check crate boundaries | `.\scripts\check-rust-boundaries.ps1` ([.github/workflows/rust.yml:38-40](../.github/workflows/rust.yml#L38-L40)) |
| Test workspace | `cargo test --workspace` ([.github/workflows/rust.yml:42-43](../.github/workflows/rust.yml#L42-L43)) |
| Test Iced UI feature | `cargo test -p pandamux-ui --features iced-runtime --lib` ([.github/workflows/rust.yml:45-46](../.github/workflows/rust.yml#L45-L46)) |
| Test Iced app runtime feature | `cargo test -p pandamux-app --features iced-runtime --bin pandamux` ([.github/workflows/rust.yml:48-49](../.github/workflows/rust.yml#L48-L49)) |
| Smoke Iced app shell view | `cargo run -p pandamux-app --features iced-runtime -- --iced-shell-smoke` ([.github/workflows/rust.yml:51-52](../.github/workflows/rust.yml#L51-L52)) |
| Build native binaries | `cargo build -p pandamux-app -p pandamux-cli -p pandamux-term` ([.github/workflows/rust.yml:54-55](../.github/workflows/rust.yml#L54-L55)) |

Sources: [.github/workflows/rust.yml:1-56](../.github/workflows/rust.yml#L1-L56), [scripts/check-rust-boundaries.ps1:1-33](../scripts/check-rust-boundaries.ps1#L1-L33), [CLAUDE.md:34-39](../CLAUDE.md#L34-L39)
<!-- END:AUTOGEN pandamux_02_getting-started_testing -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_native -->
## Native Dependencies and Gotchas

Native deps of note (`alacritty_terminal`, `portable-pty`, `russh` + `russh-sftp`, `iced` and its wgpu/glyphon/cosmic-text stack, `arboard`) are all pinned to exact versions rather than ranges, e.g. `iced = { version = "=0.14.0", optional = true }` and `arboard = "=3.6.1"` in `pandamux-app`'s manifest, and `serde`/`tokio`/`uuid` pinned at the workspace level ([CLAUDE.md:46](../CLAUDE.md#L46), [crates/pandamux-app/Cargo.toml:17-31](../crates/pandamux-app/Cargo.toml#L17-L31), [Cargo.toml:19-23](../Cargo.toml#L19-L23)). See the crate manifests and the Phase 2 spike report (`spikes/phase2-native-terminal/PHASE2_REPORT.md`) for the rationale ([CLAUDE.md:46](../CLAUDE.md#L46)).

`winresource` is a Windows-only build-dependency of `pandamux-app` that embeds the app icon and version metadata into `pandamux.exe` via `build.rs` ([crates/pandamux-app/Cargo.toml:41-42](../crates/pandamux-app/Cargo.toml#L41-L42)).

```toml
[target.'cfg(windows)'.build-dependencies]
winresource = "=0.1.31"
```

([crates/pandamux-app/Cargo.toml:41-42](../crates/pandamux-app/Cargo.toml#L41-L42))

A missing resource compiler is treated as a non-fatal warning, so a dev box without the Windows SDK still builds; CI (`windows-latest`) has `rc.exe` available and embeds the resources for real ([CLAUDE.md:47](../CLAUDE.md#L47)).

| Symptom | Cause | Fix |
|---|---|---|
| Freshly built Cargo test/build-script executable fails to launch, `os error 4551` | Windows Application Control intermittently blocking newly built binaries; host-policy noise, not a code bug ([CLAUDE.md:48](../CLAUDE.md#L48)) | Rerun the command, or `cargo clean -p <pkg>` then rerun ([CLAUDE.md:48](../CLAUDE.md#L48)). |
| Dev box build lacks embedded icon/version metadata | No Windows SDK / `rc.exe` present locally, so `winresource` treats the resource compiler as missing ([CLAUDE.md:47](../CLAUDE.md#L47)) | Non-fatal locally; CI has `rc.exe` and embeds resources correctly for release builds ([CLAUDE.md:47](../CLAUDE.md#L47)). |

Sources: [CLAUDE.md:44-48](../CLAUDE.md#L44-L48), [crates/pandamux-app/Cargo.toml:16-42](../crates/pandamux-app/Cargo.toml#L16-L42), [Cargo.toml:19-23](../Cargo.toml#L19-L23)
<!-- END:AUTOGEN pandamux_02_getting-started_native -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_conventions -->
## Development Conventions

Before every `git commit`, update `CHANGELOG.md` and bump `[workspace.package] version` in the root `Cargo.toml`, then write the commit message to a file and commit with `git commit -F` rather than an inline `-m` ([CLAUDE.md:149](../CLAUDE.md#L149), [.claude/rules/commit-changelog.md:1-4](../.claude/rules/commit-changelog.md#L1-L4)). The version is single-sourced from `[workspace.package] version`, currently `0.53.0`; every crate inherits it via `version.workspace = true`, and it drives `CARGO_PKG_VERSION`, which the in-app updater compares against GitHub releases ([Cargo.toml:12-13](../Cargo.toml#L12-L13), [CLAUDE.md:148](../CLAUDE.md#L148)).

Every commit bumps at least the Patch segment of SemVer (`Major.Minor.Patch`); Major is never bumped autonomously, and Minor vs. Patch ambiguity should be raised with the user rather than guessed ([.claude/rules/commit-changelog.md:22-30](../.claude/rules/commit-changelog.md#L22-L30)).

| Segment | When to increment |
|---|---|
| Major | Breaking changes: API contract changes, breaking schema migrations, auth flow changes, removed public endpoints ([.claude/rules/commit-changelog.md:15](../.claude/rules/commit-changelog.md#L15)) |
| Minor | New features or enhancements ([.claude/rules/commit-changelog.md:16](../.claude/rules/commit-changelog.md#L16)) |
| Patch | Bug fixes, security patches, performance, dependency bumps, docs, refactors, config, chores, and anything else ([.claude/rules/commit-changelog.md:17](../.claude/rules/commit-changelog.md#L17)) |

`.claude/` is the source of truth for how the repo runs: it holds the commit/changelog rule, the LL-G and BP knowledge-base checks that must be consulted before code/config work, and the custom agents to use instead of built-in subagent types ([CLAUDE.md:149](../CLAUDE.md#L149)). The crate-isolation invariant is a hard rule enforced by CI, not just a convention: never import Iced outside `pandamux-ui`, never leak `alacritty_terminal` types outside `pandamux-term` ([CLAUDE.md:146](../CLAUDE.md#L146)).

Writing style across files, code, and comments avoids em dashes and double dashes; use commas, colons, parentheses, or semicolons instead ([CLAUDE.md:150](../CLAUDE.md#L150)).

Sources: [CLAUDE.md:143-150](../CLAUDE.md#L143-L150), [.claude/rules/commit-changelog.md:1-45](../.claude/rules/commit-changelog.md#L1-L45), [Cargo.toml:12-13](../Cargo.toml#L12-L13)
<!-- END:AUTOGEN pandamux_02_getting-started_conventions -->

---
