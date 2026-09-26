# PandaMUX Phase 0 (Spikes) Evaluation Report

Status: In Progress (S1, S2, and S6 complete; S3 through S5 queued)
Date: 2026-09-26

## 1. Executive Summary

Phase 0 tests architectural risks and confirms assumptions before starting Phase 1 implementation. Three critical spikes have completed successfully:

1. **S1 (GPUI Chat Spike)**: Passed. Verifies that GPUI 0.3.6 and gpui-kit 0.6.6 provide high-performance markdown streaming, virtualized 2,000-message scrolling, multi-line composition with attachment chips, collapsible tool calls, and inline diffs on Windows MSVC.
2. **S2 (Codex and Claude Drivers)**: Passed. Verifies zero-session auth detection, stream-json protocol with permission prompt tools, sub-agent tree parsing, Codex JSON-RPC schema contracts, developer instructions injection, and rate-limit parsing.
3. **S6 (Pins and License Audit)**: Passed. Verifies that all gpui-pre and gpui-kit crates are Apache-2.0, with zero GPL contamination in the desktop graph, satisfying the license gate for zed#55470.

| Spike | Title | Gate Status | Verdict | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **S1** | GPUI Chat Spike | Gating | **PASS** | 2,000 items in 1.09ms, ~50 tok/s stream, Direct3D rendering verified |
| **S2** | Codex and Claude Drivers | Gating | **PASS** | Zero-session auth probe, stream-json, sub-agents, developer instructions |
| **S3** | Remote Bootstrap | Non-gating | Queued | SSH exec, daemon lifecycle, reconnect |
| **S4** | Packaging | Non-gating | Queued | NSIS multi-binary installer |
| **S5a** | Antigravity Integration | Gating | Queued | Managed bundle, OAuth relay, permissions |
| **S5b** | Best-effort ACP | Non-gating | Queued | Cursor, Grok, OpenCode ACP probes |
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

## 5. Next Phase 0 Milestones

1. **S3**: Validate remote bootstrap against Galahad over SSH exec and SFTP.
2. **S4**: Validate cargo-packager NSIS installer generation on Windows.
3. **S5a**: Validate Antigravity managed install, OAuth sign-in relay, and ACP session flow.
4. **S5b**: Best-effort ACP probes for Cursor, Grok, and OpenCode.
