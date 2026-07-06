# Phase 2 Native Terminal Spike Report

Date: 2026-07-06

## What Landed

- Disposable Rust spike at `spikes/phase2-native-terminal`, intentionally outside the future production `crates/` workspace.
- Native Iced 0.14 shell with a reusable canvas-backed `TerminalViewport` that projects an `alacritty_terminal` grid snapshot into a terminal-like surface.
- Headless `alacritty_terminal` grid harness that parses ANSI bytes, color escapes, CJK, and emoji.
- `portable-pty` Windows smoke harness that spawns PowerShell, captures output, answers CPR (`ESC[6n`) with `ESC[1;1R`, and avoids the initial EOF hang.
- Burst smoke that streams 2,000 PowerShell lines through the PTY and then through the Alacritty grid.
- Text stack smoke that shapes ASCII, box drawing, CJK, emoji, and RTL through `cosmic-text`, while compiling references to `glyphon`, `swash`, `wgpu`, and `russh`.
- Native russh auth smoke paths for direct key-file auth, Windows OpenSSH-compatible named-pipe agent auth, optional 1Password-compatible agent auth, and password auth.
- Headless glyphon/wgpu renderer smoke that renders Unicode into an offscreen texture, reads pixels back, and measures static plus input-line frame timings.
- Visual QA smoke that writes `phase2-visual-qa.bmp` from the same glyphon/wgpu readback path and checks row-level nonblack pixels for box drawing, CJK/wide glyphs, emoji, combining marks, powerline, RTL, and ligature text.
- Maintenance-tax check: `cargo info` reports the pinned `iced = 0.14.0` and `alacritty_terminal = 0.26.0` versions as the current crates.io releases, so there is no newer minor bump available to test on 2026-07-06.

## Exact Pins Tested

```text
alacritty_terminal = 0.26.0
anyhow = 1.0.100
cosmic-text = 0.19.0
glyphon = 0.11.0
iced = 0.14.0
portable-pty = 0.9.0
russh = 0.62.2, default-features = false, features = ["ring"]
swash = 0.2.9
tokio = 1.52.3
wgpu = 30.0.0
wgpu_glyphon = 29.0.4, used by glyphon 0.11 render smoke
```

## Verification

```powershell
cargo check
cargo fmt --check
cargo run -- --grid-smoke
cargo run -- --iced-widget-smoke
cargo run -- --pty-smoke
cargo run -- --burst-smoke 2000
cargo run -- --text-stack-smoke
cargo run -- --russh-galahad-smoke
cargo run -- --russh-galahad-agent-smoke
cargo run -- --russh-galahad-1password-smoke
cargo run -- --russh-galahad-password-smoke
cargo run -- --gpu-render-smoke
cargo run -- --visual-qa-smoke phase2-visual-qa.bmp
```

Results on this machine:

- `cargo check`: passed.
- `cargo fmt --check`: passed after formatting.
- `--grid-smoke`: passed, output included `alpha`, `red`, `wide:猫`, and `emoji:🚀`.
- `--iced-widget-smoke`: passed, `PANDAMUX_ICED_WIDGET_SMOKE_OK`, `lines=23`, `columns=80`, `rows=24`.
- `--pty-smoke`: passed, output included `PANDAMUX_PHASE2_PTY_OK`.
- `--burst-smoke 2000`: passed, `pty_ms=498`, `total_ms=503`, `bytes=40965`, final marker reached the grid.
- `--text-stack-smoke`: passed, `lines=1`, `glyphs=43`, `format=Bgra8UnormSrgb`, and resolved `glyphon::TextRenderer`, `glyphon::TextAtlas`, `swash::FontRef`, and `russh::client::Config`.
- `--russh-galahad-smoke`: passed, `setup_bytes=124`, `reattach_bytes=125`, `claude_bytes=718`.
- `--russh-galahad-agent-smoke`: passed after enabling the Windows OpenSSH `ssh-agent` service and loading the Galahad identity, `setup_bytes=124`, `reattach_bytes=125`, `claude_bytes=718`.
- `--russh-galahad-1password-smoke`: passed after enabling the 1Password-compatible SSH-agent provider, `setup_bytes=124`, `reattach_bytes=125`, `claude_bytes=718`.
- `--russh-galahad-password-smoke`: passed using the environment-provided Galahad password, `setup_bytes=124`, `reattach_bytes=125`, `claude_bytes=718`.
- `--gpu-render-smoke`: passed with `adapter=NVIDIA GeForce RTX 5090`, `backend=Vulkan`, `layout_lines=9`, `glyphs=256`, `nonblack_pixels=12526`, `first_frame_ms=14`, `avg_frame_ms=0.452`, `avg_dynamic_frame_ms=1.287`, `max_frame_ms=14`, `fps_estimate=2212.6`. Visual row checks also passed: box `1716`, wide `971`, emoji `820`, combining `938`, powerline `807`, RTL `423`, ligature `1129`.
- `--visual-qa-smoke phase2-visual-qa.bmp`: passed, wrote `spikes/phase2-native-terminal/phase2-visual-qa.bmp`; manual inspection showed readable ASCII, CJK/wide glyphs, emoji, combining marks, powerline, RTL, ligature text, and the live input row.

