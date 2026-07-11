<h1 align="center">PandaMUX</h1>
<p align="center">A visibility layer for Claude Code on Windows: see what your AI agent does in real-time</p>

<p align="center">
  Native, GPU-rendered Windows terminal multiplexer for AI agents. Built in Rust
  (Iced + alacritty_terminal + portable-pty + russh). Named-pipe protocol lineage
  traces to <a href="https://github.com/manaflow-ai/cmux">cmux</a>.
</p>

<p align="center">
  <a href="https://github.com/BoardPandas/Pandamux"><img src="https://img.shields.io/badge/platform-Windows-0078D4?logo=windows" alt="Windows" /></a>
  <a href="https://github.com/BoardPandas/Pandamux/releases"><img src="https://img.shields.io/github/v/release/BoardPandas/Pandamux?label=release&color=555" alt="Release" /></a>
  <a href="https://github.com/BoardPandas/Pandamux/blob/master/LICENSE"><img src="https://img.shields.io/badge/license-MIT-555" alt="License" /></a>
</p>

## What it is

PandaMUX is a Windows terminal multiplexer built for running many Claude Code (and other CLI) agents in parallel, each in its own visible pane. It passively observes Claude Code without changing how it works: auto-configured hooks report agent and tool activity to the sidebar, so you can see at a glance which sessions are working, done, or waiting for you.

It is a fully native Rust application (no web view). The terminal grid is GPU-rendered for smooth output at any speed, and the whole app is coordinated through a single named-pipe API that the CLI, shell integration, and the bundled orchestrator plugin all speak.

> **History**: this repo began as an Electron/TypeScript prototype and was rewritten to a native Rust workspace. The prototype has been removed. The in-app CDP browser pane was intentionally dropped; agents use Claude Code's own browser tooling instead. See [`tasks/plan-repo.md`](tasks/plan-repo.md) for the full plan and phase history.

## Features

- **Passive Claude Code integration** — auto-injects a small block into `~/.claude/CLAUDE.md`, installs the pandamux-orchestrator plugin, and reads agent/tool activity into the sidebar. No API keys; runs through your existing Claude Code session.
- **Splits, tabs, workspaces** — split any pane right or down, zoom to full screen, keep multiple keep-alive tabs per pane (PTY sessions stay live when switching), and organize panes into workspaces. Drag-and-drop a tab to split.
- **Sessions panel** — every shell context indexed across workspaces, grouped by project / type / host; select one to focus its pane.
- **SSH remote surfaces** — run Claude Code on remote Linux hosts over SSH (russh). Durable via remote `tmux` with reconnect-on-disconnect. Host profiles + `~/.ssh/config` import; agent/key/password auth.
- **Copy/paste over SSH (OSC 52)** — copy from a remote session to your local clipboard and paste back, with bracketed-paste handling.
- **Image paste** — copy a screenshot and press `Ctrl+V`: locally the temp path is injected; on a remote surface the image is uploaded via SFTP and the remote path is injected, so Claude Code reads it directly.
- **Notifications** — panes flash and the bell lights up when agents finish or need attention (OSC 9/99/777, `pandamux notify`, idle detection); a panel lists everything with click-to-jump.
- **Terminal themes** — bundled themes plus import from Windows Terminal `settings.json` or Ghostty config; per-surface color schemes.
- **pandamux-orchestrator plugin** — decomposes a task into parallel agents, each in its own visible pane, coordinated in dependency-aware waves with automated review. Activate with `/pandamux:orchestrate`.
- **Scriptable** — named-pipe server (`\\.\pipe\pandamux`) with a JSON-RPC API: create workspaces, split panes, send keystrokes, read terminal content, drive SSH/clipboard, and spawn sub-agent terminals.
- **In-app updates** — checks GitHub Releases and offers to install a newer signed build (past a quarantine window). Find-in-terminal, clickable links, session restore, and multi-window are all built in.

## Install

### Download (recommended)

