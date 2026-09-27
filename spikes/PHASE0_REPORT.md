# PandaMUX Phase 0 (Spikes) Evaluation Report

Status: Complete (All 7 Spikes Passed: S1, S2, S3, S4, S5a, S5b, S6)
Date: 2026-09-26

## 1. Executive Summary

Phase 0 tests architectural risks and confirms assumptions before starting Phase 1 implementation. All seven spikes have completed successfully:

1. **S1 (GPUI Chat Spike)**: Passed. Verifies that GPUI 0.3.6 and gpui-kit 0.6.6 provide high-performance markdown streaming, virtualized 2,000-message scrolling, multi-line composition with attachment chips, collapsible tool calls, and inline diffs on Windows MSVC.
2. **S2 (Codex and Claude Drivers)**: Passed. Verifies zero-session auth detection, stream-json protocol with permission prompt tools, sub-agent tree parsing, Codex JSON-RPC schema contracts, developer instructions injection, and rate-limit parsing.
3. **S3 (Remote Bootstrap)**: Passed. Verifies remote detection without PTY on Galahad (Linux x86_64), static Linux musl binary packaging and delivery via SSH, remote SHA-256 bit-for-bit hash verification, `setsid` background daemon lifecycle and `server.json` discovery, SSH exec proxy tunnel architecture (avoiding sshd streamlocal restrictions), mid-turn connection severing with daemon survival, and `sinceSeq` reconnection replaying missed events and background timer heartbeats.
4. **S4 (Packaging)**: Passed. Verifies cargo-packager NSIS multi-binary installer generation for Windows currentUser installations. Confirms bundling of three Windows executables (desktop, server, CLI) and bundled Linux node binaries as resources. Verifies code signing integration: omission of `before-packaging-command` guarantees that pre-applied Azure Trusted Signing Authenticode signatures remain bit-for-bit intact through packaging and extraction.
5. **S5a (Antigravity Integration)**: Passed. Verifies managed bundle validation, two-entry hardened extraction, replaced environment with isolated `GEMINI_HOME`, OAuth URL parsing and remote redirect relay, ACP session flow, prompt injection warning detection, and zero-spawn reliability rules.
6. **S5b (Best-effort ACP Providers)**: Passed. Verifies fixture contracts for Cursor (`cursor-agent acp`), Grok CLI (`grok acp`), and OpenCode (`opencode acp` vs `opencode serve`). Establishes that Cursor, Grok, and OpenCode ACP use fallback preamble injection and coarse permissions, while OpenCode serve mode supports native session-level system prompts and per-tool allow/deny filters.
7. **S6 (Pins and License Audit)**: Passed. Verifies that all gpui-pre and gpui-kit crates are Apache-2.0, with zero GPL contamination in the desktop graph, satisfying the license gate for zed#55470.

| Spike | Title | Gate Status | Verdict | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **S1** | GPUI Chat Spike | Gating | **PASS** | 2,000 items in 1.09ms, ~50 tok/s stream, Direct3D rendering verified |
| **S2** | Codex and Claude Drivers | Gating | **PASS** | Zero-session auth probe, stream-json, sub-agents, developer instructions |
| **S3** | Remote Bootstrap | Non-gating | **PASS** | Exec without PTY, musl binary delivery, remote SHA-256, setsid daemon, exec proxy tunnel, mid-turn disconnect and replay |
| **S4** | Packaging | Non-gating | **PASS** | NSIS multi-binary installer, Authenticode preservation, Linux musl resources |
| **S5a** | Antigravity Integration | Gating | **PASS** | Managed bundle, isolated env, OAuth relay, ACP permissions, reliability rules |
| **S5b** | Best-effort ACP | Non-gating | **PASS** | Cursor, Grok, and OpenCode probes; preamble vs native matrix |
| **S6** | Pins and Licenses | Gating | **PASS** | Apache-2.0 confirmed, zero GPL contamination |

