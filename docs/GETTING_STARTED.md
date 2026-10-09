<!-- PAGE_ID: pandamux_02_getting-started -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [CLAUDE.md:121-407](../CLAUDE.md#L121-L407)
- [Cargo.toml:1-48](../Cargo.toml#L1-L48)
- [rust-toolchain.toml:1-4](../rust-toolchain.toml#L1-L4)
- [.github/workflows/rust.yml:1-207](../.github/workflows/rust.yml#L1-L207)
- [tools/pandamux-boundary-check/src/lib.rs:1-761](../tools/pandamux-boundary-check/src/lib.rs#L1-L761)
- [tools/pandamux-boundary-check/tests/fixture_workspaces.rs:1-80](../tools/pandamux-boundary-check/tests/fixture_workspaces.rs#L1-L80)
- [scripts/check-claude-wiring.mjs:1-233](../scripts/check-claude-wiring.mjs#L1-L233)
- [.claude/rules/commit-changelog.md:1-51](../.claude/rules/commit-changelog.md#L1-L51)

</details>

# Getting Started

> **Related Pages**: [Overview](OVERVIEW.md), [Toolchain and Dependency Policy](operations/DEPENDENCIES.md), [Release and Packaging](operations/RELEASE.md)

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_prerequisites -->
## Prerequisites

PandaMUX is a Rust workspace with native desktop and operating-system integration. Install the tools required by the checks you intend to run.

| Tool | Required version or platform | Purpose |
|---|---|---|
| Rust | Development/evidence `1.98.1`; scoped MSRV `1.89` | Compile and test the Rust 2024 workspace with a reproducible development toolchain while retaining explicit minimum-version jobs ([rust-toolchain.toml:1-4](../rust-toolchain.toml#L1-L4), [Cargo.toml:25-30](../Cargo.toml#L25-L30)) |
| Windows build tools | MSVC toolchain | Build the native Windows desktop and IPC implementation ([CLAUDE.md:121-127](../CLAUDE.md#L121-L127)) |
| Node.js | 24 for parity with CI | Run the repository wiring guard; Node is not an application runtime dependency ([CLAUDE.md:121-127](../CLAUDE.md#L121-L127), [rust.yml:47-54](../.github/workflows/rust.yml#L47-L54)) |
| `cargo-deny` | Current local installation, optional outside CI | Reproduce the advisory, license, ban, and source policy enforced by CI ([rust.yml:81-85](../.github/workflows/rust.yml#L81-L85)) |
| musl tools | Linux x86_64 artifact builds only | Build the server and CLI for `x86_64-unknown-linux-musl`; pure-orchestrator musl checks use `cargo check` and require only the Rust targets ([rust.yml:124-148](../.github/workflows/rust.yml#L124-L148), [rust.yml:150-181](../.github/workflows/rust.yml#L150-L181)) |

The workspace has no package-manager install step. Cargo resolves the dependencies declared in the root and crate manifests.

Sources: [Cargo.toml:25-48](../Cargo.toml#L25-L48), [rust-toolchain.toml:1-4](../rust-toolchain.toml#L1-L4), [CLAUDE.md:123-129](../CLAUDE.md#L123-L129), [rust.yml:39-85](../.github/workflows/rust.yml#L39-L85), [rust.yml:124-207](../.github/workflows/rust.yml#L124-L207)
<!-- END:AUTOGEN pandamux_02_getting-started_prerequisites -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_build -->
## Building the Workspace

Build the three runtime binaries together when you need a complete local artifact set:

```bash
cargo build --release -p pandamux-cli -p pandamux-server -p pandamux-desktop
```

That is the same release-binary set built by the Windows CI job and the same core set built on Apple Silicon ([rust.yml:87-95](../.github/workflows/rust.yml#L87-L95), [rust.yml:115-119](../.github/workflows/rust.yml#L115-L119)). For a quicker compile-only pass over the main dependency graph, use the scoped check documented by the repository:

```bash
cargo check -p pandamux-protocol -p pandamux-client -p pandamux-cli -p pandamux-server -p pandamux-desktop
```

Run Cargo operations serially within one checkout. Parallel Cargo builds and tests share the target directory and package cache, so serialization avoids avoidable contention ([CLAUDE.md:152-186](../CLAUDE.md#L152-L186)).

Sources: [CLAUDE.md:152-186](../CLAUDE.md#L152-L186), [rust.yml:87-95](../.github/workflows/rust.yml#L87-L95), [rust.yml:115-119](../.github/workflows/rust.yml#L115-L119)
<!-- END:AUTOGEN pandamux_02_getting-started_build -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_run -->
## Running PandaMUX

Start the Hub and desktop from the same checkout for the most direct development loop. The desktop can discover or launch a sibling server, but starting the server first is useful when the binaries do not share a target directory ([CLAUDE.md:129-150](../CLAUDE.md#L129-L150)).

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

These are repository-defined entry points, not release or deployment commands ([CLAUDE.md:129-150](../CLAUDE.md#L129-L150)).

Sources: [CLAUDE.md:129-150](../CLAUDE.md#L129-L150)
<!-- END:AUTOGEN pandamux_02_getting-started_run -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_testing -->
## Checks and Tests

The complete source gate combines formatting, Clippy, the workspace test suite, resolved dependency-boundary validation, repository-agent wiring validation, and dependency policy.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --locked -p pandamux-boundary-check
cargo run --locked -p pandamux-boundary-check --quiet
node scripts/check-claude-wiring.mjs
cargo deny check advisories licenses bans sources
```

| Check | What it protects |
|---|---|
| `cargo fmt --all --check` | Rust formatting across the workspace ([rust.yml:65-66](../.github/workflows/rust.yml#L65-L66)) |
| Clippy with `-D warnings` | Lint-clean code across all targets ([rust.yml:68-69](../.github/workflows/rust.yml#L68-L69)) |
| `cargo test --workspace` | Workspace Rust behavior ([rust.yml:81-82](../.github/workflows/rust.yml#L81-L82)) |
| Boundary-checker fixtures | Prove the checker rejects transitive GPUI, terminal-feature, embedding, and GPUI-version regressions ([fixture_workspaces.rs:39-62](../tools/pandamux-boundary-check/tests/fixture_workspaces.rs#L39-L62)) |
| Resolved graph guard | Reconstructs per-root features from target-filtered locked Cargo metadata and enforces exact direct pins plus implementation ownership across five declared targets ([lib.rs:9-63](../tools/pandamux-boundary-check/src/lib.rs#L9-L63), [lib.rs:140-449](../tools/pandamux-boundary-check/src/lib.rs#L140-L449)) |
| Claude wiring guard | Detects dead rule globs, invalid hook matchers, silenced hooks, and excessive always-on context ([check-claude-wiring.mjs:3-22](../scripts/check-claude-wiring.mjs#L3-L22)) |
| `cargo-deny` | Applies the advisory, license, duplicate-version, and dependency-source policy ([rust.yml:81-85](../.github/workflows/rust.yml#L81-L85)) |

CI also evaluates built Windows artifacts, compiles the product and headless graphs at the declared MSRV, tests the pure orchestrator on macOS, and compiles its library and test targets for musl x64 and arm64 ([rust.yml:87-95](../.github/workflows/rust.yml#L87-L95), [rust.yml:118-119](../.github/workflows/rust.yml#L118-L119), [rust.yml:150-207](../.github/workflows/rust.yml#L150-L207)). The boundary command and cross-target `cargo check` analyze or compile declared graphs; they do not replace native builds, execute cross-target tests, or establish live platform acceptance.

Sources: [CLAUDE.md:154-188](../CLAUDE.md#L154-L188), [rust.yml:69-95](../.github/workflows/rust.yml#L69-L95), [rust.yml:118-119](../.github/workflows/rust.yml#L118-L119), [rust.yml:150-207](../.github/workflows/rust.yml#L150-L207), [lib.rs:140-449](../tools/pandamux-boundary-check/src/lib.rs#L140-L449), [fixture_workspaces.rs:39-62](../tools/pandamux-boundary-check/tests/fixture_workspaces.rs#L39-L62), [check-claude-wiring.mjs:3-22](../scripts/check-claude-wiring.mjs#L3-L22)
<!-- END:AUTOGEN pandamux_02_getting-started_testing -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_platforms -->
## Platform Notes

Platform coverage differs by component. The Windows job is the full native gate; macOS validates the core client/server/desktop set plus the pure orchestrator; Linux musl builds the x64 remote-node binaries and compile-checks the pure orchestrator on x64 and arm64.

| Platform | Current automated coverage |
|---|---|
| Windows MSVC | Development-toolchain wiring, formatting, Clippy, resolved boundaries, complete cargo-deny policy, workspace tests, release binary build, and built-artifact smoke; a separate job checks the complete product graph at Rust 1.89 ([rust.yml:39-95](../.github/workflows/rust.yml#L39-L95), [rust.yml:183-194](../.github/workflows/rust.yml#L183-L194)) |
| macOS job | Tests the listed core/client/server/desktop set, explicitly tests `pandamux-orchestrator`, and builds server, CLI, and desktop release binaries with Rust 1.98.1 ([rust.yml:97-122](../.github/workflows/rust.yml#L97-L122)) |
| Linux musl | Cross-builds x64 `pandamux-server` and `pandamux-cli`; compile-checks the pure orchestrator and all of its test targets for x64 and arm64; and separately checks the headless workspace graph at Rust 1.89 ([rust.yml:124-181](../.github/workflows/rust.yml#L124-L181), [rust.yml:196-207](../.github/workflows/rust.yml#L196-L207)) |

- The GPUI desktop is the local interactive surface.
- The server supports a Hub role and a Linux node-daemon role.
- A successful Linux build does not establish native Windows UI behavior.
- Before a long or silent command, state what will run, estimate its duration, and say whether cancellation is safe ([CLAUDE.md:180-186](../CLAUDE.md#L180-L186)).

Sources: [rust.yml:39-207](../.github/workflows/rust.yml#L39-L207), [CLAUDE.md:182-188](../CLAUDE.md#L182-L188)
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
- Do not claim a build, test, artifact, commit, push, deployment, or live acceptance without direct evidence ([CLAUDE.md:398-407](../CLAUDE.md#L398-L407)).

The commit rule requires user-facing changelog entries categorized under Keep a Changelog headings and requires reviewing the staged diff before describing the change ([commit-changelog.md:7-19](../.claude/rules/commit-changelog.md#L7-L19)). The development guide is authoritative where older rule prose still refers to the removed JavaScript-era version file.

The wiring guard is intentionally falsifiable: it exits successfully only when no wiring errors were collected, otherwise it prints every error and exits 1 ([check-claude-wiring.mjs:221-233](../scripts/check-claude-wiring.mjs#L221-L233)).

Sources: [CLAUDE.md:398-407](../CLAUDE.md#L398-L407), [commit-changelog.md:7-19](../.claude/rules/commit-changelog.md#L7-L19), [check-claude-wiring.mjs:221-233](../scripts/check-claude-wiring.mjs#L221-L233)
<!-- END:AUTOGEN pandamux_02_getting-started_conventions -->

---
