# T3 Code and PandaMUX Product Comparison

> **Review status:** Repository verified, not runtime tested
>
> **T3 Code snapshot:** `pingdotgg/t3code` commit
> [`9c524d57718e639a6abe0a5fa3cbf6e9b086df86`][t3-snapshot], reviewed September 24, 2026.
> The [server manifest] and [desktop manifest] identify version `0.0.42`, and the desktop product
> name still includes `Alpha`.
>
> **PandaMUX snapshot:** commit `35294b99cd294ffa9be8496ce20f15e04b0310e3`, workspace
> version `0.53.3` in the [PandaMUX workspace manifest], reviewed September 24, 2026.
>
> **Verification boundary:** This review inspected both repositories, their maintained
> documentation, manifests, and relevant implementation files. It did not install or run T3 Code,
> exercise its hosted services, or conduct hands-on usability and performance testing. A feature
> described as present is repository-verified, not independently acceptance-tested.

## Executive summary

T3 Code and PandaMUX overlap, but they are not the same category of product.

**T3 Code is a conversation-first agent harness and remote control surface.** Its server owns
projects, provider processes, conversations, files, Git operations, terminals, checkpoints, and
durable state. Web, Electron desktop, and iOS or Android clients connect to that server over
authenticated HTTP and WebSocket RPC. Provider adapters translate Codex, Claude, Cursor, Grok
Build, OpenCode, and Antigravity into one thread-oriented experience. See the [T3 Code README],
[architecture overview], and [provider constraints].

**PandaMUX is a terminal-first, native Windows visibility and orchestration layer.** It keeps the
agent's own CLI and terminal UI intact, then adds GPU-rendered panes, splits, tabs, projects, a
cross-project Home dashboard, notifications, SSH terminals, automation over a local named pipe,
and an optional visible multi-agent orchestrator. See the [PandaMUX README], [PandaMUX
architecture], and [agent orchestration guide].

The strongest direction is not to turn PandaMUX into a T3 Code clone. PandaMUX should stay the
best terminal-native operations cockpit for agents, while borrowing the parts of T3 Code that make
parallel work easier to start, understand, find, isolate, and finish:

1. First-class task and worktree launches.
2. Structured agent attention states and an activity timeline.
3. Durable task lifecycle, search, snooze, settle, and archive behavior.
4. Provider launch profiles for multiple accounts and configurations.
5. Lightweight Git and pull request context.
6. Batch prompt launch and comparison workflows.
7. A prompt shelf and safe queued handoff built on explicit readiness signals.

PandaMUX should defer or reject T3 Code features that create a second product and trust model:
a normalized chat client, full web and mobile parity, a cloud relay and identity service, an
embedded browser with imported credentials, mobile-device emulation, and a complete pull request
hosting client.

## Recommended position for the review meeting

Use this as the working product statement:

> PandaMUX is the terminal-native control room for local and SSH-based AI agents. It should add
> task lifecycle, isolation, semantic status, and code-host context without replacing the agents'
> own interfaces or becoming a cloud editor.

That boundary gives PandaMUX room to adopt many of T3 Code's best workflow ideas while preserving
its main advantages: native Windows performance, arbitrary CLI compatibility, transparent
terminals, visible parallel panes, direct SSH sessions, and a small local automation surface.

Both repositories use the MIT License. Feature ideas can be reimplemented freely, and source can
be reused under the license's notice requirements. Because the products use different application
stacks and state models, concept-level reimplementation is likely cleaner than transplanting T3
Code modules directly. This is a product and engineering observation, not legal advice. See the
[T3 Code license] and [PandaMUX license].

## Product model at a glance

