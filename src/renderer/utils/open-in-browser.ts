import { useStore } from '../store';
import { splitNode, getAllPaneIds } from '../store/split-utils';
import { PaneId, SplitNode, SurfaceRef, WorkspaceId } from '../../shared/types';
import { uuid } from '../../shared/id';

function findLeaf(tree: SplitNode, paneId: PaneId): (SplitNode & { type: 'leaf' }) | null {
  if (tree.type === 'leaf') return tree.paneId === paneId ? tree : null;
  return findLeaf(tree.children[0], paneId) ?? findLeaf(tree.children[1], paneId);
}

/** Recursively collect all surfaces from a split tree. */
function getAllSurfaces(node: SplitNode): SurfaceRef[] {
  if (node.type === 'leaf') return node.surfaces;
  return [...getAllSurfaces(node.children[0]), ...getAllSurfaces(node.children[1])];
}

/**
 * Open a URL in the pandamux browser panel.
 * - If Ctrl/Cmd is held, always opens in the system browser.
 * - Otherwise, finds or creates a browser surface in the active workspace,
 *   then navigates to the URL.
 */
export function openInPandaMUXBrowser(url: string, opts?: { forceExternal?: boolean }): void {
  if (opts?.forceExternal) {
    window.pandamux?.system?.openExternal?.(url);
    return;
  }

  const state = useStore.getState();
  const wsId = state.activeWorkspaceId as WorkspaceId;
  if (!wsId) {
    window.pandamux?.system?.openExternal?.(url);
    return;
  }

  const ws = state.workspaces.find(w => w.id === wsId);
  if (!ws) {
    window.pandamux?.system?.openExternal?.(url);
    return;
  }

  // Check if a browser surface already exists in this workspace
  const allSurfaces = getAllSurfaces(ws.splitTree);
  const browserSurface = allSurfaces.find(s => s.type === 'browser');

  if (browserSurface) {
    // Browser exists — just navigate
    window.dispatchEvent(new CustomEvent('pandamux:browser-navigate', { detail: { url, surfaceId: browserSurface.id } }));
    return;
  }

  // No browser — split a new pane to the right with a browser surface
  const paneIds = getAllPaneIds(ws.splitTree);
  const targetPaneId = paneIds[0];
  if (!targetPaneId) {
    window.pandamux?.system?.openExternal?.(url);
    return;
  }

  const newPaneId = `pane-${uuid()}` as PaneId;
  const newTree = splitNode(ws.splitTree, targetPaneId, newPaneId, 'browser', 'horizontal');
  state.updateSplitTree(wsId, newTree);

  // Resolve the surfaceId of the newly created browser pane from the updated tree
  const newSurfaceId = findLeaf(newTree, newPaneId)?.surfaces[0]?.id;

  // Wait for React to mount the BrowserPane + webview dom-ready, then navigate
  // 600ms covers: React render (~16ms) + webview init (~200-500ms)
  setTimeout(() => {
    window.dispatchEvent(new CustomEvent('pandamux:browser-navigate', { detail: { url, surfaceId: newSurfaceId } }));
  }, 600);
}
