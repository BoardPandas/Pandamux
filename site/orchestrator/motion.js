/*
 * motion.js — scroll motion engine for pandamux-orchestrator marketing site
 * Agent D · Wave 2 · orch-781419
 *
 * Drives CSS custom properties on scroll (--scroll on <html>, --progress on
 * .how__diagram), toggles [data-active] on steps/waves as they come into view,
 * fades in elements via IntersectionObserver, wires up smooth-scroll anchors,
 * highlights the nav link for the section in view, hints the wave-sim play
 * button until first play, and wires the /plugin install copy button.
 *
 * No imports. No build step. Plain browser JS. Everything null-checked.
 */
(function () {
  'use strict';

  const root = document.documentElement;
  const prefersReduced =
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ------------------------------------------------------------------ *
   * Shared rAF scheduler: scroll + resize push all layout-reading work  *
   * into a single frame so we stay 60fps no matter how many listeners.  *
   * ------------------------------------------------------------------ */

  let rafPending = false;

  function scheduleFrame() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      updateScrollProgress();
      updateHowProgress();
      updateNavActive();
    });
  }

  /* ------------------------------------------------------------------ *
   * 1. Global scroll progress bar                                       *
   * ------------------------------------------------------------------ */

  function updateScrollProgress() {
    const scrollable = root.scrollHeight - window.innerHeight;
    if (scrollable <= 0) {
      root.style.setProperty('--scroll', '0');
      return;
    }
    const p = Math.max(0, Math.min(1, window.scrollY / scrollable));
    root.style.setProperty('--scroll', p.toFixed(4));
  }

  function setupScroll() {
    window.addEventListener('scroll', scheduleFrame, { passive: true });
    window.addEventListener('resize', scheduleFrame, { passive: true });
    window.addEventListener('load', scheduleFrame, { passive: true });
  }

  /* ------------------------------------------------------------------ *
   * 2. Reveal-on-view                                                   *
   * ------------------------------------------------------------------ */

  const REVEAL_SEEDS = [
    '.hero__eyebrow',
    '.hero__lede',
    '.hero__meta',
    '.section__eyebrow',
    '.section__title',
    '.section__lede',
    '.section__fine',
    '.how__step',
    '.how__wave',
    '.zones__file',
    '.cmp__col',
    '.cmp__row',
    '.qs__term',
    '.qs__line',
    '.arch__layer',
    '.arch__footnote',
    '.links__item'
  ];

  function markReveals() {
    REVEAL_SEEDS.forEach((selector) => {
      const nodes = document.querySelectorAll(selector);
      nodes.forEach((el, idx) => {
        el.classList.add('reveal');
        if (!el.dataset.delay && nodes.length > 1) {
          el.dataset.delay = String((idx % 4) + 1);
        }
      });
    });
  }

  function setupRevealObserver() {
    markReveals();
    const targets = document.querySelectorAll('.reveal');
    if (!targets.length) return;

    if (!('IntersectionObserver' in window)) {
      targets.forEach((el) => el.classList.add('is-visible'));
      return;
    }

    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          entry.target.classList.add('is-visible');
          io.unobserve(entry.target);
        });
      },
      { threshold: 0.15, rootMargin: '0px 0px -10% 0px' }
    );

    targets.forEach((el) => io.observe(el));
  }

  /* ------------------------------------------------------------------ *
   * 3. Hero title staggered reveal                                      *
   * ------------------------------------------------------------------ */

  function setupHeroReveal() {
    const lines = document.querySelectorAll('.hero__title__line');
    if (!lines.length) return;

    lines.forEach((el) => el.classList.add('reveal'));

    if (prefersReduced) {
      lines.forEach((el) => el.classList.add('is-visible'));
      return;
    }

    const hero = document.getElementById('hero');
    const run = () => {
      lines.forEach((el, i) => {
        if (el.classList.contains('is-visible')) return;
        setTimeout(() => el.classList.add('is-visible'), i * 120);
      });
    };

    if (!hero || !('IntersectionObserver' in window)) {
      run();
      return;
    }

    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          run();
          io.disconnect();
        });
      },
      { threshold: 0.1 }
    );
    io.observe(hero);
  }

  /* ------------------------------------------------------------------ *
   * 4. How-it-works scroll-driven walkthrough                           *
   * ------------------------------------------------------------------ */

  let howSection = null;
  let howDiagram = null;
  let howSteps = [];
  let howWaves = [];
  let lastActiveStep = -1;

  function setupHowItWorks() {
    howSection = document.getElementById('how-it-works');
    howDiagram = document.querySelector('.how__diagram');
    howSteps = Array.from(document.querySelectorAll('.how__step'));
    howWaves = Array.from(document.querySelectorAll('.how__wave'));
  }

  function updateHowProgress() {
    if (!howSection || !howDiagram) return;

    const rect = howSection.getBoundingClientRect();
    const vh = window.innerHeight;
    const total = rect.height + vh;
    // 0 when top of section just hits bottom of viewport
    // 1 when bottom of section has scrolled out above viewport
    const progress = Math.max(0, Math.min(1, (vh - rect.top) / total));
    howDiagram.style.setProperty('--progress', progress.toFixed(4));

    if (!howSteps.length) return;

    const pivot = vh * 0.5;
    let activeIdx = -1;
    let bestDist = Infinity;

    for (let i = 0; i < howSteps.length; i += 1) {
      const step = howSteps[i];
      const r = step.getBoundingClientRect();
      if (r.bottom < 0 || r.top > vh) continue;
      const center = r.top + r.height / 2;
      const dist = Math.abs(center - pivot);
      if (dist < bestDist) {
        bestDist = dist;
        activeIdx = i;
      }
    }

    if (activeIdx === lastActiveStep) return;
    lastActiveStep = activeIdx;

    if (activeIdx < 0) {
      howSteps.forEach((s) => s.removeAttribute('data-active'));
      howWaves.forEach((w) => w.removeAttribute('data-active'));
      return;
    }

    const activeStep = howSteps[activeIdx];
    const stepNum = activeStep.getAttribute('data-step');

    howSteps.forEach((s, i) => {
      if (i === activeIdx) s.setAttribute('data-active', 'true');
      else s.removeAttribute('data-active');
    });

    howWaves.forEach((w) => {
      if (w.getAttribute('data-wave') === stepNum) {
        w.setAttribute('data-active', 'true');
      } else {
        w.removeAttribute('data-active');
      }
    });
  }

  /* ------------------------------------------------------------------ *
   * 5. Smooth-scroll for in-page anchors                                *
   * ------------------------------------------------------------------ */

  function setupSmoothScroll() {
    document.addEventListener('click', (event) => {
      if (event.defaultPrevented) return;
      if (event.button !== 0) return;
      if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;

      const anchor = event.target && event.target.closest && event.target.closest('a[href^="#"]');
      if (!anchor) return;

      const href = anchor.getAttribute('href');
      if (!href || href === '#' || href.length < 2) return;

      let target = null;
      try {
        target = document.querySelector(href);
      } catch (err) {
        return;
      }
      if (!target) return;

      event.preventDefault();
      if (prefersReduced) {
        target.scrollIntoView({ block: 'start' });
      } else {
        target.scrollIntoView({ behavior: 'smooth', block: 'start' });
      }
    });
  }

  /* ------------------------------------------------------------------ *
   * 6. Active nav highlight                                             *
   * ------------------------------------------------------------------ */

  let navEntries = [];

  function setupNavHighlight() {
    const links = document.querySelectorAll('.site-header__link');
    navEntries = [];
    links.forEach((link) => {
      const href = link.getAttribute('href');
      if (!href || !href.startsWith('#')) return;
      let section = null;
      try {
        section = document.querySelector(href);
      } catch (err) {
        return;
      }
      if (!section) return;
      navEntries.push({ link, section });
    });
  }

  function updateNavActive() {
    if (!navEntries.length) return;
    const vh = window.innerHeight;
    const pivot = vh * 0.35;
    let best = null;
    let bestDist = Infinity;

    navEntries.forEach((entry) => {
      const rect = entry.section.getBoundingClientRect();
      if (rect.bottom < 0 || rect.top > vh) return;
      const dist = Math.abs(rect.top - pivot);
      if (dist < bestDist) {
        bestDist = dist;
        best = entry;
      }
    });

    navEntries.forEach((entry) => {
      if (entry === best) entry.link.classList.add('is-active');
      else entry.link.classList.remove('is-active');
    });
  }

  /* ------------------------------------------------------------------ *
   * 7. Wave-sim play hint                                               *
   * ------------------------------------------------------------------ */

  let hintTimer = null;
  let hasPlayed = false;

  function setupWaveSimHint() {
    if (prefersReduced) return;

    document.addEventListener(
      'click',
      (event) => {
        const btn = event.target && event.target.closest && event.target.closest('.wsim__play');
        if (!btn) return;
        stopHint();
      },
      { passive: true }
    );

    let tries = 0;
    const waitForButton = () => {
      if (hasPlayed) return;
      const btn = document.querySelector('.wsim__play');
      if (btn) {
        startHint(btn);
        return;
      }
      tries += 1;
      if (tries < 40) setTimeout(waitForButton, 150);
    };
    waitForButton();
  }

  function startHint(btn) {
    if (hintTimer || hasPlayed) return;
    const pulse = () => {
      if (hasPlayed) return stopHint();
      btn.classList.add('is-hint');
      setTimeout(() => {
        btn.classList.remove('is-hint');
      }, 700);
      hintTimer = setTimeout(pulse, 3000);
    };
    pulse();
  }

  function stopHint() {
    hasPlayed = true;
    if (hintTimer) {
      clearTimeout(hintTimer);
      hintTimer = null;
    }
    document.querySelectorAll('.wsim__play').forEach((b) => b.classList.remove('is-hint'));
  }

  /* ------------------------------------------------------------------ *
   * 8. Copy install button                                              *
   * ------------------------------------------------------------------ */

  function setupCopyButton() {
    const btn = document.querySelector('.qs__copy');
    if (!btn) return;

    btn.addEventListener('click', async () => {
      const text = btn.getAttribute('data-copy') || '';
      if (!text) return;

      const hasClipboard =
        typeof navigator !== 'undefined' &&
        navigator.clipboard &&
        typeof navigator.clipboard.writeText === 'function';
      if (!hasClipboard) return;

      const original = btn.textContent;
      try {
        await navigator.clipboard.writeText(text);
      } catch (err) {
        return;
      }
      btn.textContent = 'copied';
      btn.setAttribute('data-copied', 'true');
      setTimeout(() => {
        btn.textContent = original;
        btn.removeAttribute('data-copied');
      }, 1500);
    });
  }

  /* ------------------------------------------------------------------ *
   * 9. Keyboard easter egg — type "wave" to fire the simulator          *
   * ------------------------------------------------------------------ */

  const EASTER_WORD = 'wave';
  const EASTER_WINDOW_MS = 1500;
  let easterBuffer = '';
  let easterLastTs = 0;
  let easterToastEl = null;
  let easterToastFadeTimer = null;
  let easterToastRemoveTimer = null;

  function isEditableTarget(target) {
    if (!target || target.nodeType !== 1) return false;
    const tag = target.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
    if (target.isContentEditable) return true;
    return false;
  }

  function setupEasterEgg() {
    document.addEventListener('keydown', (event) => {
      if (event.ctrlKey || event.metaKey || event.altKey) {
        easterBuffer = '';
        return;
      }
      if (isEditableTarget(event.target)) {
        easterBuffer = '';
        return;
      }
      const key = event.key;
      if (!key || key.length !== 1) {
        easterBuffer = '';
        return;
      }
      const now = Date.now();
      if (now - easterLastTs > EASTER_WINDOW_MS) easterBuffer = '';
      easterLastTs = now;
      easterBuffer = (easterBuffer + key.toLowerCase()).slice(-EASTER_WORD.length);
      if (easterBuffer === EASTER_WORD) {
        easterBuffer = '';
        triggerEasterEgg();
      }
    });
  }

  function triggerEasterEgg() {
    const section = document.getElementById('wave-sim');
    if (section) {
      if (prefersReduced) {
        section.scrollIntoView({ block: 'center' });
      } else {
        section.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }
    }
    const playBtn = document.querySelector('.wsim__play');
    if (playBtn && !prefersReduced) showEasterToast(playBtn);
    const delay = prefersReduced ? 0 : 420;
    setTimeout(() => {
      const btn = document.querySelector('.wsim__play');
      if (btn && !btn.hidden) btn.click();
    }, delay);
  }

  function showEasterToast(anchor) {
    if (easterToastFadeTimer) {
      clearTimeout(easterToastFadeTimer);
      easterToastFadeTimer = null;
    }
    if (easterToastRemoveTimer) {
      clearTimeout(easterToastRemoveTimer);
      easterToastRemoveTimer = null;
    }
    if (!easterToastEl) {
      const toast = document.createElement('span');
      toast.setAttribute('aria-hidden', 'true');
      toast.textContent = '\u2318 wave';
      toast.style.cssText = [
        'position:fixed',
        'z-index:9999',
        'padding:6px 10px',
        'font:11px/1 ui-monospace,SFMono-Regular,Menlo,monospace',
        'letter-spacing:0.08em',
        'text-transform:uppercase',
        'color:#e9f2ff',
        'background:rgba(10,14,22,0.92)',
        'border:1px solid rgba(122,162,255,0.45)',
        'border-radius:6px',
        'box-shadow:0 4px 16px rgba(0,0,0,0.45),0 0 0 1px rgba(122,162,255,0.15)',
        'pointer-events:none',
        'opacity:0',
        'transform:translateY(4px)',
        'transition:opacity 180ms ease,transform 180ms ease',
      ].join(';');
      document.body.appendChild(toast);
      easterToastEl = toast;
    }
    const rect = anchor.getBoundingClientRect();
    const top = Math.max(8, rect.top - 34);
    const left = Math.max(8, Math.min(window.innerWidth - 120, rect.left));
    easterToastEl.style.top = `${top}px`;
    easterToastEl.style.left = `${left}px`;
    // force reflow so the opening transition plays even on repeat triggers
    void easterToastEl.offsetWidth;
    easterToastEl.style.opacity = '1';
    easterToastEl.style.transform = 'translateY(0)';

    easterToastFadeTimer = setTimeout(() => {
      if (!easterToastEl) return;
      easterToastEl.style.opacity = '0';
      easterToastEl.style.transform = 'translateY(4px)';
      easterToastRemoveTimer = setTimeout(() => {
        if (easterToastEl && easterToastEl.parentNode) {
          easterToastEl.parentNode.removeChild(easterToastEl);
        }
        easterToastEl = null;
        easterToastRemoveTimer = null;
      }, 220);
      easterToastFadeTimer = null;
    }, 1800);
  }

  /* ------------------------------------------------------------------ *
   * Bootstrap                                                            *
   * ------------------------------------------------------------------ */

  function init() {
    setupHowItWorks();
    setupNavHighlight();
    setupScroll();
    setupRevealObserver();
    setupHeroReveal();
    setupSmoothScroll();
    setupWaveSimHint();
    setupCopyButton();
    setupEasterEgg();
    scheduleFrame();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