```text
T3 Code

Web, desktop, and mobile clients
                ↓ authenticated HTTP and WebSocket RPC
Environment server
    ├─ durable event log, threads, settings, and checkpoints
    ├─ Git, files, terminals, browser previews, and devices
    └─ provider adapters → Codex, Claude, Cursor, Grok, OpenCode, Antigravity

PandaMUX

Native Iced UI, pandamux CLI, and optional orchestrator
                ↓ local Windows named pipe
Single authoritative backend
    ├─ projects, workspaces, panes, surfaces, status, and persistence
    ├─ local PTYs → any terminal program or agent CLI
    └─ SSH channels → remote tmux, SFTP, OSC 52, and remote agent CLIs
```

The architectural distinction drives nearly every difference below. T3 Code can render a
provider-neutral conversation because it mediates provider protocols. PandaMUX can support any CLI
without a provider adapter because it observes and controls terminal surfaces.

## What T3 Code does

### Agent conversation and provider control

T3 Code presents one structured thread UI across six provider families. The server starts provider
processes, translates their protocols through adapters, persists normalized events, and exposes
models, permission modes, questions, approvals, tool activity, usage, commands, and skills to its
clients. It also supports multiple provider instances for separate accounts or configurations.

The composer adds capabilities that do not exist in a raw terminal: file and media attachments,
inline context references, response citations, queued or immediate follow-ups, prompt recall,
prompt stashes, offline mobile queues, voice input, custom models, and editing from an earlier turn.
See [messages and context], [permission modes], and the [Codex provider guide].

### Durable task and workspace management

The primary unit is a durable thread associated with an environment-local project. Users can start
work in the project checkout or a new worktree, launch the same task against several models, pin and
reorder threads, snooze them, settle completed work, archive them, search messages, and inspect
delegated agents. Server-side rules can automatically settle inactive work or work whose linked pull
requests have completed. See [working with threads].

Each turn can produce a hidden Git checkpoint. Supported providers can rewind their conversation,
and exclusive worktrees can also restore files. The implementation explicitly refuses file restore
when another thread or agent uses the same directory. See [messages and context] and the
[architecture overview].

### Source control and code review

T3 Code integrates with GitHub, GitLab, Forgejo, Gitea, Bitbucket, and Azure DevOps. Depending on
the host, it can clone or publish repositories, commit and push changes, create pull requests,
review diffs and comments, mark files viewed, link several pull requests to a thread, merge, and
manage GitHub stacks. Host credentials remain on an environment machine. See [source control].

### Remote and multi-device operation

The environment server stays on the machine that owns the workspace and credentials. Clients can
reach it directly on a LAN, through Tailscale, through desktop-managed SSH, or through the optional
T3 Connect service. The desktop SSH flow installs or reuses a T3 server on the remote host and
forwards its API, rather than opening only an SSH terminal. A desktop client can also run in
remote-only mode with no local environment. See [remote access] and [remote architecture].

T3 Code ships a web client, an Electron desktop app, and native mobile clients. Mobile supports
drafts, file sharing, push notifications, live agent activity, remote approvals, and terminal or
device viewing. The clients and servers may update independently, so wire capabilities are
negotiated.

### Browser, device, capture, and usage surfaces

The desktop app includes collaborative browser previews that agents can inspect, navigate, click,
type into, resize, evaluate, and record through preview tools. It can import selected browser
profiles for signed-in preview sessions. See the [preview toolkit] and [browser import guide].

The Device panel streams and controls iOS Simulators and Android Emulators, while agents use a
small device toolkit and companion CLI. SnapShots capture another desktop window and can attach its
image, title, icon, and accessibility tree to a draft. Usage pages aggregate tokens, estimated
API-equivalent cost, provider subscription limits, and selected account details. See [devices],
[SnapShots], and [usage and limits].

## What PandaMUX does

### Native terminal multiplexing

PandaMUX is a native Rust and Iced Windows application with an Alacritty terminal grid and real
local or remote PTYs. It supports split panes, keep-alive tabs, workspaces, pane zoom, tab dragging,
terminal find, links, selection, configurable scrollback, themes, custom keybindings, multiple
windows, and a cross-project Home grid of pinned live sessions. The agent remains visible in its
own terminal UI. See the [PandaMUX README], [UI shell guide], and [terminal engine guide].

