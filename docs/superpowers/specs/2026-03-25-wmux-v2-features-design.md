# wmux v2 Features: Notification Center, Scriptable Browser, Sub-Agent Spawning

> **Historical design record:** This page captures earlier design or implementation work and is not current guidance. Verify behavior against the [documentation index](../../README.md), the [developer guide](../../../CLAUDE.md), and the current Rust source.

> **For agentic workers:** This spec defines the next batch of features for wmux, the Windows terminal multiplexer for AI coding agents.

**Goal:** Add a notification center panel, a CDP-powered scriptable browser API for AI agent browser control, and intelligent sub-agent terminal spawning with round-robin distribution across panes.

**Context:** wmux v0.1.0 is live with split panes, workspaces, sidebar metadata, basic browser panel, notification rings, and a named pipe JSON-RPC API. These features build on top of the existing architecture without restructuring it.

---

## 1. Notification Center

### Problem
Notifications exist (pane rings, sidebar badges, OS toasts) but there's no centralized place to see all pending notifications and jump to them.

### Design

**Titlebar bell icon:**
- SVG bell icon added to the titlebar's left section, after the existing `?` and `</>` buttons
- Displays a red badge with unread count (hidden when 0)
- Click toggles the notification dropdown panel
- Badge pulses briefly when a new notification arrives

**Notification panel (dropdown):**
- Absolutely positioned below the bell icon
- Width: 350px, max-height: 400px, scrollable
- Dark theme: `#1a1a1a` background, `1px solid rgba(255,255,255,0.08)` border, `border-radius: 8px`, subtle box-shadow
- Header row: "Notifications" label + "Mark all read" button
- Each notification row:
  - Workspace name + surface label (e.g., "Workspace 1 > Terminal 2")
  - Notification text (single line, truncated with ellipsis)
  - Relative timestamp ("2m ago", "1h ago", "yesterday")
  - Unread indicator (blue dot on left edge)
  - Click: jumps to the source workspace/pane/surface, marks as read, closes panel
  - Hover: subtle highlight (`rgba(255,255,255,0.04)`)
- Empty state: centered "No notifications" text
- Click outside panel or press Escape: closes panel
- Panel closes automatically when jumping to a notification

**Keyboard shortcut:** `Ctrl+Alt+N` toggles the notification panel (matches existing `showNotifications` binding in settings-slice — `Ctrl+Shift+N` is already bound to `newWindow`).

**Notification retention:** Max 200 notifications stored. When the limit is reached, oldest read notifications are evicted first (FIFO). Unread notifications are never evicted.

### Files
- **New:** `src/renderer/components/Titlebar/NotificationBell.tsx`
- **New:** `src/renderer/components/Titlebar/NotificationPanel.tsx`
- **New:** `src/renderer/styles/notification-panel.css`
- **Modified:** `src/renderer/components/Titlebar/Titlebar.tsx` — add NotificationBell component
- **Modified:** `src/renderer/styles/titlebar.css` — bell icon styles
- **Modified:** `src/renderer/hooks/useKeyboardShortcuts.ts` — wire existing `showNotifications` shortcut to toggle panel

### Data Flow
- NotificationPanel reads from Zustand `notificationSlice.notifications`
- Unread count computed from `notifications.filter(n => !n.read).length`
- Jump-to-notification: walk the active workspace's split tree to find which pane contains the target surfaceId (using `findLeaf()` from split-utils), then call `selectWorkspace()` + `selectSurface(workspaceId, paneId, surfaceIndex)` + `markRead()`
- `NotificationInfo` extended with optional `paneId?: PaneId` — populated at creation time for faster lookup, with split-tree walk as fallback
- No new IPC needed — all data already in renderer state

---

## 2. Scriptable Browser API (CDP Bridge)

### Problem
The browser panel exists but agents can't control it programmatically. Claude Code needs to navigate, read page content, click buttons, and fill forms — all visible to the user in real-time.

### Design

**Architecture:**
- Electron's `<webview>` tag runs its own Chromium renderer process
- Main process attaches to the webview's `webContents.debugger` via Electron's debugger API
- CDP commands (`DOM.getDocument`, `Accessibility.getFullAXTree`, `Runtime.evaluate`, `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent`, `Page.captureScreenshot`) are sent through the debugger
- JSON-RPC commands arrive via the named pipe, get translated to CDP calls in main process, results returned to caller

**CDP Bridge module (`cdp-bridge.ts`):**

