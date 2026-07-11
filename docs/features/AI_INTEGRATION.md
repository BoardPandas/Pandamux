<!-- PAGE_ID: pandamux_11_ai-integration -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [claude-context.ts:8-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L8-L19)
- [claude-context.ts:32-91](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L32-L91)
- [claude-context.ts:110-159](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L110-L159)
- [claude-context.ts:169-201](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L169-L201)
- [claude-context.ts:208-244](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L208-L244)
- [claude-context.ts:271-336](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L271-L336)
- [claude-context.ts:341-389](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L341-L389)
- [claude-observer.ts:33-53](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L33-L53)
- [claude-observer.ts:68-166](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L68-L166)
- [claude-observer.ts:188-210](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L188-L210)
- [opencode-context.ts:14-29](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L14-L29)
- [opencode-context.ts:47-67](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L47-L67)
- [opencode-context.ts:92-110](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L92-L110)
- [index.ts:276-286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L276-L286)
- [ipc-handlers.ts:53-60](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L53-L60)

</details>

# AI Agent Integration

> **Related Pages**: [Agent Orchestration](AGENT_ORCHESTRATION.md), [Shell Integration and Status](SHELL_INTEGRATION.md)

---

<!-- BEGIN:AUTOGEN pandamux_11_ai-integration_overview -->
## Overview

PandaMUX is built for AI coding agents running in its terminal panes, and it configures Claude Code and OpenCode automatically the first time it starts rather than requiring manual setup. All of this work happens once, at app startup, in `app.whenReady()`: the main process injects a pandamux context block into each agent's global instructions file, wires Claude Code's hook system to the pandamux CLI, installs the bundled `pandamux-orchestrator` plugin, and installs an OpenCode plugin, in that order (in [index.ts:276-286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L276-L286)).

Two modules do the heavy lifting: `src/main/claude-context.ts` handles everything Claude-Code-specific (CLAUDE.md injection, hooks, chrome-devtools-mcp rewiring, the orchestrator plugin), and `src/main/opencode-context.ts` mirrors the context-injection and plugin-install pattern for OpenCode's `AGENTS.md` and plugin directory. A third module, `src/main/claude-observer.ts`, is not a setup routine at all: it parses live PTY output from Claude Code sessions to drive the sidebar's per-surface activity display, and it accepts externally-pushed activity so the OpenCode plugin can feed the same UI.

Sources: [claude-context.ts:1-20](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L1-L20), [opencode-context.ts:1-10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L1-L10), [index.ts:276-286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L276-L286)
<!-- END:AUTOGEN pandamux_11_ai-integration_overview -->

---

<!-- BEGIN:AUTOGEN pandamux_11_ai-integration_context -->
## Claude Context Injection

`ensureClaudeContext()` keeps the user's global `~/.claude/CLAUDE.md` in sync with a bundled instructions block, without disturbing anything else the user has written in that file.

The block source is read from `resources/claude-instructions.md` in dev, or `process.resourcesPath/claude-instructions/claude-instructions.md` when packaged ([claude-context.ts:8-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L8-L19)). The function then walks four cases against the existing `CLAUDE.md` content, delimited by `<!-- pandamux:start -->` / `<!-- pandamux:end -->` markers ([claude-context.ts:5-6](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L5-L6)):

| Existing state | Action |
|---|---|
| `~/.claude/CLAUDE.md` does not exist | Create the directory and file with just the pandamux block ([claude-context.ts:45-54](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L45-L54)) |
| File exists, no `pandamux:start` marker | Append the block after a normalized separator, preserving all existing content ([claude-context.ts:61-67](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L61-L67)) |
| Start marker present, end marker missing (broken markers) | Truncate at the start marker and rewrite the block, dropping whatever followed it ([claude-context.ts:69-75](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L69-L75)) |
| Both markers present | Diff the current block against the bundled one; replace only if different, otherwise no-op ([claude-context.ts:77-87](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L77-L87)) |

The whole function is wrapped in a try/catch that only logs a warning on failure, so a malformed `CLAUDE.md` or a permissions error never blocks app startup ([claude-context.ts:88-90](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L88-L90)).