### Project and session visibility

PandaMUX groups sessions by logical project or tool type. It recognizes the same repository across
local and SSH locations using project paths and normalized Git remotes, supports project merge and
split corrections, restores sessions, and lets users rename projects and sessions. Native pollers
and shell integration track cwd, Git state, ports, and shell context. Notifications and attention
signals can come from OSC events, the CLI, or idle detection. See the [core domain guide] and [shell
integration guide].

This is visibility around terminal sessions, not a durable provider-neutral conversation model.
PandaMUX does not currently own message history, approvals, models, accounts, tool calls, or usage
records.

### Visible multi-agent orchestration

The bundled orchestrator can decompose a task into dependency-aware waves, launch agents in
separate visible panes, pass results between waves, and run a review pass with corrective work. It
can optionally isolate workers in Git worktrees and falls back to Claude Code subagents when
PandaMUX is unavailable. Installation is manual. See the [agent orchestration guide] and
[orchestrator README].

### SSH as a durable terminal transport

PandaMUX opens remote PTYs directly with `russh`, runs sessions through remote `tmux`, reconnects
after transport failures, verifies host keys, and pools one authenticated connection per host.
Folder browsing, Git identity reads, terminals, and SFTP image uploads reuse that pool. OSC 52
supports remote clipboard copy, and image paste uploads a local clipboard image before injecting
its remote path. See [SSH remote surfaces].

This is a strong remote terminal experience, but it is not a remotely accessible PandaMUX server.
The Iced UI and canonical application state remain on the Windows machine.

### Local automation

The `pandamux` CLI and optional orchestrator use a JSON RPC protocol over the local Windows named
pipe. They can create workspaces, split panes, send text or keys, read terminal output, manage
agents, drive SSH sessions, manipulate layouts, and query the workspace tree. UI actions and pipe
actions reach the same backend dispatcher. See the [named pipe guide] and [CLI reference].

The request envelope has a token field, but the current server does not validate it. The pipe is a
local control boundary and must not be exposed as a network API without a separate authentication,
authorization, and transport design.

## Detailed feature comparison

The recommendation column uses three labels:

- **Adopt:** Fits PandaMUX's current product and architecture.
- **Adapt:** Useful, but should be reshaped for a terminal-first product.
- **Skip:** Creates a different product, trust boundary, or maintenance burden.

### Agent workflow

| Capability | T3 Code | PandaMUX today | Recommendation |
|---|---|---|---|
| Primary interaction | Structured chat thread mediated by provider adapters | Native terminal containing the provider's own CLI UI | Keep PandaMUX terminal-first |
| Provider support | Codex, Claude, Cursor, Grok Build, OpenCode, and Antigravity adapters | Preset launches for Claude, Codex, Gemini, terminal types, and arbitrary commands | Adapt provider launch profiles, avoid a mandatory adapter for every CLI |
| Multiple accounts | Multiple provider instances, homes, environment variables, and account switching where supported | Custom commands can set up variants externally, but there is no first-class profile model | Adopt named provider profiles with environment and home-directory configuration |
| Model and effort selection | Structured model catalog and per-thread options | Controlled inside each agent's CLI | Adapt only if the provider exposes stable metadata or launch arguments |
| Permissions and approvals | Structured supervised, auto-edit, automatic-review, and full-access modes | The native agent TUI asks in the terminal | Adapt semantic attention events, keep final approval in the provider UI unless a safe adapter exists |
| Questions | Structured questions survive reconnects and can be answered from any client | Questions appear in the terminal; notifications may indicate attention | Adapt question detection and click-to-focus first, remote answering later |
| Prompt composition | Rich editor, files, citations, context chips, history, stashes, and voice | Terminal input, CLI send-text, clipboard, and image-path paste | Adopt a lightweight prompt shelf, file drop, and reusable snippets, not a second full chat editor |
| Follow-up queue or steer | Explicit queue and immediate steer behavior | Text can be injected, but PandaMUX does not know provider-safe turn boundaries | Adapt only after explicit provider readiness events exist |
| Parallel work | Background threads, model fan-out, worktrees, and delegated-agent views | Workspaces, splits, tabs, Home grid, agent batches, and orchestrator waves | Adopt a simple launch-the-same-task-to-selected-profiles flow |
| Subagent visibility | Delegated work is shown in an Agents view and timeline | Orchestrated agents receive their own visible terminal panes | Keep PandaMUX's pane-native differentiation, add aggregate status |
| Durable conversation | Event-sourced thread history and provider resume state | Terminal buffers and session layout restore, plus durable remote tmux | Adapt as task metadata and activity history, not duplicated provider transcripts |
| Cross-task search | Search across threads and selected messages | Find within a terminal and navigate sessions | Adopt task, session, project, label, branch, and bounded-output search |
| Usage and limits | Token, cost estimate, quota windows, accounts, and widgets | Not present | Adapt later as a clearly labeled, best-effort dashboard |