Core responsibilities:
1. Attach/detach debugger to webview's webContents
2. Build accessibility tree with numbered refs (`@e1`, `@e2`, ...)
3. Resolve ref → DOM nodeId → coordinates for click/type
4. Execute CDP commands and return results

**Accessibility tree snapshot format:**
```
@e1: document "My App"
  @e2: navigation "Main"
    @e3: link "Home"
    @e4: link "About"
  @e5: main
    @e6: heading "Welcome"
    @e7: textbox "Email" value=""
    @e8: textbox "Password" value=""
    @e9: button "Sign In"
```
- Each node prefixed with `@eN` ref
- Role + name + value shown
- Tree indented to show hierarchy
- Only interactive and semantically meaningful nodes included (skip generic divs/spans unless they have ARIA roles)

**JSON-RPC commands (pipe V2):**

| Method | Params | Returns | Description |
|---|---|---|---|
| `browser.navigate` | `{url: string, timeout?: number}` | `{ok: true}` | Navigate to URL. Resolves after `Page.loadEventFired` (default 30s timeout). Returns `timeout` error if page doesn't finish loading. |
| `browser.snapshot` | `{}` | `{tree: string}` | Accessibility tree with refs |
| `browser.click` | `{ref: string}` | `{ok: true}` | Click element by ref |
| `browser.type` | `{ref: string, text: string}` | `{ok: true}` | Type text into element (keystroke simulation) |
| `browser.fill` | `{ref: string, value: string}` | `{ok: true}` | Set input value directly |
| `browser.screenshot` | `{fullPage?: boolean}` | `{data: string}` | Base64 PNG screenshot |
| `browser.get_text` | `{ref?: string}` | `{text: string}` | Get text content (page or element) |
| `browser.eval` | `{js: string}` | `{result: any}` | Execute JavaScript, return result |
| `browser.wait` | `{ref?: string, timeout?: number}` | `{ok: true}` | Wait for element to appear or navigation |
| `browser.batch` | `{commands: Command[]}` | `{results: Result[]}` | Execute commands sequentially. Stops on first error. Returns all results up to and including the error. Refs are NOT auto-refreshed between batch commands — include a `browser.snapshot` in the batch if needed. |

**Error handling:**
- All errors are returned via the standard `V2Response.error` envelope (same as existing pipe protocol), NOT inside result objects.
- `browser.eval` runs in the webview's sandboxed renderer process — no access to Node.js APIs or the main process.
- If browser panel is closed: return `{error: "browser_not_open", message: "Browser panel is not open"}`
- If ref not found: return `{error: "ref_not_found", message: "@eN not found in current snapshot"}`
- If navigation timeout: return `{error: "timeout", message: "Navigation timed out"}`
- Snapshot is invalidated after any action that changes the page (click, navigate, type). Agent must re-snapshot after mutations.

**IPC flow:**
```
Named pipe → pipe-server.ts → ipcMain → cdp-bridge.ts → webContents.debugger → CDP → result back
```

### Files
- **New:** `src/main/cdp-bridge.ts` — CDP bridge: attach debugger, build accessibility tree, resolve refs, execute commands
- **Modified:** `src/main/pipe-server.ts` — Register `browser.*` V2 handlers
- **Modified:** `src/main/ipc-handlers.ts` — IPC channels for CDP bridge (renderer requests webContents ID, main process manages debugger)
- **Modified:** `src/shared/types.ts` — `CDPCommand`, `SnapshotNode`, `BrowserCommandResult` types, new IPC channel names
- **Modified:** `src/cli/wmux.ts` — New CLI commands: `browser snapshot`, `browser click @e5`, `browser type @e3 "hello"`, etc.
- **Modified:** `src/renderer/components/Browser/BrowserPane.tsx` — Expose webview's `webContentsId` to main process via IPC on mount

### Webview webContentsId Discovery
1. BrowserPane listens for the `did-attach` event on the `<webview>` element
2. On attach, calls `webviewRef.current.getWebContentsId()` (Electron 28+ API)
3. Sends the ID to main process via `ipcRenderer.send('cdp:attach', webContentsId)`
4. If webview is destroyed/recreated (e.g., navigation to new origin), main process detaches the old debugger and re-attaches when the new `cdp:attach` arrives
5. CDP bridge tracks the current webContentsId and returns `browser_not_open` error if no webview is attached

### Ref System
- Refs are ephemeral — valid only for the current page state
- Each `browser.snapshot` call generates fresh refs
- Refs are stored in-memory in cdp-bridge.ts as a Map<string, {nodeId: number, backendNodeId: number}>
- Click resolution: ref → backendNodeId → `DOM.getBoxModel` → center coordinates → `Input.dispatchMouseEvent`

