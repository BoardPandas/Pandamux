import type { Terminal } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';

export type RendererKind = 'dom' | 'webgl';

export interface RendererHandle {
  kind: RendererKind;
  dispose(): void;
}

/**
 * Chromium hard-caps WebGL contexts (~16 per renderer process) and force-loses
 * the oldest one past the cap, which used to freeze whole sessions when every
 * keep-alive tab held a context. Only VISIBLE panes need a GPU renderer, so we
 * attach WebGL on show / release on hide and budget well under the cap (the
 * browser webview and devtools also consume contexts).
 */
export const MAX_WEBGL_TERMINALS = 12;

let activeWebglCount = 0;

export function getActiveWebglCount(): number {
  return activeWebglCount;
}

/** Handle for a terminal left on xterm's built-in DOM renderer (no addon). */
function domHandle(): RendererHandle {
  return { kind: 'dom', dispose: () => { /* default renderer, nothing to release */ } };
}

/**
 * Attach the best available renderer to a terminal that just became visible.
 * Preference order: WebGL (maintained upstream, correct wide-char/CJK and
 * cursor rendering) → xterm's default DOM renderer. The Canvas addon was
 * dropped in xterm 6 (never republished for the 6.0 API and already deprecated
 * for mispainting rows under load, issues #23/#30), so DOM is now the only
 * fallback.
 */
export function attachVisibleRenderer(terminal: Terminal): RendererHandle {
  // Over the WebGL budget: stay on the DOM renderer for this terminal.
  if (activeWebglCount >= MAX_WEBGL_TERMINALS) return domHandle();

  let webgl: WebglAddon;
  try {
    webgl = new WebglAddon();
    terminal.loadAddon(webgl);
  } catch (err) {
    console.warn('[pandamux] WebGL renderer unavailable, staying on DOM renderer:', err);
    return domHandle();
  }
  activeWebglCount++;

  let released = false;
  const release = (): void => {
    if (released) return;
    released = true;
    activeWebglCount--;
  };

  const handle: RendererHandle = {
    kind: 'webgl',
    dispose: () => {
      release();
      try { webgl.dispose(); } catch { /* already disposed with terminal */ }
    },
  };

  webgl.onContextLoss(() => {
    // GPU evicted this context (driver reset, context pressure elsewhere…).
    // Downgrade this terminal to the DOM renderer instead of leaving it frozen.
    console.warn('[pandamux] WebGL context lost, downgrading terminal to DOM renderer');
    release();
    try { webgl.dispose(); } catch { /* no-op */ }
    const fallback = domHandle();
    handle.kind = fallback.kind;
    handle.dispose = fallback.dispose;
  });

  return handle;
}