### Projects, Git, and safety

| Capability | T3 Code | PandaMUX today | Recommendation |
|---|---|---|---|
| Project identity | Environment-local project records with related checkouts | Logical project identity can merge local and SSH copies using path, remote, and folder name | Keep PandaMUX's cross-host identity model |
| First-class worktrees | New thread can create or reuse a worktree | Optional orchestrator worktree isolation, not a general launcher workflow | Adopt in the core launcher |
| Worktree cleanup | Policy-driven cleanup with dirty, active, shared, and merge guards | No core cleanup policy | Adopt only after ownership and safety checks are explicit |
| Checkpoints and rewind | Hidden Git refs per turn; supported conversations and exclusive files can rewind | Not present | Adapt as opt-in worktree checkpoints with preview and exclusivity checks |
| Git status | Full workspace and host-aware source-control model | Native branch and dirty-state polling, plus Git remote project identity | Extend current polling with ahead, behind, worktree, and linked-review context |
| Pull requests | Create, review, comment, link, merge, and stack operations across several hosts | No pull request model or host integration | Adopt link, status, checks, and open-in-browser first; defer mutations |
| Diff and file surfaces | Integrated file viewer, review comments, attachments, and diffs | Markdown and diff surface types exist in the pipe and UI, but are not a full review workflow | Adapt existing surfaces for task summaries and linked-review context |
| Automatic project settings | Environment defaults, project overrides, device preferences, and mixed-state editing | Per-machine JSON settings and live configuration | Adopt project launch profiles before general settings inheritance |

### Terminals, remote access, and platforms

| Capability | T3 Code | PandaMUX today | Recommendation |
|---|---|---|---|
| Terminal role | Supporting panel owned by the environment server | Primary product surface with native splits, tabs, workspaces, and Home grid | Keep PandaMUX's core advantage |
| Terminal persistence | Server-owned PTYs and capped retained output, reachable by several clients | In-process local PTYs, restored sessions, and remote tmux durability | Preserve, then add clearer post-restart status for local process loss |
| SSH model | Start or reuse a complete T3 server through SSH and forward its API | Direct SSH PTY with pooled channels, tmux, SFTP, and OSC 52 | Keep PandaMUX's direct model; consider a helper only for optional richer metadata |
| Remote control | Authenticated LAN, Tailscale, SSH, and T3 Connect clients | Local named pipe only; SSH controls remote shells, not PandaMUX itself | Adapt as a narrow authenticated companion only if remote monitoring is strategic |
| Web client | Full browser client | None | Skip unless PandaMUX deliberately becomes a client-server product |
| Mobile clients | iOS and Android with push, live status, drafts, approvals, and remote control | None | Adapt a read-only attention companion before considering full control |
| Host platforms | Windows, macOS, and Linux servers or desktop packages, plus mobile clients | Native Windows app with remote Linux or other SSH sessions | Keep Windows-first; do not trade native quality for broad parity without demand |
| Background service | User service on macOS and Linux; desktop-hosted service elsewhere | App or headless local pipe server | Adapt only as required by an authenticated companion |
| Updates | Independent client, desktop, server, provider, and mobile update flows | Signed Windows installer and in-app update flow | Keep PandaMUX's simpler single-product release model |

