# Saved Sessions — Design Spec

> **Historical design record:** This page captures earlier design or implementation work and is not current guidance. Verify behavior against the [documentation index](../../README.md), the [developer guide](../../../CLAUDE.md), and the current Rust source.

> Save and restore workspace layouts with terminal CWDs, split configurations, and browser URLs.

## Problem

Every time wmux starts, users get a blank workspace. They have to manually `cd` into project directories, split panes, and navigate the browser to their dev server. For users with consistent workflows (frontend + backend + browser on localhost), this is repetitive friction.

## Design

### Data Model

```typescript
interface SavedSession {
  name: string;                    // User-chosen name
  savedAt: number;                 // Unix timestamp
  workspaces: Array<{
    title: string;                 // Workspace title
    customColor?: string;          // Workspace color
    shell: string;                 // Shell executable
    cwd: string;                   // Working directory
    splitTree: SplitNode;          // Split layout
  }>;
  browserUrl?: string;             // Browser panel URL
  sidebarWidth: number;            // Sidebar width
}
```

**Saved**: layout, CWDs, titles, colors, shell, browser URL, sidebar width.
**Not saved**: terminal content, git state, notifications (these are runtime state).

### Storage

- Location: `%APPDATA%/wmux/sessions/saved/`
- One JSON file per session: `<sanitized-name>.json`
- Last used session tracked in `%APPDATA%/wmux/sessions/last-session.txt` (just the filename)

### UX Flow

**Save (Ctrl+Shift+S or sidebar button):**
1. User clicks "💾" button in sidebar footer (or `Ctrl+Shift+S`)
2. Inline input appears asking for a name
3. Enter → saved to disk, toast notification "Session saved"
4. If name already exists → overwrites silently

**Load (sidebar button or command palette):**
1. User clicks "📂" button in sidebar footer
2. Dropdown appears with saved sessions list
3. Each entry: name + workspace count + relative date ("2h ago", "yesterday")
4. Click → replaces all current workspaces with saved ones
5. Hover → "✕" delete button appears
6. Also available via command palette: `Ctrl+Shift+P` → "Load session..."

**Startup:**
1. wmux reads `last-session.txt` to find the last used session
2. If found → auto-loads that session (recreates workspaces with saved CWDs, splits, browser URL)
3. If not found → starts with default blank workspace

**Load behavior:**
- Kills all existing PTYs
- Clears all workspaces
- Creates new workspaces from saved data
- Each terminal starts in the saved CWD
- Browser navigates to saved URL (if any)
- Sidebar width restored

### IPC Channels

```typescript
SESSION_SAVE_NAMED: 'session:save-named'      // renderer → main (save)
SESSION_LOAD_NAMED: 'session:load-named'       // renderer → main (load) → returns SessionData
SESSION_LIST_NAMED: 'session:list-named'       // renderer → main → returns SavedSession[]
SESSION_DELETE_NAMED: 'session:delete-named'   // renderer → main (delete)
```

### Files Changed

| File | Action | Description |
|------|--------|-------------|
| `src/main/session-persistence.ts` | Modified | `saveNamedSession()`, `loadNamedSession()`, `listNamedSessions()`, `deleteNamedSession()`, `getLastSessionName()`, `setLastSessionName()` |
| `src/main/ipc-handlers.ts` | Modified | IPC handlers for named session CRUD |
| `src/main/index.ts` | Modified | Auto-load last session on startup |
| `src/shared/types.ts` | Modified | `SavedSession` type, new IPC channel names |
| `src/preload/index.ts` | Modified | Expose `session.save/load/list/delete` |
| `src/renderer/App.tsx` | Modified | Auto-load on mount, `replaceAllWorkspaces()` handler |
| `src/renderer/store/workspace-slice.ts` | Modified | `replaceAllWorkspaces()` action |
| `src/renderer/components/Sidebar/Sidebar.tsx` | Modified | Save/Load buttons in footer |
| `src/renderer/components/Sidebar/SessionMenu.tsx` | New | Dropdown listing saved sessions |
| `src/renderer/styles/sidebar.css` | Modified | Styles for session buttons and menu |
| `src/renderer/hooks/useKeyboardShortcuts.ts` | Modified | `Ctrl+Shift+S` binding |

### What Does NOT Change

- Auto-save every 30s (existing, for crash recovery)
- Session restore is additive to the existing auto-save (they coexist)
- Command palette, settings, notifications — unchanged
- Agent spawning, browser CDP proxy, hooks — unchanged
