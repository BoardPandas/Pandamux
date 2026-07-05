/**
 * wave-sim.js — interactive wave simulator for the pandamux-orchestrator page.
 *
 * Mounts inside #wave-sim-root and renders a faux PandaMUX window with wave lanes
 * and agent panes. When the user clicks "Orchestrate" the simulator runs a
 * scripted two-wave orchestration followed by a reviewer card. Everything is
 * driven by setTimeout + requestAnimationFrame — no real agents.
 *
 * Class-name contract with styles.css (Agent A):
 *   .wsim, .wsim__toolbar, .wsim__title, .wsim__play, .wsim__reset,
 *   .wsim__body, .wsim__lane[data-wave][data-active],
 *   .wsim__lane-label, .wsim__panes, .wsim__arrow,
 *   .wsim__pane[data-agent][data-state],
 *     .wsim__pane__header, .wsim__pane__id, .wsim__pane__dot,
 *     .wsim__pane__label, .wsim__pane__log, .wsim__pane__logline,
 *     .wsim__pane__meta, .wsim__pane__tooluse, .wsim__pane__time,
 *   .wsim__review[data-state], .wsim__review__label, .wsim__review__body.
 * JS sets data-state / data-active / --wsim-progress; CSS owns every visual.
 * Log-lines get an `.is-visible` class one frame after insertion so CSS can
 * fade them in.
 */