```typescript
// src/main/claude-context.ts:77-87
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
```

Sources: [claude-context.ts:1-91](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L1-L91)
<!-- END:AUTOGEN pandamux_11_ai-integration_context -->

---

<!-- BEGIN:AUTOGEN pandamux_11_ai-integration_hooks -->
## Hook and Plugin Configuration

`ensureClaudeHooks()` writes PostToolUse, Notification, and Stop hooks into `~/.claude/settings.json` so pandamux's sidebar and diff view stay current with what Claude Code is doing, and so the notification bell fires when an agent needs input or finishes a turn. It only runs if `settings.json` already exists (it does not create one), and it resolves an absolute path to `pandamux-hook.js` outside the ASAR, because a packaged Node process spawned by Claude Code cannot read files inside `app.asar` ([claude-context.ts:169-193](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L169-L193)).

The actual merge is done by a pure, unit-testable helper, `applyPandaMUXHooks(settings, hookScript)`, extracted specifically so the hook-merge logic can be tested without touching the filesystem ([claude-context.ts:113-120](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L113-L120)). It strips any prior pandamux-authored entries (matched by `command?.includes('pandamux-hook')`) while leaving every user-authored hook entry untouched ([claude-context.ts:130-135](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L130-L135)):

| Hook event | Entries written | Command shape | Purpose |
|---|---|---|---|
| `PostToolUse` | One matcher entry per tracked tool: `Bash`, `Read`, `Write`, `Edit`, `Grep`, `Glob`, `Agent`, `WebSearch`, `WebFetch`, `Skill` ([claude-context.ts:111](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L111)) | `node "<hookScript>" <Tool> 2>/dev/null \|\| true` ([claude-context.ts:127](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L127)) | Drives the sidebar/diff view with per-tool activity ([claude-context.ts:137-144](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L137-L144)) |
| `Notification` | One entry, no matcher | `node "<hookScript>" --event Notification 2>/dev/null \|\| true` ([claude-context.ts:128](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L128)) | Fires a pandamux notification when Claude Code is waiting on input/permission ([claude-context.ts:146-150](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L146-L150)) |
| `Stop` | One entry, no matcher | `node "<hookScript>" --event Stop 2>/dev/null \|\| true` | Fires a pandamux notification when Claude Code finishes its turn ([claude-context.ts:152-156](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L152-L156)) |

```typescript
// src/main/claude-context.ts:121-144
export function applyPandaMUXHooks(settings: any, hookScript: string): any {
  const next = { ...(settings || {}) };
  next.hooks = { ...(next.hooks || {}) };

  const makeToolCmd = (tool: string) => `node "${hookScript}" ${tool} 2>/dev/null || true`;
  const makeEventCmd = (event: string) => `node "${hookScript}" --event ${event} 2>/dev/null || true`;

  const stripPandaMUX = (entries: any): any[] =>
    (Array.isArray(entries) ? entries : []).filter((e: any) => {
      if (!Array.isArray(e.hooks)) return true;
      return !e.hooks.some((h: any) => h.command?.includes('pandamux-hook'));
    });

  next.hooks.PostToolUse = [
    ...stripPandaMUX(next.hooks.PostToolUse),
    ...TRACKED_TOOLS.map(tool => ({
      matcher: tool,
      hooks: [{ type: 'command', command: makeToolCmd(tool) }],
    })),
  ];
```

Separately, `ensureChromeDevtoolsConfig()` also edits `~/.claude/settings.json`: it disables the `chrome-devtools-mcp@claude-plugins-official` plugin (which would otherwise launch its own Chrome instance) and registers a custom `chrome-devtools` MCP server pointed at pandamux's own CDP proxy with `--browserUrl=http://127.0.0.1:9222`, only writing the file if either value actually needs to change ([claude-context.ts:208-244](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L208-L244)).

