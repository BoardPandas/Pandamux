<!-- PAGE_ID: pandamux_07_cli-reference -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [pandamux.ts:1-26](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L1-L26)
- [pandamux.ts:28-68](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L28-L68)
- [pandamux.ts:70-107](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L70-L107)
- [pandamux.ts:109-143](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L109-L143)
- [pandamux.ts:145-224](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L145-L224)
- [pandamux.ts:226-300](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L226-L300)
- [pandamux.ts:305-397](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L305-L397)
- [pandamux.ts:399-453](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L399-L453)
- [pandamux-hook.ts:1-83](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux-hook.ts#L1-L83)

</details>

# CLI Reference

> **Related Pages**: [Named Pipe Control Plane](../features/NAMED_PIPE_IPC.md), [Agent Orchestration](../features/AGENT_ORCHESTRATION.md)

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_overview -->
## Overview and Invocation

The `pandamux` CLI is a thin Node script that never talks to the Electron app directly; every command opens a client connection to the Windows named pipe `\\.\pipe\pandamux` and either writes plain text (V1) or a JSON-RPC request (V2) ([pandamux.ts:10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L10)). There are two separate entry-point scripts in `src/cli/`, both compiled to `dist/cli/` and shipped in `resources/cli/`.

| Entry point | Invoked as | Purpose | Source |
|---|---|---|---|
| `pandamux.ts` | `pandamux <command> [options]` | Full CLI: workspace/pane/surface/agent/browser control | ([pandamux.ts:399-425](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L399-L425)) |
| `pandamux-hook.ts` | `node pandamux-hook.js <tool-name>` or `--event <Event>` | Lightweight forwarder wired into Claude Code's `PostToolUse` / `Notification` / `Stop` hooks; reads the hook JSON payload from stdin and fires a single `hook.event` V2 request | ([pandamux-hook.ts:1-14](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux-hook.ts#L1-L14)) |

The pipe path respects `PANDAMUX_PIPE` when set, so a CLI launched inside a pane spawned by one pandamux instance (`PANDAMUX_INSTANCE`) always talks back to that same instance ([pandamux.ts:8-10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L8-L10)). V2 requests carry an auth token resolved from `PANDAMUX_PIPE_TOKEN`, falling back to a per-instance token file under `%APPDATA%\pandamux[-instance]\pipe-token` ([pandamux.ts:12-26](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L12-L26)).

| Transport | Wire format | Timeout | Used by | Source |
|---|---|---|---|---|
| V1 (`sendV1`) | Raw newline-terminated text, response read until socket end or timeout | 5s | `ping`, `notify` | ([pandamux.ts:28-39](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L28-L39)) |
| V2 (`sendV2`) | JSON `{method, params, id, token}` request, JSON response parsed on the first newline; `response.error` rejects the promise | 5s | Everything else | ([pandamux.ts:41-68](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L41-L68)) |

`sendV2` also auto-attaches the caller's own `PANDAMUX_SURFACE_ID` as `caller` on any `browser.*` method so concurrent agents each get routed to their own browser pane instead of clobbering a single shared one (issue #62) ([pandamux.ts:42-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L42-L47)).

Dispatch is a flat lookup table (`COMMANDS`), not a switch statement: `argv[0]` is the key, and an unknown command prints usage and exits 1 ([pandamux.ts:305-413](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L305-L413)). Handler errors are caught centrally in `main()`: `ENOENT`/`ECONNREFUSED` is reported as "pandamux is not running", anything else prints the raw error message and exits 1 ([pandamux.ts:415-424](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L415-L424)).

```bash
pandamux ping
pandamux identify
PANDAMUX_PIPE='\\.\pipe\pandamux-myinstance' pandamux capabilities
```

Sources: [pandamux.ts:1-68](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L1-L68), [pandamux.ts:399-453](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L399-L453)
<!-- END:AUTOGEN pandamux_07_cli-reference_overview -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_system -->
## System Commands

System commands query pandamux's identity, capabilities, window list, the full pane/surface tree, and the user's `config.toml`; none require a workspace, pane, or surface argument.

| Command | Arguments | Description | Source |
|---|---|---|---|
| `ping` | none | Sends the raw V1 text `ping` and prints the reply verbatim (does not go through `sendV2`) ([pandamux.ts:307](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L307)) |
| `identify` | none | `system.identify` V2 request ([pandamux.ts:308](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L308)) |
| `capabilities` | none | `system.capabilities` V2 request ([pandamux.ts:309](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L309)) |
| `list-windows` | none | `window.list` ([pandamux.ts:310](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L310)) |
| `focus-window` | `<id>` | `window.focus` with the given window id ([pandamux.ts:311](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L311)) |
| `tree` | none | `system.tree` -- the full workspace/pane/surface tree ([pandamux.ts:354](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L354)) |
| `config show` \| `config get` | none | `config.get`, the merged `~/.pandamux/config.toml` state ([pandamux.ts:166-167](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L166-L167)) |
| `config reload` | none | `config.reload`, re-reads `config.toml` from disk ([pandamux.ts:168-169](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L168-L169)) |
| `config path` | none | Prints `%USERPROFILE%\.pandamux\config.toml` locally, with no pipe round trip ([pandamux.ts:170-172](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L170-L172)) |
| `reload-config` | none | Shorthand for `config reload` ([pandamux.ts:339](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L339)) |
| `list-themes` \| `themes` | none | `theme.list` (both names alias the same handler) ([pandamux.ts:335-336](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L335-L336)) |

```bash
pandamux identify
pandamux config path
pandamux tree
```

Sources: [pandamux.ts:164-176](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L164-L176), [pandamux.ts:305-397](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L305-L397)
<!-- END:AUTOGEN pandamux_07_cli-reference_system -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_workspace -->
## Workspace Commands

A workspace is a top-level window/session container. All workspace commands map 1:1 onto `workspace.*` V2 methods.

| Command | Arguments | Description | Source |
|---|---|---|---|
| `new-workspace` | `--title T` `--shell S` `--cwd D` | `workspace.create`; any of title/shell/cwd may be omitted ([pandamux.ts:226-234](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L226-L234)) |
| `close-workspace` | `<id>` | `workspace.close` ([pandamux.ts:315](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L315)) |
| `select-workspace` | `<id>` | `workspace.select` ([pandamux.ts:316](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L316)) |
| `rename-workspace` | `<id> <title>` | `workspace.rename` ([pandamux.ts:317](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L317)) |
| `list-workspaces` | none | `workspace.list` ([pandamux.ts:318](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L318)) |

```bash
pandamux new-workspace --title "Build" --shell pwsh --cwd D:\Dev\Repos\Pandamux
pandamux rename-workspace ws-1234 "Renamed"
```

Sources: [pandamux.ts:226-234](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L226-L234), [pandamux.ts:313-318](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L313-L318)
<!-- END:AUTOGEN pandamux_07_cli-reference_workspace -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_surface-pane -->
## Surface and Pane Commands

Surfaces are the tabs inside a pane (terminal, browser, or markdown); panes are the split-tree leaves that host them. `split` and `pane new`/`pane split` are two spellings of the same operation, kept for backwards compatibility with issue #4's example usage ([pandamux.ts:437](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L437)).

| Command | Arguments | Description | Source |
|---|---|---|---|
| `new-surface` | `--type T` `--color-scheme NAME` | `surface.create`; type defaults to `terminal` ([pandamux.ts:321-325](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L321-L325)) |
| `close-surface` | `<id>` | `surface.close` ([pandamux.ts:326](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L326)) |
| `focus-surface` | `<id>` | `surface.focus` ([pandamux.ts:327](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L327)) |
| `list-surfaces` | `--pane P` | `surface.list` scoped to a pane id ([pandamux.ts:328](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L328)) |
| `set-color-scheme` | `[surfaceId] <scheme>` | `surface.set_color_scheme`; if only one positional arg is given it is treated as the scheme name and applied to `PANDAMUX_SURFACE_ID` ([pandamux.ts:236-249](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L236-L249)) |
| `clear-color-scheme` | `[surfaceId]` | `surface.set_color_scheme` with `colorScheme: null` ([pandamux.ts:330-334](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L330-L334)) |
| `split` | `--down` `--type T` `--color-scheme NAME` | `pane.split`; `--down` selects direction `down`, otherwise `right` ([pandamux.ts:343-348](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L343-L348)) |
| `pane new` \| `pane split` | `--down` `--type T` `--color-scheme NAME` | Verb form of `split`, same `pane.split` request ([pandamux.ts:147-152](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L147-L152)) |
| `pane close` \| `pane focus` \| `pane list` | `<id>` / `--workspace W` | `pane.close` / `pane.focus` / `pane.list` verb forms ([pandamux.ts:153-158](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L153-L158)) |
| `close-pane` | `<id>` | `pane.close` ([pandamux.ts:350](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L350)) |
| `focus-pane` | `<id>` | `pane.focus` ([pandamux.ts:351](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L351)) |
| `zoom-pane` | `<id>` | `pane.zoom` ([pandamux.ts:352](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L352)) |
| `list-panes` | `--workspace W` | `pane.list` ([pandamux.ts:353](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L353)) |
| `layout grid` | `--count N` `--type T` `--anchor-surface ID` `--anchor-pane ID` `--workspace W` | `layout.grid`; `--count` is required and must be >= 1; if no anchor is given it falls back to `PANDAMUX_SURFACE_ID` so the command works from inside a pane ([pandamux.ts:178-194](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L178-L194)) |
| `markdown <file>` | file path | Creates a new markdown surface (`surface.create`) then calls `markdown.load_file` with the path resolved against the caller's cwd ([pandamux.ts:212-219](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L212-L219)) |
| `markdown set` | `<id> --content TEXT` \| `<id> --file PATH` | `markdown.set_content` (inline text) or `markdown.load_file` (resolved file path) on an existing surface id ([pandamux.ts:198-211](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L198-L211)) |

```bash
pandamux split --down --type markdown
pandamux layout grid --count 4 --type terminal
pandamux markdown ./tasks/plan-repo.md
```

Sources: [pandamux.ts:145-224](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L145-L224), [pandamux.ts:236-249](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L236-L249), [pandamux.ts:321-353](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L321-L353)
<!-- END:AUTOGEN pandamux_07_cli-reference_surface-pane -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_terminal-io -->
## Terminal I/O Commands

These commands write into, or read from, a terminal surface's PTY. All accept `--surface <id>` and fall back to `PANDAMUX_SURFACE_ID` (the env var pandamux injects into every shell it spawns) when omitted.

| Command | Arguments | Description | Source |
|---|---|---|---|
| `send` | `[--surface ID] <text...>` | `surface.send_text`; `--surface` is stripped from the free-form text args before the remainder is joined and sent ([pandamux.ts:251-258](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L251-L258)) |
| `send-key` | `<key> [--ctrl] [--shift] [--alt] [--surface ID]` | `surface.send_key` with a `modifiers` array built from the flags present ([pandamux.ts:260-270](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L260-L270)) |
| `read-screen` | `--lines N` | `surface.read_text`; defaults to 50 lines if `--lines` is omitted ([pandamux.ts:362-365](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L362-L365)) |
| `trigger-flash` | `<id>` | `surface.trigger_flash`, flashes a pane's border to draw attention ([pandamux.ts:366](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L366)) |

```bash
pandamux send "pnpm test"
pandamux send-key Enter
pandamux read-screen --lines 100
```

Sources: [pandamux.ts:251-270](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L251-L270), [pandamux.ts:360-366](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L360-L366)
<!-- END:AUTOGEN pandamux_07_cli-reference_terminal-io -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_browser -->
## Browser Commands

`pandamux browser <sub>` drives the CDP-controlled browser pane. Every subcommand is a small lookup in `BROWSER_CMDS`, and unknown subcommands exit 1 with an error ([pandamux.ts:103-107](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L103-L107)).

| Command | Arguments | Description | Source |
|---|---|---|---|
| `browser open` | `<url>` | `browser.navigate` ([pandamux.ts:89](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L89)) |
| `browser snapshot` | none | `browser.snapshot`, returns the accessibility tree with `@eN` refs ([pandamux.ts:90](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L90)) |
| `browser click` | `<ref>` | `browser.click` ([pandamux.ts:91](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L91)) |
| `browser type` | `<ref> <text...>` | `browser.type` ([pandamux.ts:92](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L92)) |
| `browser fill` | `<ref> <value...>` | `browser.fill` ([pandamux.ts:93](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L93)) |
| `browser screenshot` | `[--full]` | `browser.screenshot`; `--full` sets `fullPage: true` ([pandamux.ts:94](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L94)) |
| `browser get-text` | `<ref>` | `browser.get_text` ([pandamux.ts:95](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L95)) |
| `browser eval` | `<js...>` | `browser.eval` ([pandamux.ts:96](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L96)) |
| `browser wait` | `<ref> [timeoutMs]` | `browser.wait`; timeout parses to `undefined` if not a number ([pandamux.ts:97](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L97)) |
| `browser back` \| `browser forward` \| `browser reload` | none | `browser.back` / `browser.forward` / `browser.reload` ([pandamux.ts:98-100](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L98-L100)) |

```bash
pandamux browser open https://example.com
pandamux browser snapshot
pandamux browser click @e3
```

Sources: [pandamux.ts:86-107](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L86-L107)
<!-- END:AUTOGEN pandamux_07_cli-reference_browser -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_agent -->
## Agent Commands

`pandamux agent <sub>` spawns and manages agent PTYs across panes. `hook` and `agent-activity` are grouped here too since both feed the same agent-activity/sidebar tracking pipeline that agent orchestration depends on.

| Command | Arguments | Description | Source |
|---|---|---|---|
| `agent spawn` | `--cmd C` `--label L` `--cwd D` `--pane P` `--workspace W` | `agent.spawn`; `--cmd` is required (exits 1 if missing), `--label` defaults to the first whitespace-delimited token of `--cmd` ([pandamux.ts:109-121](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L109-L121)) |
| `agent spawn-batch` | `--json '[...]'` `--strategy distribute\|stack\|split` | `agent.spawn_batch`; JSON array is parsed with `JSON.parse`, strategy defaults to `distribute` ([pandamux.ts:123-129](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L123-L129)) |
| `agent status` | `<id>` | `agent.status` ([pandamux.ts:134](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L134)) |
| `agent list` | `--workspace W` | `agent.list` ([pandamux.ts:135](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L135)) |
| `agent kill` | `<id>` | `agent.kill` ([pandamux.ts:136](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L136)) |
| `hook` | `--event E` `--tool T` `--agent A` | `hook.event`; every present flag is copied straight into the params object ([pandamux.ts:281-289](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L281-L289)) |
| `agent-activity` | `--surface ID` `--tool T` `--skill S` `--done` \| `--active` | `agent.activity`; surface id required (falls back to `PANDAMUX_SURFACE_ID`, else exits 1), `--done`/`--active` set the boolean `done` flag ([pandamux.ts:291-300](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L291-L300)) |

The standalone `pandamux-hook.js` script is a separate code path used directly in Claude Code hook configuration (not through the `pandamux hook` argv dispatch above): it reads the hook JSON payload from stdin, extracts `tool_input.file_path` and `message`, and fires the same `hook.event` V2 method, silently exiting if pandamux is not running ([pandamux-hook.ts:34-70](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux-hook.ts#L34-L70)).

```bash
pandamux agent spawn --cmd "npm run build" --label build
pandamux agent spawn-batch --json '[{"cmd":"npm test"},{"cmd":"npm run lint"}]' --strategy distribute
```

Sources: [pandamux.ts:109-143](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L109-L143), [pandamux.ts:281-300](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L281-L300), [pandamux-hook.ts:1-83](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux-hook.ts#L1-L83)
<!-- END:AUTOGEN pandamux_07_cli-reference_agent -->

---

<!-- BEGIN:AUTOGEN pandamux_07_cli-reference_notify -->
## Notification and Sidebar Commands

These commands post desktop notifications, drive the sidebar status/progress widgets, and trigger the diff panel refresh.

| Command | Arguments | Description | Source |
|---|---|---|---|
| `notify` | `<text...>` `[--title T]` `[--body B]` | Sends a raw **V1** text command `notify <surfaceId> <text>` (not V2); text is assembled by filtering the `--title`/`--body` flag pairs out of argv, falling back to `--body`'s value ([pandamux.ts:272-279](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L272-L279)) |
| `list-notifications` | none | `notification.list` ([pandamux.ts:379](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L379)) |
| `clear-notifications` | `<id>` | `notification.clear` ([pandamux.ts:380](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L380)) |
| `set-status` | `<key> <value>` | `sidebar.set_status` ([pandamux.ts:383](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L383)) |
| `set-progress` | `<value> [--label L]` | `sidebar.set_progress`; value parsed with `parseFloat` ([pandamux.ts:384-387](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L384-L387)) |
| `log` | `<level> <message...>` | `sidebar.log` ([pandamux.ts:388](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L388)) |
| `sidebar-state` | none | `sidebar.get_state` ([pandamux.ts:389](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L389)) |
| `diff` | `[--file PATH]` | `diff.refresh`; file defaults to an empty string when omitted ([pandamux.ts:391-394](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L391-L394)) |

```bash
pandamux set-progress 0.5 --label "Building"
pandamux notify "Build complete" --title "pandamux"
```

Sources: [pandamux.ts:272-300](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L272-L300), [pandamux.ts:378-394](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/cli/pandamux.ts#L378-L394)
<!-- END:AUTOGEN pandamux_07_cli-reference_notify -->

---