(function () {
  'use strict';

  const TIMELINE = {
    waves: [
      {
        label: 'WAVE 1 — FOUNDATION',
        agents: [
          {
            id: 'a', label: 'shared types',
            logs: [
              { t: 200, msg: 'reading src/auth/types.ts' },
              { t: 800, msg: 'tracing imports' },
              { t: 1600, msg: 'writing src/shared/schema.ts' },
              { t: 2400, msg: 'tool-use count: 12' },
              { t: 3000, msg: 'done — 1 file written' },
            ],
            duration: 3200,
            tools: 12,
          },
          {
            id: 'b', label: 'auth service',
            logs: [
              { t: 300, msg: 'reading src/auth/service.ts' },
              { t: 1000, msg: 'refactoring to JWT' },
              { t: 2100, msg: 'updating token validation' },
              { t: 3300, msg: 'tool-use count: 18' },
              { t: 3900, msg: 'done — 2 files written' },
            ],
            duration: 4100,
            tools: 18,
          },
          {
            id: 'c', label: 'middleware',
            logs: [
              { t: 250, msg: 'reading src/auth/middleware.ts' },
              { t: 900, msg: 'rewriting verify() hook' },
              { t: 1800, msg: 'updating request context' },
              { t: 2700, msg: 'tool-use count: 9' },
              { t: 3400, msg: 'done — 1 file written' },
            ],
            duration: 3600,
            tools: 9,
          },
        ],
      },
      {
        label: 'WAVE 2 — INTEGRATION',
        agents: [
          {
            id: 'd', label: 'integration tests',
            logs: [
              { t: 300, msg: 'consuming wave 1 results' },
              { t: 1000, msg: 'writing tests/auth.spec.ts' },
              { t: 2000, msg: 'running vitest...' },
              { t: 3100, msg: '14 passed, 0 failed' },
              { t: 3700, msg: 'tool-use count: 22' },
            ],
            duration: 4000,
            tools: 22,
          },
        ],
      },
    ],
    review: {
      delay: 400,
      duration: 2000,
      lines: [
        'checking type compatibility... ok',
        'checking import chains... ok',
        'scanning for orphaned exports... ok',
        'summary: 4 files modified, 0 conflicts, 14 tests passing',
      ],
    },
  };

  const reducedMotion = typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  const state = {
    root: null,
    frame: null,
    playBtn: null,
    resetBtn: null,
    reviewEl: null,
    panesById: new Map(),
    timers: new Set(),
    rafs: new Set(),
    isPlaying: false,
    lastReset: 0,
  };

  // --- scheduler helpers --------------------------------------------------

  function scheduleTimeout(fn, delay) {
    const id = window.setTimeout(() => {
      state.timers.delete(id);
      fn();
    }, delay);
    state.timers.add(id);
    return id;
  }

  function scheduleFrame(fn) {
    const id = window.requestAnimationFrame((ts) => {
      state.rafs.delete(id);
      fn(ts);
    });
    state.rafs.add(id);
    return id;
  }

  function clearAll() {
    state.timers.forEach((id) => window.clearTimeout(id));
    state.timers.clear();
    state.rafs.forEach((id) => window.cancelAnimationFrame(id));
    state.rafs.clear();
  }

  // --- dom builders -------------------------------------------------------

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function buildSimulator() {
    const frame = el('div', 'wsim');

    const toolbar = el('div', 'wsim__toolbar');
    toolbar.append(el('div', 'wsim__title', 'WAVE SIMULATOR · orch-demo'));

    const play = el('button', 'wsim__play', 'Orchestrate');
    play.type = 'button';
    play.setAttribute('aria-label', 'Play wave simulation');

    const reset = el('button', 'wsim__reset', 'Reset');
    reset.type = 'button';
    reset.setAttribute('aria-label', 'Reset simulation');
    reset.hidden = true;

    toolbar.append(play, reset);

    const body = el('div', 'wsim__body');
    TIMELINE.waves.forEach((wave, idx) => {
      if (idx > 0) body.append(el('div', 'wsim__arrow'));
      body.append(buildLane(wave, idx + 1));
    });
    body.append(el('div', 'wsim__arrow'));
    const reviewEl = buildReview();
    body.append(reviewEl);

    frame.append(toolbar, body);

    state.frame = frame;
    state.playBtn = play;
    state.resetBtn = reset;
    state.reviewEl = reviewEl;

    play.addEventListener('click', handlePlay);
    reset.addEventListener('click', handleReset);

    return frame;
  }

  function buildLane(wave, waveNumber) {
    const lane = el('div', 'wsim__lane');
    lane.setAttribute('data-wave', String(waveNumber));
    lane.setAttribute('data-active', waveNumber === 1 ? 'true' : 'false');
    lane.append(el('div', 'wsim__lane-label', wave.label));

    const panes = el('div', 'wsim__panes');
    wave.agents.forEach((agentConfig) => panes.append(buildPane(agentConfig)));
    lane.append(panes);
    return lane;
  }

  function buildPane(agentConfig) {
    const pane = el('div', 'wsim__pane');
    pane.setAttribute('data-agent', agentConfig.id);
    pane.setAttribute('data-state', 'pending');
    pane.setAttribute('role', 'status');
    pane.setAttribute('aria-live', 'polite');

    const header = el('div', 'wsim__pane__header');
    header.append(
      el('span', 'wsim__pane__id', `AGENT-${agentConfig.id.toUpperCase()}`),
      el('span', 'wsim__pane__dot'),
    );

    const log = el('div', 'wsim__pane__log');

    const meta = el('div', 'wsim__pane__meta');
    const tooluse = el('span', 'wsim__pane__tooluse', '0 tool uses');
    const time = el('span', 'wsim__pane__time', '0.0s');
    meta.append(tooluse, time);

    pane.append(
      header,
      el('div', 'wsim__pane__label', agentConfig.label),
      log,
      meta,
    );

    state.panesById.set(agentConfig.id, {
      pane, log, tooluse, time, config: agentConfig,
    });
    return pane;
  }

  function buildReview() {
    const review = el('div', 'wsim__review');
    review.setAttribute('data-state', 'hidden');
    review.append(el('div', 'wsim__review__label', 'REVIEWER'));
    review.append(el('div', 'wsim__review__body'));
    return review;
  }

  // --- simulation runners -------------------------------------------------

  function handlePlay() {
    if (state.isPlaying) return;
    state.isPlaying = true;
    state.playBtn.hidden = true;
    state.resetBtn.hidden = true;
    runWave(0);
  }

  function handleReset() {
    const now = Date.now();
    if (now - state.lastReset < 50) return;
    state.lastReset = now;
    clearAll();
    state.isPlaying = false;
    resetUI();
  }

  function resetUI() {
    state.panesById.forEach((entry) => {
      entry.pane.setAttribute('data-state', 'pending');
      entry.log.textContent = '';
      entry.tooluse.textContent = '0 tool uses';
      entry.time.textContent = '0.0s';
      entry.pane.style.removeProperty('--wsim-progress');
    });
    state.frame.querySelectorAll('.wsim__lane').forEach((lane) => {
      const n = lane.getAttribute('data-wave');
      lane.setAttribute('data-active', n === '1' ? 'true' : 'false');
    });
    state.reviewEl.setAttribute('data-state', 'hidden');
    state.reviewEl.querySelector('.wsim__review__body').textContent = '';
    state.playBtn.hidden = false;
    state.resetBtn.hidden = true;
  }

  function runWave(index) {
    const wave = TIMELINE.waves[index];
    if (!wave) {
      scheduleTimeout(runReview, TIMELINE.review.delay);
      return;
    }
    const laneEl = state.frame.querySelector(`.wsim__lane[data-wave="${index + 1}"]`);
    if (laneEl) laneEl.setAttribute('data-active', 'true');

    let finished = 0;
    wave.agents.forEach((agentConfig, i) => {
      scheduleTimeout(() => {
        runAgent(agentConfig, () => {
          finished += 1;
          if (finished === wave.agents.length) runWave(index + 1);
        });
      }, i * 100);
    });
  }

  function runAgent(agentConfig, done) {
    const entry = state.panesById.get(agentConfig.id);
    if (!entry) { done(); return; }
    entry.pane.setAttribute('data-state', 'running');

    if (reducedMotion) {
      agentConfig.logs.forEach((line) => appendLogLine(entry.log, line.msg));
      entry.tooluse.textContent = `${agentConfig.tools} tool uses`;
      entry.time.textContent = `${(agentConfig.duration / 1000).toFixed(1)}s`;
      entry.pane.style.setProperty('--wsim-progress', '1');
      entry.pane.setAttribute('data-state', 'done');
      scheduleTimeout(done, 50);
      return;
    }

    agentConfig.logs.forEach((line) => {
      scheduleTimeout(() => appendLogLine(entry.log, line.msg), line.t);
    });

    animateCounter(entry, agentConfig);

    scheduleTimeout(() => {
      entry.tooluse.textContent = `${agentConfig.tools} tool uses`;
      entry.time.textContent = `${(agentConfig.duration / 1000).toFixed(1)}s`;
      entry.pane.style.setProperty('--wsim-progress', '1');
      entry.pane.setAttribute('data-state', 'done');
      done();
    }, agentConfig.duration);
  }

  function appendLogLine(logEl, msg) {
    const line = el('div', 'wsim__pane__logline', msg);
    logEl.append(line);
    scheduleFrame(() => {
      line.classList.add('is-visible');
      logEl.scrollTop = logEl.scrollHeight;
    });
  }

  function animateCounter(entry, agentConfig) {
    const startTs = performance.now();
    const countDuration = agentConfig.duration * 0.8;

    const step = (ts) => {
      if (!state.isPlaying) return;
      const elapsed = ts - startTs;
      const linear = Math.min(1, elapsed / agentConfig.duration);
      const counterLinear = Math.min(1, elapsed / countDuration);
      const eased = 1 - (1 - counterLinear) * (1 - counterLinear);
      const tools = Math.round(eased * agentConfig.tools);
      const seconds = Math.min(agentConfig.duration / 1000, elapsed / 1000);

      entry.tooluse.textContent = `${tools} tool use${tools === 1 ? '' : 's'}`;
      entry.time.textContent = `${seconds.toFixed(1)}s`;
      entry.pane.style.setProperty('--wsim-progress', linear.toFixed(3));

      if (linear < 1) scheduleFrame(step);
    };
    scheduleFrame(step);
  }

  function runReview() {
    const review = state.reviewEl;
    if (!review) return;
    review.setAttribute('data-state', 'running');
    const body = review.querySelector('.wsim__review__body');
    body.textContent = '';

    if (reducedMotion) {
      TIMELINE.review.lines.forEach((text) => appendLogLine(body, text));
      scheduleTimeout(finishReview, 100);
      return;
    }

    const stagger = TIMELINE.review.duration / TIMELINE.review.lines.length;
    TIMELINE.review.lines.forEach((text, idx) => {
      scheduleTimeout(() => appendLogLine(body, text), idx * stagger);
    });

    scheduleTimeout(finishReview, TIMELINE.review.duration + 300);
  }

  function finishReview() {
    if (state.reviewEl) state.reviewEl.setAttribute('data-state', 'done');
    state.isPlaying = false;
    state.resetBtn.hidden = false;
  }

  // --- init ---------------------------------------------------------------

  function mount() {
    const root = document.getElementById('wave-sim-root');
    if (!root) return;
    if (root.getAttribute('data-wsim-mounted') === 'true') return;

    state.root = root;
    while (root.firstChild) root.removeChild(root.firstChild);
    root.append(buildSimulator());
    root.setAttribute('data-wsim-mounted', 'true');

    if (typeof window.IntersectionObserver === 'function') {
      const io = new window.IntersectionObserver((entries, obs) => {
        entries.forEach((e) => {
          if (e.isIntersecting) {
            root.setAttribute('data-wsim-in-view', 'true');
            obs.disconnect();
          }
        });
      }, { threshold: 0.2 });
      io.observe(root);
    }

    // Autoplay on scroll-in: fire once when the section is ≥40% visible.
    // Disabled under prefers-reduced-motion so users can still opt in manually.
    if (!reducedMotion && typeof window.IntersectionObserver === 'function') {
      const section = document.getElementById('wave-sim') || root;
      let autoplayed = false;
      const autoIo = new window.IntersectionObserver((entries, obs) => {
        entries.forEach((entry) => {
          if (autoplayed) return;
          if (entry.isIntersecting && entry.intersectionRatio >= 0.4) {
            autoplayed = true;
            obs.disconnect();
            if (!state.isPlaying) handlePlay();
          }
        });
      }, { threshold: [0.4] });
      autoIo.observe(section);
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', mount, { once: true });
  } else {
    mount();
  }
})();