### Peripheral product surfaces

| Capability | T3 Code | PandaMUX today | Recommendation |
|---|---|---|---|
| Notifications | Desktop and mobile alerts, push, and live activity | In-app panel, pane flash, bell, OSC notifications, CLI notify, and idle detection | Adopt richer semantic categories and lifecycle actions |
| Browser preview | Embedded collaborative browser with agent tools, responsive viewports, recording, and profiles | Deliberately removed; agents use their own browser tooling | Skip and preserve the current boundary |
| Browser credential import | Copies supported browser cookies into isolated preview profiles | None | Skip because it expands credential-handling risk |
| Device emulation | Live iOS Simulator and Android Emulator panel with agent controls | None | Skip for the core product |
| Desktop capture | SnapShot window image plus optional accessibility data | Clipboard image paste into local or SSH agent prompts | Keep image paste; skip accessibility capture unless a validated Windows use case emerges |
| File and media viewer | Rich code, Markdown, HTML, PDF, audio, image, and video viewers | Terminal links plus Markdown and diff surfaces | Adapt only the formats needed for agent handoff and review |
| Themes and keybindings | Client themes, custom themes, and conditional keybinding rules | Native UI and terminal themes, imports, live keymap overrides, and pass-through chords | Continue independently; the products already overlap here |
| Public automation | Typed internal RPC plus management CLI and provider toolkits | Broad local JSON RPC and CLI for terminal and layout automation | Keep PandaMUX's scriptability as a differentiator |
| Cloud dependency | Optional Clerk and Cloudflare-backed T3 Connect relay and tunnels | No PandaMUX cloud service | Skip until a funded remote-service strategy exists |

## PandaMUX differentiators to protect

### 1. The terminal is the product, not an escape hatch

PandaMUX gives every agent a full, visible, native terminal with real PTY semantics. This keeps
provider-specific capabilities immediately available and lets unknown future CLIs work as custom
commands. A normalized chat layer would trade that compatibility for adapter maintenance.

### 2. Visible spatial orchestration

Splits, tabs, workspaces, the Home dashboard, project grouping, and orchestrator-created panes give
parallel agents a stable spatial location. T3 Code organizes work mainly as threads in a list.
PandaMUX should deepen its spatial model with aggregate status and lifecycle metadata rather than
replace it.

### 3. Direct SSH terminal quality

Remote tmux durability, pooled SSH connections, explicit host-key handling, remote folder browsing,
OSC 52, and SFTP image paste form a coherent terminal workflow. T3 Code's remote server approach is
more capable for full files, Git, and chat state, but it also installs and operates more software on
the host.

### 4. Local scriptability with one mutation path

The UI, CLI, orchestrator, and pipe clients submit the same intents to one backend. This is a good
foundation for feature expansion as long as the pipe stays local. New task, worktree, status, and
review metadata should preserve that single-dispatcher rule.

### 5. Lower trust and operational footprint

PandaMUX does not require a hosted identity provider, cloud relay, web client, or mobile release
program. It also does not ingest every provider's conversation protocol. That smaller footprint is
a product advantage, not merely missing functionality.

## Recommended feature direction

### Adopt: strong fit with the current product

#### A. First-class worktree launch and ownership

Add **Current checkout**, **New worktree**, and **Existing worktree** choices to the session
launcher. Record the worktree path, base branch, created branch, owning task, and active surfaces.
Make cleanup a separate, previewable action that refuses dirty, shared, or active worktrees.

Why this matters: parallel terminals are much safer when isolation is the default rather than an
orchestrator-only option.

#### B. Task lifecycle above sessions

