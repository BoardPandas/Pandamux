# Phase 2 Native Terminal Spike

This is a disposable de-risk spike for `tasks/plan-repo.md` Phase 2. It lives outside `crates/` on purpose: the production Rust workspace does not begin until Phase 3.

## Goals

- Prove a native Iced window can host the terminal UI shell.
- Prove `portable-pty` can spawn the local Windows shell and capture output.
- Prove `alacritty_terminal` can parse PTY output into a headless grid.
- Prove the selected text, GPU, and SSH crates resolve together under exact pins.
- Keep dependency pins exact so upgrade tax can be measured deliberately.

## Commands

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
cargo run
```

## Current Pins

```text
iced = 0.14.0
alacritty_terminal = 0.26.0
portable-pty = 0.9.0
glyphon = 0.11.0
cosmic-text = 0.19.0
swash = 0.2.9
wgpu = 30.0.0
wgpu_glyphon = 29.0.4
russh = 0.62.2
```

## Acceptance Notes

The first milestone is complete: a working Iced terminal viewport shell, PTY smoke test, headless grid parser, throughput smoke, dependency compatibility proof, native `russh` PTY reattach smoke against Galahad, direct key-file auth, Windows OpenSSH agent auth, 1Password-compatible agent auth, password auth fallback, and a headless glyphon/wgpu render plus perf and visual Unicode QA smoke. The full production widget still belongs in Phase 3, but the Phase 2 widget, renderer, visual QA, remote, auth, signing, and dependency gates are now exercised.
