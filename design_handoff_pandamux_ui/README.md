# Handoff: PandaMUX Everywhere — Native Rust UI (Iced)

## Overview
A new, modern interface for **PandaMUX Everywhere**, the tmux-inspired terminal multiplexer being rebuilt as a fully native Rust app (**Iced** + `alacritty_terminal` + `portable-pty`) for Windows. The design covers:

1. **Main workspace** — frameless window with custom titlebar, slim icon rail, session panel (grouped by Project / Type / Host), split terminal panes with per-pane tabs, status bar, and four overlay surfaces (command palette, quick-launch, notifications, settings).
2. **Drag-and-drop pane splitting** — the interaction model for dragging a tab out of a pane to re-split the layout (edge zones split, center zone moves the tab).

## About the Design Files
The files in this bundle are **design references created in HTML** — interactive prototypes showing intended look and behavior, **not production code**. The task is to recreate these designs in the Rust/Iced application using its idioms: Iced widgets, `Message` enums, and the existing crate layout (`pandamux-ui`, `pandamux-core`, `pandamux-term`). Where the prototype uses CSS `backdrop-filter` blur ("glass"), approximate in Iced with layered translucent fills — the exact rgba values are given below; blur is a nice-to-have, translucency + borders carry the look.

## Fidelity
**High-fidelity.** Colors, spacing, typography, radii, and interaction behavior are final and should be matched closely. All values below are exact.

---

## Design Tokens

### Chrome palette — dark (default)
| Token | Value | Use |
|---|---|---|
| `bg-base` | `#0a0e11` → `#0c1114` vertical gradient | window background |
| bg glow 1 | radial at 75% / −10%, `rgba(67,217,201,0.07)` | teal ambience, top-right |
| bg glow 2 | radial at −5% / 110%, `rgba(216,180,94,0.05)` | gold ambience, bottom-left |
| `overlay` rgb | `255,255,255` | all hover/border tints: `rgba(overlay, α)` |
| `t1` text primary | `#dbe6e6` | titles, row names |
| `t2` text secondary | `#8fa0a3` | secondary labels |
| `t3` text muted | `#7d8d90` | icons, status bar |
| `t4` text faint | `#55666a` | metadata, kbd hints |
| `bgc` knockout | `#0d1215` | toggle knobs, badge-dot borders |
| `inset` | `rgba(0,0,0,0.25)` | segmented-control recess |
| `panel` | `rgba(20,27,31,0.92)` + blur 24px | palette / launch / notifications |
| `panel2` | `rgba(18,25,29,0.95)` + blur 28px | settings modal |
| `scrim` | `rgba(5,8,10,0.5)` + blur 3–4px | overlay backdrop |

### Chrome palette — light (UI theme variant)
Terminal panes **stay dark in both themes** (terminal color scheme is independent of chrome theme, as in the existing app).
| Token | Value |
|---|---|
| `overlay` rgb | `0,0,0` |
| `t1` / `t2` / `t3` / `t4` | `#1c2527` / `#3f5054` / `#5c6c70` / `#8a9a9e` |
| `bgc` | `#eef2f2` |
| `inset` | `rgba(0,0,0,0.07)` |
| `panel` / `panel2` | `rgba(250,252,252,0.94)` / `rgba(252,253,253,0.97)` |
| `scrim` | `rgba(90,102,106,0.35)` |
| bg gradient | `#f2f5f5` → `#e7ecec`, teal glow `rgba(67,217,201,0.12)`, gold `rgba(216,180,94,0.08)` |

### Accent + shell colors
| Token | Dark | Light |
|---|---|---|
| Accent (`acc`) | `#43d9c9` (teal, from panda logo) | same |
| PowerShell | `#43d9c9` | `#0e9a8c` |
| SSH | `#d8b45e` | `#a17e22` |
| WSL | `#7fd88f` | `#3d9a50` |
| CMD | `#9aa7b0` | `#5c6c70` |