## 2. S1 GPUI Chat Spike Detailed Findings

The test harness is implemented in `spikes/phase0-gpui-chat/src/main.rs`. It exercises the full UI stack on native Windows.

### 2.1 Performance and Capacity
- **2,000-Item Thread Generation**: 1.09 ms (1,833 items/ms) allocation and ingestion time.
- **Virtualized Rendering**: Handled through `MessageScroller` and GPUI virtual lists with zero frame drops during rapid scrolling.
- **Scroll Synchronization**: Tail-following mode automatically tracks incoming live tokens and offers an interactive jump-to-bottom button when scrolled back.

### 2.2 Streaming Markdown and Tokens
- **Target Velocity**: Tuned to 20 ms per token cadence (50.0 tokens/s).
- **Rich Markdown Formatting**: Streams complex tables, rust code blocks, headers, and paragraphs.
- **Visual Polish**: Utilizes `TextView::stream_fade(true)` to fade incoming token runs smoothly over 350 ms instead of abruptly jumping into view.

### 2.3 Composer and Attachments
- **Multi-Line Text Area**: Styled `Textarea` with dynamic auto-grow (clamped between 1 and 5 rows).
- **Attachment Chips**: Uses `Attachment` component displaying filename, size description, status indicator (`AttachmentStatus::Complete`), and dismiss action.
- **Drag and Drop Simulation**: Validated file drop and image paste placeholder behavior.

### 2.4 Rich Message Nodes
- **Collapsible Tool Calls**: Uses `Collapsible` with spring reveal animation. Displays tool name, exit code badge, and toggles parameters and stdout/stderr output.
- **Inline Diffs**: Custom syntax-styled diff viewer rendering file headers, added lines with green background tints, and removed lines with red background tints.

### 2.5 Live Window and GPU Pipeline
- Compiles on Windows MSVC with Direct3D and DirectWrite backends.
- Passed live execution check (`cargo run -- --smoke`) with window creation, frame painting, automated benchmark execution, and clean shutdown.

## 3. S6 Pins and License Audit Detailed Findings

### 3.1 Crate Licensing Audit
All crates in the GPUI snapshot family were audited:
- `gpui-pre`: Apache-2.0
- `gpui-pre-ztracing`: Apache-2.0
- `gpui-pre-zlog`: Apache-2.0
- `gpui-pre-sum-tree`: Apache-2.0
- `gpui-kit`: Apache-2.0
- `gpui-component`: Apache-2.0

Zero GPL licensed dependencies exist in the desktop dependency graph. The zed#55470 license gate is satisfied.

### 3.2 Toolchain Requirement
- `gpui-pre-0.3.6` utilizes `std::hint::cold_path()`, which was stabilized in Rust 1.95.0.
- Compiler upgraded to Rust 1.98.1 (`stable-x86_64-pc-windows-msvc`).
- The project MSRV requirement is updated to Rust 1.95+.

## 4. S2 Codex and Claude Driver Detailed Findings

The test harness and fixtures are implemented in `spikes/phase0-drivers/`.

### 4.1 Claude Code Driver
- **Zero-Session Auth Detection**: Confirmed that `claude auth status --json` accurately probes authentication state without creating a session, allocating tokens, or touching workspace history. Live probe on Windows executed cleanly against local Claude Code installation.
- **Stream-JSON Protocol**: Verified NDJSON streaming with `--input-format stream-json` and `--output-format stream-json`.
- **Permission Prompt Protocol**: Verified `--permission-prompt-tool stdio`. Correctly parses `control_request` payloads (tool name, command, reason) and formats `control_response` decisions (allow or deny with descriptive messages).
- **Sub-Agent Hierarchy**: Verified extraction of nested Task tool invocations, mapping `parent_tool_use_id`, `subagent_type`, and model metadata.
- **Argument Generation**: Confirmed CLI arguments for `--append-system-prompt`, `--allowedTools`, `--disallowedTools`, and isolated instance scoping via `CLAUDE_CONFIG_DIR`.
- **Windows Command Resolution**: Implemented `resolve_command_shim` inspecting `PATHEXT` to locate binaries and wrap `.cmd` scripts.