Introduce a lightweight task record that groups one or more surfaces without duplicating the
provider's transcript. Suggested states are **Active**, **Waiting**, **Snoozed**, **Settled**, and
**Archived**. A task can reference a project, worktree, provider profiles, sessions, branch, linked
pull request, timestamps, and attention state.

This creates a durable unit users can search and revisit while keeping existing workspace and
surface types intact.

#### C. Semantic attention and activity timeline

Extend the current agent and notification models with explicit events such as:

- agent started, working, waiting for approval, waiting for an answer, completed, or failed;
- command or tool summary when safely supplied by the provider;
- Git branch, dirty state, and linked-review changes;
- session disconnected, reconnected, exited, or restored without a live process.

Prefer provider hooks, shell integration, OSC sequences, or the local CLI over screen-scraping.
Idle detection should remain a fallback and be labeled as inferred.

#### D. Provider launch profiles

Let users name configurations such as **Codex Work**, **Codex Personal**, **Claude Default**, or
**OpenCode Local**. A profile should select a command, arguments, working-directory policy,
environment variables, optional home directory, icon, and non-secret metadata. Secrets should be
referenced from the operating system or the provider's own storage, not copied into PandaMUX
configuration.

This captures much of T3 Code's multi-account value without making PandaMUX responsible for
provider login or protocol compatibility.

#### E. Lightweight Git and pull request context

Build on the current Git poller and normalized remote identity. Show branch, dirty state, ahead and
behind counts, worktree identity, and a linked pull request with host, number, state, and checks.
Start with read-only discovery and deep links. Commit, push, merge, and review mutations can be
evaluated later with explicit confirmations.

#### F. Batch task launch

Allow one task or prompt to launch across selected provider profiles, each in its own worktree and
visible pane. This is the direct terminal-native counterpart to T3 Code's multi-model background
launch and can reuse PandaMUX's existing batch-agent and layout primitives.

#### G. Prompt shelf and handoff

Add a small durable shelf for prompt drafts, reusable task templates, file paths, screenshots, and
session notes. Sending should still target the agent's terminal. Automatic queued delivery should
wait until a provider emits a reliable ready signal; it must not infer safety from a quiet terminal
alone.

### Adapt: useful only with PandaMUX-specific constraints

#### Checkpoints and restore

Use opt-in hidden Git refs or equivalent snapshots only for task-owned worktrees. Show the diff and
exact restore target first. Refuse restore when another live task or surface shares the directory.
Conversation rewind remains the provider's responsibility.

#### Remote companion

If remote access becomes strategic, begin with an authenticated, read-only attention view: task
status, notifications, branch, and click-to-copy connection instructions. A later control surface
can add bounded actions. Do not forward or expose the existing named pipe.

#### Per-project settings

Start with project launch defaults: provider profile, worktree behavior, layout preset, SSH target,
and notification preferences. A general inheritance engine can wait until several real settings
need it.

#### Usage and limits

Read provider-owned usage only where a stable local source exists. Separate observed tokens,
provider-reported quota, and estimated cost. Never present an API-equivalent estimate as a bill.

#### Rich files and source control

Use PandaMUX's existing Markdown and diff surfaces for task summaries, review links, and selected
files. Avoid building a full IDE or forge client until read-only context proves insufficient.

### Skip: outside the recommended boundary

- A provider-neutral replacement chat UI with complete conversation replay.
- Full web, desktop, and mobile feature parity.
- A hosted identity, relay, tunnel, and push-notification service.
- Embedded browser automation and imported browser credentials.
- iOS Simulator and Android Emulator hosting.
- A complete multi-forge pull request review and merge client.
- Accessibility-rich desktop SnapShots as a general platform feature.
- A cross-platform rewrite that compromises the native Windows terminal experience.

These are valid T3 Code capabilities. They are poor PandaMUX investments unless the product thesis
changes from a terminal multiplexer into an agent client-server platform.

## Suggested implementation sequence

This is an architectural sequence, not a delivery estimate.

### Foundation

