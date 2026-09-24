# wmux v2 Features Implementation Plan

> **Historical design record:** This page captures earlier design or implementation work and is not current guidance. Verify behavior against the [documentation index](../../README.md), the [developer guide](../../../CLAUDE.md), and the current Rust source.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add notification center panel, CDP-powered scriptable browser API, intelligent sub-agent terminal spawning, and sidebar metadata enhancements to wmux.

**Architecture:** Four independent feature modules (notification UI, CDP bridge, agent manager, sidebar polish) wired into the existing Electron IPC + named pipe infrastructure. All new main-process modules are standalone classes. Renderer changes add new components and extend the Zustand store.

**Tech Stack:** Electron 33 (webContents.debugger for CDP), React 19, Zustand 5, TypeScript 5.5, node-pty, xterm.js

**Spec:** `docs/superpowers/specs/2026-03-25-wmux-v2-features-design.md`

---

## File Structure

**New files:**
- `src/main/cdp-bridge.ts` — CDP bridge: attach debugger to webview, build accessibility tree, resolve refs, execute commands
- `src/main/agent-manager.ts` — Agent lifecycle: spawn, distribute, track, kill
- `src/renderer/components/Titlebar/NotificationBell.tsx` — Bell icon + badge in titlebar
- `src/renderer/components/Titlebar/NotificationPanel.tsx` — Dropdown notification list
- `src/renderer/styles/notification-panel.css` — Panel styles
- `tests/unit/cdp-bridge.test.ts` — CDP bridge unit tests
- `tests/unit/agent-manager.test.ts` — Agent manager + distribution unit tests
- `tests/unit/notification-panel.test.ts` — Notification panel logic tests

**Modified files:**
- `src/shared/types.ts` — New types + IPC channels
- `src/main/pipe-server.ts` — browser.* and agent.* V2 handlers
- `src/main/ipc-handlers.ts` — CDP + agent IPC handlers
- `src/main/index.ts` — Wire CDP bridge + agent manager into app lifecycle
- `src/preload/index.ts` — Expose CDP attach + agent IPC to renderer
- `src/renderer/store/agent-slice.ts` — New Zustand slice for agent metadata (Map<SurfaceId, AgentMeta>)
- `src/renderer/store/index.ts` — Add agent slice
- `src/renderer/store/notification-slice.ts` — Add retention limit + paneId support
- `src/renderer/components/Titlebar/Titlebar.tsx` — Add NotificationBell
- `src/renderer/styles/titlebar.css` — Bell icon styles
- `src/renderer/components/Browser/BrowserPane.tsx` — Send webContentsId to main
- `src/renderer/components/SplitPane/SurfaceTabBar.tsx` — Agent tab icon + label
- `src/renderer/components/Sidebar/WorkspaceRow.tsx` — Shell state dot + agent count
- `src/renderer/styles/sidebar.css` — Shell state + agent count styles
- `src/renderer/hooks/useKeyboardShortcuts.ts` — Wire showNotifications action
- `src/renderer/App.tsx` — Pass notification panel toggle, wire agent metadata
- `src/cli/wmux.ts` — New browser + agent CLI commands

---

### Task 1: Add New Types and IPC Channels

**Files:**
- Modify: `src/shared/types.ts`

- [ ] **Step 1: Add agent types after existing NotificationInfo (line 83)**

Add after line 83 in `src/shared/types.ts`:

```typescript
// Agent system
export type AgentId = `agent-${string}`;

export interface AgentInfo {
  agentId: AgentId;
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

export interface AgentSpawnParams {
  cmd: string;
  label: string;
  cwd?: string;
  env?: Record<string, string>;
  paneId?: PaneId;
  workspaceId?: WorkspaceId;
}

export interface AgentBatchParams {
  agents: AgentSpawnParams[];
  strategy: 'distribute' | 'stack' | 'split';
  workspaceId?: WorkspaceId;
}

// CDP Browser API
export interface CDPSnapshot {
  tree: string;
  refCount: number;
}
```

- [ ] **Step 2: Add paneId to NotificationInfo**

In `src/shared/types.ts`, add `paneId` to `NotificationInfo`:

```typescript
export interface NotificationInfo {
  id: string;
  surfaceId: SurfaceId;
  workspaceId: WorkspaceId;
  paneId?: PaneId;  // <-- add this line
  text: string;
  title?: string;
  timestamp: number;
  read: boolean;
}
```

- [ ] **Step 3: Add new IPC channels**

Add these entries inside the `IPC_CHANNELS` const, before the closing `} as const`:

```typescript
  // Agent
  AGENT_SPAWN: 'agent:spawn',
  AGENT_SPAWN_BATCH: 'agent:spawn-batch',
  AGENT_STATUS: 'agent:status',
  AGENT_LIST: 'agent:list',
  AGENT_KILL: 'agent:kill',
  AGENT_UPDATE: 'agent:update',
  // CDP (browser.* pipe methods map to these internal IPC channels)
  CDP_ATTACH: 'cdp:attach',
  CDP_DETACH: 'cdp:detach',
  CDP_NAVIGATE: 'cdp:navigate',
  CDP_SNAPSHOT: 'cdp:snapshot',
  CDP_CLICK: 'cdp:click',
  CDP_TYPE: 'cdp:type',
  CDP_FILL: 'cdp:fill',
  CDP_SCREENSHOT: 'cdp:screenshot',
  CDP_GET_TEXT: 'cdp:get-text',
  CDP_EVAL: 'cdp:eval',
  CDP_WAIT: 'cdp:wait',
  // Active workspace query (renderer → main)
  GET_ACTIVE_WORKSPACE: 'get-active-workspace',
```

- [ ] **Step 4: Run TypeScript compiler to verify types**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx tsc --noEmit --pretty 2>&1 | head -20`
Expected: No new errors from types.ts

- [ ] **Step 5: Commit**

```bash
git add src/shared/types.ts
git commit -m "feat: add agent, CDP, and notification types + IPC channels"
```

---

### Task 2: Notification Bell and Panel Components

**Files:**
- Create: `src/renderer/components/Titlebar/NotificationBell.tsx`
- Create: `src/renderer/components/Titlebar/NotificationPanel.tsx`
- Create: `src/renderer/styles/notification-panel.css`
- Modify: `src/renderer/styles/titlebar.css`

- [ ] **Step 1: Create NotificationPanel component**

Create `src/renderer/components/Titlebar/NotificationPanel.tsx`:

```tsx
import React from 'react';
import { NotificationInfo, WorkspaceId, PaneId, SurfaceId } from '../../../shared/types';
import '../../styles/notification-panel.css';

interface NotificationPanelProps {
  notifications: NotificationInfo[];
  workspaceNames: Map<string, string>; // workspaceId → title
  onJump: (workspaceId: WorkspaceId, surfaceId: SurfaceId, paneId?: PaneId) => void;
  onMarkAllRead: () => void;
  onClose: () => void;
}

