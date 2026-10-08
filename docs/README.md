# PandaMUX Documentation

This index routes contributors to current sources for the GPUI chat-first rewrite. Read the root [`Cargo.toml`](../Cargo.toml) for the version; documentation does not duplicate it.

## Start here

| Goal | Current source |
|---|---|
| Understand the product and current status | [`README.md`](../README.md) |
| Build, test, and follow repository conventions | [`CLAUDE.md`](../CLAUDE.md) |
| Orient an agent and resolve source authority | [`AGENTS.md`](../AGENTS.md) |
| Implement the active rewrite | [`tasks/plan-rewrite.md`](../tasks/plan-rewrite.md) |
| Inspect CI truth | [`.github/workflows/rust.yml`](../.github/workflows/rust.yml) |
| Inspect current release notes | [`CHANGELOG.md`](../CHANGELOG.md) |

## Current architecture map

| Area | Source of truth | Responsibility |
|---|---|---|
| Domain | [`crates/pandamux-core`](../crates/pandamux-core) | Framework-free IDs, settings, threads, turns, events, schedules, policy data |
| Wire protocol | [`crates/pandamux-protocol`](../crates/pandamux-protocol) | Versioned JSON-RPC and Hub-to-node proxy contracts |
| Providers | [`crates/pandamux-providers`](../crates/pandamux-providers) | Provider drivers, profiles, parsing, and process supervision |
| SSH and bootstrap | [`crates/pandamux-remote`](../crates/pandamux-remote) | SSH pool, exec, SFTP, host bootstrap, and proxy tunnels |
| Server | [`crates/pandamux-server`](../crates/pandamux-server) | Single-writer store, request contexts, provider execution, PTYs, Hub and node runtime |
| Client projections | [`crates/pandamux-client`](../crates/pandamux-client) | Event-derived view state and token smoothing |
| Desktop | [`crates/pandamux-desktop`](../crates/pandamux-desktop) | GPUI chat shell, timeline, composer, settings, surfaces, and terminal grid |
| CLI | [`crates/pandamux-cli`](../crates/pandamux-cli) | Authenticated commands and restricted MCP stdio adapter |
| Terminal primitives | [`crates/pandamux-term`](../crates/pandamux-term) | Separately selectable `grid` and `pty` features |

The server is the production owner of remote composition and durable state. The desktop is a GPUI client. Remote Linux nodes run the server daemon and are reached through the versioned SSH exec proxy; no remote TCP control port is opened.

## Active Phase 4 status

