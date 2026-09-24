# wmux Implementation Plan

> **Historical design record:** This page captures earlier design or implementation work and is not current guidance. Verify behavior against the [documentation index](../../README.md), the [developer guide](../../../CLAUDE.md), and the current Rust source.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build wmux, a Windows terminal multiplexer for AI agents — a 1:1 port of cmux (macOS) using Electron + React + TypeScript + xterm.js.

**Architecture:** Two-process Electron app. Main process owns PTY management, named pipe server, config loading, notifications, and session persistence. Renderer process runs a React/Zustand app with xterm.js terminals, split panes, and a sidebar. Communication via typed contextBridge IPC.

**Tech Stack:** Electron 33+, React 19, TypeScript 5.5, xterm.js 5, node-pty, Zustand, Vite, electron-builder

**Spec:** `docs/superpowers/specs/2026-03-25-wmux-design.md`

---

## Phase 1: Project Scaffold & Electron Shell

### Task 1: Initialize Electron + React + Vite project

**Files:**
- Create: `package.json`
- Create: `tsconfig.json`
- Create: `tsconfig.node.json`
- Create: `electron-builder.json`
- Create: `vite.config.ts`
- Create: `.gitignore`
- Create: `src/main/index.ts`
- Create: `src/preload/index.ts`
- Create: `src/renderer/index.html`
- Create: `src/renderer/index.tsx`
- Create: `src/renderer/App.tsx`
- Create: `src/renderer/styles/global.css`

- [ ] **Step 1: Create package.json with all dependencies**

```json
{
  "name": "wmux",
  "version": "0.1.0",
  "description": "Windows terminal multiplexer for AI agents",
  "main": "dist/main/index.js",
  "scripts": {
    "dev": "concurrently \"vite\" \"wait-on http://localhost:5173 && electron .\"",
    "build": "tsc && vite build && electron-builder",
    "build:main": "tsc -p tsconfig.node.json",
    "build:renderer": "vite build",
    "test": "vitest run",
    "test:watch": "vitest",
    "lint": "eslint src/"
  },
  "dependencies": {
    "electron-updater": "^6.3.0",
    "marked": "^15.0.0",
    "node-pty": "^1.0.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "uuid": "^11.0.0",
    "zustand": "^5.0.0",
    "@xterm/addon-fit": "^0.10.0",
    "@xterm/addon-image": "^0.8.0",
    "@xterm/addon-search": "^0.15.0",
    "@xterm/addon-web-links": "^0.11.0",
    "@xterm/addon-webgl": "^0.18.0",
    "@xterm/addon-unicode11": "^0.8.0",
    "@xterm/xterm": "^5.5.0"
  },
  "devDependencies": {
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@types/uuid": "^10.0.0",
    "concurrently": "^9.0.0",
    "electron": "^33.0.0",
    "electron-builder": "^25.0.0",
    "eslint": "^9.0.0",
    "typescript": "^5.5.0",
    "vite": "^6.0.0",
    "vitest": "^2.0.0",
    "wait-on": "^8.0.0",
    "@vitejs/plugin-react": "^4.0.0"
  }
}
```

- [ ] **Step 2: Create tsconfig.json (renderer)**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "baseUrl": ".",
    "paths": {
      "@main/*": ["src/main/*"],
      "@renderer/*": ["src/renderer/*"],
      "@shared/*": ["src/shared/*"]
    }
  },
  "include": ["src/renderer/**/*", "src/shared/**/*"]
}
```

- [ ] **Step 3: Create tsconfig.node.json (main + preload)**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "CommonJS",
    "moduleResolution": "node",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "outDir": "dist",
    "rootDir": "src",
    "declaration": true
  },
  "include": ["src/main/**/*", "src/preload/**/*", "src/shared/**/*"]
}
```

- [ ] **Step 4: Create vite.config.ts**

```typescript
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';

export default defineConfig({
  plugins: [react()],
  root: 'src/renderer',
  base: './',
  build: {
    outDir: '../../dist/renderer',
    emptyOutDir: true,
  },
  resolve: {
    alias: {
      '@renderer': path.resolve(__dirname, 'src/renderer'),
      '@shared': path.resolve(__dirname, 'src/shared'),
    },
  },
  server: {
    port: 5173,
  },
});
```

- [ ] **Step 5: Create electron-builder.json**

```json
{
  "appId": "com.wmux.app",
  "productName": "wmux",
  "directories": {
    "output": "release"
  },
  "files": [
    "dist/**/*",
    "node_modules/**/*",
    "!node_modules/**/build/**/*"
  ],
  "win": {
    "target": ["nsis", "portable"],
    "icon": "resources/icons/icon.ico"
  },
  "nsis": {
    "oneClick": false,
    "allowToChangeInstallationDirectory": true
  },
  "extraResources": [
    { "from": "resources/themes", "to": "themes" },
    { "from": "resources/sounds", "to": "sounds" },
    { "from": "src/shell-integration", "to": "shell-integration" }
  ]
}
```

- [ ] **Step 6: Create .gitignore**

```
node_modules/
dist/
release/
.vite/
*.log
.env
```

- [ ] **Step 7: Create src/shared/types.ts — shared type definitions**

```typescript
// ID types
export type WorkspaceId = `ws-${string}`;
export type PaneId = `pane-${string}`;
export type SurfaceId = `surf-${string}`;
export type WindowId = `win-${string}`;

// Split tree
export type SplitNode =
  | { type: 'leaf'; paneId: PaneId; surfaces: SurfaceRef[]; activeSurfaceIndex: number }
  | { type: 'branch'; direction: 'horizontal' | 'vertical'; ratio: number; children: [SplitNode, SplitNode] };

export interface SurfaceRef {
  id: SurfaceId;
  type: 'terminal' | 'browser' | 'markdown';
}

// Workspace
export interface WorkspaceInfo {
  id: WorkspaceId;
  title: string;
  customColor?: string;
  pinned: boolean;
  shell: string;
  splitTree: SplitNode;
  unreadCount: number;
  gitBranch?: string;
  gitDirty?: boolean;
  cwd?: string;
  prNumber?: number;
  prStatus?: 'open' | 'merged' | 'closed';
  prLabel?: string;
  ports?: number[];
  notificationText?: string;
  shellState?: 'idle' | 'running';
}

// Surface
export interface SurfaceInfo {
  id: SurfaceId;
  type: 'terminal' | 'browser' | 'markdown';
  title?: string;
}

// Pane
export interface PaneInfo {
  id: PaneId;
  surfaces: SurfaceInfo[];
  activeSurfaceId: SurfaceId;
}

// Window
export interface WindowInfo {
  id: WindowId;
  bounds: { x: number; y: number; width: number; height: number };
  workspaceIds: WorkspaceId[];
  activeWorkspaceId: WorkspaceId;
}

// Theme
export interface ThemeConfig {
  name: string;
  background: string;
  foreground: string;
  cursor: string;
  cursorText: string;
  selectionBackground: string;
  selectionForeground: string;
  palette: string[]; // 16 ANSI colors
  fontFamily: string;
  fontSize: number;
  backgroundOpacity: number;
}

// Notification
export interface NotificationInfo {
  id: string;
  surfaceId: SurfaceId;
  workspaceId: WorkspaceId;
  text: string;
  title?: string;
  timestamp: number;
  read: boolean;
}

// Shell
export interface ShellInfo {
  name: string;
  command: string;
  args: string[];
  available: boolean;
}

// Sidebar metadata
export interface SidebarMetadata {
  gitBranch?: string;
  gitDirty?: boolean;
  cwd?: string;
  prNumber?: number;
  prStatus?: string;
  prLabel?: string;
  ports?: number[];
  notificationText?: string;
  shellState?: 'idle' | 'running';
  statusEntries?: Record<string, string>;
  progress?: { value: number; label?: string };
  logs?: Array<{ level: string; message: string; timestamp: number }>;
}

// IPC channel names
export const IPC_CHANNELS = {
  // PTY
  PTY_CREATE: 'pty:create',
  PTY_WRITE: 'pty:write',
  PTY_RESIZE: 'pty:resize',
  PTY_KILL: 'pty:kill',
  PTY_DATA: 'pty:data',
  PTY_EXIT: 'pty:exit',
  // Workspace
  WORKSPACE_CREATE: 'workspace:create',
  WORKSPACE_CLOSE: 'workspace:close',
  WORKSPACE_SELECT: 'workspace:select',
  WORKSPACE_RENAME: 'workspace:rename',
  WORKSPACE_LIST: 'workspace:list',
  WORKSPACE_REORDER: 'workspace:reorder',
  WORKSPACE_MOVE_TO_WINDOW: 'workspace:moveToWindow',
  // Surface
  SURFACE_CREATE: 'surface:create',
  SURFACE_CLOSE: 'surface:close',
  SURFACE_FOCUS: 'surface:focus',
  SURFACE_LIST: 'surface:list',
  SURFACE_READ_TEXT: 'surface:readText',
  SURFACE_SEND_TEXT: 'surface:sendText',
  SURFACE_SEND_KEY: 'surface:sendKey',
  SURFACE_TRIGGER_FLASH: 'surface:triggerFlash',
  // Pane
  PANE_SPLIT: 'pane:split',
  PANE_CLOSE: 'pane:close',
  PANE_FOCUS: 'pane:focus',
  PANE_ZOOM: 'pane:zoom',
  PANE_LIST: 'pane:list',
  // Notification
  NOTIFICATION_FIRE: 'notification:fire',
  NOTIFICATION_LIST: 'notification:list',
  NOTIFICATION_CLEAR: 'notification:clear',
  NOTIFICATION_JUMP: 'notification:jump',
  // Settings
  SETTINGS_GET: 'settings:get',
  SETTINGS_SET: 'settings:set',
  SETTINGS_CHANGED: 'settings:changed',
  // Window
  WINDOW_CREATE: 'window:create',
  WINDOW_CLOSE: 'window:close',
  WINDOW_FOCUS: 'window:focus',
  WINDOW_LIST: 'window:list',
  WINDOW_MINIMIZE: 'window:minimize',
  WINDOW_MAXIMIZE: 'window:maximize',
  WINDOW_IS_MAXIMIZED: 'window:isMaximized',
  // Config
  CONFIG_GET_THEME: 'config:getTheme',
  CONFIG_GET_THEME_LIST: 'config:getThemeList',
  CONFIG_IMPORT_WT: 'config:importWindowsTerminal',
  CONFIG_IMPORT_GHOSTTY: 'config:importGhostty',
  // System
  SYSTEM_GET_SHELLS: 'system:getShells',
  SYSTEM_OPEN_EXTERNAL: 'system:openExternal',
  // Metadata events (main → renderer)
  METADATA_UPDATE: 'metadata:update',
} as const;
```

