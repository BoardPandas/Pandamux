import type { Terminal } from '@xterm/xterm';

interface CursorLikeLayer {
  _cursorRenderers?: unknown;
  _cursorBlinkStateManager?: { value?: { restartBlinkAnimation?: () => void } };
  _render?: (triggeredByAnimationFrame: boolean) => void;
  handleCursorMove?: () => void;
  handleGridChanged?: (start: number, end: number) => void;
  __pandamuxCursorSynced?: boolean;
}

/**
 * Make the Canvas renderer's cursor layer repaint synchronously on every cursor
 * move / grid change.
 *
 * Why: xterm's CanvasAddon renders the cursor in a SEPARATE layer that, when
 * cursorBlink is on, only repaints via the blink timer's requestAnimationFrame
 * (CursorRenderLayer.handleGridChanged → restartBlinkAnimation, deferred). The
 * WebGL renderer instead draws the cursor inline with text every frame, so it
 * always tracks the caret. Under pandamux's Electron renderer (many panes + a live
 * webview browser panel) that blink rAF gets throttled, so the block cursor
 * visibly lags behind typing in TUIs like Claude Code (issue #23). We replicate
 * WebGL's behavior for Canvas by forcing a synchronous _render on move/change
 * while still restarting the blink timer so blinking keeps working.
 *
 * Reaches into private xterm internals (member names survive the addon's
 * minified bundle); fully guarded + idempotent so a future xterm refactor just
 * falls back to the default (laggy) behavior instead of throwing.
 *
 * @returns true if the cursor layer was found and patched.
 */
export function forceSyncCursorRendering(terminal: Terminal): boolean {
  try {
    const renderer: any = (terminal as any)?._core?._renderService?._renderer?.value;
    const layers: unknown = renderer?._renderLayers;
    if (!Array.isArray(layers)) return false;

    const cursor = layers.find(
      (l): l is CursorLikeLayer =>
        !!l && typeof l === 'object' && '_cursorRenderers' in l && typeof (l as any)._render === 'function'
    );
    if (!cursor || cursor.__pandamuxCursorSynced) return false;
    cursor.__pandamuxCursorSynced = true;

    const drawNow = (): void => {
      // restartBlinkAnimation() sets isCursorVisible=true (so _render won't
      // early-return mid-blink) and resets the blink timer so the caret stays
      // solid while the user types.
      cursor._cursorBlinkStateManager?.value?.restartBlinkAnimation?.();
      cursor._render?.(false);
    };

    cursor.handleCursorMove = drawNow;
    cursor.handleGridChanged = drawNow;
    return true;
  } catch {
    return false;
  }
}