Download the latest `PandaMUX-Setup-<version>.exe` from [GitHub Releases](https://github.com/BoardPandas/Pandamux/releases/latest) and run it. The installer and app are Authenticode-signed (Azure Trusted Signing), and the app updates itself from within.

### From source

Requires the Rust stable toolchain (rustup) and the MSVC build tools.

```bash
git clone https://github.com/BoardPandas/Pandamux.git
cd Pandamux
cargo run -p pandamux-app --features iced-runtime -- --iced-shell
```

See [`CLAUDE.md`](CLAUDE.md) for the full build, test, and release documentation.

## Why PandaMUX?

Running many Claude Code sessions in parallel on Windows is painful. Windows Terminal has tabs but no notification system, so you check each tab manually to see if an agent finished or is waiting. tmux works in WSL but loses Windows integration. PandaMUX is a visibility layer for AI coding agents: it does not replace Claude Code or change how it works; it observes and shows you what is happening. The sidebar shows each agent's git branch, open ports, and whether it needs attention, reported over the named pipe by shell-integration scripts in real time.

## pandamux-orchestrator

A bundled Claude Code plugin for parallel multi-agent orchestration. Activate with `/pandamux:orchestrate` in any Claude Code session. It analyzes the codebase, decomposes the task into independent units, assigns each to an agent in its own pane, runs them in dependency-aware waves, and has a reviewer agent inspect the combined output and trigger auto-fixes. Auto-installed into the Claude plugin cache on startup; also works without PandaMUX by falling back to native subagents. Bundled under `resources/pandamux-orchestrator/`.

## Shell Integration

PandaMUX injects integration scripts into your shells (PowerShell, CMD, Bash/Zsh in WSL) that report CWD, git branch/dirty state, and shell state (working/done/interrupted) over the named pipe. Per-session cwd tracking also uses OSC 9;9 / OSC 7.

Environment variables available in all shells:

| Variable | Description |
|----------|-------------|
| `PANDAMUX` | Always `1` inside PandaMUX |
| `PANDAMUX_CLI` | Path to the pandamux CLI |
| `PANDAMUX_SURFACE_ID` | Current surface (tab) ID |
| `PANDAMUX_PIPE` | Named pipe path (`\\.\pipe\pandamux`) |
| `PANDAMUX_AGENT_ID` | Agent ID (set for orchestrator-spawned panes) |

## CLI

The `pandamux` CLI communicates with the running app over the named pipe.

```bash
pandamux ping                          # Check if pandamux is running
pandamux notify "Build complete"       # Send a notification
pandamux new-workspace --title "API"   # Create a workspace
pandamux list-workspaces               # List all workspaces
pandamux split --right                 # Split focused pane
pandamux send "cargo test"             # Send text to terminal
pandamux send-key Enter --ctrl         # Send keystroke
pandamux read-screen --lines 50        # Read terminal content

# SSH remote surfaces
pandamux ssh connect <host>            # Open a remote surface (tmux-durable)
pandamux ssh list                      # List remote sessions
pandamux ssh import                    # Import hosts from ~/.ssh/config

# Clipboard (OSC 52) + image paste
pandamux clipboard copy "text"         # Set the OS clipboard
pandamux paste                         # Paste into the focused surface
pandamux paste-image                   # Paste/upload a clipboard image

# Agents
pandamux agent spawn --cmd "claude --resume abc" --label "Research"
pandamux agent spawn-batch --json '[{"cmd":"claude","label":"Agent 1"},{"cmd":"claude","label":"Agent 2"}]'
pandamux agent list                    # List all agents
pandamux agent status <agent-id>       # Check agent status
pandamux agent kill <agent-id>         # Kill an agent

pandamux tree                          # Workspace / pane / surface hierarchy
```

## Socket API

Connect to `\\.\pipe\pandamux` for programmatic control. Two protocols:

**V1** (text, used by shell integration):
```
report_pwd <surface_id> <path>
report_git_branch <surface_id> <branch> [dirty]
report_shell_state <surface_id> idle|running|interrupted
notify <surface_id> <text>
ping
```

**V2** (JSON-RPC, used by the CLI and automation):
```json
{"method": "workspace.create", "params": {"title": "Agent 1"}}
{"method": "surface.send_text", "params": {"id": "surf-...", "text": "cargo test\n"}}
{"method": "surface.read_text", "params": {"id": "surf-...", "lines": 50}}
{"method": "ssh.connect", "params": {"host": "galahad"}}
{"method": "clipboard.copy", "params": {"text": "hello"}}
{"method": "agent.spawn", "params": {"cmd": "claude --resume abc", "label": "Research"}}
{"method": "agent.spawn_batch", "params": {"agents": [], "strategy": "distribute"}}
{"method": "system.tree", "params": {}}
```

`system.capabilities` reports `browser: false`; there is no `pandamux browser` command (use Claude Code's own browser tooling).

## Architecture

Native Rust workspace. The backend owns all canonical state; the Iced UI is a read-projection that submits intents. The named-pipe server (CLI/agents/orchestrator) and the UI both submit the same intents to the same dispatcher, so CLI-driven and UI-driven mutations are identical at the state layer.

```
crates/
  pandamux-core/   Domain types, split tree, session/agent/notification/ssh models. Zero UI deps.
  pandamux-term/   Terminal engine: alacritty_terminal grid, portable-pty, russh (remote PTY + SFTP),
                   OSC 52, search/serialize/link detection, shell lifecycle.
  pandamux-ui/     Iced app: GPU terminal viewport, panes/splits/tabs, chrome, overlays, theming.
  pandamux-app/    pandamux.exe: tokio runtime, canonical state, pipe server, pollers, persistence,
                   updater, Claude-context integration.
  pandamux-cli/    pandamux-cli.exe: the `pandamux` CLI (pipe client).
resources/         Runtime assets: themes, sounds, shell-integration, pandamux-orchestrator.
```

Full developer docs, including the release/signing pipeline, are in [`CLAUDE.md`](CLAUDE.md).

## Lineage

PandaMUX is an independent Windows project whose named-pipe protocol and design philosophy trace to [cmux](https://github.com/manaflow-ai/cmux), the macOS terminal for multitasking. It is wire-compatible with cmux's socket protocol but does not reuse cmux's source code.

## Contributing

- [GitHub Issues](https://github.com/BoardPandas/Pandamux/issues): bug reports and feature requests
- [GitHub Discussions](https://github.com/BoardPandas/Pandamux/discussions): questions and ideas

## License

PandaMUX is open source under the [MIT License](LICENSE). Its socket protocol and design are inspired by cmux; it does not incorporate cmux's source code.
