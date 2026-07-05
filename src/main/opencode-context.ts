import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

const START_MARKER = '<!-- pandamux:start';
const END_MARKER = '<!-- pandamux:end -->';

/**
 * Pure: insert/replace the pandamux block within existing content, preserving the rest.
 * Trailing whitespace on the block is normalized away so re-applying is idempotent
 * even when the instructions source ends with a newline (otherwise the trailing
 * newline accumulates on every run).
 */
export function injectPandaMUXBlock(existing: string, pandamuxBlock: string): string {
  const block = pandamuxBlock.trimEnd();
  if (existing.trim() === '') return block;
  const startIdx = existing.indexOf(START_MARKER);
  const endIdx = existing.indexOf(END_MARKER);
  if (startIdx === -1) {
    const separator = existing.endsWith('\n') ? '\n' : '\n\n';
    return existing + separator + block;
  }
  if (endIdx === -1) {
    return existing.substring(0, startIdx) + block;
  }
  const before = existing.substring(0, startIdx);
  const after = existing.substring(endIdx + END_MARKER.length);
  return before + block + after;
}

function getInstructionsPath(): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { app } = require('electron') as typeof import('electron');
    if (app.isPackaged) {
      return path.join(process.resourcesPath, 'claude-instructions', 'claude-instructions.md');
    }
  } catch { /* not running under Electron */ }
  return path.join(__dirname, '../../resources/claude-instructions.md');
}

function getAgentsMdPath(): string {
  return path.join(os.homedir(), '.config', 'opencode', 'AGENTS.md');
}

/** Ensures ~/.config/opencode/AGENTS.md contains the pandamux block. */
export function ensureOpencodeContext(): void {
  try {
    const instructionsPath = getInstructionsPath();
    if (!fs.existsSync(instructionsPath)) {
      console.warn('[pandamux] instructions source not found at', instructionsPath);
      return;
    }
    const pandamuxBlock = fs.readFileSync(instructionsPath, 'utf-8');
    const agentsPath = getAgentsMdPath();
    const dir = path.dirname(agentsPath);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    const existing = fs.existsSync(agentsPath) ? fs.readFileSync(agentsPath, 'utf-8') : '';
    const next = injectPandaMUXBlock(existing, pandamuxBlock);
    if (next !== existing) {
      fs.writeFileSync(agentsPath, next, 'utf-8');
      console.log('[pandamux] Updated pandamux context in ~/.config/opencode/AGENTS.md');
    }
  } catch (err) {
    console.warn('[pandamux] Failed to update OpenCode context:', err);
  }
}

const VERSION_RE = /pandamux-plugin-version:\s*(\S+)/;

/** Pure: compare embedded version markers to decide whether to re-install. */
export function pluginNeedsUpdate(src: string, target: string | null): boolean {
  if (target === null) return true;
  const s = src.match(VERSION_RE)?.[1];
  const t = target.match(VERSION_RE)?.[1];
  if (s === undefined) return true; // fail safe: unversioned source → always reinstall
  return s !== t;
}

function getPluginSrcPath(): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { app } = require('electron') as typeof import('electron');
    if (app.isPackaged) {
      return path.join(process.resourcesPath, 'opencode-plugin', 'pandamux.js');
    }
  } catch { /* not running under Electron */ }
  return path.join(__dirname, '../../resources/opencode-plugin/pandamux.js');
}

/** Installs/updates the pandamux OpenCode plugin into ~/.config/opencode/plugin/. */
export function ensureOpencodePlugin(): void {
  try {
    const srcPath = getPluginSrcPath();
    if (!fs.existsSync(srcPath)) {
      console.warn('[pandamux] opencode plugin source not found at', srcPath);
      return;
    }
    const src = fs.readFileSync(srcPath, 'utf-8');
    const destDir = path.join(os.homedir(), '.config', 'opencode', 'plugin');
    const dest = path.join(destDir, 'pandamux.js');
    const target = fs.existsSync(dest) ? fs.readFileSync(dest, 'utf-8') : null;
    if (!pluginNeedsUpdate(src, target)) return;
    if (!fs.existsSync(destDir)) fs.mkdirSync(destDir, { recursive: true });
    fs.writeFileSync(dest, src, 'utf-8');
    console.log('[pandamux] Installed pandamux OpenCode plugin to', dest);
  } catch (err) {
    console.warn('[pandamux] Failed to install OpenCode plugin:', err);
  }
}