`ensureOrchestratorPlugin()` auto-installs the bundled `pandamux-orchestrator` Claude Code plugin from `resources/pandamux-orchestrator/` (or `process.resourcesPath/pandamux-orchestrator` when packaged) into `~/.claude/plugins/cache/pandamux-orchestrator/{version}/`, reading the version out of the plugin's own `.claude-plugin/plugin.json` ([claude-context.ts:271-301](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L271-L301)). If the same version is already installed at the cache path it skips the copy but still ensures registration; otherwise it removes the old version directory and recursively copies the plugin tree with `copyDirSync()` ([claude-context.ts:303-329](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L303-L329)). The helper `ensurePluginRegistered()` then writes an entry into `~/.claude/plugins/installed_plugins.json` (scope `user`, install path, version, timestamps) and flips `settings.json`'s `enabledPlugins['pandamux-orchestrator@pandamux']` to `true`, each write gated on the value actually differing from what is already on disk ([claude-context.ts:341-389](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L341-L389)).

Sources: [claude-context.ts:93-389](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-context.ts#L93-L389)
<!-- END:AUTOGEN pandamux_11_ai-integration_hooks -->

---

<!-- BEGIN:AUTOGEN pandamux_11_ai-integration_observer -->
## Claude Activity Observer

`claude-observer.ts` turns the raw text that flows through a Claude Code PTY into structured per-surface activity for the sidebar. It is fed by `ipc-handlers.ts`, which calls `observePtyData(id, data)` on every chunk emitted by a PTY's `onData` callback, wrapped in its own try/catch so a parsing failure never breaks the terminal data path ([ipc-handlers.ts:53-60](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L53-L60)).

Incoming data is first stripped of ANSI escape codes and OSC sequences ([claude-observer.ts:11-13](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L11-L13)), then matched line by line against a fixed set of regular expressions that recognize Claude Code's terminal UI conventions ([claude-observer.ts:33-53](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L33-L53)):

| Pattern | Matches | Effect on state |
|---|---|---|
| `agentBatchStart` | `Running (\d+) agents` | Resets `agents` to `[]`, clears `isDone` ([claude-observer.ts:89-95](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L89-L95)) |
| `agentDetail` | `├─ Name · N tool uses · Xk tokens` | Adds or updates an entry in `agents[]` by name ([claude-observer.ts:98-114](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L98-L114)) |
| `agentDone` | `⎿  Done` | Marks the last agent in the list as `done` ([claude-observer.ts:117-125](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L117-L125)) |
| `agentBatchDone` | `(\d+) \w+ agents? finished` | Marks every agent in the batch `done` ([claude-observer.ts:128-133](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L128-L133)) |
| `skillLoad` | `Skill(name)` or `Skill(ns:name)` | Sets `activeSkill` ([claude-observer.ts:136-141](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L136-L141)) |
| `toolUse` | `● Bash(...)`, `● Read(...)`, etc. | Sets `lastTool`, clears `isDone` ([claude-observer.ts:144-150](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L144-L150)) |
| `mcpTool` | `● plugin:name:tool` | Sets `lastTool` to `"name:tool"`, clears `isDone` ([claude-observer.ts:153-159](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L153-L159)) |
| `responseDone` | `✻ Baked for ...` or `✻ Cost: ...` | Sets `isDone = true`, clears `lastTool` and `activeSkill` ([claude-observer.ts:80-86](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L80-L86)) |

State is kept per `SurfaceId` in an in-memory `Map<SurfaceId, ClaudeActivity>` ([claude-observer.ts:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L30)), and any line match sets a `changed` flag; if any line in the chunk changed the activity, the module updates `lastUpdate` and calls `broadcast()`, which pushes the activity object to every open `BrowserWindow` over `IPC_CHANNELS.CLAUDE_ACTIVITY` (`claude:activity`) so the renderer's sidebar can render it ([claude-observer.ts:162-165](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L162-L165), [claude-observer.ts:204-210](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L204-L210), [types.ts:312](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/shared/types.ts#L312)).

The module also exposes `getActivity()` and `clearActivity()` for surface lookup and teardown ([claude-observer.ts:171-180](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L171-L180)), and `applyExternalActivity(surfaceId, partial)`, which merges a partial `ClaudeActivity` pushed from an external producer (the OpenCode plugin, over the pipe) into the same per-surface map and re-broadcasts on the same channel, making the sidebar agent-agnostic rather than Claude-Code-specific ([claude-observer.ts:188-199](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L188-L199)).

Sources: [claude-observer.ts:1-211](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/claude-observer.ts#L1-L211), [ipc-handlers.ts:53-60](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L53-L60)
<!-- END:AUTOGEN pandamux_11_ai-integration_observer -->

---

<!-- BEGIN:AUTOGEN pandamux_11_ai-integration_opencode -->
## OpenCode Support

`opencode-context.ts` gives OpenCode the same treatment `claude-context.ts` gives Claude Code, targeting OpenCode's own config locations instead of `~/.claude/`.

`ensureOpencodeContext()` injects the same bundled instructions block (`resources/claude-instructions.md`, resolved the same way as the Claude Code path) into `~/.config/opencode/AGENTS.md`, creating the `opencode` config directory if needed ([opencode-context.ts:31-43](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L31-L43), [opencode-context.ts:47-67](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L47-L67)). The marker-based merge is the same `<!-- pandamux:start -->` / `<!-- pandamux:end -->` scheme as Claude Code, but here it is factored into a standalone pure function, `injectPandaMUXBlock(existing, pandamuxBlock)`, so it is directly unit-testable: it trims trailing whitespace off the block for idempotency, handles an empty target file, appends after the existing content if no start marker is found, truncates at the start marker if the end marker is missing, and otherwise replaces the delimited region in place ([opencode-context.ts:14-29](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L14-L29)).

```typescript
// src/main/opencode-context.ts:14-29
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
```

`ensureOpencodePlugin()` installs a separate artifact, `resources/opencode-plugin/pandamux.js` (or `process.resourcesPath/opencode-plugin/pandamux.js` when packaged), into `~/.config/opencode/plugin/pandamux.js` ([opencode-context.ts:80-89](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L80-L89), [opencode-context.ts:92-110](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L92-L110)). Whether to overwrite the installed copy is decided by `pluginNeedsUpdate(src, target)`, which compares a `pandamux-plugin-version: <value>` marker embedded in the source against the same marker in the installed file: a `null` target (nothing installed yet) or a source with no version marker both force a reinstall, otherwise the two version strings are compared directly ([opencode-context.ts:69-78](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L69-L78)).

| Artifact | Source path (packaged) | Installed path | Update check |
|---|---|---|---|
| Instructions block | `process.resourcesPath/claude-instructions/claude-instructions.md` ([opencode-context.ts:31-40](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L31-L40)) | `~/.config/opencode/AGENTS.md` ([opencode-context.ts:42](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L42)) | Content diff via marker replacement ([opencode-context.ts:58-63](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L58-L63)) |
| OpenCode plugin | `process.resourcesPath/opencode-plugin/pandamux.js` ([opencode-context.ts:80-89](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L80-L89)) | `~/.config/opencode/plugin/pandamux.js` ([opencode-context.ts:100-101](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L100-L101)) | `pandamux-plugin-version:` marker comparison ([opencode-context.ts:69-78](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L69-L78)) |

The OpenCode plugin itself (`resources/opencode-plugin/pandamux.js`) is the piece that pushes activity into `applyExternalActivity()` in the observer described above, giving OpenCode sessions the same sidebar activity display Claude Code gets natively; the plugin's own contents are outside `src/main/` and are `_TBD_` for this page (no `.ts` source under `src/main` documents its wire format, only the install mechanics above).

Both `ensureOpencodeContext()` and `ensureOpencodePlugin()` are called unconditionally on every app start alongside the Claude Code setup functions, right after `ensureOrchestratorPlugin()` ([index.ts:285-286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L285-L286)).

Sources: [opencode-context.ts:1-111](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/opencode-context.ts#L1-L111), [index.ts:276-286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L276-L286)
<!-- END:AUTOGEN pandamux_11_ai-integration_opencode -->

---