Recheck after adding the native russh smoke:

- `cargo check`: passed.
- `cargo fmt --check`: passed.
- `--grid-smoke`: passed.
- `--text-stack-smoke`: passed.
- `--pty-smoke`: passed.
- `--burst-smoke 2000`: passed, `pty_ms=473`, `total_ms=478`, `bytes=40965`.
- `--gpu-render-smoke`: passed after switching the dynamic typing simulation from whole-buffer reshaping to a separate input-line buffer.
- Auth matrix recheck:
  - Direct key-file auth passed.
  - Windows OpenSSH agent mode passed after the service was enabled and an ED25519 certificate identity was loaded.
  - 1Password-compatible agent mode passed through the same PTY, tmux reattach, Claude Code launch, and cleanup path.
  - Password auth fallback passed.
- Iced viewport and visual QA recheck:
  - `--iced-widget-smoke`: passed, proving the canvas-backed viewport constructs against the pinned Iced stack.
  - `--visual-qa-smoke`: passed, wrote the BMP artifact and passed every row-level glyph family check.

## Remote Validation

Galahad was validated as the Phase 2 Linux host on 2026-07-06:

- `ssh galahad` connected successfully to Fedora Linux as `chaz`.
- Claude Code is installed and reports `2.1.200 (Claude Code)`.
- `tmux` was not present initially, then installed with `dnf`.
- A `pandamux-phase2-smoke` tmux session was created, detached by ending SSH, captured after reconnect, and confirmed alive with `PANDAMUX_REMOTE_TMUX_REATTACHED`.
- Claude Code was launched inside a bounded tmux smoke session and reached the interactive workspace trust prompt for `/home/chaz`.
- Smoke sessions were cleaned up after validation.

This initial pass validated the external host, tmux durability pattern, and Claude Code launch path through the system OpenSSH client. Native russh validation is recorded below; Windows OpenSSH agent-pipe authentication remains separate.

Native russh validation was added after the initial host check:

- `--russh-galahad-smoke` used the configured Galahad host details: `10.55.88.48:22`, user `chaz`, and the local `~/.ssh/galahad` private key.
- The smoke connected with `russh`, authenticated with direct public-key auth, requested a PTY, created `pandamux-russh-smoke` in tmux, disconnected, reconnected with a fresh russh session, captured the same tmux session, and cleaned it up.
- The same smoke launched Claude Code inside `pandamux-russh-claude`, captured the workspace trust prompt, and cleaned up that session.
- No temporary authorized-key entry was added to Galahad.
- `--russh-galahad-agent-smoke` adds the Windows OpenSSH service auth path: russh connects to `\\.\pipe\openssh-ssh-agent` via `AgentClient<NamedPipeClient>`, requests identities, and tries `authenticate_publickey_with` or `authenticate_certificate_with` for each identity. Live validation passed with `ssh-agent` running as `Automatic` and a loaded `chaz-windows` ED25519 certificate identity.
- `--russh-galahad-1password-smoke` exercises the same OpenSSH-compatible named-pipe contract with a 1Password provider label. This is optional provider support, not a product requirement that every user have 1Password installed. Live validation passed through the same PTY, tmux reattach, Claude Code launch, and cleanup path.
- `--russh-galahad-password-smoke` verifies password fallback through the same PTY, tmux reattach, Claude Code launch, and cleanup path.
- A final system OpenSSH cleanup check showed no lingering tmux sessions after the auth smokes.

