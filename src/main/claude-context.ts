import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

const START_MARKER = '<!-- pandamux:start';
const END_MARKER = '<!-- pandamux:end -->';

function getInstructionsPath(): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { app } = require('electron') as typeof import('electron');
    if (app.isPackaged) {
      return path.join(process.resourcesPath, 'claude-instructions', 'claude-instructions.md');
    }
  } catch {
    // Not in Electron
  }
  return path.join(__dirname, '../../resources/claude-instructions.md');
}

function getClaudeMdPath(): string {
  return path.join(os.homedir(), '.claude', 'CLAUDE.md');
}

/**
 * Ensures the user's global ~/.claude/CLAUDE.md contains the pandamux section.
 * - Creates ~/.claude/ and CLAUDE.md if they don't exist
 * - Inserts the pandamux block if not present
 * - Updates the pandamux block if it's outdated
 * - Never touches content outside the <!-- pandamux:start --> / <!-- pandamux:end --> markers
 */
export function ensureClaudeContext(): void {
  try {
    const instructionsPath = getInstructionsPath();
    if (!fs.existsSync(instructionsPath)) {
      console.warn('[pandamux] claude-instructions.md not found at', instructionsPath);
      return;
    }

    const pandamuxBlock = fs.readFileSync(instructionsPath, 'utf-8');
    const claudeMdPath = getClaudeMdPath();
    const claudeDir = path.dirname(claudeMdPath);

    // Ensure ~/.claude/ exists
    if (!fs.existsSync(claudeDir)) {
      fs.mkdirSync(claudeDir, { recursive: true });
    }

    if (!fs.existsSync(claudeMdPath)) {
      // No CLAUDE.md yet — create with just the pandamux block
      fs.writeFileSync(claudeMdPath, pandamuxBlock, 'utf-8');
      console.log('[pandamux] Created ~/.claude/CLAUDE.md with pandamux context');
      return;
    }

    // CLAUDE.md exists — check for existing pandamux block
    const existing = fs.readFileSync(claudeMdPath, 'utf-8');
    const startIdx = existing.indexOf(START_MARKER);
    const endIdx = existing.indexOf(END_MARKER);

    if (startIdx === -1) {
      // No pandamux block — append it
      const separator = existing.endsWith('\n') ? '\n' : '\n\n';
      fs.writeFileSync(claudeMdPath, existing + separator + pandamuxBlock, 'utf-8');
      console.log('[pandamux] Appended pandamux context to ~/.claude/CLAUDE.md');
      return;
    }

    if (endIdx === -1) {
      // Broken markers — replace from start marker to end of file
      const before = existing.substring(0, startIdx);
      fs.writeFileSync(claudeMdPath, before + pandamuxBlock, 'utf-8');
      console.log('[pandamux] Fixed and updated pandamux context in ~/.claude/CLAUDE.md');
      return;
    }

    // Both markers found — replace the block
    const currentBlock = existing.substring(startIdx, endIdx + END_MARKER.length);
    if (currentBlock.trim() === pandamuxBlock.trim()) {
      // Already up to date
      return;
    }

    const before = existing.substring(0, startIdx);
    const after = existing.substring(endIdx + END_MARKER.length);
    fs.writeFileSync(claudeMdPath, before + pandamuxBlock + after, 'utf-8');
    console.log('[pandamux] Updated pandamux context in ~/.claude/CLAUDE.md');
  } catch (err) {
    console.warn('[pandamux] Failed to update Claude context:', err);
  }
}

const HOOK_MARKER = 'pandamux-hook';

function getSettingsPath(): string {
  return path.join(os.homedir(), '.claude', 'settings.json');
}

function getCliAbsolutePath(): string {
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { app } = require('electron') as typeof import('electron');
    if (app.isPackaged) {
      return path.join(process.resourcesPath, 'cli', 'pandamux.js');
    }
  } catch {}
  return path.resolve(path.join(__dirname, '../cli/pandamux.js'));
}

/** Tools tracked via PostToolUse hooks for the sidebar/diff view. */
const TRACKED_TOOLS = ['Bash', 'Read', 'Write', 'Edit', 'Grep', 'Glob', 'Agent', 'WebSearch', 'WebFetch', 'Skill'];

/**
 * Pure builder for the pandamux hook blocks. Given the parsed settings object and
 * the absolute path to pandamux-hook.js, returns a new settings object whose
 * `hooks` contains fresh pandamux PostToolUse/Notification/Stop entries, with any
 * prior pandamux entries replaced and all non-pandamux (user) hooks preserved.
 * Extracted so the merge logic is unit-testable without touching the fs
 * (issue #53).
 */