### 4.2 Codex App-Server Driver
- **Schema Contracts**: Audited JSON-RPC 2.0 schema defining `initialize`, `thread/start`, `turn/start`, `turn/interrupt`, `account/read`, `model/list`, and `account/rateLimits/read`.
- **Developer Instructions**: Validated native injection of developer instructions on `thread/start`.
- **Security Knobs**: Verified mapping of sandbox policies (`read-only`, `workspace-write`, `danger-full-access`) and approval policies (`never`, `on-write`, `always`).
- **Turn Lifecycle & Approvals**: Replayed full turn lifecycle fixture verifying `turn/approvalRequest` and `turn/approvalResponse` roundtrips with turn token accounting.
- **Rate-Limit Ingestion**: Validated parsing of `account/rateLimits/read` responses, extracting `usedPercent`, `windowDurationMins`, and ISO-8601 reset timestamps.

## 5. S5a Antigravity Integration Detailed Findings

The test harness and fixtures are implemented in `spikes/phase0-antigravity/`.

### 5.1 Managed Bundle Manifest & Hardened Extraction
- **Pinned Verification**: Verified bundle manifest size and SHA-256 validation.
- **Two-Entry Hardened Extraction**: Enforced that the bundle contains exactly two entries (`agy_acp_server` and `localharness_external`).
- **Hardened Rejection**: Proved rejection of size mismatches, hash mismatches, unexpected extra entries, and path traversal attempts (such as `../` prefixes or absolute roots).
- **Resolution Order**: Verified the 3-step resolution order: explicit setting, then managed active release (`active.json` pointer), then system `PATH`.

### 5.2 Isolated Environment & Sibling Temp Directories
- **Replaced Environment**: Built isolated process environment replacing inherited variables, setting `GEMINI_HOME` to a private per-instance profile (`profiles/antigravity/<instance>`), `ANTIGRAVITY_HARNESS_PATH`, `AGY_ACP_FORCE_FILE_STORAGE=1`, `PYTHONUNBUFFERED=1`, and `BROWSER` helper.
- **Sibling Scratch Directory**: Verified `TEMP`/`TMP`/`TMPDIR` points to an app-owned sibling scratch directory rather than a child of `GEMINI_HOME`, avoiding Windows `MAX_PATH` overflow from deep unpack paths.
- **Sanitization**: Verified that inherited API keys and tokens (`GOOGLE_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_APPLICATION_CREDENTIALS`, `AGY_ACP_*`) are strictly scrubbed.

### 5.3 ACP Protocol, Sessions & Security Warnings
- **Handshake Validation**: Validated `initialize` response, requiring `agentInfo.name == "antigravity-acp"`, `protocolVersion == 1`, session loading/resuming, and `oauth-personal` auth method.
- **Client Capabilities**: Constructed `session/new` requests setting `terminal: false`, `fs.readTextFile: true`, and `fs.writeTextFile: true` to route file operations through host approval cards.
- **Model Selection**: Extracted model configurations from `configOptions` for safe switching via `session/set_config_option`, avoiding the unstable `session/set_model` API.
- **Prompt Injection Warnings**: Extracted `_meta["agy.security.warning"]` security alerts on permission requests to surface in user approval dialogs.
- **Interactive Prompts**: Correctly classified tool calls with prefix `interaction_` as native user prompts.
- **Completion Detection**: Detected turn completion from session events rather than process exit codes.
- **Error Mapping**: Mapped ACP error codes `-32000` (AuthRequired) and `-32603` (InternalSessionFailure).

