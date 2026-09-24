# wmux Visibility Layer — Pure Observer Architecture

> **Historical design record:** This page captures earlier design or implementation work and is not current guidance. Verify behavior against the [documentation index](../../README.md), the [developer guide](../../../CLAUDE.md), and the current Rust source.

> wmux becomes a passive observer of Claude Code. No instructions, no CLI overrides, no behavior changes. Claude Code works natively; wmux watches and displays.

## Problem

wmux currently tries to control Claude Code via CLI instructions injected into `~/.claude/CLAUDE.md`. This approach:
- Forces Claude Code to use non-native tools (`wmux browser open`, `wmux agent spawn`)
- Conflicts with Claude Code's built-in Agent tool, WebSearch, Firecrawl, Playwright, etc.
- Creates a parallel tool system that users don't want
- Requires an Anthropic API key for the now-removed browser agent

The user's vision: wmux is a **visibility layer** — it shows what Claude Code does, it doesn't change what Claude Code does.

## Design

Three systems, all passive:

```
Claude Code (unchanged)
    |
    |-- Uses chrome-devtools-mcp (native browser tools)
    |       |
    |       +---> CDP WebSocket proxy (localhost:9222)
    |               |
    |               +---> wmux browser panel (user sees everything live)
    |
    |-- Uses Agent tool (native sub-agents)
    |       |
    |       +---> PostToolUse hook sends event to wmux pipe
    |               |
    |               +---> wmux sidebar (agent name, status)
    |
    +-- Finishes task / is interrupted
            |
            +---> Shell integration (already in place)
                    |
                    +---> wmux dots (orange/green/red)
```

---

## 1. CDP Proxy for Browser Visibility

### What it does

Exposes wmux's browser panel as a Chrome DevTools Protocol target on `localhost:9222`. The `chrome-devtools-mcp` plugin (already enabled in the user's Claude Code settings) connects to it natively. Claude Code uses its standard browser tools — wmux displays the results.

### Architecture

```
chrome-devtools-mcp (Claude Code native plugin)
    |
    +-- HTTP GET /json/list --> returns wmux webview target
    +-- HTTP GET /json/version --> returns metadata
    +-- WebSocket ws://localhost:9222/devtools/page/1
            |
            +-- CDP Proxy (bidirectional)
                    |
                    +-- webContents.debugger.sendCommand()
                    +-- webContents.debugger events --> WebSocket
```

### New file: `src/main/cdp-proxy.ts`

Responsibilities:
1. Start HTTP server on port 9222 (configurable)
2. Serve `/json/list` and `/json/version` endpoints matching Chrome's format
3. Accept WebSocket upgrade on `/devtools/page/{id}`
4. Forward incoming CDP commands to `webContents.debugger.sendCommand()`
5. Forward debugger events back to the WebSocket client
6. Handle attach/detach lifecycle when browser panel opens/closes

### HTTP endpoints

**GET /json/version**
```json
{
  "Browser": "wmux/0.3.0",
  "Protocol-Version": "1.3",
  "webSocketDebuggerUrl": "ws://localhost:9222/devtools/page/1"
}
```

**GET /json/list**
```json
[{
  "id": "1",
  "type": "page",
  "title": "<current page title>",
  "url": "<current page URL>",
  "webSocketDebuggerUrl": "ws://localhost:9222/devtools/page/1"
}]
```

### WebSocket proxy behavior

- Client sends JSON → parse → `debugger.sendCommand(method, params)` → response JSON back
- Debugger emits event → forward as JSON to WebSocket client
- Multiple clients not supported (one debugger session at a time)
- If browser panel is closed, return error on connect
- If webview navigates to new origin, debugger may detach/reattach — proxy handles this transparently

### Port selection

- Default: 9222
- If port is busy (another Chrome in debug mode), try 9223, 9224, up to 9230
- Log the actual port to console: `[wmux] CDP proxy listening on localhost:9222`

### Integration with existing CDP bridge

The existing `cdp-bridge.ts` attaches to the same `webContents.debugger`. CDP only allows one debugger session. Two options:

**Option A (recommended):** CDP proxy replaces the direct bridge. All CDP access goes through the proxy — both external (chrome-devtools-mcp) and internal (pipe API browser commands). The pipe handlers call the proxy locally instead of the bridge directly.

**Option B:** CDP proxy coexists with the bridge by sharing the same debugger instance. Both forward commands to `webContents.debugger.sendCommand()`. This works because the debugger API is stateless per-call.

Recommended: **Option B** — simpler, no refactoring of existing pipe handlers. The debugger instance is shared; both the bridge and the proxy call `sendCommand()` on it.

---