1. Define `TaskId`, task metadata, lifecycle states, and explicit versus inferred status.
2. Define worktree ownership and exclusivity rules.
3. Add provider launch profiles without embedding secrets.
4. Extend named-pipe capabilities for task, profile, and status operations.

### Core workflow

1. Add worktree choices to the launcher.
2. Add task grouping, lifecycle actions, and an attention inbox to the Sessions panel and Home.
3. Add batch launch across profiles.
4. Add branch, worktree, and linked-review context.

### Safety and retrieval

1. Add task and session search.
2. Add snooze, settle, archive, and safe cleanup.
3. Add a prompt shelf and explicit queued delivery where readiness is known.
4. Add opt-in checkpoints with preview and shared-directory refusal.

### Optional expansion

1. Validate demand for a read-only remote companion.
2. Design a new authenticated network boundary if demand is real.
3. Add bounded remote actions only after threat modeling, auditability, and revocation exist.

## Architecture guardrails

1. **Keep provider integration optional.** Core terminal operation must not depend on a provider
   adapter or a particular agent protocol.
2. **Preserve the no-MCP decision.** PandaMUX integration should continue through the local CLI,
   named pipe, shell hooks, or narrowly defined provider hooks unless that repository-level
   decision is explicitly revisited.
3. **Never equate quiet output with completion.** Semantic provider events are explicit; idle
   detection is inferred. Show the distinction in state and UI.
4. **Do not expose the named pipe remotely.** Its token is not currently validated. Remote access
   requires a separate authenticated and authorized service boundary.
5. **Treat worktree cleanup and restore as destructive operations.** Resolve exact ownership,
   inspect dirty state, preview the action, and refuse shared active paths.
6. **Keep one mutation path.** UI, CLI, and automation should continue to reach the same backend
   dispatcher and canonical state.
7. **Prefer links and context before mutations.** Pull request status and deep links are much
   cheaper and safer than recreating an entire source-control client.
8. **Label estimates and inferred facts.** This applies to usage cost, attention state, task
   completion, and remote reachability.

## Questions to settle with the team

The recommended answer is included after each question.

1. **Is PandaMUX staying terminal-first?** Recommended: yes. Add overlays and metadata, but keep the
   agent's native CLI as the authoritative interaction surface.
2. **What is the next durable unit above a surface?** Recommended: a lightweight task that groups
   surfaces, worktree, status, branch, and review links without storing a second transcript.
3. **Should provider-specific integration exist?** Recommended: yes, but as optional capability
   adapters at the boundary. Unknown custom commands must continue to work.
4. **How much Git hosting should PandaMUX own?** Recommended: read-only pull request discovery,
   status, checks, and deep links first.
5. **Is remote control a near-term product goal?** Recommended: no full client yet. Validate a
   read-only companion use case before adding a network service.
6. **Does the embedded browser remain out?** Recommended: yes. Preserve the documented decision to
   rely on the agent's browser tooling.
7. **Should worktree isolation become the default for agent tasks?** Recommended: offer it
   prominently and remember the project preference, but keep the current checkout available for
   quick or non-mutating work.
8. **Which T3 Code flow should be prototyped first?** Recommended: new task → new worktree → chosen
   provider profile → visible pane → semantic status → linked pull request.

## Proposed meeting walkthrough

1. **Product boundary:** Agree that the comparison is conversation-first versus terminal-first,
   not feature-complete versus incomplete.
2. **Current strengths:** Demo PandaMUX Home, split panes, SSH durability, notifications, and the
   orchestrator.
3. **Highest-value gaps:** Review worktrees, task lifecycle, semantic attention, provider profiles,
   and Git or pull request context.
4. **Explicit exclusions:** Confirm browser, device lab, cloud relay, and full chat replacement stay
   out.
5. **First vertical slice:** Approve or revise the worktree-backed task flow from question 8.

## Evidence map

### T3 Code primary sources