## Renderer Validation

- The Iced app shell now uses `TerminalViewport`, a canvas-backed terminal surface module, instead of plain scrollable text. This proves the app can host a reusable native terminal viewport in Iced. The full production widget should still be built in Phase 3 inside the real workspace crates.
- `--gpu-render-smoke` creates a real offscreen wgpu texture, shapes terminal-like Unicode text with glyphon/cosmic-text, renders through `glyphon::TextRenderer`, reads pixels back from the GPU, and asserts a nonblack pixel count.
- Unicode sample includes ASCII, box drawing, CJK, emoji, combining marks, powerline glyphs, RTL Arabic, and ligature text.
- Static renderer loop: 120 frames, `avg_frame_ms=0.452` on the dev machine.
- Dynamic input loop: 60 frames, updating only a separate input-line buffer, `avg_dynamic_frame_ms=1.287`.
- Visual QA artifact: `phase2-visual-qa.bmp`, manually inspected in Codex. The artifact is not a substitute for production app screenshots in Phase 3, but it closes the Phase 2 rendering-risk gate with a durable inspectable image.
- The initial whole-buffer dynamic loop averaged about 27.9ms per frame. Do not reshape scrollback for every keystroke in production; keep input and changed lines isolated.

## Gotchas

- Initial PTY capture blocked waiting for EOF. The smoke harness now stops when an expected marker arrives and then waits for process exit.
- PowerShell emitted a cursor position request (`ESC[6n`) before the marker. The harness must answer with `ESC[1;1R`; carry this forward alongside the existing DA1 handling when porting PTY lifecycle semantics.
- The current exact stack compiles but resolves three `cosmic-text` versions:
  - `0.15.0` via Iced and cryoglyph.
  - `0.18.2` via glyphon 0.11.
  - `0.19.0` as the direct plan pin.
- `russh` was tested as a compile dependency with `ring` to avoid the heavier default `aws-lc-rs` path in the spike. Galahad now covers native `russh` request_pty with direct key auth, Windows OpenSSH agent auth, 1Password-compatible agent auth, and password auth.
- Galahad's SSH server did not send an exit-status message for the PTY exec before closing the channel. The smoke treats marker-plus-clean-close as success. Keep explicit markers in production remote-session tests.
- `glyphon 0.11.0` depends on `wgpu 29.0.4`, while the plan's direct GPU probe uses `wgpu 30.0.0`. The render smoke uses a `wgpu_glyphon` alias pinned to 29.0.4. Phase 3 should either align on glyphon's wgpu version or wait for a glyphon release that moves to wgpu 30.
- 1Password should be documented as an optional OpenSSH-compatible agent provider. Users without 1Password still work via direct key-file auth, Windows OpenSSH agent auth, or password auth.
- Enabling Iced's `canvas` feature pulled in `lyon` tessellation crates. That is expected for canvas-backed drawing; keep it isolated to `pandamux-ui` in Phase 3.
- The planned one-minor maintenance-tax bump could not be performed on 2026-07-06 because crates.io reports `iced 0.14.0` and `alacritty_terminal 0.26.0` as current. Carry the bump check to the first Phase 3 dependency refresh.

## Phase 3 Carry-Forward

- Build the production widget in the real Rust workspace crates, using this spike's `TerminalViewport` and glyphon/wgpu smoke as references.
- Add production app screenshots or golden-image tests once the Phase 3 widget can render in-window.

## Decision

Continue with the Rust direction. Phase 2 is complete enough to start Phase 3 scaffolding. The remaining work is productionization, not de-risking: move the proven pieces into workspace crates, preserve the isolation boundary, and keep visual QA in CI or release validation where feasible.