export function applyPandaMUXHooks(settings: any, hookScript: string): any {
  const next = { ...(settings || {}) };
  next.hooks = { ...(next.hooks || {}) };

  // PostToolUse passes the tool name as a positional arg; Notification/Stop
  // pass an --event flag so the helper reports an event type instead.
  const makeToolCmd = (tool: string) => `node "${hookScript}" ${tool} 2>/dev/null || true`;
  const makeEventCmd = (event: string) => `node "${hookScript}" --event ${event} 2>/dev/null || true`;

  // Drop any prior pandamux entry from a hook array, preserving user hooks.
  const stripPandaMUX = (entries: any): any[] =>
    (Array.isArray(entries) ? entries : []).filter((e: any) => {
      if (!Array.isArray(e.hooks)) return true;
      return !e.hooks.some((h: any) => h.command?.includes('pandamux-hook'));
    });

  // PostToolUse — one entry per tracked tool for specific sidebar tracking.
  next.hooks.PostToolUse = [
    ...stripPandaMUX(next.hooks.PostToolUse),
    ...TRACKED_TOOLS.map(tool => ({
      matcher: tool,
      hooks: [{ type: 'command', command: makeToolCmd(tool) }],
    })),
  ];

  // Notification — Claude Code is asking for input/permission (waiting on you).
  next.hooks.Notification = [
    ...stripPandaMUX(next.hooks.Notification),
    { hooks: [{ type: 'command', command: makeEventCmd('Notification') }] },
  ];

  // Stop — Claude Code finished its turn and is back at the prompt.
  next.hooks.Stop = [
    ...stripPandaMUX(next.hooks.Stop),
    { hooks: [{ type: 'command', command: makeEventCmd('Stop') }] },
  ];

  return next;
}

/**
 * Ensures Claude Code's ~/.claude/settings.json has the pandamux hooks:
 *  - PostToolUse  → drives the sidebar/diff view (tool activity)
 *  - Notification → fires a pandamux notification when the agent needs input/permission
 *  - Stop         → fires a pandamux notification when the agent finishes its turn
 * Uses absolute CLI paths (not env var). Never touches non-pandamux hook entries
 * (issue #53): existing user hooks in each array are preserved.
 */
export function ensureClaudeHooks(): void {
  try {
    const settingsPath = getSettingsPath();
    if (!fs.existsSync(settingsPath)) return;

    const raw = fs.readFileSync(settingsPath, 'utf-8');
    let settings: any;
    try { settings = JSON.parse(raw); } catch { return; }

    // Use absolute path to the hook helper script OUTSIDE the ASAR.
    // __dirname is inside app.asar when packaged — Node.js outside Electron
    // can't read ASAR files, so we use the standalone copy in resources/cli/.
    let hookScript: string;
    try {
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const { app } = require('electron') as typeof import('electron');
      if (app.isPackaged) {
        hookScript = path.join(process.resourcesPath, 'cli', 'pandamux-hook.js');
      } else {
        hookScript = path.resolve(path.join(__dirname, '../../resources/cli/pandamux-hook.js'));
      }
    } catch {
      hookScript = path.resolve(path.join(__dirname, '../../resources/cli/pandamux-hook.js'));
    }
    hookScript = hookScript.split(path.sep).join('/');

    const updated = applyPandaMUXHooks(settings, hookScript);
    fs.writeFileSync(settingsPath, JSON.stringify(updated, null, 2), 'utf-8');
    console.log('[pandamux] Configured PostToolUse/Notification/Stop hooks in ~/.claude/settings.json');
  } catch (err) {
    console.warn('[pandamux] Failed to update Claude hooks:', err);
  }
}

/**
 * Configures chrome-devtools-mcp to connect to pandamux's CDP proxy on localhost:9222.
 * Disables the plugin version and adds a custom MCP server in settings.json with
 * --browserUrl pointing to pandamux. This is more reliable than modifying the plugin cache.
 */
export function ensureChromeDevtoolsConfig(): void {
  try {
    const settingsPath = getSettingsPath();
    if (!fs.existsSync(settingsPath)) return;

    const raw = fs.readFileSync(settingsPath, 'utf-8');
    let settings: any;
    try { settings = JSON.parse(raw); } catch { return; }

    let changed = false;

    // Disable the plugin (it launches its own Chrome)
    if (settings.enabledPlugins?.['chrome-devtools-mcp@claude-plugins-official'] !== false) {
      if (!settings.enabledPlugins) settings.enabledPlugins = {};
      settings.enabledPlugins['chrome-devtools-mcp@claude-plugins-official'] = false;
      changed = true;
    }

    // Add as custom MCP server with --browserUrl
    if (!settings.mcpServers) settings.mcpServers = {};
    const existing = settings.mcpServers['chrome-devtools'];
    if (!existing || !JSON.stringify(existing).includes('9222')) {
      settings.mcpServers['chrome-devtools'] = {
        command: 'npx',
        args: ['-y', 'chrome-devtools-mcp@latest', '--browserUrl=http://127.0.0.1:9222'],
      };
      changed = true;
    }

    if (changed) {
      fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf-8');
      console.log('[pandamux] Configured chrome-devtools-mcp as custom MCP server → localhost:9222');
    }
  } catch (err) {
    console.warn('[pandamux] Failed to configure chrome-devtools-mcp:', err);
  }
}

/**
 * Recursively copies a directory tree from src to dest.
 * Creates dest and any intermediate directories as needed.
 */
