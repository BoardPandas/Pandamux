/* ambient.js — pandamux-orchestrator marketing site
 *
 * Two ambient interactive layers:
 *   1. Cursor-follow glow: writes --mouse-x / --mouse-y on <html>, fed to
 *      .cursor-glow's top/left in styles.css. Disabled on touch-only and
 *      reduced-motion.
 *   2. Activity rail: faux orchestrator event stream rendered into
 *      <aside class="activity-rail">. Hidden below 1280px by CSS.
 *
 * No imports, no build step. IIFE, plain browser JS.
 */

(function () {
  'use strict';

  // ---------------------------------------------------------------------
  // Constants
  // ---------------------------------------------------------------------

  const RAIL_MIN_WIDTH = 1280;
  const MAX_LINES = 24;
  const INITIAL_LINES = 12;
  const TICK_MIN_MS = 1800;
  const TICK_MAX_MS = 2400;
  const TICK_REDUCED_MS = 5000;

  const AGENTS = ['A', 'B', 'C', 'D'];

  const FILE_PATHS = [
    'src/auth/types.ts',
    'src/auth/service.ts',
    'src/auth/middleware.ts',
    'src/auth/jwt.ts',
    'src/auth/session.ts',
    'src/auth/guards.ts',
    'src/auth/refresh.ts',
    'tests/auth.spec.ts',
    'tests/jwt.spec.ts',
    'tests/middleware.spec.ts',
    'tests/session.spec.ts',
    'src/api/routes.ts',
    'src/api/handlers.ts',
    'src/db/migrations/004_users.sql',
    'src/db/queries/users.ts',
    'src/lib/crypto.ts',
    'src/utils/errors.ts',
    'src/index.ts',
    'package.json',
    'tsconfig.json',
    'README.md',
    'docs/auth.md'
  ];

  const FS_TOOLS = ['Read', 'Edit', 'Write', 'Grep', 'Glob'];
  const BASH_CMDS = [
    'npm test',
    'npm run lint',
    'tsc --noEmit',
    'pnpm install',
    'git status -sb',
    'rg "TODO"',
    'vitest run',
    'eslint src/'
  ];
  const GREP_TARGETS = [
    '"interface User"',
    '"export function"',
    '"@deprecated"',
    '"useAuth"',
    '"verifyToken"'
  ];

  // ---------------------------------------------------------------------
  // Tiny helpers
  // ---------------------------------------------------------------------

  function pick(arr) {
    return arr[Math.floor(Math.random() * arr.length)];
  }

  function randInt(min, max) {
    return Math.floor(Math.random() * (max - min + 1)) + min;
  }

  function pad2(n) {
    return n < 10 ? '0' + n : '' + n;
  }

  function formatTime(d) {
    return pad2(d.getHours()) + ':' + pad2(d.getMinutes()) + ':' + pad2(d.getSeconds());
  }

  // ---------------------------------------------------------------------
  // Cursor follow
  // ---------------------------------------------------------------------

  function setupCursorFollow(prefersReduced, isTouchOnly) {
    const docEl = document.documentElement;

    if (prefersReduced) {
      docEl.style.setProperty('--mouse-x', '50vw');
      docEl.style.setProperty('--mouse-y', '50vh');
      return;
    }
    if (isTouchOnly) return;

    let pendingX = window.innerWidth / 2;
    let pendingY = window.innerHeight / 2;
    let queued = false;

    function flush() {
      docEl.style.setProperty('--mouse-x', pendingX + 'px');
      docEl.style.setProperty('--mouse-y', pendingY + 'px');
      queued = false;
    }

    window.addEventListener(
      'mousemove',
      function (e) {
        pendingX = e.clientX;
        pendingY = e.clientY;
        if (!queued) {
          queued = true;
          requestAnimationFrame(flush);
        }
      },
      { passive: true }
    );
  }

  // ---------------------------------------------------------------------
  // Activity rail event buffer
  // ---------------------------------------------------------------------

  function buildEventBuffer() {
    const out = [];

    function push(id, text) {
      out.push({ id: id, text: text });
    }

    // Wave 1 — foundations
    push('orch', 'wave-1 spawning 2 agents');
    push('A', 'tool_use Read src/auth/types.ts');
    push('B', 'tool_use Read src/auth/service.ts');
    push('A', 'tool_use Grep ' + pick(GREP_TARGETS));
    push('A', 'tool_use Edit src/auth/types.ts');
    push('B', 'tool_use Bash npm test');
    push('B', 'status running 14 tools');
    push('orch', 'wave-1 progress 40%');
    push('A', 'tool_use Write src/auth/types.ts');
    push('A', 'tool_use Read tests/auth.spec.ts');
    push('orch', 'wave-1 progress 67%');
    push('A', 'status done 9 tools');
    push('B', 'tool_use Bash tsc --noEmit');
    push('B', 'status done 18 tools');
    push('orch', 'wave-1 complete 2 files modified');

    // Wave 2 — dependents
    push('orch', 'wave-2 spawning 2 agents');
    push('C', 'tool_use Read src/auth/types.ts');
    push('D', 'tool_use Read src/auth/types.ts');
    push('C', 'tool_use Read src/auth/service.ts');
    push('C', 'tool_use Edit src/auth/middleware.ts');
    push('D', 'tool_use Read tests/auth.spec.ts');
    push('C', 'tool_use Bash npm run lint');
    push('orch', 'wave-2 progress 50%');
    push('D', 'tool_use Write tests/middleware.spec.ts');
    push('C', 'tool_use Edit src/auth/middleware.ts');
    push('C', 'status done 22 tools');
    push('D', 'status done 17 tools');
    push('orch', 'wave-2 complete 4 files modified');

    // Reviewer pass
    push('reviewer', 'scanning imports');
    push('reviewer', 'checking type compatibility');
    push('reviewer', 'verifying export chains');
    push('reviewer', 'auto-fixing 1 unused import');
    push('reviewer', 'ok 0 conflicts');
    push('orch', 'complete 4 files modified · awaiting commit');

    // Filler — varied tool calls so the loop doesn't feel repetitive
    for (let i = 0; i < 50; i++) {
      const agent = pick(AGENTS);
      const roll = Math.random();
      if (roll < 0.5) {
        push(agent, 'tool_use ' + pick(FS_TOOLS) + ' ' + pick(FILE_PATHS));
      } else if (roll < 0.7) {
        push(agent, 'tool_use Bash ' + pick(BASH_CMDS));
      } else if (roll < 0.85) {
        push(agent, 'status running ' + randInt(3, 25) + ' tools');
      } else {
        push('orch', 'heartbeat agent-' + agent.toLowerCase() + ' ok');
      }
    }

    return out;
  }

  // ---------------------------------------------------------------------
  // Activity rail DOM
  // ---------------------------------------------------------------------

  function buildLineEl(item) {
    const el = document.createElement('div');
    el.className = 'activity-rail__line';

    const t = document.createElement('span');
    t.className = 't';
    t.textContent = item.t;

    const id = document.createElement('span');
    id.className = 'id';
    id.textContent = item.id;

    el.appendChild(t);
    el.appendChild(id);
    el.appendChild(document.createTextNode(item.text));
    return el;
  }

  function setupActivityRail(prefersReduced) {
    const rail = document.querySelector('.activity-rail');
    if (!rail) return;
    if (window.innerWidth < RAIL_MIN_WIDTH) return;

    const buffer = buildEventBuffer();
    if (buffer.length === 0) return;

    let cursor = 0;
    let stopped = false;
    let timer = null;

    function takeNext() {
      const e = buffer[cursor % buffer.length];
      cursor++;
      return e;
    }

    function appendItem(rawEvent, timestamp, animate) {
      const item = {
        t: timestamp,
        id: rawEvent.id,
        text: rawEvent.text
      };
      const el = buildLineEl(item);
      if (!animate) {
        el.style.animation = 'none';
        el.style.opacity = '1';
      }
      rail.appendChild(el);

      // Trim oldest line(s) once we exceed the cap.
      // Pseudo-elements (::before label, ::after fade) are NOT in .children,
      // so children[0] is always the oldest line.
      while (rail.children.length > MAX_LINES) {
        const first = rail.children[0];
        if (!first) break;
        first.remove();
      }

      // Defensive: nudge scroll to bottom in case CSS overflow ever changes.
      rail.scrollTop = rail.scrollHeight;
    }

    // Initial fill: stagger timestamps backwards from "now" so the rail
    // looks like it's been alive for ~30 seconds when the page loads.
    const now = new Date();
    const startBase = now.getTime() - 30000;
    for (let i = 0; i < INITIAL_LINES; i++) {
      const ts = new Date(startBase + i * 2200);
      appendItem(takeNext(), formatTime(ts), !prefersReduced);
    }

    // Live ticker
    function schedule() {
      if (stopped) return;
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      const delay = prefersReduced
        ? TICK_REDUCED_MS
        : randInt(TICK_MIN_MS, TICK_MAX_MS);
      timer = setTimeout(tickOnce, delay);
    }

    function tickOnce() {
      timer = null;
      if (stopped) return;
      if (document.visibilityState !== 'visible') return;
      appendItem(takeNext(), formatTime(new Date()), !prefersReduced);
      schedule();
    }

    schedule();

    // Pause when the tab is hidden — resume on visibility return.
    document.addEventListener('visibilitychange', function () {
      if (stopped) return;
      if (document.visibilityState === 'visible') {
        if (!timer) schedule();
      } else if (timer) {
        clearTimeout(timer);
        timer = null;
      }
    });

    // If the viewport drops below the rail breakpoint, stop ticking.
    // We don't tear down existing lines — CSS already hides the rail.
    window.addEventListener(
      'resize',
      function () {
        if (window.innerWidth < RAIL_MIN_WIDTH && !stopped) {
          stopped = true;
          if (timer) {
            clearTimeout(timer);
            timer = null;
          }
        }
      },
      { passive: true }
    );
  }

  // ---------------------------------------------------------------------
  // Polish — cursor glow tuning, grain dim, hint card, footer cursor
  // ---------------------------------------------------------------------

  function tuneCursorGlow() {
    const glow = document.querySelector('.cursor-glow');
    if (!glow) return;

    // Read CSS-owned opacity and scale to ~45% so the amber glow is present
    // but not attention-grabbing. Keeps CSS authoritative.
    const computed = window.getComputedStyle(glow);
    const base = parseFloat(computed.opacity);
    let target = (isFinite(base) && base > 0 ? base : 1) * 0.45;

    // Firefox has no `mix-blend-mode: plus-lighter` — it composites normally,
    // which makes an amber glow read muddy. Drop another 30% to compensate.
    const supportsPlusLighter =
      window.CSS &&
      typeof window.CSS.supports === 'function' &&
      window.CSS.supports('mix-blend-mode', 'plus-lighter');
    if (!supportsPlusLighter) target *= 0.7;

    glow.style.opacity = target.toFixed(3);
  }

  function dimGrain() {
    const grain = document.querySelector('.grain');
    if (!grain) return;
    // Override whatever CSS set — grain should be barely perceptible.
    grain.style.opacity = '0.04';
  }

  function showKeyboardHint(prefersReduced) {
    if (prefersReduced) return;
    try {
      if (sessionStorage.getItem('pandamux-hint-seen')) return;
    } catch (e) {
      // sessionStorage unavailable (private mode, disabled cookies) — skip.
      return;
    }

    const card = document.createElement('div');
    card.setAttribute('aria-hidden', 'true');
    card.textContent = 'press wave to replay \u00b7 esc to dismiss';
    Object.assign(card.style, {
      position: 'fixed',
      right: '24px',
      bottom: '24px',
      zIndex: '100',
      background: 'rgba(18, 18, 22, 0.92)',
      backdropFilter: 'blur(12px)',
      WebkitBackdropFilter: 'blur(12px)',
      border: '1px solid var(--border-strong)',
      padding: '10px 14px',
      fontFamily: 'var(--font-mono)',
      fontSize: 'var(--fs-xs)',
      color: 'var(--text-dim)',
      borderRadius: '6px',
      boxShadow: '0 4px 20px rgba(0,0,0,0.4)',
      opacity: '0',
      transition: 'opacity 500ms var(--ease)',
      pointerEvents: 'none'
    });

    document.body.appendChild(card);

    // Double-rAF so the 0 → 1 transition actually animates.
    requestAnimationFrame(function () {
      requestAnimationFrame(function () {
        card.style.opacity = '1';
      });
    });

    let dismissed = false;
    let holdTimer = null;

    function onKey(e) {
      if (e.key === 'Escape') dismiss();
    }

    function dismiss() {
      if (dismissed) return;
      dismissed = true;
      if (holdTimer) {
        clearTimeout(holdTimer);
        holdTimer = null;
      }
      card.style.transition = 'opacity 400ms var(--ease)';
      card.style.opacity = '0';
      document.removeEventListener('keydown', onKey);
      try {
        sessionStorage.setItem('pandamux-hint-seen', '1');
      } catch (e) { /* noop */ }
      setTimeout(function () {
        if (card.parentNode) card.parentNode.removeChild(card);
      }, 420);
    }

    document.addEventListener('keydown', onKey);

    // Fade in (500ms) + hold (2000ms) → begin fade out.
    holdTimer = setTimeout(dismiss, 2500);
  }

  function injectFooterCursor(prefersReduced) {
    const footerInner = document.querySelector('.site-footer__inner');
    if (!footerInner) return;
    // Idempotent — safe if init runs twice.
    if (footerInner.querySelector('.site-footer__cursor')) return;

    const sep = document.createElement('span');
    sep.className = 'site-footer__sep';
    sep.setAttribute('aria-hidden', 'true');
    sep.textContent = '\u00b7';

    const cursor = document.createElement('span');
    // In reduced-motion, drop the animation-carrying class so the glyph is
    // rendered as a static accent mark.
    cursor.className = prefersReduced
      ? 'site-footer__cursor'
      : 'site-footer__cursor site-header__mark__cursor';
    cursor.textContent = '_';
    cursor.setAttribute('aria-hidden', 'true');
    cursor.style.color = 'var(--accent)';
    cursor.style.marginLeft = '4px';

    footerInner.appendChild(sep);
    footerInner.appendChild(cursor);
  }

  // ---------------------------------------------------------------------
  // Bootstrap
  // ---------------------------------------------------------------------

  function init() {
    const prefersReduced =
      window.matchMedia &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    const isTouchOnly =
      window.matchMedia && window.matchMedia('(hover: none)').matches;

    setupCursorFollow(prefersReduced, isTouchOnly);
    setupActivityRail(prefersReduced);
    tuneCursorGlow();
    dimGrain();
    injectFooterCursor(prefersReduced);
    showKeyboardHint(prefersReduced);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