The unfinished Phase 4 prototype was preserved under Recovery Gate R0.1. R0.2
through R0.4 establish production remote routing, exact provider-instance
resolution, and authenticated local control. R0.5 has exact live evidence for
Galahad and native Windows IPC. The user approved deferring its remaining
physical laptop checks on 2026-09-29, which closes the development gate but
does not satisfy production acceptance or release. Phase 4A supplies the
normalized usage, policy, migration, and authenticated bounded-protocol
foundation. Phase 4B adds node-owned scanning and reconciliation. Phase 4C adds
durable Hub synchronization, indexed aggregation, effective-dated integer
pricing, and cross-node coverage. Phase 4D adds conservative capacity
revisions, bounded provider health, and explicit preview-before-launch login
actions. Phase 4E adds the top-level Usage surface, named provider and
subscription presentation, canonical policy enforcement, and source-level
accessibility work. Phase 4F adds explicit compatible failover pools,
deterministic routing, monotonic side-effect boundaries, safe single retry,
audited fresh-session continuation, fail-closed unresolved-tool completion,
and independent rollback switches. Exact
0.60.15 passed accounting and cloned-store reconciliation. Exact 0.60.22 passed
its executed native Windows interaction, accessibility, visual, and normal-user
IPC rows, but P4-C3 was reopened because successful provider-login terminal
discovery and interaction were omitted and then failed in manual use. Exact
0.60.24 proved the repaired panel, terminal binding, and automatic focus
natively, but PowerShell did not execute the injected command because ConPTY
received a line feed instead of carriage return. Exact 0.60.25 proved native
command execution visually, then failed because painted Terminal output was
absent from UI Automation. Exact 0.60.26 exposed bounded visible Terminal
output and passed the successful-login interaction, High Contrast, and
disabled-animation rows, but genuine Narrator Speech Recap did not announce
READY or ECHO. Exact 0.60.27 passed the successful-login interaction, 200
percent scaling, High Contrast, and disabled-animation rows, but UI Automation
could not move focus to the Log and the advertised Tab path remained PTY input.
Exact 0.60.28 added an explicit F6 input/output focus toggle and passed
successful-login interaction, 200 percent scaling, High Contrast, and
disabled-animation rows, but Narrator displaced focus to the window before the
terminal-scoped shortcut could run. Exact 0.60.29 commit `2b3b806` passed native
build, smoke, remote lifecycle, successful-login interaction, 200 percent
scaling, High Contrast, and disabled-animation rows, but its root visual
handler still had no GPUI focus path under Narrator and did not receive F6.
Exact 0.60.30 commit `9dae237` passed native build, smoke, remote lifecycle, and
standard successful-login interaction, but Narrator still consumed or withheld
F6 before PandaMUX received it. Exact 0.60.31 commit `0394eb3` passed native
build, smoke, remote lifecycle, and standard successful-login interaction. Its
explicit controls were visible and invokable, but accessibility activation
restored window-level focus after the output-focus request, so Narrator still
could not read the terminal Log. Exact 0.60.32 commit `952b280` passed native
build, smoke, remote lifecycle, and standard successful-login interaction, but
direct UI Automation Invoke still left focus on the PandaMUX window because the
styled control fell back to synthesized pointer input under Narrator. Exact
0.60.33 commit `58452e7` passed native build, smoke, remote lifecycle, and
standard successful-login interaction, but its direct AccessKit handler
deferred the focus update and UI Automation Invoke still left focus on the
PandaMUX window. Exact 0.60.34 commit `1f1df21` passed native build, smoke,
remote lifecycle, and standard successful-login interaction, but Windows UI
Automation still restored window-level focus after the synchronous
accessibility callback. Exact 0.60.35 commit `b54f8e3` repeated the requested
focus once after platform Invoke dispatch completes. Its exact native build,
smoke, remote lifecycle, and standard row passed, but GPUI elided the delayed
same-handle request while UI Automation focus remained on the PandaMUX window.
Exact 0.60.36 commit `6fd5d02` made that move a distinct GPUI focus transition
and passed the same native build, smoke, remote lifecycle, and standard row,
but UI Automation still reported the PandaMUX window. Exact 0.60.37 commit
`f21bc2d` activated PandaMUX and passed the same gates, but its Narrator result
was unchanged. Diagnostics proved PandaMUX already owned the foreground window
and had visibly entered output-reading mode. Exact 0.60.38 commit `d328ad8`
then passed the complete local gate, native build, host smoke, remote lifecycle,
and standard successful-login interaction. Ordinary UI Automation honored its
active-descendant output focus, but Narrator did not speak READY. Diagnostics
proved that native foreground, active, and keyboard focus remained correct
while synchronized Narrator navigation reached the terminal header controls
and skipped the semantic Log group. Exact 0.60.39 commit `8b7b880` instead used
AccessKit's dedicated Terminal role and passed the complete local gate, native
build, host gates, and standard successful-login interaction. Windows reported
the output as a focusable UI Automation Document, but genuine Narrator visited
that Document without speaking its parent-only accessible name and then left
PandaMUX. Exact 0.60.40 commit `d4c9dd8` retained the Document, passed its local
gate, native build, and host gates, but failed the standard guest row because
its label-only semantic child was absent from Windows UI Automation. Exact
0.60.41 commit `c6a20ff` publishes the child value with a synthetic text
run so UIA exposes TextPattern content. Its complete local, native,
login-terminal, scaling, High Contrast, disabled-animation, genuine Narrator,
visual, and named-pipe matrices passed; those rows remain accepted. Exact
0.60.43 commit `372f2ae` passed local schema migration, live six-model Codex
discovery, and a real turn through `codex-default` plus `gpt-6-luna`. Its exact
limited-user Windows build, headless smoke, visible no-route fallback, and
authenticated picker-created `codex-default` plus `gpt-6.1-sol` thread also
passed, so P4-C3 is Pass again. A separate supporting live run later killed the
exact remote proxy after visible text and one successful file-writing shell
tool. Reconnect kept one protected attempt and the one-line marker without
replay. Exact 0.60.47 then repaired unresolved-tool false success and passed a
clean supporting run with one approval plus one loopback HTTP POST across
abrupt proxy loss. Reconnect retained one request, one terminal tool ledger
row, one protected attempt, one settled turn, and one matching managed usage
observation without replay. Exact 0.60.49 then made completed tool-result
projection atomic and passed the live pending-approval plus exact tool-result
process-crash restart rows without replay. Exact 0.60.52 repaired continuation
route persistence and passed restart while one real Codex continuation was
Running after a controlled source interruption. The loopback request count
remained one, and duplicate continuation calls refused redispatch. A supporting
Hyper-V rehearsal on exact 0.60.43
also passed a real 30-minute provider-network interruption and full-screen
terminal saved-state recovery with exact event, usage, offset, replay, and
cleanup evidence. It does not replace the required physical laptop disconnect
and sleep rows. Exact 0.60.17 partially passed P4-C5 with live scanner and Hub
durability, conservative capacity and route decisions, and independent
rollback controls. Real provider capacity exhaustion, external-service
interruption, the final archive after those rows stabilize, and the deferred
physical checks remain open. The complete
current contract is in
[`features/USAGE_OBSERVABILITY.md`](features/USAGE_OBSERVABILITY.md).

