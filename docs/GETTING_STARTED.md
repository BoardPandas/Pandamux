<!-- PAGE_ID: pandamux_02_getting-started -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [CLAUDE.md:115-377](../CLAUDE.md#L115-L377)
- [Cargo.toml:1-40](../Cargo.toml#L1-L40)
- [.github/workflows/rust.yml:1-140](../.github/workflows/rust.yml#L1-L140)
- [scripts/check-rust-boundaries.ps1:1-60](../scripts/check-rust-boundaries.ps1#L1-L60)
- [scripts/check-claude-wiring.mjs:1-233](../scripts/check-claude-wiring.mjs#L1-L233)
- [.claude/rules/commit-changelog.md:1-51](../.claude/rules/commit-changelog.md#L1-L51)

</details>

# Getting Started

> **Related Pages**: [Overview](OVERVIEW.md), [Release and Packaging](operations/RELEASE.md)

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_prerequisites -->
## Prerequisites

PandaMUX is a Rust workspace with native desktop and operating-system integration. Install the tools required by the checks you intend to run.

| Tool | Required version or platform | Purpose |
|---|---|---|
| Rust | Stable, with project minimum `1.89` | Compile and test the Rust 2024 workspace ([Cargo.toml:23-28](../Cargo.toml#L23-L28)) |
| Windows build tools | MSVC toolchain | Build the native Windows desktop and IPC implementation ([CLAUDE.md:115-117](../CLAUDE.md#L115-L117)) |
| PowerShell | `pwsh` capable | Run the crate-boundary guard ([CLAUDE.md:146-152](../CLAUDE.md#L146-L152)) |
| Node.js | 24 for parity with CI | Run the repository wiring guard; Node is not an application runtime dependency ([CLAUDE.md:115-117](../CLAUDE.md#L115-L117), [rust.yml:43-50](../.github/workflows/rust.yml#L43-L50)) |
| musl tools | Linux x86_64 artifact builds only | Build the server and CLI for `x86_64-unknown-linux-musl` ([rust.yml:115-139](../.github/workflows/rust.yml#L115-L139)) |

The workspace has no package-manager install step. Cargo resolves the dependencies declared in the root and crate manifests.

Sources: [Cargo.toml:23-40](../Cargo.toml#L23-L40), [CLAUDE.md:115-117](../CLAUDE.md#L115-L117), [rust.yml:35-55](../.github/workflows/rust.yml#L35-L55), [rust.yml:115-139](../.github/workflows/rust.yml#L115-L139)
<!-- END:AUTOGEN pandamux_02_getting-started_prerequisites -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_build -->
## Building the Workspace

Build the three runtime binaries together when you need a complete local artifact set:

```bash
cargo build --release -p pandamux-cli -p pandamux-server -p pandamux-desktop
```

That is the same release-binary set built by the Windows CI job and the same core set built on Apple Silicon ([rust.yml:81-89](../.github/workflows/rust.yml#L81-L89), [rust.yml:109-113](../.github/workflows/rust.yml#L109-L113)). For a quicker compile-only pass over the main dependency graph, use the scoped check documented by the repository:

```bash
cargo check -p pandamux-protocol -p pandamux-client -p pandamux-cli -p pandamux-server -p pandamux-desktop
```

Run Cargo operations serially within one checkout. Parallel Cargo builds and tests share the target directory and package cache, so serialization avoids avoidable contention ([CLAUDE.md:142-165](../CLAUDE.md#L142-L165)).

Sources: [CLAUDE.md:142-165](../CLAUDE.md#L142-L165), [rust.yml:81-89](../.github/workflows/rust.yml#L81-L89), [rust.yml:109-113](../.github/workflows/rust.yml#L109-L113)
<!-- END:AUTOGEN pandamux_02_getting-started_build -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_run -->
## Running PandaMUX

Start the Hub and desktop from the same checkout for the most direct development loop. The desktop can discover or launch a sibling server, but starting the server first is useful when the binaries do not share a target directory ([CLAUDE.md:119-140](../CLAUDE.md#L119-L140)).

```bash
cargo run -p pandamux-server
cargo run -p pandamux-desktop
```

| Goal | Command |
|---|---|
| Headless desktop verification | `cargo run -p pandamux-desktop -- --headless` |
| Render-and-exit desktop smoke | `cargo run -p pandamux-desktop -- --smoke` |
| Remote Linux node daemon | `cargo run -p pandamux-server -- daemon --role node` |
| SSH exec proxy bridge | `cargo run -p pandamux-server -- proxy` |
| Inspect CLI help | `cargo run -p pandamux-cli -- --help` |
| Check Hub connectivity | `cargo run -p pandamux-cli -- ping` |

These are repository-defined entry points, not release or deployment commands ([CLAUDE.md:119-138](../CLAUDE.md#L119-L138)).

Sources: [CLAUDE.md:119-140](../CLAUDE.md#L119-L140)
<!-- END:AUTOGEN pandamux_02_getting-started_run -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_testing -->
## Checks and Tests

The complete local gate combines formatting, Clippy, the workspace test suite, dependency-boundary validation, and repository-agent wiring validation.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pwsh -File scripts/check-rust-boundaries.ps1
node scripts/check-claude-wiring.mjs
```

| Check | What it protects |
|---|---|
| `cargo fmt --all --check` | Rust formatting across the workspace ([rust.yml:65-66](../.github/workflows/rust.yml#L65-L66)) |
| Clippy with `-D warnings` | Lint-clean code across all targets ([rust.yml:68-69](../.github/workflows/rust.yml#L68-L69)) |
| `cargo test --workspace` | Workspace Rust behavior ([rust.yml:81-82](../.github/workflows/rust.yml#L81-L82)) |
| Rust boundary guard | Prevents implementation-framework dependencies from crossing crate ownership boundaries ([check-rust-boundaries.ps1:7-43](../scripts/check-rust-boundaries.ps1#L7-L43)) |
| Claude wiring guard | Detects dead rule globs, invalid hook matchers, silenced hooks, and excessive always-on context ([check-claude-wiring.mjs:3-22](../scripts/check-claude-wiring.mjs#L3-L22)) |

CI additionally runs advisory and license checks and evaluates the built Windows artifacts ([rust.yml:75-89](../.github/workflows/rust.yml#L75-L89)).

Sources: [CLAUDE.md:142-168](../CLAUDE.md#L142-L168), [rust.yml:65-89](../.github/workflows/rust.yml#L65-L89), [check-rust-boundaries.ps1:7-60](../scripts/check-rust-boundaries.ps1#L7-L60), [check-claude-wiring.mjs:3-22](../scripts/check-claude-wiring.mjs#L3-L22)
<!-- END:AUTOGEN pandamux_02_getting-started_testing -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_platforms -->
## Platform Notes

Platform coverage differs by component. The Windows job is the full native gate; macOS validates the core client/server/desktop set; Linux musl produces only the remote node binaries.

| Platform | Current automated coverage |
|---|---|
| Windows MSVC | Wiring guard, formatting, Clippy, crate boundaries, advisories/licenses, workspace tests, release binary build, and built-artifact smoke ([rust.yml:35-89](../.github/workflows/rust.yml#L35-L89)) |
| macOS Apple Silicon | Tests core, protocol, client, server, CLI, and desktop; builds server, CLI, and desktop release binaries ([rust.yml:91-113](../.github/workflows/rust.yml#L91-L113)) |
| Linux x86_64 musl | Cross-builds `pandamux-server` and `pandamux-cli` for node use ([rust.yml:115-139](../.github/workflows/rust.yml#L115-L139)) |

- The GPUI desktop is the local interactive surface.
- The server supports a Hub role and a Linux node-daemon role.
- A successful Linux build does not establish native Windows UI behavior.
- Before a long or silent command, state what will run, estimate its duration, and say whether cancellation is safe ([CLAUDE.md:168-170](../CLAUDE.md#L168-L170)).

Sources: [rust.yml:35-140](../.github/workflows/rust.yml#L35-L140), [CLAUDE.md:168-170](../CLAUDE.md#L168-L170)
<!-- END:AUTOGEN pandamux_02_getting-started_platforms -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_conventions -->
## Development Conventions

Repository workflow rules are designed to preserve evidence and keep generated claims aligned with what actually ran.

- Preserve unrelated work in a dirty checkout; do not reset, clean, broadly stage, or absorb it.
- Treat approved plans and acceptance matrices as contracts.
- Update the changelog and the root workspace version when preparing a commit.
- Write commit messages through a file with `git commit -F`.
- Use custom agents only when delegation is explicitly authorized.
- Do not claim a build, test, artifact, commit, push, deployment, or live acceptance without direct evidence ([CLAUDE.md:368-377](../CLAUDE.md#L368-L377)).

The commit rule requires user-facing changelog entries categorized under Keep a Changelog headings and requires reviewing the staged diff before describing the change ([commit-changelog.md:7-19](../.claude/rules/commit-changelog.md#L7-L19)). The development guide is authoritative where older rule prose still refers to the removed JavaScript-era version file.

The wiring guard is intentionally falsifiable: it exits successfully only when no wiring errors were collected, otherwise it prints every error and exits 1 ([check-claude-wiring.mjs:221-233](../scripts/check-claude-wiring.mjs#L221-L233)).

Sources: [CLAUDE.md:368-377](../CLAUDE.md#L368-L377), [commit-changelog.md:7-19](../.claude/rules/commit-changelog.md#L7-L19), [check-claude-wiring.mjs:221-233](../scripts/check-claude-wiring.mjs#L221-L233)
<!-- END:AUTOGEN pandamux_02_getting-started_conventions -->

---