Shell badge chips: `bg rgba(<shell>,0.10–0.12)`, `border 1px rgba(<shell>,0.25–0.35)`, radius 7–8px.
Accent is user-configurable — alternates offered: `#d8b45e`, `#4d9fff`, `#b48ead`.

### Terminal pane scheme (fixed, both themes)
Surface `rgba(13,19,22,0.8)` (≈`#10171b`), text `#b7c6c6`, dim `#6b7c80`, success `#7fd88f`, warn/gold `#d8b45e`, prompt = accent. Cursor: 7×15px block in prompt color, blink 1.1s step-end.

### Typography
- **UI**: Segoe UI / system-ui. Sizes: 13px titles (600), 12–12.5px body (400–500), 11px secondary, 10.5px group headers (600, uppercase, letter-spacing 0.8–1.2px).
- **Mono**: JetBrains Mono (400/500/600). Terminal 12–12.5px / line-height 1.7–1.75; metadata 10px; kbd chips 10–10.5px; status bar 10.5px.

### Radii & shadows
- Panes 12px · overlay panels 14–16px · rows/tabs 7–9px · chips 4–6px · rail buttons 10px.
- Pane shadow: `0 8px 30px rgba(0,0,0,0.25)`.
- Focused pane: border `1px rgba(67,217,201,0.35)` + `0 0 0 1px rgba(67,217,201,0.12)` + `0 0 24px rgba(67,217,201,0.07)`.
- Overlays: `0 20–30px 60–90px rgba(0,0,0,0.35–0.65)`.

### Spacing
Workspace padding 10px, gap between panes/columns 8px. Session panel 264px wide (compact variant 216px), rail 52px, titlebar 40px, tab bar 34–36px, status bar 26px. Session rows: padding 7–8px, gap 10px, 30px badge.

---

## Screens / Views

### 1. Titlebar (40px, frameless window)
- Left: 20px logo (`assets/pandamux_logo.png`, radius 5px), "PandaMUX" 13px/600, "Everywhere" 11px `t4`.
- Center: session-switcher pill (click → command palette): search icon, active session name 12px `t2`, `Ctrl K` kbd chip. Pill: `rgba(ov,0.035)` bg, `rgba(ov,0.06)` border, radius 7px.
- Right: bell icon with accent unread dot (7px, 1.5px `bgc` border), settings icon, then min/max/close (40×28px hit areas; close hover `rgba(224,90,90,0.85)` + white glyph).
- Whole bar is the drag region except interactive controls.

### 2. Icon rail (52px)
38×38px buttons, radius 10px, icon stroke ~1.3px 16px. Top→bottom: Sessions (active), Command palette, New session, Notifications; spacer; Settings pinned bottom.
- Active: `rgba(67,217,201,0.12)` bg, accent icon, inset 1px `rgba(67,217,201,0.25)` ring.
- Hover: `rgba(ov,0.08)` bg.

### 3. Session panel (264px)
- Header: "SESSIONS" 11px/600 uppercase + count (mono, `t4`).
- **Grouping switcher**: 3-segment control (Project / Type / Host), recess `inset` bg, active segment `rgba(67,217,201,0.12–0.2)` + accent text 11px/600. Switching regroups the list live.
- Groups: header = 11px icon (folder/chip/globe/pin) + uppercase label + count. **Pinned** group always first (gold dot icon).
  - By Project: one group per repo/folder.
  - By Type: PowerShell / SSH / WSL / CMD (group icon tinted per shell).
  - By Host: "This machine" + "Remote hosts".
- **Session row** (grid: 30px badge / flexible text / 8px dot, gap 10px):
  - Badge: shell abbreviation (PS/SSH/WSL/CMD) 9px mono 600 in shell-tinted chip.
  - Line 1: name 12.5px/500 `t1`, ellipsized; optional gold pin dot.
  - Line 2: metadata 10px mono `t4` (branch · activity, e.g. `master* · cargo watch`).
  - Status dot: running = accent + glow `0 0 8px`, pulse 2.4s; busy (agent) = gold, pulse 1.2s; idle = `rgba(ov,0.16)`.
  - Active row: gradient `rgba(67,217,201,0.13)→0.04` left-to-right, border `rgba(67,217,201,0.25)`, inset 2px accent left rail.
  - Hover (inactive): `rgba(ov,0.045)`.