function timeAgo(timestamp: number): string {
  const seconds = Math.floor((Date.now() - timestamp) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return 'yesterday';
}

export default function NotificationPanel({
  notifications,
  workspaceNames,
  onJump,
  onMarkAllRead,
  onClose,
}: NotificationPanelProps) {
  const sorted = [...notifications].sort((a, b) => b.timestamp - a.timestamp);

  return (
    <div className="notif-panel" onClick={(e) => e.stopPropagation()}>
      <div className="notif-panel__header">
        <span className="notif-panel__title">Notifications</span>
        {notifications.some((n) => !n.read) && (
          <button className="notif-panel__mark-all" onClick={onMarkAllRead}>
            Mark all read
          </button>
        )}
      </div>
      <div className="notif-panel__list">
        {sorted.length === 0 ? (
          <div className="notif-panel__empty">No notifications</div>
        ) : (
          sorted.map((n) => (
            <div
              key={n.id}
              className={`notif-panel__item ${!n.read ? 'notif-panel__item--unread' : ''}`}
              onClick={() => {
                onJump(n.workspaceId, n.surfaceId, n.paneId);
                onClose();
              }}
            >
              {!n.read && <span className="notif-panel__dot" />}
              <div className="notif-panel__content">
                <span className="notif-panel__source">{workspaceNames.get(n.workspaceId) || 'Unknown'}</span>
                <span className="notif-panel__text">{n.text}</span>
                <span className="notif-panel__time">{timeAgo(n.timestamp)}</span>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Create NotificationBell component**

Create `src/renderer/components/Titlebar/NotificationBell.tsx`:

```tsx
import React, { useState, useRef, useEffect } from 'react';
import NotificationPanel from './NotificationPanel';
import { NotificationInfo, WorkspaceId, PaneId, SurfaceId } from '../../../shared/types';

interface NotificationBellProps {
  notifications: NotificationInfo[];
  workspaceNames: Map<string, string>;
  onJump: (workspaceId: WorkspaceId, surfaceId: SurfaceId, paneId?: PaneId) => void;
  onMarkAllRead: () => void;
  isOpen: boolean;
  onToggle: () => void;
}

export default function NotificationBell({
  notifications,
  workspaceNames,
  onJump,
  onMarkAllRead,
  isOpen,
  onToggle,
}: NotificationBellProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const unreadCount = notifications.filter((n) => !n.read).length;

  // Close on outside click
  useEffect(() => {
    if (!isOpen) return;
    function handleClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onToggle();
      }
    }
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [isOpen, onToggle]);

  // Close on Escape
  useEffect(() => {
    if (!isOpen) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onToggle();
    }
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [isOpen, onToggle]);

  return (
    <div ref={containerRef} className="notif-bell" style={{ position: 'relative' }}>
      <button
        className="titlebar__btn notif-bell__btn"
        onClick={onToggle}
        title="Notifications"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
          <path d="M8 1.5A3.5 3.5 0 0 0 4.5 5v2.947c0 .346-.102.683-.294.97l-1.703 2.556a.25.25 0 0 0 .208.389L13.29 11.86a.25.25 0 0 0 .208-.389l-1.703-2.556a1.75 1.75 0 0 1-.294-.97V5A3.5 3.5 0 0 0 8 1.5ZM6.5 13a1.5 1.5 0 0 0 3 0h-3Z" />
        </svg>
        {unreadCount > 0 && (
          <span className="notif-bell__badge">{unreadCount > 99 ? '99+' : unreadCount}</span>
        )}
      </button>
      {isOpen && (
        <NotificationPanel
          notifications={notifications}
          workspaceNames={workspaceNames}
          onJump={onJump}
          onMarkAllRead={onMarkAllRead}
          onClose={onToggle}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 3: Create notification panel styles**

Create `src/renderer/styles/notification-panel.css`:

```css
.notif-bell {
  position: relative;
  -webkit-app-region: no-drag;
}

.notif-bell__btn {
  position: relative;
}

.notif-bell__badge {
  position: absolute;
  top: 2px;
  right: 2px;
  min-width: 14px;
  height: 14px;
  padding: 0 3px;
  background: #e53935;
  color: #fff;
  font-size: 9px;
  font-weight: 700;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
  pointer-events: none;
}

.notif-panel {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  width: 350px;
  max-height: 400px;
  background: #1a1a1a;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  z-index: 1000;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.notif-panel__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  flex-shrink: 0;
}

.notif-panel__title {
  font-size: 12px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.7);
}

.notif-panel__mark-all {
  background: none;
  border: none;
  color: #0091FF;
  font-size: 11px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 4px;
}

.notif-panel__mark-all:hover {
  background: rgba(0, 145, 255, 0.1);
}

.notif-panel__list {
  overflow-y: auto;
  flex: 1;
}

.notif-panel__empty {
  padding: 32px 12px;
  text-align: center;
  color: rgba(255, 255, 255, 0.25);
  font-size: 12px;
}

.notif-panel__item {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 12px;
  cursor: pointer;
  transition: background 0.1s;
}

.notif-panel__item:hover {
  background: rgba(255, 255, 255, 0.04);
}

.notif-panel__item--unread {
  background: rgba(0, 145, 255, 0.04);
}

.notif-panel__dot {
  width: 6px;
  height: 6px;
  min-width: 6px;
  border-radius: 50%;
  background: #0091FF;
  margin-top: 5px;
}

.notif-panel__content {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.notif-panel__source {
  font-size: 10px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.4);
}

.notif-panel__text {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.7);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.notif-panel__time {
  font-size: 10px;
  color: rgba(255, 255, 255, 0.3);
}
```

- [ ] **Step 4: Commit**

```bash
git add src/renderer/components/Titlebar/NotificationBell.tsx src/renderer/components/Titlebar/NotificationPanel.tsx src/renderer/styles/notification-panel.css
git commit -m "feat: add notification bell icon and dropdown panel components"
```

---

### Task 3: Wire Notification Panel into App

**Files:**
- Modify: `src/renderer/components/Titlebar/Titlebar.tsx`
- Modify: `src/renderer/App.tsx`
- Modify: `src/renderer/hooks/useKeyboardShortcuts.ts`
- Modify: `src/renderer/store/notification-slice.ts`

- [ ] **Step 1: Add notification retention limit to notification-slice.ts**

In `src/renderer/store/notification-slice.ts`, add a `MAX_NOTIFICATIONS` const at the top (after imports) and modify `addNotification`:

Add after line 4:
```typescript
const MAX_NOTIFICATIONS = 200;
```

Replace the `set` call inside `addNotification` (lines 36-38) with:
```typescript
    set((state) => {
      let updated = [...state.notifications, newNotification];
      // Enforce retention limit: evict oldest read notifications first
      if (updated.length > MAX_NOTIFICATIONS) {
        const readToEvict = updated.filter((n) => n.read);
        const evictCount = updated.length - MAX_NOTIFICATIONS;
        const evictIds = new Set(readToEvict.slice(0, evictCount).map((n) => n.id));
        updated = updated.filter((n) => !evictIds.has(n.id));
      }
      return { notifications: updated };
    });
```

- [ ] **Step 2: Update Titlebar to accept notification props**

Replace the entire `src/renderer/components/Titlebar/Titlebar.tsx`:

```tsx
import React from 'react';
import logoSrc from '../../assets/logo.png';
import NotificationBell from './NotificationBell';
import { NotificationInfo, WorkspaceId, PaneId, SurfaceId } from '../../../shared/types';
import '../../styles/titlebar.css';

interface TitlebarProps {
  title?: string;
  onHelpClick?: () => void;
  onDevToolsClick?: () => void;
  notifications: NotificationInfo[];
  workspaceNames: Map<string, string>;
  notificationPanelOpen: boolean;
  onToggleNotificationPanel: () => void;
  onNotificationJump: (workspaceId: WorkspaceId, surfaceId: SurfaceId, paneId?: PaneId) => void;
  onMarkAllNotificationsRead: () => void;
}

export default function Titlebar({
  title,
  onHelpClick,
  onDevToolsClick,
  notifications,
  workspaceNames,
  notificationPanelOpen,
  onToggleNotificationPanel,
  onNotificationJump,
  onMarkAllNotificationsRead,
}: TitlebarProps) {
  return (
    <div className="titlebar">
      <div className="titlebar__left">
        <img src={logoSrc} alt="wmux" className="titlebar__logo" draggable={false} />
        <button
          className="titlebar__btn"
          onClick={onHelpClick}
          title="Help / Tutorial"
        >
          ?
        </button>
        <button
          className="titlebar__btn"
          onClick={onDevToolsClick}
          title="Toggle Developer Tools"
        >
          &lt;/&gt;
        </button>
        <NotificationBell
          notifications={notifications}
          workspaceNames={workspaceNames}
          isOpen={notificationPanelOpen}
          onToggle={onToggleNotificationPanel}
          onJump={onNotificationJump}
          onMarkAllRead={onMarkAllNotificationsRead}
        />
      </div>

      <span className="titlebar__title">{title ?? ''}</span>

      <div className="titlebar__right" />
    </div>
  );
}
```

- [ ] **Step 3: Wire notification panel state into App.tsx**

In `src/renderer/App.tsx`:

Add to the destructured `useStore()` call (around line 61):
```typescript
  const {
    // ... existing ...
    notifications,
    markRead,
    markAllRead,
  } = useStore();
```

Add new state (after `tutorialOpen` state, around line 79):
```typescript
  const [notifPanelOpen, setNotifPanelOpen] = useState(false);
```

Add `findLeaf` to the existing import at the top of App.tsx (line 6 already imports from split-utils):
```typescript
import { updateRatio, getAllPaneIds, findLeaf } from './store/split-utils';
```

Also add `selectSurface` to the destructured `useStore()` call:
```typescript
    selectSurface,
```

Add notification jump handler (after `handlePaletteAction`):
```typescript
  // Build workspace name map for notification panel
  const workspaceNames = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const ws of workspaces) map.set(ws.id, ws.title);
    return map;
  }, [workspaces]);

  const handleNotificationJump = useCallback(
    (workspaceId: WorkspaceId, surfaceId: SurfaceId, _paneId?: PaneId) => {
      selectWorkspace(workspaceId);
      // Find the pane containing this surface by walking the split tree
      const ws = useStore.getState().workspaces.find((w) => w.id === workspaceId);
      if (!ws) return;
      // Walk all leaves to find the one containing this surfaceId
      function findPaneForSurface(node: SplitNode): { paneId: PaneId; index: number } | null {
        if (node.type === 'leaf') {
          const idx = node.surfaces.findIndex((s) => s.id === surfaceId);
          if (idx !== -1) return { paneId: node.paneId, index: idx };
          return null;
        }
        return findPaneForSurface(node.children[0]) || findPaneForSurface(node.children[1]);
      }
      const found = findPaneForSurface(ws.splitTree);
      if (found) {
        setFocusedPaneId(found.paneId);
        selectSurface(workspaceId, found.paneId, found.index);
      }
      markRead(surfaceId);
    },
    [selectWorkspace, markRead, selectSurface],
  );

  const handleToggleNotifPanel = useCallback(() => {
    setNotifPanelOpen((o) => !o);
  }, []);
```

Update the `<Titlebar>` JSX to pass new props:
```tsx
      <Titlebar
        title={titlebarText}
        onHelpClick={() => setTutorialOpen(true)}
        onDevToolsClick={() => window.wmux?.system?.toggleDevTools?.()}
        notifications={notifications}
        workspaceNames={workspaceNames}
        notificationPanelOpen={notifPanelOpen}
        onToggleNotificationPanel={handleToggleNotifPanel}
        onNotificationJump={handleNotificationJump}
        onMarkAllNotificationsRead={() => markAllRead()}
      />
```

- [ ] **Step 4: Wire showNotifications shortcut in useKeyboardShortcuts.ts**

In `src/renderer/hooks/useKeyboardShortcuts.ts`, update the hook signature to accept a new callback:

```typescript
export function useKeyboardShortcuts(
  focusedPaneId: PaneId | null,
  onOpenSettings?: (open: boolean) => void,
  onToggleBrowser?: () => void,
  onToggleNotifications?: () => void,
): void {
```

Add a case in `dispatchAction` (around line 180, in the default/unimplemented section):
```typescript
        case 'showNotifications': {
          onToggleNotifications?.();
          break;
        }
```

Add `onToggleBrowser` and `onToggleNotifications` to the `useEffect` deps array (line 193-208). Note: `onToggleBrowser` was already missing from the deps — fix that too.

In `App.tsx`, update the `useKeyboardShortcuts` call:
```typescript
  useKeyboardShortcuts(focusedPaneId, setSettingsOpen, () => setBrowserOpen(o => !o), handleToggleNotifPanel);
```

- [ ] **Step 5: Verify compilation**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx tsc --noEmit --pretty 2>&1 | head -30`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add src/renderer/components/Titlebar/Titlebar.tsx src/renderer/App.tsx src/renderer/hooks/useKeyboardShortcuts.ts src/renderer/store/notification-slice.ts
git commit -m "feat: wire notification bell panel into titlebar with keyboard shortcut"
```

---

### Task 4: CDP Bridge (Main Process)

**Files:**
- Create: `src/main/cdp-bridge.ts`
- Create: `tests/unit/cdp-bridge.test.ts`

- [ ] **Step 1: Write CDP bridge test**

Create `tests/unit/cdp-bridge.test.ts`:

```typescript
import { describe, it, expect } from 'vitest';
import { buildAccessibilityTree, resolveRef } from '../../src/main/cdp-bridge';

describe('CDP Bridge', () => {
  describe('buildAccessibilityTree', () => {
    it('formats AX nodes with refs', () => {
      const nodes = [
        { nodeId: 1, role: { value: 'document' }, name: { value: 'My Page' }, childIds: [2, 3] },
        { nodeId: 2, role: { value: 'button' }, name: { value: 'Submit' }, childIds: [] },
        { nodeId: 3, role: { value: 'textbox' }, name: { value: 'Email' }, value: { value: '' }, childIds: [] },
      ];
      const result = buildAccessibilityTree(nodes);
      expect(result.tree).toContain('@e1: document "My Page"');
      expect(result.tree).toContain('@e2: button "Submit"');
      expect(result.tree).toContain('@e3: textbox "Email"');
      expect(result.refCount).toBe(3);
    });

    it('skips generic nodes without ARIA roles', () => {
      const nodes = [
        { nodeId: 1, role: { value: 'document' }, name: { value: '' }, childIds: [2] },
        { nodeId: 2, role: { value: 'generic' }, name: { value: '' }, childIds: [3] },
        { nodeId: 3, role: { value: 'button' }, name: { value: 'OK' }, childIds: [] },
      ];
      const result = buildAccessibilityTree(nodes);
      expect(result.tree).not.toContain('generic');
      expect(result.tree).toContain('button "OK"');
    });
  });

  describe('resolveRef', () => {
    it('returns nodeId for valid ref', () => {
      const refMap = new Map([['@e1', { nodeId: 5, backendNodeId: 10 }]]);
      expect(resolveRef(refMap, '@e1')).toEqual({ nodeId: 5, backendNodeId: 10 });
    });

    it('returns null for invalid ref', () => {
      const refMap = new Map();
      expect(resolveRef(refMap, '@e99')).toBeNull();
    });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx vitest run tests/unit/cdp-bridge.test.ts 2>&1`
Expected: FAIL — module not found

- [ ] **Step 3: Create CDP bridge implementation**

Create `src/main/cdp-bridge.ts`:

```typescript
import { webContents } from 'electron';
import { CDPSnapshot } from '../shared/types';

interface RefEntry {
  nodeId: number;
  backendNodeId: number;
}

// Roles to skip (generic containers with no semantic meaning)
const SKIP_ROLES = new Set([
  'generic', 'none', 'presentation', 'InlineTextBox', 'LineBreak',
]);

export function buildAccessibilityTree(
  nodes: any[],
): CDPSnapshot & { refMap: Map<string, RefEntry> } {
  const refMap = new Map<string, RefEntry>();
  let refCounter = 0;
  const nodeMap = new Map<number, any>();
  for (const node of nodes) nodeMap.set(node.nodeId, node);

  const lines: string[] = [];

  function walk(nodeId: number, depth: number): void {
    const node = nodeMap.get(nodeId);
    if (!node) return;

    const role = node.role?.value || '';
    const name = node.name?.value || '';
    const value = node.value?.value;

    // Skip generic/invisible nodes
    if (SKIP_ROLES.has(role) && !name) {
      // Still walk children
      for (const childId of node.childIds || []) walk(childId, depth);
      return;
    }

    refCounter++;
    const ref = `@e${refCounter}`;
    refMap.set(ref, {
      nodeId: node.nodeId,
      backendNodeId: node.backendNodeId || node.nodeId,
    });

    const indent = '  '.repeat(depth);
    let line = `${indent}${ref}: ${role}`;
    if (name) line += ` "${name}"`;
    if (value !== undefined && value !== '') line += ` value="${value}"`;
    lines.push(line);

    for (const childId of node.childIds || []) {
      walk(childId, depth + 1);
    }
  }

  // Start from root (first node)
  if (nodes.length > 0) {
    walk(nodes[0].nodeId, 0);
  }

  return {
    tree: lines.join('\n'),
    refCount: refCounter,
    refMap,
  };
}

export function resolveRef(
  refMap: Map<string, RefEntry>,
  ref: string,
): RefEntry | null {
  return refMap.get(ref) ?? null;
}

/**
 * CDPBridge — attaches to a webview's webContents.debugger and provides
 * high-level commands: snapshot, click, type, fill, screenshot, eval, etc.
 */
export class CDPBridge {
  private webContentsId: number | null = null;
  private attached = false;
  private currentRefMap = new Map<string, RefEntry>();

  attach(wcId: number): void {
    this.webContentsId = wcId;
    try {
      const wc = webContents.fromId(wcId);
      if (wc && !wc.debugger.isAttached()) {
        wc.debugger.attach('1.3');
        this.attached = true;
      }
    } catch (err) {
      console.error('[cdp-bridge] Failed to attach:', err);
    }
  }

  detach(): void {
    if (this.webContentsId !== null) {
      try {
        const wc = webContents.fromId(this.webContentsId);
        if (wc?.debugger.isAttached()) {
          wc.debugger.detach();
        }
      } catch {}
    }
    this.attached = false;
    this.webContentsId = null;
    this.currentRefMap.clear();
  }

  get isAttached(): boolean {
    return this.attached && this.webContentsId !== null;
  }

  private getDebugger() {
    if (!this.webContentsId) throw new Error('browser_not_open');
    const wc = webContents.fromId(this.webContentsId);
    if (!wc || !wc.debugger.isAttached()) throw new Error('browser_not_open');
    return wc.debugger;
  }

  private async sendCommand(method: string, params?: any): Promise<any> {
    const dbg = this.getDebugger();
    return dbg.sendCommand(method, params);
  }

  async navigate(url: string, timeout = 30000): Promise<void> {
    const dbg = this.getDebugger();
    const wc = webContents.fromId(this.webContentsId!);

    // Navigate and wait for load
    const loadPromise = new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('timeout')), timeout);
      const onFinish = () => {
        clearTimeout(timer);
        wc?.removeListener('did-finish-load', onFinish);
        resolve();
      };
      wc?.once('did-finish-load', onFinish);
    });

    await this.sendCommand('Page.navigate', { url });
    await loadPromise;
  }

  async snapshot(): Promise<CDPSnapshot> {
    const result = await this.sendCommand('Accessibility.getFullAXTree');
    const { tree, refCount, refMap } = buildAccessibilityTree(result.nodes || []);
    this.currentRefMap = refMap;
    return { tree, refCount };
  }

  async click(ref: string): Promise<void> {
    const entry = resolveRef(this.currentRefMap, ref);
    if (!entry) throw new Error('ref_not_found');

    // Get element box model to find center coordinates
    const { model } = await this.sendCommand('DOM.getBoxModel', {
      backendNodeId: entry.backendNodeId,
    });
    const content = model.content;
    // content is [x1,y1, x2,y2, x3,y3, x4,y4] — take center
    const x = (content[0] + content[2] + content[4] + content[6]) / 4;
    const y = (content[1] + content[3] + content[5] + content[7]) / 4;

    await this.sendCommand('Input.dispatchMouseEvent', {
      type: 'mousePressed', x, y, button: 'left', clickCount: 1,
    });
    await this.sendCommand('Input.dispatchMouseEvent', {
      type: 'mouseReleased', x, y, button: 'left', clickCount: 1,
    });
  }

  async type(ref: string, text: string): Promise<void> {
    await this.click(ref); // Focus the element first
    for (const char of text) {
      await this.sendCommand('Input.dispatchKeyEvent', {
        type: 'keyDown', text: char,
      });
      await this.sendCommand('Input.dispatchKeyEvent', {
        type: 'keyUp', text: char,
      });
    }
  }

  async fill(ref: string, value: string): Promise<void> {
    const entry = resolveRef(this.currentRefMap, ref);
    if (!entry) throw new Error('ref_not_found');

    // Resolve to a DOM node and set value via JS
    const { nodeId } = await this.sendCommand('DOM.resolveNode', {
      backendNodeId: entry.backendNodeId,
    });
    const { object } = await this.sendCommand('DOM.resolveNode', {
      backendNodeId: entry.backendNodeId,
    });
    await this.sendCommand('Runtime.callFunctionOn', {
      objectId: object.objectId,
      functionDeclaration: `function(v) { this.value = v; this.dispatchEvent(new Event('input', {bubbles:true})); }`,
      arguments: [{ value }],
    });
  }

  async screenshot(fullPage = false): Promise<string> {
    const params: any = { format: 'png' };
    if (fullPage) {
      const { contentSize } = await this.sendCommand('Page.getLayoutMetrics');
      params.clip = {
        x: 0, y: 0,
        width: contentSize.width,
        height: contentSize.height,
        scale: 1,
      };
    }
    const { data } = await this.sendCommand('Page.captureScreenshot', params);
    return data; // base64
  }

  async getText(ref?: string): Promise<string> {
    if (ref) {
      const entry = resolveRef(this.currentRefMap, ref);
      if (!entry) throw new Error('ref_not_found');
      const { object } = await this.sendCommand('DOM.resolveNode', {
        backendNodeId: entry.backendNodeId,
      });
      const result = await this.sendCommand('Runtime.callFunctionOn', {
        objectId: object.objectId,
        functionDeclaration: 'function() { return this.innerText || this.textContent || ""; }',
        returnByValue: true,
      });
      return result.result.value || '';
    }
    // Full page text
    const result = await this.sendCommand('Runtime.evaluate', {
      expression: 'document.body.innerText',
      returnByValue: true,
    });
    return result.result.value || '';
  }

  async evaluate(js: string): Promise<any> {
    const result = await this.sendCommand('Runtime.evaluate', {
      expression: js,
      returnByValue: true,
      awaitPromise: true,
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.text || 'eval error');
    }
    return result.result.value;
  }

  async wait(ref?: string, timeout = 10000): Promise<void> {
    const start = Date.now();
    while (Date.now() - start < timeout) {
      if (ref) {
        const snap = await this.snapshot();
        if (resolveRef(this.currentRefMap, ref)) return;
      } else {
        // Wait for navigation
        await new Promise((r) => setTimeout(r, 200));
        return;
      }
      await new Promise((r) => setTimeout(r, 300));
    }
    throw new Error('timeout');
  }
}
```

- [ ] **Step 4: Run tests**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx vitest run tests/unit/cdp-bridge.test.ts 2>&1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/cdp-bridge.ts tests/unit/cdp-bridge.test.ts
git commit -m "feat: add CDP bridge for scriptable browser control"
```

---

### Task 5: Wire CDP Bridge into Main Process

**Files:**
- Modify: `src/main/ipc-handlers.ts`
- Modify: `src/main/index.ts`
- Modify: `src/preload/index.ts`
- Modify: `src/renderer/components/Browser/BrowserPane.tsx`

- [ ] **Step 1: Add CDP IPC handlers in ipc-handlers.ts**

In `src/main/ipc-handlers.ts`, add import at top:
```typescript
import { CDPBridge } from './cdp-bridge';
```

Create and export the bridge instance (after `notificationManager`):
```typescript
const cdpBridge = new CDPBridge();
```

Add handlers inside `registerIpcHandlers` (before the closing `}`):
```typescript
  // CDP Bridge handlers
  ipcMain.on(IPC_CHANNELS.CDP_ATTACH, (_event, webContentsId: number) => {
    cdpBridge.attach(webContentsId);
  });

  ipcMain.on(IPC_CHANNELS.CDP_DETACH, () => {
    cdpBridge.detach();
  });
```

Export `cdpBridge`:
```typescript
export { ptyManager, cdpBridge };
```

- [ ] **Step 2: Add CDP preload bridge**

In `src/preload/index.ts`, add to the `wmux` object inside `contextBridge.exposeInMainWorld`:

```typescript
  cdp: {
    attach: (webContentsId: number) =>
      ipcRenderer.send(IPC_CHANNELS.CDP_ATTACH, webContentsId),
    detach: () =>
      ipcRenderer.send(IPC_CHANNELS.CDP_DETACH),
  },
```

- [ ] **Step 3: Modify BrowserPane to send webContentsId**

In `src/renderer/components/Browser/BrowserPane.tsx`, add a `useEffect` to detect webview attachment and send the webContentsId. Add after the existing `useEffect` (around line 64):

```typescript
  // Send webview's webContentsId to main process for CDP bridge
  useEffect(() => {
    const wv = webviewRef.current;
    if (!wv) return;

    const onAttach = () => {
      // Electron webview exposes getWebContentsId() after dom-ready
      const wcId = wv.getWebContentsId?.();
      if (wcId && window.wmux?.cdp?.attach) {
        window.wmux.cdp.attach(wcId);
      }
    };

    wv.addEventListener('dom-ready', onAttach);
    return () => {
      wv.removeEventListener('dom-ready', onAttach);
      window.wmux?.cdp?.detach?.();
    };
  }, []);
```

- [ ] **Step 4: Wire CDP commands into pipe server**

In `src/main/index.ts`, import cdpBridge:
```typescript
import { registerIpcHandlers, cdpBridge } from './ipc-handlers';
```

Add CDP V2 handlers inside the existing `pipeServer.on('v2', ...)` switch (before the `default` case):

```typescript
      // Browser CDP commands
      case 'browser.navigate':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.navigate(request.params.url, request.params.timeout)
          .then(() => respond({ ok: true }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.snapshot':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.snapshot()
          .then((snap) => respond(snap))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.click':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.click(request.params.ref)
          .then(() => respond({ ok: true }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.type':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.type(request.params.ref, request.params.text)
          .then(() => respond({ ok: true }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.fill':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.fill(request.params.ref, request.params.value)
          .then(() => respond({ ok: true }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.screenshot':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.screenshot(request.params.fullPage)
          .then((data) => respond({ data }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.get_text':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.getText(request.params.ref)
          .then((text) => respond({ text }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.eval':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.evaluate(request.params.js)
          .then((result) => respond({ result }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.wait':
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        cdpBridge.wait(request.params.ref, request.params.timeout)
          .then(() => respond({ ok: true }))
          .catch((err) => respondError(-32000, err.message));
        break;
      case 'browser.batch': {
        if (!cdpBridge.isAttached) { respondError(-32000, 'Browser panel is not open'); break; }
        const results: any[] = [];
        (async () => {
          for (const cmd of request.params.commands || []) {
            try {
              const handler: any = {
                'browser.navigate': () => cdpBridge.navigate(cmd.params?.url, cmd.params?.timeout).then(() => ({ ok: true })),
                'browser.snapshot': () => cdpBridge.snapshot(),
                'browser.click': () => cdpBridge.click(cmd.params?.ref).then(() => ({ ok: true })),
                'browser.type': () => cdpBridge.type(cmd.params?.ref, cmd.params?.text).then(() => ({ ok: true })),
                'browser.fill': () => cdpBridge.fill(cmd.params?.ref, cmd.params?.value).then(() => ({ ok: true })),
                'browser.screenshot': () => cdpBridge.screenshot(cmd.params?.fullPage).then((d: string) => ({ data: d })),
                'browser.get_text': () => cdpBridge.getText(cmd.params?.ref).then((t: string) => ({ text: t })),
                'browser.eval': () => cdpBridge.evaluate(cmd.params?.js).then((r: any) => ({ result: r })),
                'browser.wait': () => cdpBridge.wait(cmd.params?.ref, cmd.params?.timeout).then(() => ({ ok: true })),
              }[cmd.method];
              if (!handler) {
                results.push({ error: { code: -32601, message: `Unknown: ${cmd.method}` } });
                break;
              }
              results.push({ result: await handler() });
            } catch (err: any) {
              results.push({ error: { code: -32000, message: err.message } });
              break; // Stop on first error
            }
          }
          respond({ results });
        })();
        break;
      }
```

- [ ] **Step 5: Verify compilation**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx tsc --noEmit --pretty 2>&1 | head -30`
Expected: No errors

- [ ] **Step 6: Commit**

```bash
git add src/main/ipc-handlers.ts src/main/index.ts src/preload/index.ts src/renderer/components/Browser/BrowserPane.tsx
git commit -m "feat: wire CDP bridge into pipe server with browser.* commands"
```

---

### Task 6: Agent Manager

**Files:**
- Create: `src/main/agent-manager.ts`
- Create: `tests/unit/agent-manager.test.ts`

- [ ] **Step 1: Write agent manager distribution tests**

Create `tests/unit/agent-manager.test.ts`:

```typescript
import { describe, it, expect } from 'vitest';
import { distributeAgents } from '../../src/main/agent-manager';

describe('Agent Manager', () => {
  describe('distributeAgents', () => {
    it('distributes evenly across panes', () => {
      const panes = [
        { paneId: 'pane-1', tabCount: 1 },
        { paneId: 'pane-2', tabCount: 1 },
        { paneId: 'pane-3', tabCount: 1 },
      ];
      const result = distributeAgents(3, panes);
      // 3 agents, 3 panes: 1 each
      expect(result).toEqual(['pane-1', 'pane-2', 'pane-3']);
    });

    it('fills least-loaded panes first', () => {
      const panes = [
        { paneId: 'pane-1', tabCount: 3 },
        { paneId: 'pane-2', tabCount: 1 },
        { paneId: 'pane-3', tabCount: 2 },
      ];
      const result = distributeAgents(3, panes);
      // Should fill pane-2 first (1 tab), then pane-3 (2 tabs), then pane-1 (3 tabs)
      expect(result).toEqual(['pane-2', 'pane-3', 'pane-1']);
    });

    it('round-robins when more agents than panes', () => {
      const panes = [
        { paneId: 'pane-1', tabCount: 1 },
        { paneId: 'pane-2', tabCount: 1 },
      ];
      const result = distributeAgents(5, panes);
      // 2 panes, 5 agents: alternating assignments (each pane gets picked when it's least loaded)
      expect(result.length).toBe(5);
      expect(result.filter((p) => p === 'pane-1').length).toBe(3); // 3 in pane-1
      expect(result.filter((p) => p === 'pane-2').length).toBe(2); // 2 in pane-2
    });

    it('handles single pane', () => {
      const panes = [{ paneId: 'pane-1', tabCount: 0 }];
      const result = distributeAgents(4, panes);
      expect(result).toEqual(['pane-1', 'pane-1', 'pane-1', 'pane-1']);
    });
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx vitest run tests/unit/agent-manager.test.ts 2>&1`
Expected: FAIL — module not found

- [ ] **Step 3: Create agent manager implementation**

Create `src/main/agent-manager.ts`:

```typescript
import { v4 as uuid } from 'uuid';
import { PtyManager } from './pty-manager';
import {
  AgentId,
  AgentInfo,
  AgentSpawnParams,
  PaneId,
  SurfaceId,
  WorkspaceId,
} from '../shared/types';

export interface PaneLoadInfo {
  paneId: string;
  tabCount: number;
}

/**
 * Distribute N agents across panes, filling least-loaded first.
 * Returns array of paneId assignments (one per agent).
 */
export function distributeAgents(
  count: number,
  panes: PaneLoadInfo[],
): string[] {
  // Clone so we can mutate tab counts
  const loads = panes.map((p) => ({ ...p }));
  const assignments: string[] = [];

  for (let i = 0; i < count; i++) {
    // Sort by tab count ascending — pick least loaded
    loads.sort((a, b) => a.tabCount - b.tabCount);
    const target = loads[0];
    assignments.push(target.paneId);
    target.tabCount++;
  }

  return assignments;
}

/**
 * AgentManager spawns agent processes as PTY sessions.
 *
 * IMPORTANT: It does NOT set up PTY data/exit forwarding to the renderer.
 * The caller (ipc-handlers.ts) must set up onData/onExit forwarding to
 * the BrowserWindow, just like PTY_CREATE handler does. See Task 7 Step 1
 * for the required forwarding setup.
 */
export class AgentManager {
  private agents = new Map<AgentId, AgentInfo>();
  private ptyManager: PtyManager;

  constructor(ptyManager: PtyManager) {
    this.ptyManager = ptyManager;
  }

  spawn(
    params: AgentSpawnParams & { paneId: PaneId; workspaceId: WorkspaceId },
  ): { agentId: AgentId; surfaceId: SurfaceId } {
    const agentId: AgentId = `agent-${uuid()}`;

    // Spawn a shell PTY — use cmd.exe as the shell host
    // Then write the agent command into it after a short delay
    const surfaceId = this.ptyManager.create({
      shell: 'cmd.exe',
      cwd: params.cwd || process.env.USERPROFILE || 'C:\\',
      env: {
        ...(params.env || {}),
        WMUX_AGENT_ID: agentId,
        WMUX_AGENT_LABEL: params.label,
      },
    });

    // Send the actual command to the shell after PTY is ready
    setTimeout(() => {
      this.ptyManager.write(surfaceId, params.cmd + '\r');
    }, 500);

    const info: AgentInfo = {
      agentId,
      surfaceId,
      paneId: params.paneId,
      workspaceId: params.workspaceId,
      label: params.label,
      cmd: params.cmd,
      status: 'running',
      spawnTime: Date.now(),
      pid: this.ptyManager.getPid(surfaceId),
    };

    this.agents.set(agentId, info);

    // Track exit
    this.ptyManager.onExit(surfaceId, (code) => {
      const agent = this.agents.get(agentId);
      if (agent) {
        agent.status = 'exited';
        agent.exitCode = code;
      }
    });

    return { agentId, surfaceId };
  }

  getStatus(agentId: AgentId): AgentInfo | undefined {
    return this.agents.get(agentId);
  }

  list(workspaceId?: WorkspaceId): AgentInfo[] {
    const all = Array.from(this.agents.values());
    if (workspaceId) return all.filter((a) => a.workspaceId === workspaceId);
    return all;
  }

  kill(agentId: AgentId): boolean {
    const agent = this.agents.get(agentId);
    if (!agent) return false;
    this.ptyManager.kill(agent.surfaceId);
    agent.status = 'exited';
    agent.exitCode = -1;
    return true;
  }

  getAgentBySurface(surfaceId: SurfaceId): AgentInfo | undefined {
    for (const agent of this.agents.values()) {
      if (agent.surfaceId === surfaceId) return agent;
    }
    return undefined;
  }
}
```

- [ ] **Step 4: Run tests**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx vitest run tests/unit/agent-manager.test.ts 2>&1`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/agent-manager.ts tests/unit/agent-manager.test.ts
git commit -m "feat: add agent manager with distribution algorithm"
```

---

### Task 7: Wire Agent Manager into Main Process

**Files:**
- Modify: `src/main/ipc-handlers.ts`
- Modify: `src/main/index.ts`
- Modify: `src/preload/index.ts`

- [ ] **Step 1: Create agent manager in ipc-handlers.ts with PTY data forwarding**

In `src/main/ipc-handlers.ts`, import AgentManager:
```typescript
import { AgentManager } from './agent-manager';
```

Create instance (after `cdpBridge`):
```typescript
const agentManager = new AgentManager(ptyManager);
```

Add IPC handlers inside `registerIpcHandlers` (after CDP handlers):
```typescript
  // Agent handlers
  ipcMain.handle(IPC_CHANNELS.AGENT_LIST, async (_event, workspaceId?: string) => {
    return agentManager.list(workspaceId as any);
  });

  ipcMain.handle(IPC_CHANNELS.AGENT_STATUS, async (_event, agentId: string) => {
    return agentManager.getStatus(agentId as any);
  });
```

Add a helper function to set up PTY forwarding for agent-spawned terminals.
**This is critical — without this, agent terminal data never reaches xterm.js in the renderer:**
```typescript
export function setupAgentPtyForwarding(surfaceId: string, window: BrowserWindow): void {
  // Forward PTY data to renderer (same pattern as PTY_CREATE handler at line 33-42)
  ptyManager.onData(surfaceId as any, (data) => {
    if (window && !window.isDestroyed()) {
      window.webContents.send(IPC_CHANNELS.PTY_DATA, surfaceId, data);
    }
  });
  ptyManager.onExit(surfaceId as any, (code) => {
    if (window && !window.isDestroyed()) {
      window.webContents.send(IPC_CHANNELS.PTY_EXIT, surfaceId, code);
    }
  });
}
```

Export `agentManager`:
```typescript
export { ptyManager, cdpBridge, agentManager, setupAgentPtyForwarding };
```

- [ ] **Step 2: Add agent preload bridge**

In `src/preload/index.ts`, add to the `wmux` object:
```typescript
  agent: {
    list: (workspaceId?: string) =>
      ipcRenderer.invoke(IPC_CHANNELS.AGENT_LIST, workspaceId),
    status: (agentId: string) =>
      ipcRenderer.invoke(IPC_CHANNELS.AGENT_STATUS, agentId),
    onUpdate: (callback: (agent: any) => void) => {
      const handler = (_event: any, agent: any) => callback(agent);
      ipcRenderer.on(IPC_CHANNELS.AGENT_UPDATE, handler);
      return () => ipcRenderer.removeListener(IPC_CHANNELS.AGENT_UPDATE, handler);
    },
  },
```

- [ ] **Step 3: Wire agent commands into pipe server**

In `src/main/index.ts`, import agentManager + PTY forwarding:
```typescript
import { registerIpcHandlers, cdpBridge, agentManager, setupAgentPtyForwarding } from './ipc-handlers';
```

Import distributeAgents:
```typescript
import { distributeAgents } from './agent-manager';
```

**IMPORTANT**: All agent V2 handlers use an IIFE `(async () => { ... })()` wrapper for async operations (same pattern as `browser.batch`). The V2 event handler is synchronous — we cannot use `await` directly in the switch case.

Add agent V2 handlers inside the `pipeServer.on('v2', ...)` switch (before `default`):

```typescript
      // Agent commands — use IIFE for async operations (pipe handler is sync)
      case 'agent.spawn': {
        (async () => {
          try {
            const params = request.params;
            // Resolve workspace via renderer query
            let workspaceId = params.workspaceId;
            if (!workspaceId) {
              const wins = BrowserWindow.getAllWindows();
              if (wins.length > 0) {
                workspaceId = await wins[0].webContents.executeJavaScript(
                  'window.__wmux_getActiveWorkspaceId?.()'
                );
              }
            }
            if (!workspaceId) { respondError(-32000, 'No active workspace'); return; }

            // Resolve pane
            let paneId = params.paneId;
            if (!paneId) {
              const paneLoads = await BrowserWindow.getAllWindows()[0]?.webContents.executeJavaScript(
                'window.__wmux_getPaneLoads?.()'
              );
              if (paneLoads && paneLoads.length > 0) {
                paneId = distributeAgents(1, paneLoads)[0];
              }
            }
            if (!paneId) { respondError(-32000, 'No panes available'); return; }

            // Spawn the agent (synchronous — creates PTY)
            const result = agentManager.spawn({
              cmd: params.cmd,
              label: params.label,
              cwd: params.cwd,
              env: params.env,
              paneId,
              workspaceId,
            });

            // Set up PTY data forwarding to renderer (CRITICAL — without this, agent terminals are blank)
            const win = BrowserWindow.getAllWindows()[0];
            if (win && !win.isDestroyed()) {
              setupAgentPtyForwarding(result.surfaceId, win);
            }

            // Notify renderer to add surface to pane + store agent metadata
            BrowserWindow.getAllWindows().forEach(w => {
              if (!w.isDestroyed()) {
                w.webContents.send(IPC_CHANNELS.AGENT_UPDATE, {
                  type: 'spawned',
                  ...result,
                  paneId,
                  workspaceId,
                  label: params.label,
                });
              }
            });

            respond(result);
          } catch (err: any) {
            respondError(-32000, err.message);
          }
        })();
        break;
      }

      case 'agent.spawn_batch': {
        (async () => {
          try {
            const { agents: agentParams, strategy = 'distribute', workspaceId: wsId } = request.params;

            let workspaceId = wsId;
            if (!workspaceId) {
              const wins = BrowserWindow.getAllWindows();
              if (wins.length > 0) {
                workspaceId = await wins[0].webContents.executeJavaScript(
                  'window.__wmux_getActiveWorkspaceId?.()'
                );
              }
            }
            if (!workspaceId) { respondError(-32000, 'No active workspace'); return; }

            const paneLoads = await BrowserWindow.getAllWindows()[0]?.webContents.executeJavaScript(
              'window.__wmux_getPaneLoads?.()'
            ) || [];
            if (paneLoads.length === 0) { respondError(-32000, 'No panes available'); return; }

            let assignments: string[];
            if (strategy === 'distribute') {
              assignments = distributeAgents(agentParams.length, paneLoads);
            } else if (strategy === 'stack') {
              const sorted = [...paneLoads].sort((a: any, b: any) => a.tabCount - b.tabCount);
              assignments = agentParams.map(() => sorted[0].paneId);
            } else {
              // 'split' — deferred to future iteration (requires renderer-side tree splitting).
              // Falls back to distribute with a console warning.
              console.warn('[wmux] split strategy not yet implemented, falling back to distribute');
              assignments = distributeAgents(agentParams.length, paneLoads);
            }

            const win = BrowserWindow.getAllWindows()[0];
            const results: any[] = [];
            for (let i = 0; i < agentParams.length; i++) {
              try {
                const result = agentManager.spawn({
                  ...agentParams[i],
                  paneId: assignments[i] as any,
                  workspaceId,
                });

                // Set up PTY forwarding for each agent
                if (win && !win.isDestroyed()) {
                  setupAgentPtyForwarding(result.surfaceId, win);
                }

                BrowserWindow.getAllWindows().forEach(w => {
                  if (!w.isDestroyed()) {
                    w.webContents.send(IPC_CHANNELS.AGENT_UPDATE, {
                      type: 'spawned',
                      ...result,
                      paneId: assignments[i],
                      workspaceId,
                      label: agentParams[i].label,
                    });
                  }
                });

                results.push(result);
              } catch (err: any) {
                results.push({ error: err.message });
              }
            }
            respond({ agents: results });
          } catch (err: any) {
            respondError(-32000, err.message);
          }
        })();
        break;
      }

      case 'agent.status': {
        const info = agentManager.getStatus(request.params.agentId);
        if (!info) { respondError(-32000, 'Agent not found'); break; }
        respond(info);
        break;
      }

      case 'agent.list': {
        respond({ agents: agentManager.list(request.params.workspaceId) });
        break;
      }

      case 'agent.kill': {
        const killed = agentManager.kill(request.params.agentId);
        if (!killed) { respondError(-32000, 'Agent not found'); break; }
        respond({ ok: true });
        break;
      }
```

- [ ] **Step 4: Create Zustand agent slice for reactive agent metadata**

Create `src/renderer/store/agent-slice.ts`:

```typescript
import { StateCreator } from 'zustand';
import { SurfaceId } from '../../shared/types';

export interface AgentMeta {
  agentId: string;
  label: string;
  status?: 'running' | 'exited';
  exitCode?: number;
}

export interface AgentSlice {
  agentMeta: Map<SurfaceId, AgentMeta>;
  setAgentMeta: (surfaceId: SurfaceId, meta: AgentMeta) => void;
  removeAgentMeta: (surfaceId: SurfaceId) => void;
  getAgentMeta: (surfaceId: SurfaceId) => AgentMeta | undefined;
}

export const createAgentSlice: StateCreator<AgentSlice, [], [], AgentSlice> = (set, get) => ({
  agentMeta: new Map(),

  setAgentMeta(surfaceId: SurfaceId, meta: AgentMeta): void {
    set((state) => {
      const newMap = new Map(state.agentMeta);
      newMap.set(surfaceId, meta);
      return { agentMeta: newMap };
    });
  },

  removeAgentMeta(surfaceId: SurfaceId): void {
    set((state) => {
      const newMap = new Map(state.agentMeta);
      newMap.delete(surfaceId);
      return { agentMeta: newMap };
    });
  },

  getAgentMeta(surfaceId: SurfaceId): AgentMeta | undefined {
    return get().agentMeta.get(surfaceId);
  },
});
```

Update `src/renderer/store/index.ts` to include the agent slice:

```typescript
import { create } from 'zustand';
import { WorkspaceSlice, createWorkspaceSlice } from './workspace-slice';
import { SettingsSlice, createSettingsSlice } from './settings-slice';
import { NotificationSlice, createNotificationSlice } from './notification-slice';
import { SurfaceSlice, createSurfaceSlice } from './surface-slice';
import { AgentSlice, createAgentSlice } from './agent-slice';

export type WmuxStore = WorkspaceSlice & SettingsSlice & NotificationSlice & SurfaceSlice & AgentSlice;

export const useStore = create<WmuxStore>()((...args) => ({
  ...createWorkspaceSlice(...args),
  ...createSettingsSlice(...args),
  ...createNotificationSlice(...args),
  ...createSurfaceSlice(...args),
  ...createAgentSlice(...args),
}));
```

- [ ] **Step 5: Expose renderer helpers for agent system**

In `src/renderer/App.tsx`, add at the top of the `App` component (after state declarations), expose helper functions on window for main process to query:

```typescript
  // Expose helpers for main process agent queries
  useEffect(() => {
    (window as any).__wmux_getActiveWorkspaceId = () => {
      return useStore.getState().activeWorkspaceId;
    };
    (window as any).__wmux_getPaneLoads = () => {
      const state = useStore.getState();
      const ws = state.workspaces.find((w) => w.id === state.activeWorkspaceId);
      if (!ws) return [];
      const paneIds = getAllPaneIds(ws.splitTree);
      return paneIds.map((pid) => {
        const leaf = findLeafFromTree(ws.splitTree, pid);
        return { paneId: pid, tabCount: leaf ? leaf.surfaces.length : 0 };
      });
    };
    return () => {
      delete (window as any).__wmux_getActiveWorkspaceId;
      delete (window as any).__wmux_getPaneLoads;
    };
  }, []);
```

Also add the helper to find a leaf from tree (at the top of the file, after `getAllSurfaces`):
```typescript
function findLeafFromTree(node: SplitNode, paneId: PaneId): (SplitNode & { type: 'leaf' }) | null {
  if (node.type === 'leaf') return node.paneId === paneId ? node : null;
  return findLeafFromTree(node.children[0], paneId) || findLeafFromTree(node.children[1], paneId);
}
```

- [ ] **Step 6: Listen for agent updates in App.tsx to add surfaces to panes**

In `src/renderer/App.tsx`, add `setAgentMeta` to the destructured `useStore()` call:
```typescript
    setAgentMeta,
```

Add a `useEffect` for agent spawn events:

```typescript
  // Listen for agent spawn events from main process
  useEffect(() => {
    if (!window.wmux?.agent?.onUpdate) return;
    const unsub = window.wmux.agent.onUpdate((event: any) => {
      if (event.type === 'spawned') {
        const { surfaceId, paneId, workspaceId, label } = event;
        const state = useStore.getState();
        const ws = state.workspaces.find((w) => w.id === workspaceId);
        if (!ws) return;

        // Add the agent's terminal surface to the target pane in the split tree
        const addSurfaceToLeaf = (node: SplitNode): SplitNode => {
          if (node.type === 'leaf' && node.paneId === paneId) {
            return {
              ...node,
              surfaces: [...node.surfaces, { id: surfaceId, type: 'terminal' }],
              activeSurfaceIndex: node.surfaces.length, // Focus the new tab
            };
          }
          if (node.type === 'branch') {
            return {
              ...node,
              children: [
                addSurfaceToLeaf(node.children[0]),
                addSurfaceToLeaf(node.children[1]),
              ] as [SplitNode, SplitNode],
            };
          }
          return node;
        };

        state.updateSplitTree(workspaceId, addSurfaceToLeaf(ws.splitTree));

        // Store agent metadata in Zustand (reactive — triggers tab re-render)
        setAgentMeta(surfaceId, { agentId: event.agentId, label, status: 'running' });
      }
    });
    return unsub;
  }, [setAgentMeta]);
```

- [ ] **Step 6: Verify compilation**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx tsc --noEmit --pretty 2>&1 | head -30`
Expected: No errors

- [ ] **Step 7: Commit**

```bash
git add src/main/ipc-handlers.ts src/main/index.ts src/preload/index.ts src/renderer/App.tsx
git commit -m "feat: wire agent manager into pipe server with spawn/batch/status/kill commands"
```

---

### Task 8: Agent Tab Visual Distinction

**Files:**
- Modify: `src/renderer/components/SplitPane/SurfaceTabBar.tsx`

- [ ] **Step 1: Add agent metadata lookup to SurfaceTabBar**

In `src/renderer/components/SplitPane/SurfaceTabBar.tsx`, modify the `surfaceIcon` and `surfaceLabel` functions and component to support agent tabs:

Replace the `surfaceIcon` function:
```typescript
function surfaceIcon(type: string, isAgent: boolean): string {
  if (isAgent) return '>_';
  switch (type) {
    case 'terminal': return '>';
    case 'browser': return '◎';
    case 'markdown': return '¶';
    default: return '○';
  }
}
```

Replace the `surfaceLabel` function:
```typescript
function surfaceLabel(surface: SurfaceRef, agentLabel?: string): string {
  if (agentLabel) return agentLabel;
  switch (surface.type) {
    case 'terminal': return 'Terminal';
    case 'browser': return 'Browser';
    case 'markdown': return 'Markdown';
    default: return 'Tab';
  }
}
```

Add import for the store at the top of the file:
```typescript
import { useStore } from '../../store';
```

In the component body, look up agent metadata from Zustand:
```typescript
  // Look up agent metadata from Zustand store (reactive — re-renders when agents are added)
  const agentMeta = useStore((state) => state.agentMeta);
  const getAgentMeta = (surfaceId: string) => agentMeta.get(surfaceId as any);
```

Update the tab rendering (inside the `surfaces.map` callback):
```typescript
          {surfaces.map((surface, index) => {
            const isActive = index === activeSurfaceIndex;
            const agentMeta = getAgentMeta(surface.id);
            const isAgent = !!agentMeta;
            return (
              <div
                key={surface.id}
                className={[
                  'surface-tab',
                  isActive ? 'surface-tab--active' : '',
                  isAgent ? 'surface-tab--agent' : '',
                  dragOverIndex === index ? 'surface-tab--drag-over' : '',
                ].filter(Boolean).join(' ')}
                // ... rest of props unchanged
              >
                <span className="surface-tab__icon">{surfaceIcon(surface.type, isAgent)}</span>
                <span className="surface-tab__label">{surfaceLabel(surface, agentMeta?.label)}</span>
                {/* ... close button unchanged */}
              </div>
            );
          })}
```

- [ ] **Step 2: Add agent tab styles**

Add to `src/renderer/styles/split-pane.css` (or wherever surface-tab styles live):

```css
.surface-tab--agent .surface-tab__icon {
  color: #0091FF;
}

.surface-tab--agent .surface-tab__label {
  color: rgba(0, 145, 255, 0.8);
}
```

- [ ] **Step 3: Commit**

```bash
git add src/renderer/components/SplitPane/SurfaceTabBar.tsx
git commit -m "feat: add agent tab visual distinction with blue icon and label"
```

---

### Task 9: Sidebar Metadata Additions

**Files:**
- Modify: `src/renderer/components/Sidebar/WorkspaceRow.tsx`
- Modify: `src/renderer/styles/sidebar.css`

- [ ] **Step 1: Add shell state indicator and agent count to WorkspaceRow**

In `src/renderer/components/Sidebar/WorkspaceRow.tsx`, add before the title in the header section (around line 81, inside `workspace-row__header`):

```tsx
        {/* Shell state indicator */}
        <span
          className={`workspace-row__state-dot ${
            workspace.shellState === 'running' ? 'workspace-row__state-dot--running' : ''
          }`}
          title={workspace.shellState === 'running' ? 'Command running' : 'Idle'}
        />
```

Add agent count display. Add a new prop and read it. First, update the interface:
```typescript
interface WorkspaceRowProps {
  // ... existing props ...
  agentCount?: number;
}
```

Add `agentCount = 0` to destructuring.

Then add in the metadata section (after ports display, before the closing `</div>` of metadata):
```tsx
          {/* Agent count */}
          {agentCount > 0 && (
            <div className="workspace-row__meta-line workspace-row__agents">
              {agentCount} agent{agentCount !== 1 ? 's' : ''}
            </div>
          )}
```

Update the metadata visibility condition to also check `agentCount`:
```tsx
      {(workspace.notificationText ||
        workspace.gitBranch ||
        workspace.cwd ||
        workspace.prNumber != null ||
        portsStr ||
        agentCount > 0) && (
```

- [ ] **Step 2: Add shell state dot and agent count styles**

Add to `src/renderer/styles/sidebar.css`:

```css
.workspace-row__state-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.2);
  flex-shrink: 0;
  margin-right: 4px;
}

.workspace-row__state-dot--running {
  background: #4caf50;
  animation: pulse-dot 1.5s ease-in-out infinite;
}

@keyframes pulse-dot {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

.workspace-row__agents {
  color: rgba(0, 145, 255, 0.6);
}
```

- [ ] **Step 3: Pass agent count from Sidebar to WorkspaceRow**

In the Sidebar component that renders WorkspaceRow, pass the agent count. The Sidebar needs to query the agent list. In `src/renderer/components/Sidebar/Sidebar.tsx`, add:

```typescript
  // Get agent counts per workspace
  const [agentCounts, setAgentCounts] = useState<Record<string, number>>({});

  useEffect(() => {
    const interval = setInterval(async () => {
      if (!window.wmux?.agent?.list) return;
      try {
        const agents = await window.wmux.agent.list();
        const counts: Record<string, number> = {};
        for (const agent of agents || []) {
          if (agent.status === 'running') {
            counts[agent.workspaceId] = (counts[agent.workspaceId] || 0) + 1;
          }
        }
        setAgentCounts(counts);
      } catch {}
    }, 3000);
    return () => clearInterval(interval);
  }, []);
```

And pass to each `<WorkspaceRow>`:
```tsx
  agentCount={agentCounts[workspace.id] || 0}
```

- [ ] **Step 4: Commit**

```bash
git add src/renderer/components/Sidebar/WorkspaceRow.tsx src/renderer/components/Sidebar/Sidebar.tsx src/renderer/styles/sidebar.css
git commit -m "feat: add shell state indicator and agent count to sidebar"
```

---

### Task 10: CLI Commands

**Files:**
- Modify: `src/cli/wmux.ts`

- [ ] **Step 1: Update browser CLI commands to use ref-based API**

In `src/cli/wmux.ts`, replace the `case 'browser'` section (lines 119-133):

```typescript
      case 'browser': {
        const sub = args[1];
        switch (sub) {
          case 'open': console.log(JSON.stringify(await sendV2('browser.navigate', { url: args[2] }), null, 2)); break;
          case 'snapshot': console.log(JSON.stringify(await sendV2('browser.snapshot'), null, 2)); break;
          case 'click': console.log(JSON.stringify(await sendV2('browser.click', { ref: args[2] }), null, 2)); break;
          case 'type': console.log(JSON.stringify(await sendV2('browser.type', { ref: args[2], text: args.slice(3).join(' ') }), null, 2)); break;
          case 'fill': console.log(JSON.stringify(await sendV2('browser.fill', { ref: args[2], value: args.slice(3).join(' ') }), null, 2)); break;
          case 'screenshot': console.log(JSON.stringify(await sendV2('browser.screenshot', { fullPage: args.includes('--full') }), null, 2)); break;
          case 'get-text': console.log(JSON.stringify(await sendV2('browser.get_text', { ref: args[2] }), null, 2)); break;
          case 'eval': console.log(JSON.stringify(await sendV2('browser.eval', { js: args.slice(2).join(' ') }), null, 2)); break;
          case 'wait': console.log(JSON.stringify(await sendV2('browser.wait', { ref: args[2], timeout: parseInt(args[3]) || undefined }), null, 2)); break;
          case 'back': console.log(JSON.stringify(await sendV2('browser.back'), null, 2)); break;
          case 'forward': console.log(JSON.stringify(await sendV2('browser.forward'), null, 2)); break;
          case 'reload': console.log(JSON.stringify(await sendV2('browser.reload'), null, 2)); break;
          default: console.error(`Unknown browser command: ${sub}`); process.exit(1);
        }
        break;
      }
```

- [ ] **Step 2: Add agent CLI commands**

Add new cases after the `browser` case:

```typescript
      // Agent
      case 'agent': {
        const sub = args[1];
        switch (sub) {
          case 'spawn': {
            const params: any = {};
            for (let i = 2; i < args.length; i += 2) {
              if (args[i] === '--cmd') params.cmd = args[i + 1];
              if (args[i] === '--label') params.label = args[i + 1];
              if (args[i] === '--cwd') params.cwd = args[i + 1];
              if (args[i] === '--pane') params.paneId = args[i + 1];
              if (args[i] === '--workspace') params.workspaceId = args[i + 1];
            }
            if (!params.cmd) { console.error('--cmd is required'); process.exit(1); }
            if (!params.label) params.label = params.cmd.split(/\s+/)[0];
            console.log(JSON.stringify(await sendV2('agent.spawn', params), null, 2));
            break;
          }
          case 'spawn-batch': {
            // Read JSON from stdin or --json arg
            const jsonIdx = args.indexOf('--json');
            if (jsonIdx === -1) { console.error('Usage: wmux agent spawn-batch --json \'[...]\''); process.exit(1); }
            const json = args[jsonIdx + 1];
            const parsed = JSON.parse(json);
            const strategy = args.find((a, i) => args[i - 1] === '--strategy') || 'distribute';
            console.log(JSON.stringify(await sendV2('agent.spawn_batch', { agents: parsed, strategy }), null, 2));
            break;
          }
          case 'status': console.log(JSON.stringify(await sendV2('agent.status', { agentId: args[2] }), null, 2)); break;
          case 'list': console.log(JSON.stringify(await sendV2('agent.list', { workspaceId: args.find((a, i) => args[i - 1] === '--workspace') }), null, 2)); break;
          case 'kill': console.log(JSON.stringify(await sendV2('agent.kill', { agentId: args[2] }), null, 2)); break;
          default: console.error(`Unknown agent command: ${sub}`); process.exit(1);
        }
        break;
      }
```

- [ ] **Step 3: Update usage text**

Update `printUsage()` to include new commands:

```typescript
function printUsage() {
  console.log(`wmux CLI — Windows terminal multiplexer

Usage: wmux <command> [options]

System:     ping, identify, capabilities, list-windows, focus-window <id>
Workspace:  new-workspace, close-workspace, select-workspace, rename-workspace, list-workspaces
Surface:    new-surface, close-surface, focus-surface, list-surfaces
Pane:       split, close-pane, focus-pane, zoom-pane, list-panes, tree
Terminal:   send <text>, send-key <key>, read-screen, trigger-flash
Browser:    browser open|snapshot|click|type|fill|screenshot|get-text|eval|wait|back|forward|reload
Agent:      agent spawn|spawn-batch|status|list|kill
Markdown:   markdown set <id> --content <text> | --file <path>
Notify:     notify <text>, list-notifications, clear-notifications
Sidebar:    set-status, set-progress, log, sidebar-state
`);
}
```

- [ ] **Step 4: Commit**

```bash
git add src/cli/wmux.ts
git commit -m "feat: add browser ref-based and agent CLI commands"
```

---

### Task 11: Run Full Test Suite

**Files:**
- All test files

- [ ] **Step 1: Run all tests**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx vitest run 2>&1`
Expected: All tests pass (existing + new)

- [ ] **Step 2: Fix any test failures**

If any tests fail, fix them and re-run.

- [ ] **Step 3: Run TypeScript compiler**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npx tsc --noEmit --pretty 2>&1`
Expected: No errors

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat: wmux v2 — notification center, scriptable browser API, sub-agent spawning"
```

---

## Summary

| Task | Feature | New Files | Modified Files |
|------|---------|-----------|----------------|
| 1 | Types & IPC | — | types.ts |
| 2 | Notification UI | NotificationBell.tsx, NotificationPanel.tsx, notification-panel.css | — |
| 3 | Wire Notifications | — | Titlebar.tsx, App.tsx, useKeyboardShortcuts.ts, notification-slice.ts |
| 4 | CDP Bridge | cdp-bridge.ts, cdp-bridge.test.ts | — |
| 5 | Wire CDP | — | ipc-handlers.ts, index.ts, preload/index.ts, BrowserPane.tsx |
| 6 | Agent Manager | agent-manager.ts, agent-manager.test.ts | — |
| 7 | Wire Agents | — | ipc-handlers.ts, index.ts, preload/index.ts, App.tsx |
| 8 | Agent Tab UI | — | SurfaceTabBar.tsx |
| 9 | Sidebar Metadata | — | WorkspaceRow.tsx, Sidebar.tsx, sidebar.css |
| 10 | CLI Commands | — | wmux.ts |
| 11 | Test & Verify | — | all |
