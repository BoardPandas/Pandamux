#!/usr/bin/env node
// Generates a set of small, pleasant CC0 notification chimes (synthesized, so
// there are no licensing/attribution strings attached) into
// src/renderer/assets/sounds/. Run with: node scripts/gen-notification-sounds.js
//
// These are additional options for issue #39 (more notification sounds). They
// are deliberately short (<0.5s), soft, and harmonically simple.
const fs = require('fs');
const path = require('path');

const SR = 44100;
const OUT_DIR = path.join(__dirname, '..', 'src', 'renderer', 'assets', 'sounds');

// Render a mono 16-bit WAV from a sample function f(t) → [-1, 1].
function renderWav(durationSec, sampleFn) {
  const n = Math.floor(durationSec * SR);
  const data = Buffer.alloc(44 + n * 2);
  // RIFF header
  data.write('RIFF', 0);
  data.writeUInt32LE(36 + n * 2, 4);
  data.write('WAVE', 8);
  data.write('fmt ', 12);
  data.writeUInt32LE(16, 16);       // fmt chunk size
  data.writeUInt16LE(1, 20);        // PCM
  data.writeUInt16LE(1, 22);        // mono
  data.writeUInt32LE(SR, 24);
  data.writeUInt32LE(SR * 2, 28);   // byte rate
  data.writeUInt16LE(2, 32);        // block align
  data.writeUInt16LE(16, 34);       // bits per sample
  data.write('data', 36);
  data.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) {
    const t = i / SR;
    let s = sampleFn(t);
    if (s > 1) s = 1; else if (s < -1) s = -1;
    data.writeInt16LE(Math.round(s * 32767), 44 + i * 2);
  }
  return data;
}

// 3ms raised-cosine attack to avoid a click, then an exponential decay.
function env(t, attack, tau) {
  const a = t < attack ? 0.5 - 0.5 * Math.cos((Math.PI * t) / attack) : 1;
  return a * Math.exp(-t / tau);
}
const sine = (t, f) => Math.sin(2 * Math.PI * f * t);

// A struck-bell partial stack with inharmonic overtones.
function bell(t, f, tau) {
  return (
    1.0 * sine(t, f) * Math.exp(-t / tau) +
    0.5 * sine(t, f * 2.01) * Math.exp(-t / (tau * 0.7)) +
    0.25 * sine(t, f * 2.99) * Math.exp(-t / (tau * 0.5)) +
    0.12 * sine(t, f * 4.1) * Math.exp(-t / (tau * 0.35))
  );
}

const sounds = {
  // Soft two-note rising chime (C6 → G6).
  chime: () => renderWav(0.5, (t) => {
    const a = env(t, 0.004, 0.18) * bell(t, 1046.5, 0.18);
    const t2 = t - 0.12;
    const b = t2 > 0 ? env(t2, 0.004, 0.22) * bell(t2, 1568.0, 0.22) : 0;
    return 0.42 * (a + b);
  }),
  // Single clean high ping (E6) with a touch of octave shimmer.
  ping: () => renderWav(0.38, (t) => {
    const e = env(t, 0.003, 0.11);
    return 0.5 * e * (sine(t, 1318.5) + 0.3 * sine(t, 2637.0));
  }),
  // Warm wooden marimba-ish tap (D5) — fast decay, strong octave.
  marimba: () => renderWav(0.42, (t) => {
    const e = env(t, 0.003, 0.13);
    return 0.5 * e * (sine(t, 587.33) + 0.5 * sine(t, 1174.7) + 0.15 * sine(t, 1761.0));
  }),
  // Short soft "pop" — a quick downward blip.
  pop: () => renderWav(0.16, (t) => {
    const e = env(t, 0.002, 0.05);
    const f = 760 - 360 * Math.min(1, t / 0.08); // 760 → 400 Hz glide
    return 0.5 * e * sine(t, f);
  }),
};

fs.mkdirSync(OUT_DIR, { recursive: true });
for (const [name, gen] of Object.entries(sounds)) {
  const buf = gen();
  const file = path.join(OUT_DIR, `${name}.wav`);
  fs.writeFileSync(file, buf);
  console.log(`wrote ${file} (${buf.length} bytes)`);
}
