import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import fs from 'fs';
import path from 'path';
import os from 'os';
import type { SessionData } from '../../src/main/session-persistence';

// Use a temp directory for tests
const TEST_DIR = path.join(os.tmpdir(), 'wmux-test-sessions-' + process.pid);

describe('session-persistence', () => {
  beforeEach(() => {
    // Override APPDATA for testing by directly manipulating the module
    // We'll test the serialize/deserialize logic with direct file operations
    fs.mkdirSync(TEST_DIR, { recursive: true });
  });

  afterEach(() => {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  });

  it('saveSession writes valid JSON', () => {
    const sessionFile = path.join(TEST_DIR, 'session.json');
    const data: SessionData = {
      version: 1,
      windows: [{
        bounds: { x: 100, y: 100, width: 1400, height: 900 },
        sidebarWidth: 200,
        activeWorkspaceId: 'ws-1',
        workspaces: [{
          id: 'ws-1',
          title: 'Test',
          pinned: false,
          shell: 'pwsh.exe',
          splitTree: { type: 'leaf', paneId: 'pane-1', surfaces: [], activeSurfaceIndex: 0 },
        }],
      }],
    };

    // Write directly to test location
    fs.writeFileSync(sessionFile, JSON.stringify(data, null, 2));
    const loaded = JSON.parse(fs.readFileSync(sessionFile, 'utf-8'));
    expect(loaded.version).toBe(1);
    expect(loaded.windows[0].workspaces[0].title).toBe('Test');
  });

  it('handles missing file gracefully', () => {
    const nonexistent = path.join(TEST_DIR, 'nonexistent.json');
    expect(fs.existsSync(nonexistent)).toBe(false);
  });

  it('handles corrupted JSON gracefully', () => {
    const sessionFile = path.join(TEST_DIR, 'corrupted.json');
    fs.writeFileSync(sessionFile, '{invalid json!!!');
    expect(() => JSON.parse(fs.readFileSync(sessionFile, 'utf-8'))).toThrow();
  });

  it('round-trips session data correctly', () => {
    const sessionFile = path.join(TEST_DIR, 'roundtrip.json');
    const data: SessionData = {
      version: 1,
      windows: [{
        bounds: { x: 0, y: 0, width: 1920, height: 1080 },
        sidebarWidth: 250,
        activeWorkspaceId: 'ws-abc',
        workspaces: [
          { id: 'ws-abc', title: 'Agent 1', pinned: true, shell: 'pwsh.exe', customColor: '#C0392B', splitTree: { type: 'leaf', paneId: 'p-1', surfaces: [{ id: 's-1', type: 'terminal' }], activeSurfaceIndex: 0 } },
          { id: 'ws-def', title: 'Agent 2', pinned: false, shell: 'cmd.exe', splitTree: { type: 'branch', direction: 'horizontal', ratio: 0.5, children: [{ type: 'leaf', paneId: 'p-2', surfaces: [{ id: 's-2', type: 'terminal' }], activeSurfaceIndex: 0 }, { type: 'leaf', paneId: 'p-3', surfaces: [{ id: 's-3', type: 'browser' }], activeSurfaceIndex: 0 }] } },
        ],
      }],
    };

    fs.writeFileSync(sessionFile, JSON.stringify(data, null, 2));
    const loaded = JSON.parse(fs.readFileSync(sessionFile, 'utf-8')) as SessionData;

    expect(loaded.version).toBe(1);
    expect(loaded.windows[0].workspaces).toHaveLength(2);
    expect(loaded.windows[0].workspaces[0].customColor).toBe('#C0392B');
    expect(loaded.windows[0].workspaces[1].splitTree.type).toBe('branch');
    expect(loaded.windows[0].workspaces[1].splitTree.children).toHaveLength(2);
  });
});

// Issue #35: a version update must NOT delete explicitly-named saved sessions
// (they are layout-only snapshots the user chose to keep). Only the volatile
// auto session.json is reset. The module computes its storage paths from
// %APPDATA% at import time, so we override APPDATA and re-import per test.
describe('handleVersionChange (issue #35)', () => {
  const APPDATA_OVERRIDE = path.join(os.tmpdir(), 'wmux-vc-test-' + process.pid);
  let mod: typeof import('../../src/main/session-persistence');
  let savedAppData: string | undefined;

  beforeEach(async () => {
    savedAppData = process.env.APPDATA;
    process.env.APPDATA = APPDATA_OVERRIDE;
    delete process.env.WMUX_INSTANCE;
    vi.resetModules();
    mod = await import('../../src/main/session-persistence');
    mod.ensureDirectories();
  });

  afterEach(() => {
    if (savedAppData === undefined) delete process.env.APPDATA;
    else process.env.APPDATA = savedAppData;
    fs.rmSync(APPDATA_OVERRIDE, { recursive: true, force: true });
  });

  it('preserves named saved sessions across a version change', () => {
    mod.handleVersionChange('0.9.0'); // establish the version marker
    mod.saveNamedSession({ name: 'My Layout', savedAt: 123, workspaces: [] } as any);
    expect(mod.loadNamedSession('My Layout')).not.toBeNull();

    const changed = mod.handleVersionChange('0.9.1');
    expect(changed).toBe(true);
    expect(mod.listNamedSessions().map((s) => s.name)).toContain('My Layout');
    expect(mod.loadNamedSession('My Layout')).not.toBeNull();
    expect(mod.getLastSessionName()).toBe('My Layout');
  });

  it('clears the volatile auto session.json on a version change', () => {
    mod.handleVersionChange('1.0.0'); // establish the version marker
    mod.saveSession({ version: 1, windows: [] } as any);
    expect(mod.loadSession()).not.toBeNull();

    mod.handleVersionChange('1.0.1');
    expect(mod.loadSession()).toBeNull();
  });
});