---

## 3. Sub-Agent Terminal Spawning

### Problem
When Claude Code spawns sub-agents, the user can't see what each sub-agent is doing. Sub-agents should get their own visible terminals distributed across existing panes.

### Design

**Agent Manager module (`agent-manager.ts`):**

Manages agent lifecycle:
- Spawn agent processes as PTY sessions
- Track agent metadata (label, status, parent surface, spawn time)
- Distribute across panes using configurable strategy
- Report status back via pipe

**Single spawn — `agent.spawn`:**
```json
{
  "method": "agent.spawn",
  "params": {
    "cmd": "claude --resume abc123",
    "label": "Research agent",
    "cwd": "C:/project",
    "env": {"TASK": "investigate auth bug"},
    "paneId": "pane-xxx"
  }
}
```
- `cmd`: command to run (required)
- `label`: display name in tab bar (required)
- `cwd`: working directory (optional, defaults to parent's CWD)
- `env`: additional environment variables (optional)
- `paneId`: explicit pane placement (optional — if omitted, placed in least-loaded pane)

Returns: `{surfaceId: "surf-...", agentId: "agent-..."}`

**Batch spawn — `agent.spawn_batch`:**
```json
{
  "method": "agent.spawn_batch",
  "params": {
    "agents": [
      {"cmd": "claude --resume a", "label": "Agent 1"},
      {"cmd": "claude --resume b", "label": "Agent 2"},
      {"cmd": "claude --resume c", "label": "Agent 3"}
    ],
    "strategy": "distribute"
  }
}
```

**Distribution strategies:**
- `"distribute"` (default): Round-robin across all panes in the target workspace. 3 panes + 6 agents = 2 per pane. Fills least-loaded panes first.
- `"stack"`: All agents as tabs in a single pane (the least-loaded one).
- `"split"`: Auto-split to create new panes for agents. Max 4 new splits — if more agents than 4, overflow agents become tabs in the newly created panes. Split direction alternates: first horizontal, then vertical, etc.

**Distribution algorithm for `"distribute"`:**
```
1. Get all leaf panes in the target workspace's split tree
2. Sort panes by current tab count (ascending)
3. For each agent in the batch:
   a. Pick the pane with fewest tabs
   b. Create a new terminal surface in that pane
   c. Spawn PTY with the agent's cmd
   d. Increment that pane's count
```

**Workspace resolution:** Both `agent.spawn` and `agent.spawn_batch` accept an optional `workspaceId` parameter. If omitted, the main process queries the renderer for the current `activeWorkspaceId` via a synchronous IPC round-trip (`ipcMain.handle('get-active-workspace')`).

**Agent lifecycle states:**
- `spawning` → `running` → `exited(code)`
- AgentInfo stored in AgentManager: `{agentId, surfaceId, paneId, workspaceId, label, cmd, status, exitCode?, spawnTime, pid}`

**Status and control commands:**
| Method | Params | Returns |
|---|---|---|
| `agent.spawn` | `{cmd, label, ...}` | `{agentId, surfaceId}` |
| `agent.spawn_batch` | `{agents[], strategy}` | `{agents: [{agentId, surfaceId}]}` |
| `agent.status` | `{agentId}` | `{status, exitCode?, pid, label}` |
| `agent.list` | `{workspaceId?}` | `{agents: AgentInfo[]}` |
| `agent.kill` | `{agentId}` | `{ok: true}` |

**Visual distinction for agent tabs:**
- Agent-spawned tabs show a small `>_` icon (different from regular terminal `>` icon) in the tab bar
- Tab label shows the agent label: "Agent 1: research" instead of "Terminal 3"
- When agent exits: tab label gets "[exited: 0]" suffix, icon dims
- Agent tabs can be closed normally by the user

**Integration with existing PTY system:**
- `agent-manager.ts` uses `pty-manager.ts` to spawn processes (same as regular terminals)
- The difference is metadata tracking + distribution logic
- Agent terminals get `WMUX_AGENT_ID` and `WMUX_AGENT_LABEL` env vars in addition to standard WMUX vars

### Files
- **New:** `src/main/agent-manager.ts` — Agent lifecycle, distribution logic, status tracking
- **Modified:** `src/main/pipe-server.ts` — Register `agent.*` V2 handlers
- **Modified:** `src/main/ipc-handlers.ts` — IPC for agent status/list queries from renderer
- **Modified:** `src/shared/types.ts` — `AgentInfo`, `AgentSpawnParams`, `AgentBatchParams`, `AgentStatus` types
- **Modified:** `src/renderer/store/surface-slice.ts` — Add agent metadata lookup: a `Map<SurfaceId, {agentId: string, agentLabel: string}>` in the store. SurfaceTabBar queries this map by surfaceId to get agent info, keeping the split tree's `SurfaceRef` type lightweight (no changes to SurfaceRef itself).
- **Modified:** `src/renderer/components/SplitPane/SurfaceTabBar.tsx` — Agent tab icon + label + exit status display
- **Modified:** `src/cli/wmux.ts` — New commands: `agent spawn`, `agent spawn-batch`, `agent status`, `agent list`, `agent kill`

---

## 4. Sidebar Metadata Additions

### Problem
Two small pieces of metadata are missing from the sidebar workspace rows.

### Design

**Shell state indicator:**
- Small dot next to the workspace title
- Green pulsing dot when shell is `running` (command executing)
- Gray static dot when `idle`
- Data already flows via shell integration → pipe server → metadata updates
- Just needs the visual component

**Agent count:**
- When agents are spawned in a workspace, show "3 agents" or "3a" in the metadata row
- Reads from AgentManager's agent list filtered by workspaceId
- Hidden when 0 agents

### Files
- **Modified:** `src/renderer/components/Sidebar/WorkspaceRow.tsx` — Add ShellStateIndicator + agent count display
- **Modified:** `src/renderer/styles/sidebar.css` — Styles for state dot and agent count

---

## 5. Type Definitions Summary

New types added to `src/shared/types.ts`:

```typescript
// Agent system
interface AgentInfo {
  agentId: string;
  surfaceId: SurfaceId;
  paneId: PaneId;
  workspaceId: WorkspaceId;
  label: string;
  cmd: string;
  status: 'spawning' | 'running' | 'exited';
  exitCode?: number;
  pid?: number;
  spawnTime: number;
}

interface AgentSpawnParams {
  cmd: string;
  label: string;
  cwd?: string;
  env?: Record<string, string>;
  paneId?: PaneId;
  workspaceId?: WorkspaceId;
}

interface AgentBatchParams {
  agents: AgentSpawnParams[];
  strategy: 'distribute' | 'stack' | 'split';
  workspaceId?: WorkspaceId;
}

// CDP Browser API
// Browser command results use the standard V2Response envelope for errors.
// Each command returns a specific result type via V2Response.result:
interface CDPSnapshot {
  tree: string;    // Accessibility tree as formatted text
  refCount: number; // Number of refs in the snapshot
}
// browser.navigate → {ok: true}
// browser.snapshot → CDPSnapshot
// browser.click → {ok: true}
// browser.type → {ok: true}
// browser.fill → {ok: true}
// browser.screenshot → {data: string} (base64 PNG)
// browser.get_text → {text: string}
// browser.eval → {result: unknown}
// browser.wait → {ok: true}
// browser.batch → {results: Array<V2Response>}
```

New IPC channels:
```typescript
AGENT: {
  SPAWN: 'agent:spawn',
  SPAWN_BATCH: 'agent:spawn-batch',
  STATUS: 'agent:status',
  LIST: 'agent:list',
  KILL: 'agent:kill',
}
// Note: The `browser.*` JSON-RPC methods (external pipe API) map to
// internal `cdp:*` IPC channels between pipe-server and cdp-bridge.
CDP: {
  ATTACH: 'cdp:attach',
  DETACH: 'cdp:detach',
  NAVIGATE: 'cdp:navigate',
  SNAPSHOT: 'cdp:snapshot',
  CLICK: 'cdp:click',
  TYPE: 'cdp:type',
  FILL: 'cdp:fill',
  SCREENSHOT: 'cdp:screenshot',
  GET_TEXT: 'cdp:get-text',
  EVAL: 'cdp:eval',
  WAIT: 'cdp:wait',
}
```

---

## 6. What Is NOT Changing

- Split pane system (no structural changes)
- Existing terminal rendering (xterm.js, WebGL)
- Session persistence format (agent state is ephemeral — agents don't survive restart)
- Theme system
- Settings window
- Tutorial
- Command palette (could add commands later, not in this scope)
- Shell integration scripts (already report all needed data)
- Window management

---

## 7. Testing Strategy

- **Unit tests:** Distribution algorithm, accessibility tree building, ref resolution, notification panel state
- **Integration tests:** Pipe V2 agent.spawn → verify terminal created in correct pane, CDP commands → verify correct CDP calls made
- **Manual verification:** Notification panel dropdown UX, agent tab visual distinction, browser snapshot readability
