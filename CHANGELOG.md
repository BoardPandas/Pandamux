# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Full repo review and native Rust rewrite master plan (`tasks/plan-repo.md`): approved direction to rebuild wmux as a fully native Rust app (Iced + alacritty_terminal + portable-pty + russh), drop the browser pane, add SSH copy/paste, remote Claude Code, and image-paste-over-SSH features, migrate the interim Electron app from npm to pnpm, and package with Velopack + Azure Artifact Signing.
- Claude Code developer tooling under `.claude/` (agents, skills, rules, hooks, references, scripts).
- `.gitattributes` enforcing LF line endings on shell scripts so shebangs work on Git Bash/macOS/Linux.

### Changed

- Expanded `.gitignore` with language, IDE, OS, and secret-file patterns plus Claude Code local files.

## [0.15.1]

- Baseline prior to changelog tracking. See git history for earlier changes.