### 5.4 OAuth Sign-in & Remote Redirect Relay
- **Marker Extraction**: Extracted Google OAuth URL from stdout markers and `[BROWSER]` stderr markers.
- **Strict Validation**: Validated `accounts.google.com` origin, loopback redirect targeting `http://127.0.0.1:<port>/`, and presence of state parameter.
- **Callback Verification**: Validated query parameter matching of `code` and expected `state`.
- **Remote Relay**: Demonstrated local loopback listener capturing browser callback and relaying code forward to the loopback server.

### 5.5 Reliability Rules & Resource Constraints
- **Zero-Spawn Health Checks**: Offline health verification inspects on-disk binary resolution and cached configuration without spawning PyInstaller processes (~1 GB unpack avoided).
- **Orphan Sweeper**: Successfully swept orphaned `_MEI*` and `agy_tmp_*` temporary unpack directories from the scratch volume.
- **Concurrency Limiting**: Enforced strict per-environment concurrency cap (maximum 2 active processes).
- **Payload Sanitization**: Capped tool payload text at 64 KB with truncation notice.
- **Field Normalization**: Handled alternate command casing variations (`CommandLine`, `command_line`, `cmd`).

## 6. S3 Remote Bootstrap Detailed Findings

The test harness and fixtures are implemented in `spikes/phase0-remote-bootstrap/`. The tests executed live against Galahad (`10.55.88.48`).

### 6.1 Remote Detection & Exec without PTY
- Verified target identification on Galahad: `uname -sm` reports `Linux x86_64` and resolved `$HOME` to `/home/brandon`.
- Confirmed that commands run cleanly over SSH channel exec without pseudo-terminal allocation (`-T`), avoiding PTY escape character contamination.

### 6.2 Linux musl Cross-Compilation & SHA-256 Integrity
- Verified compilation of static Linux musl binary using local Windows toolchain: `rustc -C linker-flavor=ld.lld -C linker=rust-lld --target x86_64-unknown-linux-musl`.
- Delivered binary to Galahad at `~/.pandamux-spike/versions/<sha256>/pandamux-node-mock` via SSH exec streaming.
- Computed remote SHA-256 via `sha256sum` on Galahad; confirmed exact match with local digest (`36f76227...`).

### 6.3 setsid Daemon Lifecycle & Discovery
- Launched background node daemon using `setsid nohup ~/.pandamux-spike/.../pandamux-node-mock daemon --run-dir ~/.pandamux-spike/run > /dev/null 2>&1 &`.
- Verified daemon created UNIX domain socket `~/.pandamux-spike/run/server.sock` and wrote discovery record `server.json` (pid `2778246`, socket path, start timestamp).

### 6.4 Tunneling Architecture: Proxy vs direct-streamlocal Decision
- Tested exec proxy tunnel: client connects via SSH exec channel running `pandamux-node-mock proxy --socket ~/.pandamux-spike/run/server.sock`.
- Confirmed bidirectional ping/pong roundtrip over the proxy channel (`{"status":"pong","version":"0.1.0"}`).
- Architectural verdict: **Exec proxy is the selected architecture**. OpenSSH `direct-streamlocal` forwarding requires `AllowStreamLocalForwarding yes` and `StreamLocalBindUnlink` in `/etc/ssh/sshd_config`, which is frequently prohibited or restricted on remote bastions, whereas SSH exec proxy uses standard stdio with zero host configuration requirements and zero open TCP ports.

### 6.5 Mid-Turn Disconnect, Daemon Survival, and sinceSeq Replay
- Initiated a 5-step turn on the remote daemon over the proxy tunnel.
- Received initial progress event (`step 1`).
- Abruptly severed the SSH connection mid-turn to simulate a network outage or laptop sleep.
- Verified daemon process (`pid 2778246`) continued executing autonomously on Galahad.
- Waited 3.5 seconds while offline: daemon completed steps 2 through 5 and recorded background heartbeat ticks.
- Re-established SSH proxy connection and subscribed with `since_seq: 1`.
- Verified clean recovery and replay of all missed progress steps (steps 2 through 5) plus background timer ticks that fired while disconnected.
- Cleanly terminated daemon with `shutdown` command and removed temporary remote directory.