The [Phase 4 live acceptance record](operations/PHASE4_ACCEPTANCE.md) holds hashes,
commands, results, and unresolved rows. The authoritative completion criteria
remain in [`tasks/plan-rewrite.md`](../tasks/plan-rewrite.md).

## Security boundary

Local requests authenticate before routing and receive a server-owned `RequestContext`. Desktop and CLI control, MCP, provider-child, node-proxy, and internal calls have distinct scopes. Windows uses a SID-scoped protected named pipe; Unix uses a private runtime directory and socket plus peer credentials.

This boundary does not isolate a hostile process already running as the same OS user. FullAccess provider sessions intentionally have host-user authority. See [`CLAUDE.md`](../CLAUDE.md#server-and-local-ipc) for the exact contract.

## Historical wiki

Except for [`features/USAGE_OBSERVABILITY.md`](features/USAGE_OBSERVABILITY.md)
and [`operations/PHASE4_ACCEPTANCE.md`](operations/PHASE4_ACCEPTANCE.md), the
pages below document the removed Iced multiplexer and remain available for
implementation history. They are not authoritative for the active GPUI
workspace, current commands, crate names, IPC security, or release readiness.

- [`OVERVIEW.md`](OVERVIEW.md)
- [`GETTING_STARTED.md`](GETTING_STARTED.md)
- [`core/`](core/)
- [`api/CLI_REFERENCE.md`](api/CLI_REFERENCE.md)
- [`features/`](features/)
- [`operations/RELEASE.md`](operations/RELEASE.md)
- [`GLOSSARY.md`](GLOSSARY.md)

Older Electron material is under [`archive/`](archive/). Superseded implementation plans and specs are under [`superpowers/`](superpowers/). [`tasks/plan-repo.md`](../tasks/plan-repo.md) retains the native rewrite history and Section 12 UI specification, but it is not the current roadmap.

## Research

Decision-oriented research remains under [`research/`](research/). Verify code and external facts again before relying on an older comparison.