| Topic | Source |
|---|---|
| Product claim, supported providers, clients, and install surfaces | [T3 Code README] |
| Product principles, multi-surface architecture, event model, and repository map | [T3 Code agent guide] |
| Server ownership, RPC boundary, event sourcing, checkpoints, and compatibility | [Architecture overview] |
| Provider adapter boundaries and protocol constraints | [Provider constraints] |
| Composer, attachments, queues, citations, stashes, and rewind | [Messages and context] |
| Thread lifecycle, multi-model launch, worktrees, search, agents, and snooze | [Working with threads] |
| Git hosts, pull request creation, review, linking, merge, and stacks | [Source control] |
| LAN, Tailscale, SSH, T3 Connect, pairing, and remote-only desktop | [Remote access] |
| Authenticated remote environment ownership model | [Remote architecture] |
| PTY ownership, history retention, and Ghostty-based renderers | [Terminal runtime] |
| Browser automation tools | [Preview toolkit] |
| Device panel and agent device access | [Devices] |
| Window capture and accessibility context | [SnapShots] |
| Tokens, cost estimates, subscription limits, and widgets | [Usage and limits] |
| T3 Code license | [T3 Code license] |

### PandaMUX primary sources

| Topic | Source |
|---|---|
| Product purpose and shipped feature summary | [PandaMUX README] |
| Current architecture and repository invariants | [PandaMUX development guide] |
| Backend-owned state and crate boundaries | [PandaMUX architecture] |
| Native terminal UI and panels | [UI shell guide] |
| Terminal engine | [Terminal engine guide] |
| Project, agent, session, and notification model | [Core domain guide] |
| Named pipe limitations and method catalog | [Named pipe guide] |
| CLI automation | [CLI reference] |
| SSH, tmux, SFTP, clipboard, and image paste | [SSH remote surfaces] |
| Agent state and bundled orchestrator | [Agent orchestration guide] |
| Shell context and status polling | [Shell integration guide] |
| PandaMUX license | [PandaMUX license] |

[t3-snapshot]: https://github.com/pingdotgg/t3code/tree/9c524d57718e639a6abe0a5fa3cbf6e9b086df86
[T3 Code README]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/README.md
[T3 Code agent guide]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/AGENTS.md
[Server manifest]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/apps/server/package.json
[Desktop manifest]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/apps/desktop/package.json
[Architecture overview]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/internals/overview.md
[Provider constraints]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/internals/providers.md
[Messages and context]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/composer.md
[Permission modes]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/permission-modes.md
[Codex provider guide]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/providers-codex.md
[Working with threads]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/thread-sidebar.md
[Source control]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/source-control.md
[Remote access]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/remote-access.md
[Remote architecture]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/internals/remote.md
[Terminal runtime]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/internals/terminal-runtime.md
[Preview toolkit]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/apps/server/src/mcp/toolkits/preview/tools.ts
[Browser import guide]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/browser-import.md
[Devices]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/devices.md
[SnapShots]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/snap-shot.md
[Usage and limits]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/docs/user/usage.md
[T3 Code license]: https://github.com/pingdotgg/t3code/blob/9c524d57718e639a6abe0a5fa3cbf6e9b086df86/LICENSE
[PandaMUX README]: ../../README.md
[PandaMUX development guide]: ../../CLAUDE.md
[PandaMUX workspace manifest]: ../../Cargo.toml
[PandaMUX architecture]: ../core/ARCHITECTURE.md
[UI shell guide]: ../core/UI_SHELL.md
[Terminal engine guide]: ../core/TERMINAL_ENGINE.md
[Core domain guide]: ../core/CORE_DOMAIN.md
[Named pipe guide]: ../features/NAMED_PIPE_IPC.md
[CLI reference]: ../api/CLI_REFERENCE.md
[SSH remote surfaces]: ../features/SSH_REMOTE.md
[Agent orchestration guide]: ../features/AGENT_ORCHESTRATION.md
[Orchestrator README]: ../../resources/pandamux-orchestrator/README.md
[Shell integration guide]: ../features/SHELL_INTEGRATION.md
[PandaMUX license]: ../../LICENSE