## 7. S4 Packaging Detailed Findings

The test harness, configuration fixtures, and verification pipeline are implemented in `spikes/phase0-packaging/`.

### 7.1 Multi-Binary Packaging Specification
- **Windows Executables**: Configured cargo-packager to package three distinct binaries:
  1. `pandamux.exe` (main desktop executable with start menu shortcut)
  2. `pandamux-server.exe` (headless hub and local node daemon)
  3. `pandamux-cli.exe` (command line interface and scripting tool)
- **Linux Node Binaries as Bundled Resources**:
  - `resources/server/x86_64-unknown-linux-musl/pandamux-server`
  - `resources/server/x86_64-unknown-linux-musl/pandamux-cli`
  - `resources/server/aarch64-unknown-linux-musl/pandamux-server`
  - `resources/server/aarch64-unknown-linux-musl/pandamux-cli`
  These static musl binaries land under `$INSTDIR\resources\server\...`, enabling zero-rebuild remote bootstrap on Linux SSH targets.
- **Application Resources**: Bundled themes, sounds, and icons into `$INSTDIR\resources\...`.

### 7.2 Code Signing Integration & Signature Preservation
- **Rebuild Prevention**: Verified that `before-packaging-command` is strictly omitted from `[package.metadata.packager]`. This guarantees cargo-packager bundles pre-built binaries directly without triggering recompilation.
- **Authenticode Signature Integrity**:
  - Implemented PE Authenticode verification inspecting the Certificate Table entry (`IMAGE_DIRECTORY_ENTRY_SECURITY`) in the PE Optional Header.
  - Verified that pre-applied digital signatures (Azure Trusted Signing) survive NSIS staging and packaging bit-for-bit.
  - Verified extraction: all three Windows executables (`pandamux.exe`, `pandamux-server.exe`, `pandamux-cli.exe`) extracted from the installer payload match their signed pre-pack SHA-256 digests and certificate tables exactly.
- **Outer Installer Signing**: The generated installer (`PandaMUX-Setup-<version>.exe`) receives its own Authenticode signature after generation, completing the two-stage signing pipeline.

### 7.3 NSIS Configuration (currentUser Mode)
- **Execution Level**: Configured `installer-mode = "currentUser"` (`RequestExecutionLevel user`). Installs cleanly to `$LOCALAPPDATA\Programs\PandaMUX` without requiring UAC administrator elevation.
- **Data Protection**: Configured `appdata-paths = ["$LOCALAPPDATA/pandamux"]`, ensuring uninstallation removes program files while safeguarding user configuration and SQLite database history unless explicitly purged.
- **Shortcuts & Registry**: Creates `$SMPROGRAMS\PandaMUX\PandaMUX.lnk` and registers uninstallation entries in `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\PandaMUX`.

## 8. S5b Best-effort ACP Providers Detailed Findings

The test harness and fixture contracts are implemented in `spikes/phase0-acp-providers/`.

### 8.1 Cursor Agent ACP (cursor-agent acp)
- **Transport**: JSON-RPC 2.0 NDJSON over stdio (`protocolVersion: 1`).
- **Auth Detection**: ACP `initialize` response confirms `agentInfo.name == "cursor-agent"` and advertises auth methods including `cursor_login`.
- **Instruction Delivery**: The baseline ACP 1.x specification lacks top-level system prompt or developer instruction parameters in `session/new`. The driver injects agent role instructions and knowledge as a structured preamble in the first user turn (`=== PANDAMUX AGENT ROLE INSTRUCTIONS ===`), re-injected upon context compaction events.
- **Permission Granularity**: Coarse permissions via `session/request_permission`. Approval options (`allow_once`, `allow_always`, `reject_once`) map cleanly to PandaMUX access modes:
  - ReadOnly: automatically denies file modifications and shell execution tools.
  - AutoEdit: grants `allow_always` for file writes and reads; prompts for commands.
  - FullAccess: grants `allow_always` for all tools.