- [ ] **Step 8: Create src/main/index.ts — minimal Electron entry point**

```typescript
import { app, BrowserWindow } from 'electron';
import path from 'path';

let mainWindow: BrowserWindow | null = null;

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 800,
    minHeight: 500,
    titleBarStyle: 'hidden',
    titleBarOverlay: {
      color: '#1a1a1a',
      symbolColor: '#cccccc',
      height: 38,
    },
    backgroundColor: '#1a1a1a',
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false, // needed for node-pty IPC
    },
  });

  if (process.env.NODE_ENV === 'development') {
    mainWindow.loadURL('http://localhost:5173');
    mainWindow.webContents.openDevTools();
  } else {
    mainWindow.loadFile(path.join(__dirname, '../renderer/index.html'));
  }

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

app.whenReady().then(createWindow);

app.on('window-all-closed', () => {
  app.quit();
});
```

- [ ] **Step 9: Create src/preload/index.ts — empty contextBridge stub**

```typescript
import { contextBridge, ipcRenderer } from 'electron';

// Stub API — expanded in later tasks
contextBridge.exposeInMainWorld('wmux', {
  system: {
    platform: 'win32' as const,
  },
});
```

- [ ] **Step 10: Create src/renderer/index.html**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>wmux</title>
</head>
<body>
  <div id="root"></div>
  <script type="module" src="./index.tsx"></script>
</body>
</html>
```

- [ ] **Step 11: Create src/renderer/index.tsx and App.tsx**

`src/renderer/index.tsx`:
```tsx
import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import './styles/global.css';

const root = createRoot(document.getElementById('root')!);
root.render(<React.StrictMode><App /></React.StrictMode>);
```

`src/renderer/App.tsx`:
```tsx
export default function App() {
  return (
    <div style={{ display: 'flex', height: '100vh', background: '#272822', color: '#fdfff1' }}>
      <div style={{ width: 200, background: '#1a1a1a', borderRight: '1px solid #333' }}>
        {/* Sidebar placeholder */}
        <div style={{ padding: 10, fontSize: 12.5, fontWeight: 600 }}>wmux</div>
      </div>
      <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <span style={{ opacity: 0.5 }}>Terminal will render here</span>
      </div>
    </div>
  );
}
```

- [ ] **Step 12: Create src/renderer/styles/global.css**

```css
* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body, #root {
  height: 100%;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  background: #272822;
  color: #fdfff1;
  user-select: none;
}

::-webkit-scrollbar {
  width: 8px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.15);
  border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
  background: rgba(255, 255, 255, 0.25);
}
```

- [ ] **Step 13: Install dependencies and verify the app launches**

Run: `cd "C:/Users/aeont/OneDrive - Pulsa/Bureau/wmux" && npm install`
Then: `npm run build:main && npx electron .`
Expected: A window appears with a dark sidebar labeled "wmux" and a content area placeholder.

- [ ] **Step 14: Commit**

```bash
git add -A
git commit -m "feat: initialize Electron + React + Vite project scaffold"
```

---

> **IMPORTANT — Progressive Preload Wiring:** Task 4 only wires `pty` and `system` IPC into the preload. Each subsequent feature task that adds new IPC handlers MUST also add the corresponding methods to `src/preload/index.ts`. Specifically:
> - Task 8 (Sidebar) → add `workspace.*` methods to preload
> - Task 13 (Notifications) → add `notification.*` methods to preload
> - Task 14 (Pipe Server) → no preload change needed (main-only)
> - Task 18 (Surfaces) → add `surface.*` methods to preload
> - Task 19 (Browser) → add `browser.*` methods to preload
> - Task 20 (Markdown) → add `markdown.*` methods to preload
> - Task 21 (Session) → no preload change (main-only)
> - Task 22 (Settings) → add `settings.*` methods to preload
> - Task 25 (Multi-Window) → add `window.*` methods to preload
>
> Each task's "Wire into..." step should include updating `src/preload/index.ts` with the new IPC namespace.

## Phase 2: Terminal Emulation Core

### Task 2: Shell detector — auto-detect available shells

**Files:**
- Create: `src/main/shell-detector.ts`
- Create: `tests/unit/shell-detector.test.ts`

- [ ] **Step 1: Write test for shell detection**

```typescript
// tests/unit/shell-detector.test.ts
import { describe, it, expect } from 'vitest';
import { detectShells, getDefaultShell } from '../../src/main/shell-detector';

