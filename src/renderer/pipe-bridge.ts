/**
 * pipe-bridge.ts — Exposes Zustand store operations as window.__pandamux_* globals
 * so the main process can call them via executeJavaScript from V2 pipe handlers.
 */
import { useStore } from './store';
import { splitNode, removeLeaf, getAllPaneIds, findLeaf, buildGridLayout } from './store/split-utils';
import { killSurfacePty } from './store/pty-teardown';
import { PaneId, SurfaceId, WorkspaceId, SurfaceType } from '../shared/types';
import { uuid } from '../shared/id';

export function initPipeBridge(): void {
  const w = window as any;

  // ─── Workspace ──────────────────────────────────────────────────────────────

  w.__pandamux_createWorkspace = (params?: { title?: string; shell?: string; cwd?: string }) => {
    const store = useStore.getState();
    const id = store.createWorkspace({
      title: params?.title,
      shell: params?.shell,
      cwd: params?.cwd,
    });
    return { workspaceId: id };
  };

  w.__pandamux_closeWorkspace = (id: string) => {
    useStore.getState().closeWorkspace(id as WorkspaceId);
  };

  w.__pandamux_selectWorkspace = (id: string) => {
    useStore.getState().selectWorkspace(id as WorkspaceId);
  };

  w.__pandamux_renameWorkspace = (id: string, title: string) => {
    useStore.getState().renameWorkspace(id as WorkspaceId, title);
  };

  w.__pandamux_listWorkspaces = () => {
    const store = useStore.getState();
    return store.workspaces.map(ws => ({
      id: ws.id,
      title: ws.title,
      isActive: ws.id === store.activeWorkspaceId,
      cwd: ws.cwd,
      shell: ws.shell,
    }));
  };

  // Which workspace owns a given surface? Used by main to route browser commands
  // to a browser pane in the *caller agent's* workspace (issue #62). Returns the
  // active workspace id as a fallback when the surface isn't found.
  w.__pandamux_getWorkspaceIdForSurface = (surfaceId: string) => {
    const store = useStore.getState();
    for (const ws of store.workspaces) {
      for (const paneId of getAllPaneIds(ws.splitTree)) {
        const leaf = findLeaf(ws.splitTree, paneId);
        if (leaf?.surfaces?.some(s => s.id === surfaceId)) return ws.id;
      }
    }
    return store.activeWorkspaceId ?? null;
  };

  // All browser surface ids in a workspace. Main adopts an unbound one for a
  // caller (or creates a fresh pane) so each agent gets its own browser (#62).
  w.__pandamux_listBrowserSurfaces = (workspaceId: string) => {
    const store = useStore.getState();
    const ws = store.workspaces.find(x => x.id === workspaceId);
    if (!ws) return [];
    const ids: string[] = [];
    for (const paneId of getAllPaneIds(ws.splitTree)) {
      const leaf = findLeaf(ws.splitTree, paneId);
      for (const s of leaf?.surfaces ?? []) {
        if (s.type === 'browser') ids.push(s.id);
      }
    }
    return ids;
  };

  // ─── Pane ───────────────────────────────────────────────────────────────────

  w.__pandamux_splitPane = (params?: { direction?: string; type?: string; workspaceId?: string; colorScheme?: string }) => {
    const store = useStore.getState();
    const wsId = (params?.workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return null;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return null;

    const paneIds = getAllPaneIds(ws.splitTree);
    const targetPaneId = paneIds[0];
    if (!targetPaneId) return null;

    const newPaneId = `pane-${uuid()}` as PaneId;
    const surfaceType = (params?.type || 'terminal') as SurfaceType;
    const direction = params?.direction === 'down' || params?.direction === 'vertical'
      ? 'vertical' : 'horizontal';

    const newTree = splitNode(ws.splitTree, targetPaneId, newPaneId, surfaceType, direction);
    store.updateSplitTree(wsId, newTree);

    const newLeaf = findLeaf(newTree, newPaneId);
    const surfaceId = newLeaf?.surfaces?.[0]?.id || null;

    // Apply a per-pane color scheme override to the freshly-created surface
    // so `pandamux split --color-scheme prod` takes effect immediately.
    if (params?.colorScheme && surfaceId && newLeaf) {
      store.updateSurface(wsId, newPaneId, surfaceId as SurfaceId, { colorScheme: params.colorScheme });
    }

    return { paneId: newPaneId, surfaceId };
  };

  w.__pandamux_closePane = (paneId: string, workspaceId?: string) => {
    const store = useStore.getState();
    const wsId = (workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return;

    // Reap the pane's shells before removing it (issue #65). `pandamux close-pane`
    // dropped the leaf without killing any PTY (mirrors PaneWrapper.handleClosePane,
    // the UI path that always did kill its terminals).
    const leaf = findLeaf(ws.splitTree, paneId as PaneId);
    if (leaf) {
      for (const surface of leaf.surfaces) killSurfacePty(surface);
    }

    const newTree = removeLeaf(ws.splitTree, paneId as PaneId);
    if (newTree) {
      store.updateSplitTree(wsId, newTree);
    }
  };

  w.__pandamux_layoutGrid = (params: { count: number; type?: string; anchorSurfaceId?: string; anchorPaneId?: string; workspaceId?: string }) => {
    const store = useStore.getState();
    const wsId = (params?.workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return null;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return null;

    const count = Math.max(1, Math.floor(params.count || 1));
    if (count < 2) return { newPaneIds: [], newPanes: [] };

    // Resolve the anchor pane: explicit paneId > surface lookup > first pane
    const paneIds = getAllPaneIds(ws.splitTree);
    let anchorPaneId: PaneId | undefined;

    if (params.anchorPaneId) {
      anchorPaneId = params.anchorPaneId as PaneId;
    } else if (params.anchorSurfaceId) {
      for (const pid of paneIds) {
        const leaf = findLeaf(ws.splitTree, pid);
        if (leaf?.surfaces?.some(s => s.id === params.anchorSurfaceId)) {
          anchorPaneId = pid;
          break;
        }
      }
    }
    if (!anchorPaneId) anchorPaneId = paneIds[0];
    if (!anchorPaneId) return null;

    const surfaceType = (params.type || 'terminal') as SurfaceType;
    const { tree: newTree, newPaneIds } = buildGridLayout(ws.splitTree, anchorPaneId, count, surfaceType);
    store.updateSplitTree(wsId, newTree);

    // Resolve surface IDs for the newly-created panes so callers can target them directly.
    const newPanes = newPaneIds.map(pid => {
      const leaf = findLeaf(newTree, pid);
      return {
        paneId: pid,
        surfaceId: leaf?.surfaces?.[0]?.id || null,
      };
    });

    return { newPaneIds, newPanes, anchorPaneId, cols: Math.ceil(Math.sqrt(count)), rows: Math.ceil(count / Math.ceil(Math.sqrt(count))) };
  };

  w.__pandamux_listPanes = (workspaceId?: string) => {
    const store = useStore.getState();
    const wsId = (workspaceId || store.activeWorkspaceId) as WorkspaceId;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return [];

    const paneIds = getAllPaneIds(ws.splitTree);
    return paneIds.map(pid => {
      const leaf = findLeaf(ws.splitTree, pid);
      return {
        paneId: pid,
        surfaces: leaf?.surfaces?.map(s => ({ id: s.id, type: s.type })) || [],
        tabCount: leaf?.surfaces?.length || 0,
        activeSurfaceIndex: leaf?.activeSurfaceIndex ?? 0,
      };
    });
  };

  // ─── Surface ────────────────────────────────────────────────────────────────

  w.__pandamux_createSurface = (params?: { type?: string; paneId?: string; workspaceId?: string; colorScheme?: string }) => {
    const store = useStore.getState();
    const wsId = (params?.workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return null;

    let paneId = params?.paneId as PaneId | undefined;
    if (!paneId) {
      const ws = store.workspaces.find(w => w.id === wsId);
      if (!ws) return null;
      const paneIds = getAllPaneIds(ws.splitTree);
      paneId = paneIds[0];
    }
    if (!paneId) return null;

    const type = (params?.type || 'terminal') as SurfaceType;
    const surfaceId = store.addSurface(wsId, paneId, type, { colorScheme: params?.colorScheme });
    if (!surfaceId) return null;
    return { surfaceId, paneId };
  };

  /**
   * Update an existing surface's color scheme. Lets users switch a running
   * pane to "prod" mid-session via `pandamux surface set-color-scheme <id> prod`.
   */
  w.__pandamux_setSurfaceColorScheme = (surfaceId: string, colorScheme: string | null) => {
    const store = useStore.getState();
    for (const ws of store.workspaces) {
      const paneIds = getAllPaneIds(ws.splitTree);
      for (const pid of paneIds) {
        const leaf = findLeaf(ws.splitTree, pid);
        if (leaf?.surfaces?.some(s => s.id === surfaceId)) {
          store.updateSurface(ws.id, pid, surfaceId as SurfaceId, {
            colorScheme: colorScheme || undefined,
          });
          return { ok: true };
        }
      }
    }
    return { ok: false, error: 'Surface not found' };
  };

  w.__pandamux_closeSurface = (surfaceId: string, workspaceId?: string) => {
    const store = useStore.getState();
    const wsId = (workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return;
    const paneIds = getAllPaneIds(ws.splitTree);
    for (const pid of paneIds) {
      const leaf = findLeaf(ws.splitTree, pid);
      if (leaf?.surfaces?.some(s => s.id === surfaceId)) {
        store.closeSurface(wsId, pid, surfaceId as SurfaceId);
        return;
      }
    }
  };

  w.__pandamux_focusSurface = (surfaceId: string, workspaceId?: string) => {
    const store = useStore.getState();
    const wsId = (workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return;
    const paneIds = getAllPaneIds(ws.splitTree);
    for (const pid of paneIds) {
      const leaf = findLeaf(ws.splitTree, pid);
      if (leaf?.surfaces) {
        const idx = leaf.surfaces.findIndex(s => s.id === surfaceId);
        if (idx >= 0) {
          store.selectSurface(wsId, pid, idx);
          return;
        }
      }
    }
  };

  w.__pandamux_listSurfaces = (workspaceId?: string) => {
    const store = useStore.getState();
    const wsId = (workspaceId || store.activeWorkspaceId) as WorkspaceId;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return [];

    const paneIds = getAllPaneIds(ws.splitTree);
    const surfaces: Array<{ id: string; type: string; paneId: string; isActive: boolean }> = [];
    for (const pid of paneIds) {
      const leaf = findLeaf(ws.splitTree, pid);
      if (leaf?.surfaces) {
        leaf.surfaces.forEach((s, idx) => {
          surfaces.push({
            id: s.id,
            type: s.type,
            paneId: pid,
            isActive: idx === leaf.activeSurfaceIndex,
          });
        });
      }
    }
    return surfaces;
  };

  w.__pandamux_getActiveSurfaceId = () => {
    const store = useStore.getState();
    const wsId = store.activeWorkspaceId;
    if (!wsId) return null;
    const ws = store.workspaces.find(w => w.id === wsId);
    if (!ws) return null;
    const paneIds = getAllPaneIds(ws.splitTree);
    if (paneIds.length === 0) return null;
    const leaf = findLeaf(ws.splitTree, paneIds[0]);
    if (!leaf?.surfaces?.length) return null;
    const idx = leaf.activeSurfaceIndex ?? 0;
    return leaf.surfaces[idx]?.id || null;
  };

  // ─── Markdown ───────────────────────────────────────────────────────────────

  w.__pandamux_setMarkdownContent = (surfaceId: string, markdown: string) => {
    // Persist into the store so MarkdownPane (re)renders the content. The old
    // `pandamux:markdown-update` CustomEvent had no listener, so content never
    // displayed (issue #54).
    useStore.getState().setMarkdownContent(surfaceId as SurfaceId, markdown ?? '');
    return { ok: true };
  };

  // ─── Notifications ──────────────────────────────────────────────────────────

  w.__pandamux_listNotifications = () => {
    return useStore.getState().notifications || [];
  };

  w.__pandamux_clearNotification = (id: string) => {
    useStore.getState().clearNotification(id);
  };

  w.__pandamux_clearAllNotifications = () => {
    useStore.getState().clearAll();
  };

  // ─── Tree ───────────────────────────────────────────────────────────────────

  w.__pandamux_getTree = (workspaceId?: string) => {
    const store = useStore.getState();
    const wsId = (workspaceId || store.activeWorkspaceId) as WorkspaceId;
    if (!wsId) return null;
    const ws = store.workspaces.find(w => w.id === wsId);
    return ws?.splitTree || null;
  };
}