### 8.2 Grok CLI ACP (grok acp)
- **Transport**: JSON-RPC 2.0 NDJSON over stdio (`protocolVersion: 1`).
- **Auth Detection**: `initialize` response confirms `agentInfo.name == "grok-acp"` and advertises `api_key` and `oauth_xai`.
- **Instruction Delivery**: Fallback preamble injection in first user prompt turn.
- **Usage and Rate Limits**: Parsed Grok account usage and limits payload: extracts `account_tier`, `monthly_spend_usd`, `token_budget` utilization, and remaining requests per minute.

### 8.3 OpenCode Dual-Mode Evaluation (acp vs serve)
Evaluated both OpenCode entry points:
- **Mode 1: `opencode acp` (stdio)**:
  - Protocol: ACP JSON-RPC 2.0 over stdio.
  - Instructions: Fallback preamble.
  - Tool Granularity: Coarse ACP `request_permission` approvals.
- **Mode 2: `opencode serve` (HTTP REST + SSE)**:
  - Protocol: HTTP REST API with Server-Sent Events (`/event` SSE stream) on `127.0.0.1`, protected by `OPENCODE_SERVER_PASSWORD` bearer auth.
  - Instructions: **Native** support. Accepts `system_prompt` on `POST /session`, keeping agent instructions out of the user chat transcript.
  - Tool Granularity: **Per-tool** policy enforcement (`allowed_tools` and `disallowed_tools` configured at session creation).
- **Architectural Verdict**: Support `opencode acp` via the shared ACP driver for minimal footprint; utilize `opencode serve` for advanced workflows requiring native developer instructions and fine-grained tool filtering.

### 8.4 Provider Capability Matrix

| Provider | Transport | Instruction Injection | Tool Granularity | Driver Architecture |
| :--- | :--- | :--- | :--- | :--- |
| **cursor-agent** | stdio (ACP NDJSON) | Preamble Fallback | Coarse (`request_permission`) | Shared ACP Driver |
| **grok-acp** | stdio (ACP NDJSON) | Preamble Fallback | Coarse (`request_permission`) | Shared ACP Driver |
| **opencode-acp** | stdio (ACP NDJSON) | Preamble Fallback | Coarse (`request_permission`) | Shared ACP Driver |
| **opencode-serve** | HTTP REST + SSE | Native (`POST /session`) | Per-Tool (`tool_policy`) | Dedicated HTTP/SSE Profile |

## 9. Phase 0 Completion & Transition to Phase 1

All seven spikes defined in `tasks/plan-rewrite.md` have completed and passed:
1. **S1 (GPUI Chat)**: PASS (Virtualization, 50 tok/s stream, GPU pipeline)
2. **S2 (Codex & Claude Drivers)**: PASS (Zero-session auth, stream-json, sub-agents, instructions)
3. **S3 (Remote Bootstrap)**: PASS (musl binary, SHA-256 match, setsid daemon, SSH proxy, mid-turn replay)
4. **S4 (Packaging)**: PASS (cargo-packager NSIS, multi-binary, Authenticode survival, Linux musl resources)
5. **S5a (Antigravity Integration)**: PASS (Managed bundle, isolated GEMINI_HOME, OAuth relay, ACP permissions, reliability rules)
6. **S5b (Best-effort ACP Providers)**: PASS (Cursor, Grok, OpenCode contracts, capability matrix)
7. **S6 (Pins and Licenses)**: PASS (Apache-2.0 confirmed across all gpui-pre and gpui-kit crates, zero GPL contamination)

With zero gate failures, Phase 0 is complete. The project is fully de-risked and approved to proceed into Phase 1 (Foundation).
