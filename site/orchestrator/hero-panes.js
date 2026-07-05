/* hero-panes.js — Wave 2, agent-c
 *
 * Renders a live 4-pane PandaMUX mockup inside .hero__panes[data-mount].
 * Pure DOM, no dependencies, no external assets. The whole module is
 * an IIFE; nothing is exported.
 *
 * Architecture:
 *   - Each pane has a fixed schedule of lines: { t, text, kind? }
 *     where t is the millisecond offset (within a 22s loop) at which
 *     the line begins typing. Rendering is deterministic from
 *     (performance.now() - loopStart): we never accumulate per-frame
 *     state, so resetting the loop is just loopStart = now.
 *   - One requestAnimationFrame loop drives all 4 panes plus the
 *     fade-out / fade-in phase. One setInterval drives the cursor blink.
 *   - The Page Visibility API pauses the loop when the tab is hidden
 *     to save CPU.
 *   - prefers-reduced-motion: render a single static snapshot of the
 *     final "Wave 1 just finished" state and exit.
 */

(() => {
  const mount = document.querySelector('.hero__panes[data-mount]');
  if (!mount) return;

  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ---------------------------------------------------------------- data ---

  // Typing speed in ms per character. ~30 ms feels like a fast human.
  const CHAR_MS = 28;

  // Loop phases (ms from loop start):
  //   0      → typing begins, all panes amber
  //   ~5800  → pane A done (dot → green)
  //   ~7800  → pane B done
  //   ~9800  → pane C done
  //   11000  → orchestrator spawns reviewer
  //   16000  → reviewer ships it, all 4 panes idle
  //   17500  → hold ends, fade begins
  //   19500  → fully faded (60% opacity)
  //   22000  → fully faded back in, reset cycle
  const HOLD_END = 17500;
  const FADE_OUT_END = 19500;
  const RESET_END = 22000;
  const FADE_FLOOR = 0.6;

  const paneA = {
    label: 'agent-a · types.ts',
    lines: [
      { t:    0, text: '$ Read src/auth/schema.sql' },
      { t:  900, text: '$ Edit types.ts +42' },
      { t: 1700, text: '  export type User = {' },
      { t: 2300, text: '    id: string' },
      { t: 2800, text: '    email: string' },
      { t: 3300, text: '  }' },
      { t: 3700, text: '$ Edit types.ts +18' },
      { t: 4500, text: '  export type AuthToken = string' },
      { t: 5500, text: '✓ done · 5 tool calls · 2.1s', kind: 'done' },
    ],
  };

  const paneB = {
    label: 'agent-b · service.ts',
    lines: [
      { t:    0, text: '$ Read types.ts' },
      { t:  700, text: '$ Edit service.ts +67' },
      { t: 1500, text: '  export async function login(' },
      { t: 2100, text: '    email, password' },
      { t: 2700, text: '  )' },
      { t: 3100, text: '$ Edit service.ts +24' },
      { t: 3900, text: '  export async function verify(' },
      { t: 4500, text: '    token' },
      { t: 5000, text: '  )' },
      { t: 5500, text: '$ Bash npm test auth' },
      { t: 6500, text: '  ✓ 12 passed' },
      { t: 7500, text: '✓ done · 8 tool calls · 3.4s', kind: 'done' },
    ],
  };

  const paneC = {
    label: 'agent-c · routes.ts',
    lines: [
      { t:    0, text: '$ Read service.ts' },
      { t:  900, text: '$ Edit routes.ts +34' },
      { t: 1900, text: "  router.post('/login', login)" },
      { t: 2900, text: "  router.get('/me', currentUser)" },
      { t: 3800, text: '$ Edit routes.ts +12' },
      { t: 4700, text: '  router.use(authMiddleware)' },
      { t: 5700, text: '$ Bash curl localhost:3000/login' },
      { t: 7100, text: '  { "token": "jwt..." }' },
      { t: 8400, text: '✓ done · 6 tool calls · 2.8s', kind: 'done' },
    ],
  };

  const paneD = {
    label: 'orchestrator',
    lines: [
      { t:    0, text: '[wave 1] spawning 3 agents...' },
      { t:  600, text: '  → agent-a (types.ts)' },
      { t: 1100, text: '  → agent-b (service.ts)' },
      { t: 1600, text: '  → agent-c (routes.ts)' },
      { t: 5800, text: '[wave 1] agent-a ✓ 5 calls · 2.1s' },
      { t: 7800, text: '[wave 1] agent-b ✓ 8 calls · 3.4s' },
      { t: 9800, text: '[wave 1] agent-c ✓ 6 calls · 2.8s' },
      { t:11000, text: '[wave 2] spawning reviewer...' },
      { t:11800, text: '[review] checking exports' },
      { t:12500, text: '[review] 0 orphan exports' },
      { t:13200, text: '[review] checking types' },
      { t:13900, text: '[review] 0 type mismatches' },
      { t:14600, text: '[review] running tests' },
      { t:15300, text: '[review] ✓ all passed' },
      { t:15900, text: '[review] ✓ ship it', kind: 'done' },
    ],
  };

  const PANES = [paneA, paneB, paneC, paneD];

  // ---------------------------------------------------------------- DOM ---

  // Defensive: clear any prior content (e.g. ::after placeholder will
  // simply re-hide once children exist).
  while (mount.firstChild) mount.removeChild(mount.firstChild);

  const grid = document.createElement('div');
  grid.className = 'hero__panes__grid';
  Object.assign(grid.style, {
    position: 'absolute',
    inset: '0',
    display: 'grid',
    gridTemplateColumns: '1fr 1fr',
    gridTemplateRows: '1fr 1fr',
    gap: '6px',
    padding: '8px',
    boxSizing: 'border-box',
    fontFamily: 'var(--font-mono)',
    pointerEvents: 'none',
  });
  mount.appendChild(grid);

  // Build one pane element. Returns the handles we need each frame.
  function buildPane(pane) {
    const el = document.createElement('div');
    el.className = 'hero__panes__pane';
    Object.assign(el.style, {
      display: 'flex',
      flexDirection: 'column',
      background: 'var(--bg-elev)',
      border: '1px solid var(--border-strong)',
      borderRadius: '6px',
      overflow: 'hidden',
      minHeight: '0',
      minWidth: '0',
      transition:
        'opacity var(--dur-slow) var(--ease-out), ' +
        'border-color var(--dur) var(--ease)',
      boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.02)',
    });

    const bar = document.createElement('div');
    Object.assign(bar.style, {
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      padding: '6px 10px',
      borderBottom: '1px solid var(--border)',
      background: 'var(--bg-elev-2)',
      flexShrink: '0',
      gap: '8px',
    });

    const label = document.createElement('span');
    label.className = 'hero__panes__label';
    label.textContent = pane.label;
    Object.assign(label.style, {
      fontSize: '10px',
      letterSpacing: '0.04em',
      color: 'var(--text-dim)',
      fontFamily: 'var(--font-mono)',
      whiteSpace: 'nowrap',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
    });

    const dot = document.createElement('span');
    dot.className = 'hero__panes__dot';
    Object.assign(dot.style, {
      display: 'inline-block',
      width: '7px',
      height: '7px',
      borderRadius: '50%',
      background: 'var(--accent)',
      flexShrink: '0',
      transition: 'background var(--dur) var(--ease)',
    });
    dot.dataset.state = 'writing';

    bar.appendChild(label);
    bar.appendChild(dot);

    // body (clip) → inner (translated for tail-scroll) → lines + cursor
    const body = document.createElement('div');
    Object.assign(body.style, {
      flex: '1',
      padding: '8px 10px',
      overflow: 'hidden',
      position: 'relative',
      minHeight: '0',
    });

    const inner = document.createElement('div');
    Object.assign(inner.style, {
      fontFamily: 'var(--font-mono)',
      fontSize: '10px',
      lineHeight: '1.55',
      color: 'var(--text)',
      whiteSpace: 'pre',
      transition: 'transform var(--dur-fast) var(--ease)',
    });
    body.appendChild(inner);

    const cursor = document.createElement('span');
    cursor.className = 'hero__panes__cursor';
    cursor.textContent = '_';
    Object.assign(cursor.style, {
      color: 'var(--accent)',
      marginLeft: '1px',
      fontWeight: '700',
    });
    inner.appendChild(cursor);

    el.appendChild(bar);
    el.appendChild(body);
    grid.appendChild(el);

    return { el, dot, body, inner, cursor, lineEls: [] };
  }

  const handles = PANES.map(buildPane);

  // Pre-compute typing duration for each line so we know when "done".
  PANES.forEach((p) => {
    p.lines.forEach((l) => {
      l.dur = Math.max(120, l.text.length * CHAR_MS);
    });
  });

  // ---------------------------------------------------- reduced motion ---

  // One static snapshot — final "ship it" state, no motion at all.
  if (reduced) {
    PANES.forEach((p, i) => {
      const h = handles[i];
      // Insert lines BEFORE the cursor (which we then hide)
      p.lines.forEach((l) => {
        const ln = document.createElement('div');
        ln.textContent = l.text;
        ln.style.color = colorForLine(l);
        h.inner.insertBefore(ln, h.cursor);
      });
      h.cursor.style.display = 'none';
      h.dot.style.background = 'var(--success)';
      h.dot.dataset.state = 'done';
    });
    return;
  }

  // ----------------------------------------------------- helper: colors ---

  function colorForLine(line) {
    if (line.kind === 'done') return 'var(--success)';
    const t = line.text;
    if (t.startsWith('$')) return 'var(--text)';
    if (t.startsWith('[wave')) return 'var(--accent)';
    if (t.startsWith('[review]')) return 'var(--text-dim)';
    if (t.startsWith('  →')) return 'var(--text-dim)';
    if (t.startsWith('  ✓')) return 'var(--success)';
    return 'var(--text-faint)';
  }

  // ------------------------------------------------------- main loop ---

  let loopStart = performance.now();

  // Visibility pause: we shift loopStart forward by the elapsed pause
  // so the loop appears frozen and resumes seamlessly.
  let pausedAt = 0;
  let isPaused = false;

  // Cursor blink — single shared interval, toggles every cursor at once.
  let cursorVisible = true;
  setInterval(() => {
    cursorVisible = !cursorVisible;
    handles.forEach((h) => {
      // Only blink while the cursor is "active" (i.e. pane not done)
      if (h.cursor.parentNode && h.cursor.style.display !== 'none') {
        h.cursor.style.opacity = cursorVisible ? '1' : '0';
      }
    });
  }, 500);

  function resetCycle() {
    handles.forEach((h) => {
      // Remove every line element; keep cursor as last child
      h.lineEls.forEach((el) => el.remove());
      h.lineEls = [];
      h.cursor.style.display = '';
      h.cursor.style.opacity = '1';
      h.dot.style.background = 'var(--accent)';
      h.dot.dataset.state = 'writing';
      h.inner.style.transform = '';
    });
  }

  function frame() {
    requestAnimationFrame(frame);
    if (isPaused) return;

    const now = performance.now();
    let t = now - loopStart;

    if (t >= RESET_END) {
      loopStart = now;
      t = 0;
      resetCycle();
    }

    // Phase: opacity envelope -----------------------------------------
    let opacity = 1;
    if (t > HOLD_END && t <= FADE_OUT_END) {
      const k = (t - HOLD_END) / (FADE_OUT_END - HOLD_END);
      opacity = 1 - (1 - FADE_FLOOR) * k;
    } else if (t > FADE_OUT_END && t <= RESET_END) {
      const k = (t - FADE_OUT_END) / (RESET_END - FADE_OUT_END);
      opacity = FADE_FLOOR + (1 - FADE_FLOOR) * k;
    }

    // Per-pane render -------------------------------------------------
    for (let p = 0; p < PANES.length; p++) {
      const pane = PANES[p];
      const h = handles[p];
      h.el.style.opacity = String(opacity);

      // How many lines should be present at time t?
      let visible = 0;
      for (let i = 0; i < pane.lines.length; i++) {
        if (pane.lines[i].t <= t) visible = i + 1;
        else break;
      }

      // Append any new line divs (insert BEFORE cursor)
      while (h.lineEls.length < visible) {
        const i = h.lineEls.length;
        const ln = document.createElement('div');
        ln.style.color = colorForLine(pane.lines[i]);
        ln.textContent = '';
        h.inner.insertBefore(ln, h.cursor);
        h.lineEls.push(ln);
      }

      // Type the active (latest) line; finalize earlier lines
      if (visible > 0) {
        for (let i = 0; i < visible - 1; i++) {
          const ln = h.lineEls[i];
          const full = pane.lines[i].text;
          if (ln.textContent !== full) ln.textContent = full;
        }
        const i = visible - 1;
        const line = pane.lines[i];
        const elapsed = t - line.t;
        const chars = Math.max(
          0,
          Math.min(line.text.length, Math.floor(elapsed / CHAR_MS))
        );
        const ln = h.lineEls[i];
        if (ln.textContent.length !== chars) {
          ln.textContent = line.text.slice(0, chars);
        }
      }

      // Done = all lines visible AND last line fully typed
      const lastIdx = pane.lines.length - 1;
      const allLinesPresent = visible === pane.lines.length;
      const lastLineFull =
        allLinesPresent &&
        h.lineEls[lastIdx] &&
        h.lineEls[lastIdx].textContent === pane.lines[lastIdx].text;
      const done = allLinesPresent && lastLineFull;

      if (done) {
        if (h.dot.dataset.state !== 'done') {
          h.dot.dataset.state = 'done';
          h.dot.style.background = 'var(--success)';
          h.dot.style.opacity = '1';
        }
        if (h.cursor.style.display !== 'none') {
          h.cursor.style.display = 'none';
        }
      } else {
        if (h.dot.dataset.state !== 'writing') {
          h.dot.dataset.state = 'writing';
          h.dot.style.background = 'var(--accent)';
        }
        if (h.cursor.style.display === 'none') {
          h.cursor.style.display = '';
        }
        // Sine-pulse for the dot, locked to wall clock so all panes
        // breathe together.
        h.dot.style.opacity = String(0.55 + 0.45 * (0.5 + 0.5 * Math.sin(now * 0.0035)));
      }

      // Tail-scroll: translate the inner block up so the latest line
      // (and cursor) stay in view.
      const overflow = h.inner.offsetHeight - h.body.clientHeight;
      if (overflow > 0) {
        h.inner.style.transform = 'translateY(' + (-overflow - 4) + 'px)';
      } else if (h.inner.style.transform) {
        h.inner.style.transform = '';
      }
    }
  }

  requestAnimationFrame(frame);

  // ------------------------------------------------ visibility pause ---

  document.addEventListener('visibilitychange', () => {
    if (document.hidden) {
      isPaused = true;
      pausedAt = performance.now();
    } else if (isPaused) {
      const drift = performance.now() - pausedAt;
      loopStart += drift;
      isPaused = false;
    }
  });
})();