function copyDirSync(src: string, dest: string): void {
  fs.mkdirSync(dest, { recursive: true });
  const entries = fs.readdirSync(src, { withFileTypes: true });
  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);
    if (entry.isDirectory()) {
      copyDirSync(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

/**
 * Auto-installs the pandamux-orchestrator plugin into Claude Code's plugin cache.
 * - Copies resources/pandamux-orchestrator/ → ~/.claude/plugins/cache/pandamux-orchestrator/{version}/
 * - Registers in ~/.claude/plugins/installed_plugins.json
 * - Enables in ~/.claude/settings.json
 * Skips if already installed at the same version.
 */
export function ensureOrchestratorPlugin(): void {
  try {
    // 1. Locate plugin source directory
    let pluginSrcDir: string;
    try {
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const { app } = require('electron') as typeof import('electron');
      if (app.isPackaged) {
        pluginSrcDir = path.join(process.resourcesPath, 'pandamux-orchestrator');
      } else {
        pluginSrcDir = path.resolve(path.join(__dirname, '../../resources/pandamux-orchestrator'));
      }
    } catch {
      pluginSrcDir = path.resolve(path.join(__dirname, '../../resources/pandamux-orchestrator'));
    }

    const pluginJsonSrc = path.join(pluginSrcDir, '.claude-plugin', 'plugin.json');
    if (!fs.existsSync(pluginJsonSrc)) {
      console.warn('[pandamux] pandamux-orchestrator plugin not found at', pluginSrcDir);
      return;
    }

    // 2. Read version from plugin.json
    let pluginMeta: any;
    try {
      pluginMeta = JSON.parse(fs.readFileSync(pluginJsonSrc, 'utf-8'));
    } catch {
      console.warn('[pandamux] Failed to parse pandamux-orchestrator plugin.json');
      return;
    }
    const version: string = pluginMeta.version || '0.0.0';

    // 3. Copy to ~/.claude/plugins/cache/pandamux-orchestrator/{version}/
    const claudeDir = path.join(os.homedir(), '.claude');
    const cacheDir = path.join(claudeDir, 'plugins', 'cache', 'pandamux-orchestrator', version);
    const targetPluginJson = path.join(cacheDir, '.claude-plugin', 'plugin.json');

    // Check if already installed at same version
    if (fs.existsSync(targetPluginJson)) {
      try {
        const existing = JSON.parse(fs.readFileSync(targetPluginJson, 'utf-8'));
        if (existing.version === version) {
          // Already installed at same version — skip copy, but still ensure registration
          ensurePluginRegistered(cacheDir, version, claudeDir);
          return;
        }
      } catch {
        // Corrupted target — re-install
      }
    }

    // Remove old version directory if it exists (clean install)
    if (fs.existsSync(cacheDir)) {
      fs.rmSync(cacheDir, { recursive: true, force: true });
    }

    // Copy entire plugin directory
    copyDirSync(pluginSrcDir, cacheDir);
    console.log(`[pandamux] Installed pandamux-orchestrator v${version} to plugin cache`);

    // 4–5. Register and enable
    ensurePluginRegistered(cacheDir, version, claudeDir);
  } catch (err) {
    console.warn('[pandamux] Failed to install pandamux-orchestrator plugin:', err);
  }
}

/**
 * Registers the orchestrator plugin in installed_plugins.json and enables it in settings.json.
 */
function ensurePluginRegistered(installPath: string, version: string, claudeDir: string): void {
  const pluginKey = 'pandamux-orchestrator@pandamux';

  // Register in installed_plugins.json
  try {
    const installedPath = path.join(claudeDir, 'plugins', 'installed_plugins.json');
    let installed: any = {};
    if (fs.existsSync(installedPath)) {
      try { installed = JSON.parse(fs.readFileSync(installedPath, 'utf-8')); } catch { installed = {}; }
    } else {
      fs.mkdirSync(path.dirname(installedPath), { recursive: true });
    }

    const now = new Date().toISOString();
    const existing = installed[pluginKey];
    if (!existing || existing.version !== version || existing.installPath !== installPath) {
      installed[pluginKey] = {
        scope: 'user',
        installPath,
        version,
        installedAt: existing?.installedAt || now,
        lastUpdated: now,
      };
      fs.writeFileSync(installedPath, JSON.stringify(installed, null, 2), 'utf-8');
      console.log('[pandamux] Registered pandamux-orchestrator in installed_plugins.json');
    }
  } catch (err) {
    console.warn('[pandamux] Failed to register plugin in installed_plugins.json:', err);
  }

  // Enable in settings.json
  try {
    const settingsPath = path.join(claudeDir, 'settings.json');
    if (!fs.existsSync(settingsPath)) return;

    const raw = fs.readFileSync(settingsPath, 'utf-8');
    let settings: any;
    try { settings = JSON.parse(raw); } catch { return; }

    if (!settings.enabledPlugins) settings.enabledPlugins = {};
    if (settings.enabledPlugins[pluginKey] !== true) {
      settings.enabledPlugins[pluginKey] = true;
      fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf-8');
      console.log('[pandamux] Enabled pandamux-orchestrator in settings.json');
    }
  } catch (err) {
    console.warn('[pandamux] Failed to enable plugin in settings.json:', err);
  }
}