- Footer: full-width "+ New session" button, 1px dashed `rgba(ov,0.16)` border, radius 8px; hover → accent border/text + `rgba(67,217,201,0.05)` bg. Opens quick-launch.

### 4. Pane workspace
Columns of panes, 8px gaps, 10px outer padding. Each pane:
- **Tab bar** (36px): tabs = shell glyph (10px mono, shell color) + label 12px + × close. Active tab: `rgba(255,255,255,0.07)` bg + inset 2px accent underline, text `#e2ecec`. Inactive: `#7d8d90`. Then `+` button (opens quick-launch); right side: split-right / split-down icon buttons. SSH panes show a context chip on the right: `remote · eu-west` (gold tint).
- **Terminal area**: scheme above, padding 12–14px / 16px.
- Clicking anywhere in a pane focuses it (accent ring, see tokens).

### 5. Status bar (26px, mono 10.5px, toggleable)
Left: shell indicator (pulsing accent dot + `pwsh 7.4`), git branch icon + `master*` + gold ahead-count `↑2`, `ports 5173 · 8080`. Right: `6 sessions · 3 panes`, `UTF-8`, version. Separators = 14px gaps.

### 6. Command palette (Ctrl+K / Ctrl+Shift+P)
Centered, 560px wide, top offset 12vh, `panel` bg, radius 14px, extra ring `0 0 0 1px rgba(67,217,201,0.08)`. Header row: search icon + borderless input 14px + `esc` chip. Rows: glyph column 20px (accent, mono 10px), label 13px `t2`, right-aligned shortcut (mono 10px `t4`); hover `rgba(67,217,201,0.09)`. Live substring filtering. Content mixes commands, session switching, and theme switching. Entrance: fade+rise 6px, 140ms.

### 7. Quick-launch menu
300px popover anchored near the trigger, `panel` bg, radius 14px, 8px padding. Header "NEW SESSION" 10.5px uppercase. Rows: 28px shell chip + name 12.5px + command line 10px mono `t4` (e.g. `wsl.exe -d Ubuntu`, `ssh admin@10.0.4.11`). Profiles: PowerShell 7, Windows PowerShell 5.1, CMD, WSL distros, SSH hosts (importable from `~/.ssh/config`).

### 8. Notifications panel
320px right-side slide-over between titlebar and status bar, `panel` bg, radius 14px, slide-in 160ms from +24px. Header: "Notifications" + accent "Clear all". Cards: `rgba(ov,0.03)` bg, radius 10px; 7px colored source dot, title 12px `t1`, body 11px `t3`, relative time 9.5px mono `t4`. Sources: builds (accent), agent-needs-input (gold), deploys (green), ports (gray).

### 9. Settings modal
640×440px centered on `scrim`, `panel2` bg, radius 16px. Left nav 168px: General / Terminal / Keyboard / Notifications / Quick launch (active = accent tint pill). Right: section title 15px/600 + description 12px `t3`; rows separated by `rgba(ov,0.05)` hairlines — label 12.5px + sub 11px, control right-aligned. Controls: toggle 32×18px (on = accent track, knob = `bgc`), select chip, kbd chip.

### 10. Drag-and-drop pane splitting (see `Drag Split Panes.dc.html`)
The core layout model is a **column list**: `Vec<Column>`, each `Column { flex, panes: Vec<Pane> }`, each `Pane { tabs: Vec<Tab>, active }`. (The existing Electron app uses a binary split tree; the column model shown here is what the design expresses — either satisfies the visuals as long as drop semantics below hold.)