## 2. Claude Code Hooks for Agent Visibility

### What it does

Auto-configures Claude Code hooks in `~/.claude/settings.json` so that every tool call triggers a notification to wmux. wmux displays agent activity in the sidebar.

### Hook configuration

Injected into `settings.json` at startup, same managed-section pattern as CLAUDE.md:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "",
        "command": "node \"$WMUX_CLI\" hook --event post_tool --tool $CLAUDE_TOOL_NAME --agent $CLAUDE_AGENT_ID 2>/dev/null || true"
      }
    ]
  }
}
```

- Empty matcher = fires on ALL tool uses (we filter in the handler)
- `2>/dev/null || true` = never block Claude Code even if wmux is down
- Environment variables `$CLAUDE_TOOL_NAME` and `$CLAUDE_AGENT_ID` are provided by Claude Code's hook system

### Modified file: `src/main/claude-context.ts`

Add `ensureClaudeHooks()` function:
1. Read `~/.claude/settings.json`
2. Check for existing wmux hooks (identified by `WMUX_CLI` in the command string)
3. If not present, add the hook entry to `hooks.PostToolUse` array
4. If present but outdated, update it
5. Never touch other hook entries

### New CLI command: `wmux hook`

```bash
node "$WMUX_CLI" hook --event post_tool --tool Agent --agent agent-123
```

Sends a V2 JSON-RPC message to the pipe:
```json
{"method": "hook.event", "params": {"event": "post_tool", "tool": "Agent", "agentId": "agent-123"}}
```

### Pipe handler: `hook.event`

In `src/main/index.ts`, new V2 handler:
- Receives hook events
- Broadcasts to renderer via IPC: `IPC_CHANNELS.HOOK_EVENT`
- Responds with `{ok: true}`

### Renderer: agent activity display

**In `App.tsx`:**
- Listen for `HOOK_EVENT` IPC messages
- Track agent activity state: `Map<string, {tool: string, count: number, lastSeen: number}>`
- When `tool === 'Agent'`: add/update agent in the activity map
- When no hooks received for 5+ seconds after last agent hook: mark agents as done

**In `WorkspaceRow.tsx`:**
- New section below metadata: "Agent activity"
- Shows active agent count and last tool used
- Example: `2 agents working...` or `Agent finished`

**In `sidebar.css`:**
- Subtle agent activity styles, similar to existing metadata rows

---

## 3. Minimal CLAUDE.md

### Content

```markdown
<!-- wmux:start — AUTO-MANAGED BY wmux. Do not edit this section manually. -->

# wmux

You are running inside wmux, a terminal multiplexer. The user can see
your browser activity in a panel on the right side of their screen,
and your agent activity in the sidebar. When relevant, you can mention
this to the user (e.g. "you can see the page in your browser panel").

<!-- wmux:end -->
```

### What's removed

- All CLI browser commands
- All agent spawn instructions
- "DO NOT use Firecrawl/Playwright" directives
- Web search examples
- Notification/status CLI commands

### What stays

- The managed-section injection mechanism (`ensureClaudeContext()`)
- The `<!-- wmux:start/end -->` markers
- Auto-update on wmux startup

---

## Files Changed

| File | Action | Description |
|------|--------|-------------|
| `src/main/cdp-proxy.ts` | New | HTTP+WebSocket server proxying CDP to webview debugger |
| `src/main/claude-context.ts` | Modified | Add `ensureClaudeHooks()` for settings.json hook injection |
| `src/main/index.ts` | Modified | Start CDP proxy, add `hook.event` pipe handler |
| `src/cli/wmux.ts` | Modified | Add `hook` command |
| `src/shared/types.ts` | Modified | Add `HOOK_EVENT` IPC channel |
| `src/preload/index.ts` | Modified | Expose `hook.onEvent()` listener |
| `src/renderer/App.tsx` | Modified | Listen for hook events, track agent activity |
| `src/renderer/components/Sidebar/WorkspaceRow.tsx` | Modified | Display agent activity zone |
| `src/renderer/styles/sidebar.css` | Modified | Agent activity styles |
| `resources/claude-instructions.md` | Modified | Minimal informational text |

## What Does NOT Change

- The wmux CLI remains available (manual use, scripts)
- The internal CDP bridge remains (shared debugger with proxy)
- Shell integration hooks (dots orange/green/red)
- Auto-notifications on command finish/interrupt
- Clipboard image paste
- Session restore, themes, splits, workspaces
- Notification center

## What Is Removed

- `browser-agent.ts` (already removed)
- CLI instructions in CLAUDE.md telling Claude Code what tools to use
- The concept of wmux as a tool provider — it becomes purely an observer