describe('shell-detector', () => {
  it('returns at least one shell on Windows', async () => {
    const shells = await detectShells();
    expect(shells.length).toBeGreaterThan(0);
    expect(shells.every(s => s.name && s.command)).toBe(true);
  });

  it('getDefaultShell returns a valid shell', async () => {
    const shell = await getDefaultShell();
    expect(shell).toBeDefined();
    expect(shell.available).toBe(true);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/unit/shell-detector.test.ts`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement shell-detector.ts**

```typescript
// src/main/shell-detector.ts
import { execFile } from 'child_process';
import { promisify } from 'util';
import type { ShellInfo } from '../shared/types';

const execFileAsync = promisify(execFile);

async function commandExists(cmd: string): Promise<boolean> {
  try {
    await execFileAsync('where', [cmd], { windowsHide: true });
    return true;
  } catch {
    return false;
  }
}

export async function detectShells(): Promise<ShellInfo[]> {
  const shells: ShellInfo[] = [];

  // PowerShell 7+ (pwsh)
  const hasPwsh = await commandExists('pwsh');
  shells.push({
    name: 'PowerShell 7',
    command: 'pwsh.exe',
    args: ['-NoLogo'],
    available: hasPwsh,
  });

  // PowerShell 5 (powershell)
  const hasPowershell = await commandExists('powershell');
  shells.push({
    name: 'PowerShell',
    command: 'powershell.exe',
    args: ['-NoLogo'],
    available: hasPowershell,
  });

  // CMD
  shells.push({
    name: 'Command Prompt',
    command: 'cmd.exe',
    args: [],
    available: true, // always available on Windows
  });

  // WSL
  const hasWsl = await commandExists('wsl');
  shells.push({
    name: 'WSL',
    command: 'wsl.exe',
    args: [],
    available: hasWsl,
  });

  return shells;
}

export async function getDefaultShell(): Promise<ShellInfo> {
  const shells = await detectShells();
  // Prefer pwsh > powershell > cmd
  return shells.find(s => s.available && s.command === 'pwsh.exe')
    ?? shells.find(s => s.available && s.command === 'powershell.exe')
    ?? shells.find(s => s.available)!;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/unit/shell-detector.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/shell-detector.ts src/shared/types.ts tests/unit/shell-detector.test.ts
git commit -m "feat: add shell detector for auto-detecting available shells"
```

### Task 3: PTY manager — spawn and manage terminal processes

**Files:**
- Create: `src/main/pty-manager.ts`
- Create: `tests/unit/pty-manager.test.ts`

- [ ] **Step 1: Write test for PTY spawning**

```typescript
// tests/unit/pty-manager.test.ts
import { describe, it, expect, afterEach } from 'vitest';
import { PtyManager } from '../../src/main/pty-manager';

describe('PtyManager', () => {
  const manager = new PtyManager();

  afterEach(() => {
    manager.killAll();
  });

  it('creates a PTY and returns an ID', async () => {
    const id = await manager.create({ shell: 'cmd.exe', cwd: process.cwd(), env: {} });
    expect(id).toMatch(/^surf-/);
  });

  it('writes data to a PTY', async () => {
    const id = await manager.create({ shell: 'cmd.exe', cwd: process.cwd(), env: {} });
    expect(() => manager.write(id, 'echo hello\r')).not.toThrow();
  });

  it('resizes a PTY', async () => {
    const id = await manager.create({ shell: 'cmd.exe', cwd: process.cwd(), env: {} });
    expect(() => manager.resize(id, 120, 40)).not.toThrow();
  });

  it('receives data from PTY', async () => {
    const id = await manager.create({ shell: 'cmd.exe', cwd: process.cwd(), env: {} });
    const data = await new Promise<string>((resolve) => {
      manager.onData(id, (d) => resolve(d));
      manager.write(id, 'echo test123\r');
    });
    expect(data.length).toBeGreaterThan(0);
  });

  it('kills a PTY', async () => {
    const id = await manager.create({ shell: 'cmd.exe', cwd: process.cwd(), env: {} });
    manager.kill(id);
    expect(manager.has(id)).toBe(false);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/unit/pty-manager.test.ts`
Expected: FAIL

- [ ] **Step 3: Implement pty-manager.ts**

```typescript
// src/main/pty-manager.ts
import * as pty from 'node-pty';
import { v4 as uuid } from 'uuid';
import type { SurfaceId } from '../shared/types';

interface PtyInstance {
  id: SurfaceId;
  process: pty.IPty;
  dataListeners: Array<(data: string) => void>;
  exitListeners: Array<(code: number) => void>;
}

export class PtyManager {
  private ptys = new Map<SurfaceId, PtyInstance>();

  async create(options: {
    shell: string;
    cwd: string;
    env: Record<string, string>;
    cols?: number;
    rows?: number;
  }): Promise<SurfaceId> {
    const id = `surf-${uuid()}` as SurfaceId;

    const env = { ...process.env, ...options.env } as Record<string, string>;
    const proc = pty.spawn(options.shell, [], {
      name: 'xterm-256color',
      cols: options.cols ?? 80,
      rows: options.rows ?? 24,
      cwd: options.cwd,
      env,
      useConpty: true,
    });

    const instance: PtyInstance = {
      id,
      process: proc,
      dataListeners: [],
      exitListeners: [],
    };

    proc.onData((data) => {
      instance.dataListeners.forEach((cb) => cb(data));
    });

    proc.onExit(({ exitCode }) => {
      instance.exitListeners.forEach((cb) => cb(exitCode));
      this.ptys.delete(id);
    });

    this.ptys.set(id, instance);
    return id;
  }

  write(id: SurfaceId, data: string): void {
    const instance = this.ptys.get(id);
    if (instance) instance.process.write(data);
  }

  resize(id: SurfaceId, cols: number, rows: number): void {
    const instance = this.ptys.get(id);
    if (instance) instance.process.resize(cols, rows);
  }

  kill(id: SurfaceId): void {
    const instance = this.ptys.get(id);
    if (instance) {
      instance.process.kill();
      this.ptys.delete(id);
    }
  }

  killAll(): void {
    for (const [id] of this.ptys) {
      this.kill(id);
    }
  }

  has(id: SurfaceId): boolean {
    return this.ptys.has(id);
  }

  onData(id: SurfaceId, callback: (data: string) => void): () => void {
    const instance = this.ptys.get(id);
    if (!instance) return () => {};
    instance.dataListeners.push(callback);
    return () => {
      instance.dataListeners = instance.dataListeners.filter((cb) => cb !== callback);
    };
  }

  onExit(id: SurfaceId, callback: (code: number) => void): () => void {
    const instance = this.ptys.get(id);
    if (!instance) return () => {};
    instance.exitListeners.push(callback);
    return () => {
      instance.exitListeners = instance.exitListeners.filter((cb) => cb !== callback);
    };
  }

  getPid(id: SurfaceId): number | undefined {
    return this.ptys.get(id)?.process.pid;
  }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/unit/pty-manager.test.ts`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/main/pty-manager.ts tests/unit/pty-manager.test.ts
git commit -m "feat: add PTY manager with node-pty ConPTY spawning"
```

### Task 4: IPC handlers — wire PTY to preload bridge

**Files:**
- Create: `src/main/ipc-handlers.ts`
- Modify: `src/main/index.ts`
- Modify: `src/preload/index.ts`

- [ ] **Step 1: Create ipc-handlers.ts — PTY IPC bridge**

```typescript
// src/main/ipc-handlers.ts
import { ipcMain, BrowserWindow } from 'electron';
import { IPC_CHANNELS, SurfaceId } from '../shared/types';
import { PtyManager } from './pty-manager';
import { detectShells, getDefaultShell } from './shell-detector';

const ptyManager = new PtyManager();

export function registerIpcHandlers(): void {
  // PTY
  ipcMain.handle(IPC_CHANNELS.PTY_CREATE, async (_event, options: {
    shell: string; cwd: string; env: Record<string, string>;
  }) => {
    // Resolve CWD on main process (renderer has no access to process.env)
    const resolvedOptions = {
      ...options,
      cwd: options.cwd || process.env.USERPROFILE || 'C:\\',
    };
    const id = await ptyManager.create(resolvedOptions);
    const window = BrowserWindow.fromWebContents(_event.sender);
    ptyManager.onData(id, (data) => {
      if (window && !window.isDestroyed()) {
        window.webContents.send(IPC_CHANNELS.PTY_DATA, id, data);
      }
    });
    ptyManager.onExit(id, (code) => {
      if (window && !window.isDestroyed()) {
        window.webContents.send(IPC_CHANNELS.PTY_EXIT, id, code);
      }
    });
    return id;
  });

  ipcMain.on(IPC_CHANNELS.PTY_WRITE, (_event, id: SurfaceId, data: string) => {
    ptyManager.write(id, data);
  });

  ipcMain.on(IPC_CHANNELS.PTY_RESIZE, (_event, id: SurfaceId, cols: number, rows: number) => {
    ptyManager.resize(id, cols, rows);
  });

  ipcMain.on(IPC_CHANNELS.PTY_KILL, (_event, id: SurfaceId) => {
    ptyManager.kill(id);
  });

  // System
  ipcMain.handle(IPC_CHANNELS.SYSTEM_GET_SHELLS, async () => {
    return detectShells();
  });
}

export { ptyManager };
```

- [ ] **Step 2: Update src/preload/index.ts — expose PTY API**

```typescript
// src/preload/index.ts
import { contextBridge, ipcRenderer } from 'electron';
import { IPC_CHANNELS, SurfaceId } from '../shared/types';

contextBridge.exposeInMainWorld('wmux', {
  pty: {
    create: (options: { shell: string; cwd: string; env: Record<string, string> }): Promise<string> =>
      ipcRenderer.invoke(IPC_CHANNELS.PTY_CREATE, options),
    write: (id: string, data: string): void =>
      ipcRenderer.send(IPC_CHANNELS.PTY_WRITE, id, data),
    resize: (id: string, cols: number, rows: number): void =>
      ipcRenderer.send(IPC_CHANNELS.PTY_RESIZE, id, cols, rows),
    kill: (id: string): void =>
      ipcRenderer.send(IPC_CHANNELS.PTY_KILL, id),
    onData: (id: string, callback: (data: string) => void): (() => void) => {
      const handler = (_event: any, ptyId: string, data: string) => {
        if (ptyId === id) callback(data);
      };
      ipcRenderer.on(IPC_CHANNELS.PTY_DATA, handler);
      return () => ipcRenderer.removeListener(IPC_CHANNELS.PTY_DATA, handler);
    },
    onExit: (id: string, callback: (code: number) => void): (() => void) => {
      const handler = (_event: any, ptyId: string, code: number) => {
        if (ptyId === id) callback(code);
      };
      ipcRenderer.on(IPC_CHANNELS.PTY_EXIT, handler);
      return () => ipcRenderer.removeListener(IPC_CHANNELS.PTY_EXIT, handler);
    },
  },
  system: {
    platform: 'win32' as const,
    getShells: (): Promise<any[]> => ipcRenderer.invoke(IPC_CHANNELS.SYSTEM_GET_SHELLS),
  },
});
```

- [ ] **Step 3: Update src/main/index.ts — register IPC handlers**

Add `import { registerIpcHandlers } from './ipc-handlers';` and call `registerIpcHandlers()` before `createWindow()`.

- [ ] **Step 4: Verify the app still launches**

Run: `npm run build:main && npx electron .`
Expected: App launches. No errors in devtools console.

- [ ] **Step 5: Commit**

```bash
git add src/main/ipc-handlers.ts src/preload/index.ts src/main/index.ts
git commit -m "feat: add IPC handlers wiring PTY manager to preload bridge"
```

### Task 5: Terminal pane component — xterm.js rendering

**Files:**
- Create: `src/renderer/components/Terminal/TerminalPane.tsx`
- Create: `src/renderer/hooks/useTerminal.ts`
- Create: `src/renderer/styles/terminal.css`
- Modify: `src/renderer/App.tsx`

- [ ] **Step 1: Create useTerminal.ts hook**

```typescript
// src/renderer/hooks/useTerminal.ts
import { useEffect, useRef, useCallback } from 'react';
import { Terminal } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { ImageAddon } from '@xterm/addon-image';
import '@xterm/xterm/css/xterm.css';

declare global {
  interface Window {
    wmux: any;
  }
}

interface UseTerminalOptions {
  shell?: string;
  cwd?: string;
  fontSize?: number;
  fontFamily?: string;
  theme?: Record<string, string>;
}

export function useTerminal(options: UseTerminalOptions = {}) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const ptyIdRef = useRef<string | null>(null);
  const cleanupRef = useRef<Array<() => void>>([]);

  const fit = useCallback(() => {
    if (fitAddonRef.current && xtermRef.current) {
      fitAddonRef.current.fit();
      if (ptyIdRef.current) {
        window.wmux.pty.resize(
          ptyIdRef.current,
          xtermRef.current.cols,
          xtermRef.current.rows
        );
      }
    }
  }, []);

  useEffect(() => {
    if (!terminalRef.current) return;

    const terminal = new Terminal({
      fontSize: options.fontSize ?? 13,
      fontFamily: options.fontFamily ?? "'Cascadia Mono', 'Consolas', monospace",
      cursorBlink: true,
      cursorStyle: 'block',
      theme: {
        background: options.theme?.background ?? '#272822',
        foreground: options.theme?.foreground ?? '#fdfff1',
        cursor: options.theme?.cursor ?? '#c0c1b5',
        selectionBackground: options.theme?.selectionBackground ?? '#57584f',
        selectionForeground: options.theme?.selectionForeground ?? '#fdfff1',
      },
      allowProposedApi: true,
    });

    const fitAddon = new FitAddon();
    const searchAddon = new SearchAddon();
    const unicode11Addon = new Unicode11Addon();

    terminal.loadAddon(fitAddon);
    terminal.loadAddon(new WebLinksAddon());
    terminal.loadAddon(searchAddon);
    terminal.loadAddon(unicode11Addon);
    terminal.loadAddon(new ImageAddon());
    terminal.unicode.activeVersion = '11';

    // Ctrl+C: copy if selection exists, else send \x03 to PTY
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type === 'keydown' && event.ctrlKey && event.key === 'c') {
        if (terminal.hasSelection()) {
          navigator.clipboard.writeText(terminal.getSelection());
          terminal.clearSelection();
          return false; // prevent sending to PTY
        }
        // No selection — let xterm send \x03 (CTRL_C_EVENT via ConPTY)
        return true;
      }
      return true;
    });

    terminal.open(terminalRef.current);

    // Try WebGL, fall back to canvas
    try {
      terminal.loadAddon(new WebglAddon());
    } catch {
      console.warn('WebGL addon failed to load, using canvas renderer');
    }

    fitAddon.fit();

    xtermRef.current = terminal;
    fitAddonRef.current = fitAddon;

    // Spawn PTY
    const shell = options.shell ?? 'pwsh.exe';
    // CWD is resolved on the main process side (PTY create handler defaults to USERPROFILE)
    const cwd = options.cwd ?? '';

    window.wmux.pty.create({ shell, cwd, env: { WMUX: '1' } }).then((id: string) => {
      ptyIdRef.current = id;

      // PTY → xterm
      const unsub = window.wmux.pty.onData(id, (data: string) => {
        terminal.write(data);
      });
      cleanupRef.current.push(unsub);

      // PTY exit
      const unsubExit = window.wmux.pty.onExit(id, () => {
        terminal.write('\r\n[Process exited]\r\n');
      });
      cleanupRef.current.push(unsubExit);

      // xterm → PTY
      const disposable = terminal.onData((data) => {
        window.wmux.pty.write(id, data);
      });
      cleanupRef.current.push(() => disposable.dispose());

      // Resize after PTY is ready
      window.wmux.pty.resize(id, terminal.cols, terminal.rows);
    });

    // Resize observer
    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit();
      if (ptyIdRef.current) {
        window.wmux.pty.resize(ptyIdRef.current, terminal.cols, terminal.rows);
      }
    });
    resizeObserver.observe(terminalRef.current);
    cleanupRef.current.push(() => resizeObserver.disconnect());

    return () => {
      cleanupRef.current.forEach((fn) => fn());
      cleanupRef.current = [];
      if (ptyIdRef.current) {
        window.wmux.pty.kill(ptyIdRef.current);
      }
      terminal.dispose();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return { terminalRef, fit, xtermRef };
}
```

- [ ] **Step 2: Create TerminalPane.tsx component**

```tsx
// src/renderer/components/Terminal/TerminalPane.tsx
import { useTerminal } from '../../hooks/useTerminal';
import '../../styles/terminal.css';

interface TerminalPaneProps {
  shell?: string;
  cwd?: string;
  focused?: boolean;
}

export default function TerminalPane({ shell, cwd, focused = true }: TerminalPaneProps) {
  const { terminalRef } = useTerminal({ shell, cwd });

  return (
    <div
      className={`terminal-pane ${focused ? 'terminal-pane--focused' : ''}`}
      data-focused={focused}
    >
      <div ref={terminalRef} className="terminal-pane__container" />
    </div>
  );
}
```

- [ ] **Step 3: Create terminal.css**

```css
.terminal-pane {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

.terminal-pane__container {
  width: 100%;
  height: 100%;
}

.terminal-pane__container .xterm {
  height: 100%;
  padding: 4px;
}
```

- [ ] **Step 4: Update App.tsx to render a terminal**

```tsx
// src/renderer/App.tsx
import TerminalPane from './components/Terminal/TerminalPane';

export default function App() {
  return (
    <div style={{ display: 'flex', height: '100vh' }}>
      <div style={{ width: 200, background: '#1a1a1a', borderRight: '1px solid #333' }}>
        <div style={{
          padding: 10,
          fontSize: 12.5,
          fontWeight: 600,
          color: '#fdfff1',
          WebkitAppRegion: 'drag' as any,
          height: 38,
          display: 'flex',
          alignItems: 'center',
        }}>
          wmux
        </div>
      </div>
      <div style={{ flex: 1 }}>
        <TerminalPane />
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Verify terminal renders and accepts input**

Run: `npm run dev`
Expected: App launches with sidebar on left and a working terminal on the right. You can type commands and see output.

- [ ] **Step 6: Commit**

```bash
git add src/renderer/components/Terminal/ src/renderer/hooks/useTerminal.ts src/renderer/styles/terminal.css src/renderer/App.tsx
git commit -m "feat: add xterm.js terminal pane with WebGL rendering and PTY integration"
```

---

## Phase 3: Split Pane System

### Task 6: Split tree data model and Zustand store

**Files:**
- Create: `src/renderer/store/index.ts`
- Create: `src/renderer/store/workspace-slice.ts`
- Create: `src/renderer/store/split-slice.ts`
- Create: `tests/unit/split-tree.test.ts`

- [ ] **Step 1: Write tests for split tree operations**

```typescript
// tests/unit/split-tree.test.ts
import { describe, it, expect } from 'vitest';
import {
  createLeaf, splitNode, removeLeaf, findLeaf, updateRatio
} from '../../src/renderer/store/split-utils';

describe('split-tree', () => {
  it('creates a leaf node', () => {
    const leaf = createLeaf('pane-1', 'terminal');
    expect(leaf.type).toBe('leaf');
    expect(leaf.paneId).toBe('pane-1');
  });

  it('splits a leaf horizontally', () => {
    const leaf = createLeaf('pane-1', 'terminal');
    const result = splitNode(leaf, 'pane-1', 'pane-2', 'terminal', 'horizontal');
    expect(result.type).toBe('branch');
    if (result.type === 'branch') {
      expect(result.direction).toBe('horizontal');
      expect(result.ratio).toBe(0.5);
      expect(result.children[0].type).toBe('leaf');
      expect(result.children[1].type).toBe('leaf');
    }
  });

  it('removes a leaf and collapses parent', () => {
    const leaf = createLeaf('pane-1', 'terminal');
    const tree = splitNode(leaf, 'pane-1', 'pane-2', 'terminal', 'horizontal');
    const result = removeLeaf(tree, 'pane-2');
    expect(result?.type).toBe('leaf');
    if (result?.type === 'leaf') {
      expect(result.paneId).toBe('pane-1');
    }
  });

  it('finds a leaf by paneId', () => {
    const leaf = createLeaf('pane-1', 'terminal');
    const tree = splitNode(leaf, 'pane-1', 'pane-2', 'terminal', 'vertical');
    expect(findLeaf(tree, 'pane-2')).toBeDefined();
    expect(findLeaf(tree, 'pane-999')).toBeUndefined();
  });

  it('updates ratio of a branch', () => {
    const leaf = createLeaf('pane-1', 'terminal');
    const tree = splitNode(leaf, 'pane-1', 'pane-2', 'terminal', 'horizontal');
    const updated = updateRatio(tree, 'pane-1', 'pane-2', 0.7);
    if (updated.type === 'branch') {
      expect(updated.ratio).toBe(0.7);
    }
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run tests/unit/split-tree.test.ts`
Expected: FAIL

- [ ] **Step 3: Create split-utils.ts — pure split tree functions**

```typescript
// src/renderer/store/split-utils.ts
import { v4 as uuid } from 'uuid';
import type { SplitNode, SurfaceRef, PaneId, SurfaceId } from '../../shared/types';

export function createLeaf(
  paneId?: PaneId,
  surfaceType: 'terminal' | 'browser' | 'markdown' = 'terminal'
): SplitNode & { type: 'leaf' } {
  const pid = paneId ?? (`pane-${uuid()}` as PaneId);
  const surfId = `surf-${uuid()}` as SurfaceId;
  return {
    type: 'leaf',
    paneId: pid,
    surfaces: [{ id: surfId, type: surfaceType }],
    activeSurfaceIndex: 0,
  };
}

export function splitNode(
  tree: SplitNode,
  targetPaneId: PaneId | string,
  newPaneId: PaneId | string,
  surfaceType: 'terminal' | 'browser' | 'markdown',
  direction: 'horizontal' | 'vertical'
): SplitNode {
  if (tree.type === 'leaf') {
    if (tree.paneId === targetPaneId) {
      const newSurfId = `surf-${uuid()}` as SurfaceId;
      const newLeaf: SplitNode = {
        type: 'leaf',
        paneId: newPaneId as PaneId,
        surfaces: [{ id: newSurfId, type: surfaceType }],
        activeSurfaceIndex: 0,
      };
      return {
        type: 'branch',
        direction,
        ratio: 0.5,
        children: [tree, newLeaf],
      };
    }
    return tree;
  }

  return {
    ...tree,
    children: [
      splitNode(tree.children[0], targetPaneId, newPaneId, surfaceType, direction),
      splitNode(tree.children[1], targetPaneId, newPaneId, surfaceType, direction),
    ] as [SplitNode, SplitNode],
  };
}

export function removeLeaf(tree: SplitNode, paneId: PaneId | string): SplitNode | null {
  if (tree.type === 'leaf') {
    return tree.paneId === paneId ? null : tree;
  }

  const left = removeLeaf(tree.children[0], paneId);
  const right = removeLeaf(tree.children[1], paneId);

  if (left === null) return right;
  if (right === null) return left;

  return { ...tree, children: [left, right] as [SplitNode, SplitNode] };
}

export function findLeaf(tree: SplitNode, paneId: PaneId | string): (SplitNode & { type: 'leaf' }) | undefined {
  if (tree.type === 'leaf') {
    return tree.paneId === paneId ? tree : undefined;
  }
  return findLeaf(tree.children[0], paneId) ?? findLeaf(tree.children[1], paneId);
}

export function updateRatio(
  tree: SplitNode,
  leftPaneId: string,
  rightPaneId: string,
  newRatio: number
): SplitNode {
  if (tree.type === 'leaf') return tree;

  const leftHas = findLeaf(tree.children[0], leftPaneId);
  const rightHas = findLeaf(tree.children[1], rightPaneId);

  if (leftHas && rightHas) {
    return { ...tree, ratio: Math.max(0.1, Math.min(0.9, newRatio)) };
  }

  return {
    ...tree,
    children: [
      updateRatio(tree.children[0], leftPaneId, rightPaneId, newRatio),
      updateRatio(tree.children[1], leftPaneId, rightPaneId, newRatio),
    ] as [SplitNode, SplitNode],
  };
}

export function getAllPaneIds(tree: SplitNode): PaneId[] {
  if (tree.type === 'leaf') return [tree.paneId];
  return [...getAllPaneIds(tree.children[0]), ...getAllPaneIds(tree.children[1])];
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run tests/unit/split-tree.test.ts`
Expected: PASS

- [ ] **Step 5: Create the Zustand store with workspace and split slices**

`src/renderer/store/workspace-slice.ts`:
```typescript
import { StateCreator } from 'zustand';
import { v4 as uuid } from 'uuid';
import type { WorkspaceId, WorkspaceInfo, SplitNode } from '../../shared/types';
import { createLeaf } from './split-utils';

export interface WorkspaceSlice {
  workspaces: WorkspaceInfo[];
  activeWorkspaceId: WorkspaceId | null;
  createWorkspace: (options?: { title?: string; shell?: string }) => WorkspaceId;
  closeWorkspace: (id: WorkspaceId) => void;
  selectWorkspace: (id: WorkspaceId) => void;
  renameWorkspace: (id: WorkspaceId, title: string) => void;
  reorderWorkspaces: (ids: WorkspaceId[]) => void;
  updateWorkspaceMetadata: (id: WorkspaceId, metadata: Partial<WorkspaceInfo>) => void;
  updateSplitTree: (id: WorkspaceId, tree: SplitNode) => void;
}

export const createWorkspaceSlice: StateCreator<WorkspaceSlice> = (set, get) => ({
  workspaces: [],
  activeWorkspaceId: null,

  createWorkspace: (options) => {
    const id = `ws-${uuid()}` as WorkspaceId;
    const leaf = createLeaf();
    const workspace: WorkspaceInfo = {
      id,
      title: options?.title ?? `Workspace ${get().workspaces.length + 1}`,
      pinned: false,
      shell: options?.shell ?? 'pwsh.exe',
      splitTree: leaf,
      unreadCount: 0,
    };
    set((state) => ({
      workspaces: [...state.workspaces, workspace],
      activeWorkspaceId: state.activeWorkspaceId ?? id,
    }));
    return id;
  },

  closeWorkspace: (id) => {
    set((state) => {
      const remaining = state.workspaces.filter((w) => w.id !== id);
      let activeId = state.activeWorkspaceId;
      if (activeId === id) {
        activeId = remaining.length > 0 ? remaining[0].id : null;
      }
      return { workspaces: remaining, activeWorkspaceId: activeId };
    });
  },

  selectWorkspace: (id) => set({ activeWorkspaceId: id }),

  renameWorkspace: (id, title) => {
    set((state) => ({
      workspaces: state.workspaces.map((w) =>
        w.id === id ? { ...w, title } : w
      ),
    }));
  },

  reorderWorkspaces: (ids) => {
    set((state) => {
      const map = new Map(state.workspaces.map((w) => [w.id, w]));
      const reordered = ids.map((id) => map.get(id)!).filter(Boolean);
      return { workspaces: reordered };
    });
  },

  updateWorkspaceMetadata: (id, metadata) => {
    set((state) => ({
      workspaces: state.workspaces.map((w) =>
        w.id === id ? { ...w, ...metadata } : w
      ),
    }));
  },

  updateSplitTree: (id, tree) => {
    set((state) => ({
      workspaces: state.workspaces.map((w) =>
        w.id === id ? { ...w, splitTree: tree } : w
      ),
    }));
  },
});
```

`src/renderer/store/index.ts`:
```typescript
import { create } from 'zustand';
import { WorkspaceSlice, createWorkspaceSlice } from './workspace-slice';

export type WmuxStore = WorkspaceSlice;

export const useStore = create<WmuxStore>()((...args) => ({
  ...createWorkspaceSlice(...args),
}));
```

- [ ] **Step 6: Commit**

```bash
git add src/renderer/store/ tests/unit/split-tree.test.ts
git commit -m "feat: add split tree data model, utils, and Zustand workspace store"
```

### Task 7: Split pane renderer components

**Files:**
- Create: `src/renderer/components/SplitPane/SplitContainer.tsx`
- Create: `src/renderer/components/SplitPane/SplitDivider.tsx`
- Create: `src/renderer/components/SplitPane/PaneWrapper.tsx`
- Create: `src/renderer/styles/splitpane.css`
- Modify: `src/renderer/App.tsx`

- [ ] **Step 1: Create SplitDivider.tsx**

A draggable divider between split panes. Renders a 1px visible line with a 6px invisible hit target. Changes cursor to `col-resize` or `row-resize`. On drag, calls `onRatioChange` with the new ratio.

- [ ] **Step 2: Create PaneWrapper.tsx**

Wraps a leaf node. Renders the active surface's TerminalPane (for now — browser/markdown added later). Shows the unfocused overlay (30% opacity dark fill) when not focused. Contains the notification ring overlay element (initially hidden).

- [ ] **Step 3: Create SplitContainer.tsx**

Recursive component that renders the split tree:
- If `node.type === 'leaf'` → render `<PaneWrapper>`
- If `node.type === 'branch'` → render two children in a flex container (row for horizontal, column for vertical) with a `<SplitDivider>` between them. Flex basis set by `ratio`.

- [ ] **Step 4: Create splitpane.css**

```css
.split-container {
  display: flex;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

.split-container--horizontal { flex-direction: row; }
.split-container--vertical { flex-direction: column; }

.split-child {
  overflow: hidden;
  position: relative;
}

.split-divider {
  flex-shrink: 0;
  background: transparent;
  position: relative;
  z-index: 10;
}

.split-divider--horizontal {
  width: 6px;
  cursor: col-resize;
}

.split-divider--vertical {
  height: 6px;
  cursor: row-resize;
}

.split-divider__line {
  position: absolute;
  background: rgba(255, 255, 255, 0.06);
}

.split-divider--horizontal .split-divider__line {
  width: 1px;
  height: 100%;
  left: 50%;
  transform: translateX(-50%);
}

.split-divider--vertical .split-divider__line {
  height: 1px;
  width: 100%;
  top: 50%;
  transform: translateY(-50%);
}

.pane-wrapper {
  position: relative;
  width: 100%;
  height: 100%;
}

.pane-wrapper__unfocused-overlay {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.3);
  pointer-events: none;
  z-index: 5;
  transition: opacity 0.15s ease;
}
```

- [ ] **Step 5: Update App.tsx to use SplitContainer with the active workspace's split tree**

Wire up: workspace creation on mount → split tree from store → `<SplitContainer>` with `<TerminalPane>` for each leaf.

- [ ] **Step 6: Verify splits work**

Run: `npm run dev`
Expected: App launches with a single terminal. Implement a temporary button or keyboard shortcut to test splitting. After split, two terminals appear side by side with a draggable divider.

- [ ] **Step 7: Commit**

```bash
git add src/renderer/components/SplitPane/ src/renderer/styles/splitpane.css src/renderer/App.tsx
git commit -m "feat: add recursive split pane renderer with draggable dividers"
```

---

## Phase 4: Workspace Sidebar

### Task 8: Sidebar shell with workspace rows

**Files:**
- Create: `src/renderer/components/Sidebar/Sidebar.tsx`
- Create: `src/renderer/components/Sidebar/WorkspaceRow.tsx`
- Create: `src/renderer/components/Sidebar/UnreadBadge.tsx`
- Create: `src/renderer/components/Sidebar/SidebarResizeHandle.tsx`
- Create: `src/renderer/styles/sidebar.css`
- Modify: `src/renderer/App.tsx`

- [ ] **Step 1: Create Sidebar.tsx**

Sidebar container: 200px default width, resizable via handle. Renders workspace rows from store. "New Workspace" button at bottom. Scrollable when many workspaces. 28px top padding (for titlebar overlay area).

- [ ] **Step 2: Create WorkspaceRow.tsx**

Each row exactly matching cmux (spec Section 3):
- Title (12.5px semibold) + unread badge + close button on hover
- Notification text (10px, secondary, 2-line clamp)
- Git branch (10px monospace, 75% opacity)
- Working directory (10px monospace, 75% opacity)
- PR info (10px semibold)
- Ports (10px monospace, 75% opacity)
- Row: 10px horizontal / 8px vertical padding, 6px corner radius, 2px spacing, 6px margin
- Selected state: `#0091FF` background, white text
- Left rail indicator: 3px wide `#0091FF` capsule

- [ ] **Step 3: Create UnreadBadge.tsx**

16x16 blue circle with 9px semibold white count. `#0091FF` on inactive, `white @ 25%` on selected.

- [ ] **Step 4: Create SidebarResizeHandle.tsx**

Invisible 4px wide drag handle on the right edge of the sidebar. On drag, updates sidebar width (min 180, max 600 or 1/3 window).

- [ ] **Step 5: Create sidebar.css with exact dimensions from spec**

- [ ] **Step 6: Update App.tsx — replace placeholder sidebar with real component**

- [ ] **Step 7: Verify sidebar renders with workspace rows, selection, and resize**

- [ ] **Step 8: Commit**

```bash
git add src/renderer/components/Sidebar/ src/renderer/styles/sidebar.css src/renderer/App.tsx
git commit -m "feat: add workspace sidebar with rows, unread badges, and resize handle"
```

### Task 9: Workspace context menu and drag-to-reorder

**Files:**
- Create: `src/renderer/components/Sidebar/WorkspaceContextMenu.tsx`
- Modify: `src/renderer/components/Sidebar/WorkspaceRow.tsx`
- Modify: `src/renderer/components/Sidebar/Sidebar.tsx`

- [ ] **Step 1: Create WorkspaceContextMenu.tsx**

Right-click context menu matching spec Section 3: Pin/Unpin, Rename, Remove Name, Workspace Color submenu (16 presets + custom), Move Up/Down/Top, Move to Window submenu, Close/Close Other/Close Above/Below, Mark Read/Unread. Use a portal-based custom context menu (no native menus — they can't do color swatches).

- [ ] **Step 2: Add drag-to-reorder to Sidebar.tsx**

HTML5 drag-and-drop on workspace rows. On drop, call `reorderWorkspaces`. Multi-select with Ctrl+click / Shift+click for batch operations.

- [ ] **Step 3: Add workspace color to WorkspaceRow**

If workspace has `customColor`, use it as the left rail capsule color and tint the row background subtly.

- [ ] **Step 4: Verify context menu and drag-reorder**

- [ ] **Step 5: Commit**

```bash
git add src/renderer/components/Sidebar/
git commit -m "feat: add workspace context menu, drag-to-reorder, and custom colors"
```

### Task 10: Titlebar component

**Files:**
- Create: `src/renderer/components/Titlebar/Titlebar.tsx`
- Create: `src/renderer/styles/titlebar.css`
- Modify: `src/renderer/App.tsx`

- [ ] **Step 1: Create Titlebar.tsx**

38px height bar. Drag region (`-webkit-app-region: drag`). Shows focused surface title: "Cmd: <title>" in 12px medium weight secondary color. Native window controls handled by `titleBarOverlay`.

- [ ] **Step 2: Create titlebar.css**

- [ ] **Step 3: Wire into App.tsx**

- [ ] **Step 4: Commit**

```bash
git add src/renderer/components/Titlebar/ src/renderer/styles/titlebar.css src/renderer/App.tsx
git commit -m "feat: add custom titlebar with drag region and focused surface title"
```

---

## Phase 5: Keyboard Shortcuts

### Task 11: Keyboard shortcut system

**Files:**
- Create: `src/renderer/hooks/useKeyboardShortcuts.ts`
- Create: `src/renderer/store/settings-slice.ts`
- Modify: `src/renderer/store/index.ts`
- Modify: `src/renderer/App.tsx`

- [ ] **Step 1: Create settings-slice.ts**

Zustand slice for all user settings. Start with keyboard shortcuts as a `Record<ActionName, ShortcutBinding>` with defaults matching spec Section 13. Settings persist to `%APPDATA%\wmux\settings.json` via IPC.

- [ ] **Step 2: Create useKeyboardShortcuts.ts**

Global keyboard listener hook. Reads shortcut bindings from settings store. Matches key events against bindings. Dispatches actions:
- `Ctrl+N` → create workspace
- `Ctrl+D` → split right
- `Ctrl+Shift+D` → split down
- `Ctrl+W` → close active surface/pane
- `Ctrl+B` → toggle sidebar
- `Ctrl+Shift+Enter` → zoom pane
- `Ctrl+Alt+Arrow` → focus pane direction
- `Ctrl+1-9` → select workspace
- `Ctrl+T` → new surface
- `Ctrl+Shift+]`/`[` → next/previous surface
- `Ctrl+PageDown`/`PageUp` → next/previous workspace

Special handling: when a terminal is focused, only intercept shortcuts that use Ctrl+Shift or Ctrl+Alt (to avoid eating terminal input like Ctrl+C, Ctrl+D, etc.).

- [ ] **Step 3: Wire into App.tsx**

- [ ] **Step 4: Verify all keyboard shortcuts work**

Test: Ctrl+N creates workspace, Ctrl+D splits, Ctrl+W closes, etc.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/hooks/useKeyboardShortcuts.ts src/renderer/store/settings-slice.ts src/renderer/store/index.ts src/renderer/App.tsx
git commit -m "feat: add rebindable keyboard shortcut system with all default bindings"
```

---

## Phase 6: Config & Themes

### Task 12: Config loader — Windows Terminal + Ghostty

**Files:**
- Create: `src/main/config-loader.ts`
- Create: `src/main/theme-loader.ts`
- Create: `tests/unit/config-loader.test.ts`

- [ ] **Step 1: Write test for Windows Terminal config parsing**

Test that `parseWindowsTerminalConfig` reads font, colors, and scheme from a mock settings.json structure.

- [ ] **Step 2: Write test for Ghostty config parsing**

Test that `parseGhosttyConfig` reads key-value pairs from Ghostty's plain-text config format.

- [ ] **Step 3: Implement config-loader.ts**

Two parsers:
- `parseWindowsTerminalConfig()`: reads `%LOCALAPPDATA%\Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json`. Extracts default profile's font, color scheme, and maps scheme colors to ThemeConfig.
- `parseGhosttyConfig()`: reads `~/.config/ghostty/config`. Parses `key = value` lines for font-family, font-size, theme, background, foreground, cursor, selection-background, selection-foreground, palette entries (0-15).

Both return `ThemeConfig | null`. Gracefully handle missing files.

- [ ] **Step 4: Implement theme-loader.ts**

Loads the 450+ bundled Ghostty themes from `resources/themes/`. Each theme is a text file with `key = value` pairs (palette, background, foreground, etc.). Returns a `Map<string, ThemeConfig>`.

Note: We'll bundle a subset of ~50 popular themes initially. Full 450+ can be added later by downloading from the Ghostty themes repo.

- [ ] **Step 5: Run tests**

- [ ] **Step 6: Add IPC handlers for config/theme operations**

Wire `CONFIG_GET_THEME`, `CONFIG_GET_THEME_LIST`, `CONFIG_IMPORT_WT`, `CONFIG_IMPORT_GHOSTTY` to the config-loader.

- [ ] **Step 7: Commit**

```bash
git add src/main/config-loader.ts src/main/theme-loader.ts tests/unit/config-loader.test.ts src/main/ipc-handlers.ts
git commit -m "feat: add config loader for Windows Terminal and Ghostty configs with theme support"
```

---

## Phase 7: Notification System

### Task 13: Notification manager and OSC parser

**Files:**
- Create: `src/main/notification-manager.ts`
- Create: `src/renderer/components/Terminal/NotificationRing.tsx`
- Create: `src/renderer/store/notification-slice.ts`
- Modify: `src/renderer/hooks/useTerminal.ts`
- Modify: `src/renderer/store/index.ts`

- [ ] **Step 1: Create notification-slice.ts**

Zustand slice: notifications array, unread counts per workspace, `addNotification`, `markRead`, `markAllRead`, `clearNotification`, `jumpToUnread`.

- [ ] **Step 2: Create notification-manager.ts (main process)**

Handles Windows toast notifications (`new Notification()`), taskbar flash (`BrowserWindow.flashFrame(true)`), notification sound playback. Receives notification events from IPC and named pipe.

- [ ] **Step 3: Add OSC parser hooks to useTerminal.ts**

Register handlers for OSC 9, 99, 777 via `terminal.parser.registerOscHandler()`. When detected, send notification event via IPC.

- [ ] **Step 4: Create NotificationRing.tsx**

CSS-based notification ring overlay matching spec exactly:
- 2.5px border, `#007AFF`, inset 2px, corner radius 6px
- Box-shadow: `0 0 6px rgba(0, 122, 255, 0.6)`
- Flash animation: 0.9s double-pulse via CSS `@keyframes`

```css
@keyframes notification-flash {
  0%   { opacity: 0; }
  25%  { opacity: 1; }
  50%  { opacity: 0; }
  75%  { opacity: 1; }
  100% { opacity: 0; }
}
```

After animation: ring stays visible at opacity 1 until pane is focused.

- [ ] **Step 5: Wire notifications into PaneWrapper and sidebar UnreadBadge**

- [ ] **Step 6: Verify end-to-end: run `echo -e '\e]9;Test notification\a'` in terminal → ring appears, badge increments, toast shows**

- [ ] **Step 7: Commit**

```bash
git add src/main/notification-manager.ts src/renderer/components/Terminal/NotificationRing.tsx src/renderer/store/notification-slice.ts src/renderer/hooks/useTerminal.ts
git commit -m "feat: add notification system with OSC parsing, blue ring, toast, and taskbar flash"
```

---

## Phase 8: Named Pipe Server & Shell Integration

### Task 14: Named pipe server (V1 + V2 protocol)

**Files:**
- Create: `src/main/pipe-server.ts`
- Create: `tests/unit/pipe-server.test.ts`

- [ ] **Step 1: Write test for pipe server V1 parsing**

Test that V1 text commands like `report_pwd surf-123 C:\Users\foo` are correctly parsed into structured objects.

- [ ] **Step 2: Write test for V2 JSON-RPC parsing**

Test that JSON-RPC messages like `{"method": "workspace.create", ...}` are parsed and routed.

- [ ] **Step 3: Implement pipe-server.ts**

Windows named pipe server using Node.js `net` module:
- `net.createServer()` on `\\.\pipe\wmux`
- V1: line-based text protocol, parse `<command> <args...>`
- V2: JSON-RPC, parse `{"method": "...", "params": {...}}`, respond with `{"result": ...}`
- Route V1 commands to metadata update handlers
- Route V2 methods to appropriate managers (workspace, surface, pane, browser, etc.)
- Handle multiple simultaneous clients

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Wire pipe server into main/index.ts — start on app ready**

- [ ] **Step 6: Commit**

```bash
git add src/main/pipe-server.ts tests/unit/pipe-server.test.ts src/main/index.ts
git commit -m "feat: add named pipe server with V1 text and V2 JSON-RPC protocols"
```

### Task 15: Shell integration scripts

**Files:**
- Create: `src/shell-integration/wmux-powershell-integration.ps1`
- Create: `src/shell-integration/wmux-cmd-integration.cmd`
- Create: `src/shell-integration/wmux-bash-integration.sh`
- Modify: `src/main/pty-manager.ts`

- [ ] **Step 1: Create PowerShell integration script**

Override `prompt` function. Use `[System.IO.Pipes.NamedPipeClientStream]` to send V1 commands to `\\.\pipe\wmux`:
- `report_pwd` with `$PWD`
- `report_git_branch` from `git rev-parse --abbrev-ref HEAD` + dirty from `git status --porcelain`
- `report_shell_state idle` when at prompt
- Background job for `gh pr view` polling every 45s
- `ports_kick` after each command

- [ ] **Step 2: Create CMD integration script**

Set `PROMPT` to include OSC 9 for CWD reporting:
```cmd
@echo off
set PROMPT=$e]9;9;%CD%$e\$P$G
```

- [ ] **Step 3: Create WSL bash integration script**

Based on cmux's bash integration. Uses `PROMPT_COMMAND` for CWD and git reporting. Communication via temp file fallback initially (npiperelay bridge as future enhancement).

- [ ] **Step 4: Modify pty-manager.ts to inject integration scripts**

When spawning a PTY, detect shell type and inject:
- PowerShell: add `-NoExit -Command ". '<integration-path>'"` to args
- CMD: add `/K "<integration-path>"` to args
- WSL: set `WMUX_INTEGRATION=1` env var

Also inject env vars: `WMUX=1`, `WMUX_WORKSPACE_ID`, `WMUX_PANE_ID`, `WMUX_SURFACE_ID`, `WMUX_PIPE`.

- [ ] **Step 5: Verify shell integration**

Open a PowerShell workspace. Change directory. Check that sidebar updates with new CWD and git branch.

- [ ] **Step 6: Commit**

```bash
git add src/shell-integration/ src/main/pty-manager.ts
git commit -m "feat: add shell integration for PowerShell, CMD, and WSL"
```

### Task 16: Port scanner and git/PR poller

**Files:**
- Create: `src/main/port-scanner.ts`
- Create: `src/main/git-poller.ts`
- Create: `src/main/pr-poller.ts`

- [ ] **Step 1: Create port-scanner.ts**

Runs `netstat -ano` and parses output to find listening TCP ports. Maps PIDs to panes via process tree tracking. Coalesce pattern: 200ms debounce after kick, burst scans at [0.5, 1.5, 3, 5, 7.5, 10] seconds. Sends results to renderer via IPC metadata updates.

- [ ] **Step 2: Create git-poller.ts**

Fallback git branch detection for CMD panes (and as backup for PowerShell). Uses `fs.watch` on `.git/HEAD` in workspace CWD. When changed, runs `git rev-parse --abbrev-ref HEAD` and `git status --porcelain`. Sends results via IPC.

- [ ] **Step 3: Create pr-poller.ts**

Background polling via `gh pr view --json number,state,title` every 45 seconds per workspace. Only runs if `gh` is in PATH. Sends results via IPC.

- [ ] **Step 4: Wire all three into main process startup**

- [ ] **Step 5: Commit**

```bash
git add src/main/port-scanner.ts src/main/git-poller.ts src/main/pr-poller.ts src/main/index.ts
git commit -m "feat: add port scanner, git poller, and PR poller for sidebar metadata"
```

---

## Phase 9: CLI Tool

### Task 17: wmux CLI executable

**Files:**
- Create: `src/cli/wmux.ts`
- Modify: `package.json` (add bin entry)

- [ ] **Step 1: Create CLI entry point with argument parser**

Use a lightweight arg parser (e.g., `commander` or hand-rolled). Connects to `\\.\pipe\wmux` named pipe. Sends V1 or V2 commands. Prints results.

Implement the full command set from spec Section 7.4:
- Workspace: new-workspace, close-workspace, select-workspace, rename-workspace, list-workspaces, move-workspace-to-window
- Surface: new-surface, close-surface, focus-surface, list-surfaces
- Pane: split, close-pane, focus-pane, zoom-pane, list-panes, tree
- Terminal: send, send-key, read-screen, trigger-flash
- Browser: browser open/snapshot/click/fill/evaluate/back/forward/reload
- Markdown: markdown set
- Notification: notify, list-notifications, clear-notifications
- Sidebar: set-status, set-progress, log, sidebar-state
- System: ping, identify, capabilities, list-windows, focus-window

- [ ] **Step 2: Add `"bin": { "wmux": "dist/cli/wmux.js" }` to package.json**

- [ ] **Step 3: Test basic commands**

Run: `npx ts-node src/cli/wmux.ts ping`
Expected: "pong" (with wmux app running)

Run: `npx ts-node src/cli/wmux.ts notify "Hello from CLI"`
Expected: notification appears in wmux

- [ ] **Step 4: Commit**

```bash
git add src/cli/wmux.ts package.json
git commit -m "feat: add wmux CLI with full command set matching cmux"
```

---

## Phase 10: Surface Tab Bar

### Task 18: Surface (tab-within-pane) system

**Files:**
- Create: `src/renderer/components/SplitPane/SurfaceTabBar.tsx`
- Create: `src/renderer/store/surface-slice.ts`
- Modify: `src/renderer/components/SplitPane/PaneWrapper.tsx`
- Modify: `src/renderer/store/index.ts`

- [ ] **Step 1: Create surface-slice.ts**

Zustand slice for surface operations: createSurface, closeSurface, focusSurface, nextSurface, prevSurface, selectSurfaceByIndex.

- [ ] **Step 2: Create SurfaceTabBar.tsx**

Small tab bar shown at the top of a pane when it has 2+ surfaces. Each tab shows the surface title. Active tab highlighted. Tabs closeable with × button on hover. Only renders when `surfaces.length > 1`.

- [ ] **Step 3: Update PaneWrapper.tsx**

Render SurfaceTabBar above the active surface content. On Ctrl+T, create new surface. On Ctrl+W, close active surface (if last, close pane). Switch between surfaces with Ctrl+Shift+]/[.

- [ ] **Step 4: Verify**

Open terminal, press Ctrl+T to create new tab in same pane. Tab bar appears. Switch between tabs. Close tabs.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/components/SplitPane/SurfaceTabBar.tsx src/renderer/store/surface-slice.ts src/renderer/components/SplitPane/PaneWrapper.tsx
git commit -m "feat: add surface tab bar for multiple terminals within a single pane"
```

---

## Phase 11: Browser Panel

### Task 19: Browser pane with WebContentsView

**Files:**
- Create: `src/renderer/components/Browser/BrowserPane.tsx`
- Create: `src/renderer/components/Browser/AddressBar.tsx`
- Create: `src/renderer/styles/browser.css`
- Modify: `src/main/ipc-handlers.ts`
- Modify: `src/main/index.ts`

- [ ] **Step 1: Create browser IPC handlers in main process**

Main process manages `WebContentsView` instances. IPC handlers for:
- `browser:create` → creates a new `WebContentsView`, adds to window
- `browser:navigate` → calls `webContents.loadURL()`
- `browser:bounds` → calls `view.setBounds()` to position within pane
- `browser:back/forward/reload` → calls webContents navigation methods
- `browser:evaluate` → calls `webContents.executeJavaScript()`
- `browser:snapshot` → inject DOM walker script, return accessibility tree
- `browser:click/fill` → inject click/fill scripts

- [ ] **Step 2: Create BrowserPane.tsx**

Renders AddressBar at top + a placeholder div that reserves space for the WebContentsView overlay. On mount, sends `browser:create` IPC. On resize, sends `browser:bounds` IPC with new dimensions. On unmount, sends `browser:destroy`.

- [ ] **Step 3: Create AddressBar.tsx**

Matching spec Section 9: back/forward buttons, refresh/stop toggle, URL pill (corner radius 10px, editable), DevTools toggle (11px icon). Button size 22px, hit target 26px. Chrome background matches terminal bg.

- [ ] **Step 4: Create browser.css**

- [ ] **Step 5: Wire browser panel into PaneWrapper — when surface type is 'browser', render BrowserPane instead of TerminalPane**

- [ ] **Step 6: Verify**

Press Ctrl+Alt+D to split browser right. Address bar appears. Navigate to localhost or any URL. Page renders.

- [ ] **Step 7: Commit**

```bash
git add src/renderer/components/Browser/ src/renderer/styles/browser.css src/main/ipc-handlers.ts
git commit -m "feat: add browser panel with WebContentsView, address bar, and scriptable API"
```

---

## Phase 12: Markdown Panel

### Task 20: Markdown rendering surface

**Files:**
- Create: `src/renderer/components/Markdown/MarkdownPane.tsx`
- Create: `src/renderer/styles/markdown.css`

- [ ] **Step 1: Install marked**

Run: `npm install marked`

- [ ] **Step 2: Create MarkdownPane.tsx**

Renders markdown content as HTML using `marked`. Supports GFM (tables, task lists, fenced code blocks). Read-only. Styled to match terminal theme (dark bg, light text, monospace code). Content set via props (from IPC or V2 command).

- [ ] **Step 3: Create markdown.css**

Dark theme styling for rendered markdown: headers, code blocks with syntax highlighting, tables, lists, blockquotes.

- [ ] **Step 4: Wire into PaneWrapper — when surface type is 'markdown', render MarkdownPane**

- [ ] **Step 5: Verify**

Split with `--type markdown` via CLI or shortcut. Set content. Markdown renders correctly.

- [ ] **Step 6: Commit**

```bash
git add src/renderer/components/Markdown/ src/renderer/styles/markdown.css
git commit -m "feat: add markdown panel with GFM rendering"
```

---

## Phase 13: Session Persistence

### Task 21: Save/restore workspace state

**Files:**
- Create: `src/main/session-persistence.ts`
- Create: `tests/unit/session-persistence.test.ts`

- [ ] **Step 1: Write test for session serialization/deserialization**

Test that a workspace state (windows, workspaces, split trees, surface types, CWDs, browser URLs) serializes to JSON and deserializes back correctly.

- [ ] **Step 2: Implement session-persistence.ts**

- `saveSession(state)`: writes to `%APPDATA%\wmux\sessions\session.json` atomically (temp file + rename)
- `loadSession()`: reads and parses session file, returns state or null on failure
- Auto-save: debounced 30s timer, also triggered on workspace/split changes and before quit
- Crash recovery: if JSON parse fails, return null (app falls back to single default workspace)

- [ ] **Step 3: Wire into main process**

- On startup: load session, restore windows/workspaces/splits, spawn new PTYs in saved CWDs
- On quit (`before-quit`): save session
- On state change: IPC from renderer triggers save debounce

- [ ] **Step 4: Run tests**

- [ ] **Step 5: Verify**

Open app, create 2 workspaces, split one. Close app. Reopen. Same layout restored.

- [ ] **Step 6: Commit**

```bash
git add src/main/session-persistence.ts tests/unit/session-persistence.test.ts src/main/index.ts
git commit -m "feat: add session persistence with auto-save and crash recovery"
```

---

## Phase 14: Settings UI

### Task 22: Settings window with all tabs

**Files:**
- Create: `src/renderer/components/Settings/SettingsWindow.tsx`
- Create: `src/renderer/components/Settings/SidebarSettings.tsx`
- Create: `src/renderer/components/Settings/WorkspaceSettings.tsx`
- Create: `src/renderer/components/Settings/TerminalSettings.tsx`
- Create: `src/renderer/components/Settings/NotificationSettings.tsx`
- Create: `src/renderer/components/Settings/BrowserSettings.tsx`
- Create: `src/renderer/components/Settings/KeyboardSettings.tsx`
- Create: `src/renderer/components/Settings/ShortcutRecorder.tsx`
- Create: `src/renderer/styles/settings.css`

- [ ] **Step 1: Create SettingsWindow.tsx**

Opens as a separate `BrowserWindow` via IPC. Tabbed layout with 6 tabs: Sidebar, Workspace, Terminal, Notifications, Browser, Keyboard Shortcuts.

- [ ] **Step 2: Create each settings tab component**

Each tab renders the toggles, dropdowns, sliders, and pickers from spec Section 12. All settings read/write via the Zustand settings store + IPC to `%APPDATA%\wmux\settings.json`.

- [ ] **Step 3: Create ShortcutRecorder.tsx**

A button that, when clicked, enters "recording" mode — captures the next key combo and saves it as the new shortcut binding. Shows conflict detection (warns if binding already used).

- [ ] **Step 4: Create settings.css**

Dark theme styling matching the rest of the app.

- [ ] **Step 5: Wire Ctrl+, to open settings window**

- [ ] **Step 6: Verify**

Open settings. Change theme. Terminal updates immediately. Change font size. Terminal updates. Rebind a shortcut. New binding works.

- [ ] **Step 7: Commit**

```bash
git add src/renderer/components/Settings/ src/renderer/styles/settings.css
git commit -m "feat: add settings window with all tabs, theme picker, and shortcut recorder"
```

---

## Phase 15: Command Palette

### Task 23: Fuzzy-search command palette

**Files:**
- Create: `src/renderer/components/CommandPalette/CommandPalette.tsx`
- Create: `src/renderer/styles/command-palette.css`

- [ ] **Step 1: Create CommandPalette.tsx**

Centered overlay triggered by `Ctrl+Shift+P`. Text input with fuzzy-search against:
- All keyboard shortcut actions (by name)
- Workspace names (type to switch)
- Theme names (type to switch)
- Shell names (for new workspace)
- Settings sections
- Recent notifications

Keyboard navigation: up/down arrows, Enter to select, Escape to close. Items show action name + current shortcut binding on the right.

- [ ] **Step 2: Create command-palette.css**

Centered overlay with blur backdrop, dark background, rounded corners. Input field at top, results list below. Selected item highlighted.

- [ ] **Step 3: Wire into App.tsx**

- [ ] **Step 4: Verify**

Ctrl+Shift+P opens palette. Type "split" → shows split actions. Select one → action executes.

- [ ] **Step 5: Commit**

```bash
git add src/renderer/components/CommandPalette/ src/renderer/styles/command-palette.css
git commit -m "feat: add fuzzy-search command palette"
```

---

## Phase 16: Find in Terminal & Copy Mode

### Task 24: Find bar and copy mode

**Files:**
- Create: `src/renderer/components/Terminal/FindBar.tsx`
- Create: `src/renderer/components/Terminal/CopyMode.tsx`

- [ ] **Step 1: Create FindBar.tsx**

Triggered by `Ctrl+F`. Renders an input field at the top of the terminal pane. Uses xterm.js SearchAddon: `searchAddon.findNext(query)` / `findPrevious(query)`. Shows match count. Close with Escape.

- [ ] **Step 2: Create CopyMode.tsx**

Triggered by `Ctrl+Shift+M`. Switches terminal to "copy mode" — arrow keys move a visible cursor through scrollback without sending input to the PTY. Selection with Shift+arrows. Enter to copy selection, Escape to exit.

- [ ] **Step 3: Verify find and copy mode work**

- [ ] **Step 4: Commit**

```bash
git add src/renderer/components/Terminal/FindBar.tsx src/renderer/components/Terminal/CopyMode.tsx
git commit -m "feat: add find-in-terminal and keyboard copy mode"
```

---

## Phase 17: Multi-Window Support

### Task 25: Window manager for multiple BrowserWindows

**Files:**
- Create: `src/main/window-manager.ts`
- Modify: `src/main/index.ts`
- Modify: `src/main/ipc-handlers.ts`

- [ ] **Step 1: Create window-manager.ts**

Manages multiple `BrowserWindow` instances. Each window gets its own renderer. Methods: `createWindow()`, `closeWindow(id)`, `focusWindow(id)`, `listWindows()`. Tracks window bounds, workspace assignments per window. Handles "Move Workspace to Window" by removing workspace from source window's renderer and adding to target.

- [ ] **Step 2: Update index.ts to use WindowManager instead of direct BrowserWindow**

- [ ] **Step 3: Add window IPC handlers**

Wire `WINDOW_CREATE`, `WINDOW_CLOSE`, `WINDOW_FOCUS`, `WINDOW_LIST`, `WINDOW_MINIMIZE`, `WINDOW_MAXIMIZE`, `WINDOW_IS_MAXIMIZED`.

- [ ] **Step 4: Verify**

Ctrl+Shift+N opens new window. Each window works independently. Workspaces can be moved between windows.

- [ ] **Step 5: Commit**

```bash
git add src/main/window-manager.ts src/main/index.ts src/main/ipc-handlers.ts
git commit -m "feat: add multi-window support with workspace migration"
```

---

## Phase 18: Packaging & Auto-Update

### Task 26: Build and package as Windows installer

**Files:**
- Create: `resources/icons/icon.ico`
- Modify: `electron-builder.json`
- Modify: `package.json`
- Modify: `src/main/updater.ts`

- [ ] **Step 1: Create app icon (ico format)**

Generate a simple wmux icon or use a placeholder.

- [ ] **Step 2: Create updater.ts**

Uses `electron-updater` to check for updates on GitHub Releases. Auto-download + prompt to install.

- [ ] **Step 3: Verify build**

Run: `npm run build`
Expected: Produces `release/wmux Setup 0.1.0.exe` and `release/wmux-0.1.0-portable.exe`

- [ ] **Step 4: Test installer**

Install on a clean Windows machine (or same machine). App launches from Start Menu. Auto-update check runs on startup.

- [ ] **Step 5: Commit**

```bash
git add resources/icons/ src/main/updater.ts electron-builder.json package.json
git commit -m "feat: add electron-builder packaging and auto-update"
```

---

## Phase 19: PR Status Icons & Sidebar Details

### Task 27: PR status icons and remaining sidebar metadata

**Files:**
- Create: `src/renderer/components/Sidebar/PrStatusIcon.tsx`
- Modify: `src/renderer/components/Sidebar/WorkspaceRow.tsx`

- [ ] **Step 1: Create PrStatusIcon.tsx**

SVG icons for PR states: open (green circle), merged (purple), closed (red). Renders inline at 10px.

- [ ] **Step 2: Update WorkspaceRow.tsx with all metadata fields**

Ensure all sidebar fields render correctly with real data from the store: git branch with icon, CWD shortened with `~`, PR row with icon + underlined number + status text, ports as `:3000, :8080`, log entries with level colors, progress bar, status pills.

- [ ] **Step 3: Verify all metadata displays correctly**

- [ ] **Step 4: Commit**

```bash
git add src/renderer/components/Sidebar/PrStatusIcon.tsx src/renderer/components/Sidebar/WorkspaceRow.tsx
git commit -m "feat: add PR status icons and complete sidebar metadata rendering"
```

---

## Final Checkpoint

After completing all 27 tasks:

- [ ] Run full test suite: `npm test`
- [ ] Build production bundle: `npm run build`
- [ ] Test installer on clean Windows
- [ ] Verify all features against spec checklist:
  - [ ] Terminal emulation (xterm.js + WebGL)
  - [ ] Split panes (horizontal + vertical + zoom)
  - [ ] Workspaces with sidebar metadata
  - [ ] Surface tabs within panes
  - [ ] Notification ring + badge + toast + taskbar flash
  - [ ] Named pipe server (V1 + V2)
  - [ ] Shell integration (PowerShell + CMD + WSL)
  - [ ] CLI (wmux.exe)
  - [ ] Browser panel with scriptable API
  - [ ] Markdown panel
  - [ ] Session persistence + restore
  - [ ] Settings UI with all tabs
  - [ ] Command palette
  - [ ] Keyboard shortcuts (all rebindable)
  - [ ] Config import (Windows Terminal + Ghostty)
  - [ ] Multi-window support
  - [ ] Find in terminal + copy mode
  - [ ] Port scanning + git/PR polling
  - [ ] Auto-update
  - [ ] Windows installer (.exe)