**Interaction sequence:**
1. Pointer-down on a tab, drag threshold **6px** → drag starts. Source tab dims to 35% opacity. Cursor: grab.
2. **Ghost chip** follows cursor at +10px/+8px offset: glyph + label, `rgba(20,27,31,0.95)` bg, accent border `rgba(67,217,201,0.45)`, shadow + `0 0 16px rgba(67,217,201,0.15)`, radius 8px.
3. Hovering a pane computes a **drop zone** from relative position: `x < 25%` → left; `x > 75%` → right; `y < 30%` → top; `y > 70%` → bottom; else **center**.
4. **Zone overlay** on the hovered pane (100ms fade-in, 4px margin, radius 10px): fills the half of the pane the split would occupy (or inset 8% for center). `rgba(67,217,201,0.13)` fill, 1.5px dashed `rgba(67,217,201,0.55)` border, centered label chip ("◧ Split left", "⬒ Split up", "⊕ Move tab here"…) — 12px/600 accent text on `rgba(10,14,17,0.85)`.
5. **Drop semantics:**
   - center → append tab to target pane, make it active.
   - top/bottom → insert new pane above/below target within its column.
   - left/right → insert new single-pane **column** beside the target's column.
   - Always: remove tab from source; a pane left with 0 tabs is removed; a column left with 0 panes is removed. Focus moves to the destination pane.
   - No-op: dropping a pane's only tab back onto that same pane.
6. Status bar reports the action (`split right — new column for "build"`) and live counts.

---

## Interactions & Behavior
- **Keyboard**: Ctrl+K / Ctrl+Shift+P palette · Esc closes any overlay · (designed but not prototyped: Ctrl+Alt+arrows split, Ctrl+Tab cycle sessions, Ctrl+1..9 jump, F2 rename, Ctrl+Shift+W close pane, Ctrl+, settings).
- **Transitions**: hover/background 120ms; pane focus ring 150ms; palette/launch fade+rise 140–150ms; notifications slide 160ms; drop-zone fade 100ms.
- Overlays close on backdrop click; one overlay at a time.
- Session click → activates session (loads its pane layout); grouping switch is instant.

## State Management (suggested Iced mapping)
- `AppState { sessions, grouping: Grouping, active_session, layout: Vec<Column>, focused_pane, overlay: Option<Overlay>, palette_query, drag: Option<DragState>, drop_target: Option<(PaneId, Zone)>, ui_theme, accent, vibrancy, show_status_bar }`
- `DragState { tab, src_pane, origin: Point, pos: Point, started: bool }`
- Messages: `SelectSession`, `SetGrouping`, `FocusPane`, `SelectTab`, `TabDragStart/Move/End`, `OpenOverlay`, `CloseOverlay`, `PaletteInput`, `SetSettingsSection`, toggle messages for each setting.
- Session status (`Running`/`Busy`/`Idle`) is fed by the PTY layer + agent observer (mirrors the Electron app's `shellState` + Claude activity TTL logic in `WorkspaceRow.tsx`).

## Assets
- `assets/pandamux_logo.png` — app logo (from repo `docs/assets/pandamux_logo.png`).
- All icons are simple 1.2–1.4px-stroke line glyphs (terminal, search, plus, bell, sliders, folder, chip, globe, split-right, split-down, git graph) — recreate as SVG/iced canvas; no icon font.
- Fonts: JetBrains Mono (bundle it), Segoe UI (system).

## Files
- `PandaMUX Everywhere.dc.html` — main workspace + all overlays; tweakable props: `uiTheme` (dark/light), `accent`, `vibrancy` (0–1 → blur 6–24px), `showStatusBar`.
- `Drag Split Panes.dc.html` — working drag-and-drop splitting prototype (column/pane/tab model, ghost, zones, re-splitting).
- `assets/pandamux_logo.png`.

Open the HTML files in a browser to interact with everything; read their embedded logic (`class Component`) for exact style strings and the drop algorithm.
