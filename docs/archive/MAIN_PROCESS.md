<!-- PAGE_ID: pandamux_04_main-process -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [index.ts:1-819](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L1-L819)
- [pty-manager.ts:1-463](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L1-L463)
- [window-manager.ts:1-143](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L1-L143)
- [ipc-handlers.ts:1-372](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L1-L372)
- [agent-manager.ts:1-115](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L1-L115)
- [notification-manager.ts:1-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/notification-manager.ts#L1-L19)
- [session-persistence.ts:1-151](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L1-L151)
- [settings-store.ts:1-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L1-L47)

</details>

# Main Process Modules

> **Related Pages**: [Architecture](ARCHITECTURE.md), [Named Pipe Control Plane](../features/NAMED_PIPE_IPC.md)

---

<!-- BEGIN:AUTOGEN pandamux_04_main-process_entry -->
## Entry Point and Bootstrap

`src/main/index.ts` is the Electron main-process entry point. It wires together every other main-process module, owns the app lifecycle events, and hosts the V2 JSON-RPC method dispatch for the named pipe server.

On startup it sets the Windows `AppUserModelId` for correct taskbar grouping, strips the `Zone.Identifier` Mark-of-the-Web stream from the app directory so downloaded portable builds don't trigger SmartScreen warnings on every launch, and acquires a single-instance lock so a stray second `pandamux` launch (for example from a shell where `pandamux` resolves to the GUI exe instead of the CLI) hands off to the running window instead of spawning a second one ([index.ts:202-228](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L202-L228)). `hardenWebContents()` locks down `<webview>` tags (no Node integration, no preload), routes `window.open` popups to the OS browser, and blocks the top-level window from navigating away from its own `localhost`/`file://` origin ([index.ts:230-274](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L230-L274)).

Inside `app.whenReady()` the entry point injects Claude Code / OpenCode context files, registers the `session:save` IPC listener, calls `registerIpcHandlers()`, restores the last saved window bounds, conditionally starts the auto-updater when packaged, starts the named pipe server and CDP proxy, and wires the port/git/PR pollers to broadcast `METADATA_UPDATE` to every window ([index.ts:276-337](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L276-L337)). Auto-save runs on a 30 second debounce timer that asks every renderer to push its state via a `session:request` event ([index.ts:112-128](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L112-L128)).

The bulk of the file is the V2 named-pipe method switch, registered via `pipeServer.on('v2', ...)`. Methods that belong to other modules (`browser.*` and the uniform renderer-bridge methods such as `workspace.*`, `pane.split`, `surface.create/close/focus/list`) are routed out first by `routeSpecialV2()`; everything else (terminal I/O, markdown, notifications, sidebar, agents, hooks) is handled inline in the switch ([index.ts:24-37](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L24-L37), [index.ts:390-787](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L390-L787)).

| Function / Handler | Purpose | Location |
|---|---|---|
| `routeSpecialV2()` | Delegates `browser.*` and renderer-bridge V2 methods to their own modules before the main switch runs | [index.ts:27-37](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L27-L37) |
| `resolveAgentAssignments()` | Picks a pane per agent in a batch spawn (`stack` sorts by tab count, `distribute` round-robins via `distributeAgents`) | [index.ts:40-49](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L40-L49) |
| `spawnAgentBatch()` | Spawns each agent in a batch into its assigned pane; per-agent failures are captured as `{ error }` so one bad agent can't fail the batch | [index.ts:51-77](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L51-L77) |
| `stripMotw()` | Removes the `:Zone.Identifier` alternate data stream from every `.exe`/`.dll`/`.node`/`.lnk` under the app dir | [index.ts:94-110](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L94-L110) |
| `scheduleAutoSave()` | Debounces a 30s `session:request` broadcast to all windows | [index.ts:116-128](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L116-L128) |
| `resolvePtySurface()` | Resolves a target `surfaceId` for `surface.send_text`/`send_key`, falling back to the renderer's active surface and rejecting non-PTY panes | [index.ts:134-158](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L134-L158) |
| `translateKeyName()` | Maps named keys (`enter`, `ctrl-c`, `f1`..`f12`, arrows, etc.) to raw PTY bytes for `surface.send_key` | [index.ts:164-200](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L164-L200) |
| `hardenWebContents()` | Applies webview/navigation lockdown to every `web-contents-created` event | [index.ts:236-274](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L236-L274) |

```typescript
// index.ts:27-37
function routeSpecialV2(
  request: { method: string; params?: any },
  respond: (result: any) => void,
  respondError: (code: number, message: string) => void,
): boolean {
  if (request.method.startsWith('browser.')) {
    handleBrowserV2(request.method, request.params, respond, respondError);
    return true;
  }
  return handleBridgeV2(request.method, request.params, respond, respondError);
}
```

App lifecycle teardown kills every PTY before anything else, specifically to avoid a `remove_pty_baton` MSVC assertion from node-pty's libuv batons still being pending at process exit, then stops the pipe server, CDP proxy, port scanner, git poller, and PR poller ([index.ts:804-814](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L804-L814)).

Sources: [index.ts:1-819](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/index.ts#L1-L819)
<!-- END:AUTOGEN pandamux_04_main-process_entry -->

---

<!-- BEGIN:AUTOGEN pandamux_04_main-process_pty -->
## PTY Manager

`src/main/pty-manager.ts` owns the full lifecycle of every terminal process: shell resolution, ConPTY spawning, chunked writes, resize, and tree-kill. Its `PtyManager` class is instantiated once (in `ipc-handlers.ts`) and shared by both the IPC layer and the agent manager.

Shell resolution falls back through `pwsh.exe` -> `powershell.exe` -> `cmd.exe` on Windows, validating each candidate with `where`/`which` before accepting it ([pty-manager.ts:29-53](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L29-L53)). `buildShellArgs()` builds the launch arguments per shell type: PowerShell dot-sources the bundled integration script, `cmd.exe` runs `/K` with the cmd integration script, and WSL is launched with `WSLENV` extended so `PANDAMUX_*` variables cross the Windows/Linux boundary ([pty-manager.ts:100-134](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L100-L134)).

`create()` is idempotent per `surfaceId`: if a live PTY already exists for that id (React StrictMode can double-invoke the mount effect) it returns the existing entry marked `reused: true` instead of spawning a second shell process ([pty-manager.ts:205-221](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L205-L221)). It spawns via `node-pty`'s bundled `conpty.dll` (`useConptyDll: true`) to avoid a repaint-garbling bug in the OS-inbox ConPTY, retrying with the inbox ConPTY if the bundled DLL spawn throws ([pty-manager.ts:279-292](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L279-L292)). It also answers Primary Device Attributes (DA1) probes in-process so PSReadLine/oh-my-posh prompts never stall or leak the escape reply onto the command line ([pty-manager.ts:171-190](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L171-L190), [pty-manager.ts:307-317](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L307-L317)).

| Export | Type | Signature | Source |
|---|---|---|---|
| `CreateOptions` | interface | `{ shell, cwd, env, cols?, rows?, surfaceId?, startupCommands? }` | [pty-manager.ts:156-169](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L156-L169) |
| `PtyManager.create` | method | `(options: CreateOptions) => { id, shell, startupCommandsConsumed, reused }` | [pty-manager.ts:202-329](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L202-L329) |
| `PtyManager.write` | method | `(id: SurfaceId, data: string) => void` | [pty-manager.ts:331-354](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L331-L354) |
| `PtyManager.resize` | method | `(id: SurfaceId, cols: number, rows: number) => void` | [pty-manager.ts:379-388](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L379-L388) |
| `PtyManager.kill` | method | `(id: SurfaceId) => void` | [pty-manager.ts:390-429](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L390-L429) |
| `PtyManager.killAll` | method | `() => void` | [pty-manager.ts:431-435](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L431-L435) |
| `PtyManager.has` | method | `(id: SurfaceId) => boolean` | [pty-manager.ts:437-439](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L437-L439) |
| `PtyManager.onData` / `onExit` | method | `(id, callback) => () => void` (unsubscribe) | [pty-manager.ts:441-457](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L441-L457) |
| `PtyManager.getPid` | method | `(id: SurfaceId) => number \| undefined` | [pty-manager.ts:459-462](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L459-L462) |

`write()` takes a fast path for anything under 1 KB and enqueues longer pastes onto a per-PTY `writeChain` promise, chunking at 1 KB with a `setImmediate` between chunks so a single write can never outrun ConPTY's input pipe and silently drop bytes:

```typescript
// pty-manager.ts:331-354
write(id: SurfaceId, data: string): void {
  const entry = this.ptys.get(id);
  if (!entry || !entry.alive || data.length === 0) return;

  if (data.length <= PtyManager.CHUNK_THRESHOLD && entry.pendingChunks === 0) {
    try {
      entry.pty.write(data);
    } catch {
      // pty was killed between get() and write()
    }
    return;
  }

  entry.pendingChunks++;
  entry.writeChain = entry.writeChain
    .then(() => this.writeChunked(entry, data))
    .finally(() => {
      entry.pendingChunks = Math.max(0, entry.pendingChunks - 1);
    });
}
```

`kill()` spawns `taskkill /PID <pid> /T /F` (resolved by absolute path under `%SystemRoot%\System32`) before closing the pseudoconsole, because with `useConptyDll: true` node-pty's DLL kill path only calls `ClosePseudoConsole`, which does not terminate grandchild processes such as a Claude Code `-s` backend that outlives the console ([pty-manager.ts:390-429](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L390-L429)). `resize()` drops no-op same-size resizes because they still force PSReadLine/oh-my-posh to redraw the prompt ([pty-manager.ts:379-388](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L379-L388)).

Sources: [pty-manager.ts:1-463](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/pty-manager.ts#L1-L463)
<!-- END:AUTOGEN pandamux_04_main-process_pty -->

---

<!-- BEGIN:AUTOGEN pandamux_04_main-process_window -->
## Window Manager

`src/main/window-manager.ts` exports the `WindowManager` class, which creates and tracks every `BrowserWindow` by a branded `WindowId`.

`createWindow()` accepts optional saved `bounds`/`maximized` state from `session-persistence.ts`. Before applying saved bounds it validates them against the display they best match: bounds smaller than 400x300, bounds that no longer intersect any display's work area, are discarded; otherwise the rectangle is clamped to the target display's work area and nudged fully onto it, preventing the "tiny window" regression that can occur on multi-monitor / mixed-DPI setups ([window-manager.ts:32-57](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L32-L57)). The window itself is created with a hidden title bar and a custom `titleBarOverlay`, `contextIsolation: true`, `nodeIntegration: false`, and `webviewTag: true` so the renderer can host `<webview>` panes ([window-manager.ts:59-81](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L59-L81)). In dev mode it loads the Vite dev server URL and opens DevTools detached; in production it loads the built `index.html` ([window-manager.ts:83-91](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L83-L91)).

| Method | Signature | Purpose |
|---|---|---|
| `createWindow` | `(bounds?, maximized?) => WindowId` | Creates a `BrowserWindow`, clamping/validating saved bounds and restoring maximized state ([window-manager.ts:26-106](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L26-L106)) |
| `closeWindow` | `(id: WindowId) => void` | Closes the tracked window if it still exists ([window-manager.ts:108-113](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L108-L113)) |
| `focusWindow` | `(id: WindowId) => void` | Focuses the tracked window ([window-manager.ts:115-120](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L115-L120)) |
| `getWindow` | `(id: WindowId) => BrowserWindow \| undefined` | Returns the live `BrowserWindow`, filtering out destroyed instances ([window-manager.ts:122-125](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L122-L125)) |
| `getAllWindows` | `() => Array<{ id, window }>` | Returns every non-destroyed tracked window ([window-manager.ts:127-129](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L127-L129)) |
| `listWindows` | `() => Array<{ id, bounds, focused }>` | Serializable window list for IPC/pipe responses ([window-manager.ts:131-137](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L131-L137)) |
| `getCount` | `() => number` | Number of tracked windows ([window-manager.ts:139-141](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L139-L141)) |

```typescript
// window-manager.ts:59-81
const win = new BrowserWindow({
  width: bounds?.width ?? 1400,
  height: bounds?.height ?? 900,
  x: bounds?.x,
  y: bounds?.y,
  minWidth: 800,
  minHeight: 500,
  icon: getAppIcon(),
  titleBarStyle: 'hidden',
  titleBarOverlay: {
    color: '#1a1a1a',
    symbolColor: '#cccccc',
    height: 38,
  },
  backgroundColor: '#1a1a1a',
  webPreferences: {
    preload: path.join(__dirname, '../preload/index.js'),
    contextIsolation: true,
    nodeIntegration: false,
    sandbox: false,
    webviewTag: true,
  },
});
```

Sources: [window-manager.ts:1-143](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/window-manager.ts#L1-L143)
<!-- END:AUTOGEN pandamux_04_main-process_window -->

---

<!-- BEGIN:AUTOGEN pandamux_04_main-process_ipc -->
## IPC Handlers

`src/main/ipc-handlers.ts` instantiates the shared `PtyManager`, `NotificationManager`, `CDPBridge`, and `AgentManager` singletons at module scope and exports `registerIpcHandlers()`, which wires every renderer-facing `ipcMain` channel defined in `IPC_CHANNELS` ([ipc-handlers.ts:21-26](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L21-L26)). `ptyManager`, `cdpBridge`, and `agentManager` are re-exported so `index.ts` can reuse the same instances for V2 pipe methods and agent PTY forwarding ([ipc-handlers.ts:371](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L371)).

`PTY_CREATE` is the most involved handler: it defaults `cwd` to `USERPROFILE`, calls `ptyManager.create()`, and, for a freshly-spawned (non-reused) PTY, wires `onData`/`onExit` forwarding to `PTY_DATA`/`PTY_EXIT` IPC events on the owning window, additionally feeding every data chunk to `observePtyData()` for the Claude Code activity sidebar ([ipc-handlers.ts:39-74](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L39-L74)). Reused PTYs (StrictMode double-mount) skip re-wiring entirely to avoid double-forwarding every chunk ([ipc-handlers.ts:47-52](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L47-L52)).

| Channel Group | Handlers | Purpose |
|---|---|---|
| PTY | `PTY_CREATE`, `PTY_WRITE`, `PTY_RESIZE`, `PTY_KILL`, `PTY_HAS` | Terminal lifecycle from the renderer ([ipc-handlers.ts:39-90](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L39-L90)) |
| System | `SYSTEM_GET_SHELLS`, `SYSTEM_OPEN_EXTERNAL`, `SYSTEM_GET_VERSION`, `SYSTEM_GET_SHOULD_USE_DARK_COLORS`, `SYSTEM_PICK_FOLDER` | Shell discovery, external links, app version, OS theme, folder picker ([ipc-handlers.ts:92-114](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L92-L114), [ipc-handlers.ts:339-352](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L339-L352)) |
| Config / Theme | `CONFIG_GET_THEME`, `CONFIG_GET_THEME_LIST`, `CONFIG_IMPORT_WT`, `CONFIG_IMPORT_GHOSTTY`, `CONFIG_GET_USER_CONFIG`, `CONFIG_RELOAD_USER_CONFIG`, `config:getProjectProfiles`, `config:importWindowsTerminalProfiles` | Theme loading and `.pandamux.json`/WT/Ghostty config import ([ipc-handlers.ts:116-161](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L116-L161)) |
| Notification | `NOTIFICATION_FIRE` | Toast + taskbar flash + renderer sound cue, single chokepoint for every fired notification | [ipc-handlers.ts:163-184](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L163-L184) |
| Window | `WINDOW_CREATE`, `WINDOW_LIST`, `WINDOW_CLOSE`, `WINDOW_FOCUS`, `WINDOW_MINIMIZE`, `WINDOW_MAXIMIZE`, `WINDOW_IS_MAXIMIZED` | Delegates to `WindowManager` ([ipc-handlers.ts:186-198](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L186-L198)) |
| CDP | `CDP_ATTACH`, `CDP_DETACH` | Per-caller-isolated browser CDP session routing ([ipc-handlers.ts:200-216](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L200-L216)) |
| Agent | `AGENT_LIST`, `AGENT_STATUS` | Read-only agent queries for the renderer store ([ipc-handlers.ts:218-223](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L218-L223)) |
| Clipboard | `clipboard:write-text`, `clipboard:read-text`, `clipboard:paste-image` | OSC 52 + image paste support routed through Electron's clipboard module ([ipc-handlers.ts:225-245](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L225-L245)) |
| Session | `SESSION_SAVE_NAMED`, `SESSION_LOAD_NAMED`, `SESSION_LIST_NAMED`, `SESSION_LOAD_AUTO`, `SESSION_DELETE_NAMED`, `settings:get-all-sync`, `settings:set` | Named-session CRUD and file-backed settings ([ipc-handlers.ts:247-286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L247-L286)) |
| Diff | `DIFF_GET_FILES`, `DIFF_GET_DIFF` | Git diff viewer support ([ipc-handlers.ts:288-300](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L288-L300)) |
| Markdown / Folder | `MARKDOWN_OPEN_FILE`, `SYSTEM_PICK_FOLDER` | Native file/folder pickers with an extension whitelist + 5 MB cap ([ipc-handlers.ts:302-352](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L302-L352)) |

```typescript
// ipc-handlers.ts:306-325 (markdown.load_file guard, mirrored from the pipe handler in index.ts)
ipcMain.handle(IPC_CHANNELS.MARKDOWN_OPEN_FILE, async (event) => {
  const ALLOWED_MD_EXT = new Set(['.md', '.markdown', '.mdx', '.txt', '.text', '.rst']);
  const MAX_MD_BYTES = 5 * 1024 * 1024;
  const win = BrowserWindow.fromWebContents(event.sender) ?? undefined;
  const result = await dialog.showOpenDialog(win as BrowserWindow, {
    title: 'Open Markdown File',
    properties: ['openFile'],
    filters: [
      { name: 'Markdown / Text', extensions: ['md', 'markdown', 'mdx', 'txt', 'text', 'rst'] },
      { name: 'All Files', extensions: ['*'] },
    ],
  });
  if (result.canceled || result.filePaths.length === 0) {
    return { canceled: true };
  }
  const filePath = result.filePaths[0];
  const ext = path.extname(filePath).toLowerCase();
  if (!ALLOWED_MD_EXT.has(ext)) {
    return { error: `Unsupported file type: ${ext || '(none)'}` };
  }
  // ...
});
```

`setupAgentPtyForwarding()` mirrors the `PTY_CREATE` data/exit forwarding for agent-spawned PTYs, since those are created directly by `AgentManager.spawn()` rather than through the `PTY_CREATE` IPC channel ([ipc-handlers.ts:355-369](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L355-L369)).

Sources: [ipc-handlers.ts:1-372](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L1-L372)
<!-- END:AUTOGEN pandamux_04_main-process_ipc -->

---

<!-- BEGIN:AUTOGEN pandamux_04_main-process_persistence -->
## Session Persistence and Settings

`session-persistence.ts` and `settings-store.ts` both persist state as JSON files under the app's `%APPDATA%\pandamux` directory (`getAppDataDir()`), a location outside the portable-zip extraction folder that survives version-to-version updates ([session-persistence.ts:1-10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L1-L10), [settings-store.ts:5-14](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L5-L14)).

`saveSession()` and `saveSetting()` both use the same atomic-write pattern: write to a `.tmp` file, delete any existing target (Windows `rename` does not overwrite), then rename the temp file into place, so a crash mid-write can never leave a truncated `session.json` or `settings.json` ([session-persistence.ts:37-53](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L37-L53), [settings-store.ts:29-43](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L29-L43)). `handleVersionChange()` clears only the volatile auto-saved `session.json` on a version bump (its PTYs died with the old process) while explicitly preserving named saved sessions and the last-session pointer, since those are layout-only snapshots the user chose to keep ([session-persistence.ts:72-95](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L72-L95)).

| Export | Type | Signature | Source |
|---|---|---|---|
| `SessionData` | interface | `{ version: 1, windows: [...] }` | [session-persistence.ts:12-29](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L12-L29) |
| `saveSession` / `loadSession` | function | `(data) => void` / `() => SessionData \| null` | [session-persistence.ts:37-66](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L37-L66) |
| `getSessionPath` | function | `() => string` | [session-persistence.ts:68-70](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L68-L70) |
| `handleVersionChange` | function | `(currentVersion: string) => boolean` | [session-persistence.ts:84-95](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L84-L95) |
| `saveNamedSession` / `loadNamedSession` | function | `(session) => void` / `(name: string) => SavedSession \| null` | [session-persistence.ts:101-114](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L101-L114) |
| `listNamedSessions` / `deleteNamedSession` | function | `() => Array<{name, savedAt, workspaceCount}>` / `(name) => boolean` | [session-persistence.ts:116-138](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L116-L138) |
| `getLastSessionName` / `setLastSessionName` | function | `() => string \| null` / `(name: string) => void` | [session-persistence.ts:140-150](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L140-L150) |
| `loadSettings` | function | `() => Record<string, unknown>` | [settings-store.ts:18-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L18-L27) |
| `saveSetting` | function | `(key: string, value: unknown) => void` | [settings-store.ts:29-43](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L29-L43) |
| `getSettingsPath` | function | `() => string` | [settings-store.ts:45-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L45-L47) |

```typescript
// session-persistence.ts:84-95
export function handleVersionChange(currentVersion: string): boolean {
  ensureDirectories();
  try {
    const saved = fs.existsSync(VERSION_FILE) ? fs.readFileSync(VERSION_FILE, 'utf-8').trim() : '';
    if (saved === currentVersion) return false;
    // Reset only the volatile auto-session. Named sessions (SAVED_DIR) and the
    // last-session pointer are intentionally preserved across updates.
    try { if (fs.existsSync(SESSION_FILE)) fs.unlinkSync(SESSION_FILE); } catch {}
    fs.writeFileSync(VERSION_FILE, currentVersion, 'utf-8');
    return true;
  } catch { return false; }
}
```

`settings:get-all-sync` is deliberately a *synchronous* IPC channel (`event.returnValue`) rather than `ipcMain.handle`, so the renderer's Zustand settings slice can hydrate at module-load time without an async flash of default values ([ipc-handlers.ts:274-282](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/ipc-handlers.ts#L274-L282)).

Sources: [session-persistence.ts:1-151](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/session-persistence.ts#L1-L151), [settings-store.ts:1-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/settings-store.ts#L1-L47)
<!-- END:AUTOGEN pandamux_04_main-process_persistence -->

---

<!-- BEGIN:AUTOGEN pandamux_04_main-process_supporting -->
## Supporting Modules

Two smaller main-process modules round out the module set: `agent-manager.ts` (agent PTY spawning and pane load balancing) and `notification-manager.ts` (native OS notifications and taskbar flashing).

`AgentManager.spawn()` creates a PTY with the default shell (never a hardcoded `cmd.exe`, so agents run in the user's preferred shell) and waits for a recognizable prompt (`PS C:\path>`, `$ `, `#`, `%`, `>`) in the PTY's data stream before writing the agent's command, since a blind fixed-delay timeout was previously losing commands typed before PowerShell's 1-3 second integration-script startup finished. A 5 second fallback timer sends the command anyway if no prompt is ever detected ([agent-manager.ts:32-90](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L32-L90)). `distributeAgents()` is a stable round-robin over panes pre-sorted by tab count, used both for `agent.spawn` (single pane pick) and `agent.spawn_batch`'s `distribute` strategy ([agent-manager.ts:10-22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L10-L22)).

`NotificationManager` is a thin wrapper over Electron's `Notification` and `BrowserWindow.flashFrame`: `showToast()` shows a native toast with an optional click handler, and `flashTaskbar()` only flashes the taskbar icon when the target window is not already focused ([notification-manager.ts:3-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/notification-manager.ts#L3-L19)).

| Export | Type | Signature | Source |
|---|---|---|---|
| `distributeAgents` | function | `(count: number, panes: PaneLoadInfo[]) => string[]` | [agent-manager.ts:10-22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L10-L22) |
| `AgentManager.spawn` | method | `(params: AgentSpawnParams & {paneId, workspaceId}) => {agentId, surfaceId}` | [agent-manager.ts:32-90](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L32-L90) |
| `AgentManager.getStatus` / `list` / `kill` | method | `(agentId) => AgentInfo \| undefined` / `(workspaceId?) => AgentInfo[]` / `(agentId) => boolean` | [agent-manager.ts:92-106](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L92-L106) |
| `AgentManager.getAgentBySurface` | method | `(surfaceId: SurfaceId) => AgentInfo \| undefined` | [agent-manager.ts:108-113](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L108-L113) |
| `NotificationManager.showToast` | method | `(title: string, body: string, onClick?) => void` | [notification-manager.ts:4-8](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/notification-manager.ts#L4-L8) |
| `NotificationManager.flashTaskbar` / `stopFlash` | method | `(window: BrowserWindow) => void` | [notification-manager.ts:10-18](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/notification-manager.ts#L10-L18) |

```typescript
// agent-manager.ts:63-72
const removeDataListener = this.ptyManager.onData(surfaceId, (data) => {
  if (commandSent) return;
  // Prompt patterns: "PS C:\path>" (PowerShell), "$ " (bash), "> " (generic)
  if (/(?:PS\s.*>|[$#%>])\s*$/m.test(data)) {
    sendOnce();
  } else if (!promptDebounce) {
    // Got output but no prompt yet — shell is loading; wait a bit more
    promptDebounce = setTimeout(sendOnce, 1500);
  }
});
```

Sources: [agent-manager.ts:1-115](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/agent-manager.ts#L1-L115), [notification-manager.ts:1-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/notification-manager.ts#L1-L19)
<!-- END:AUTOGEN pandamux_04_main-process_supporting -->

---
