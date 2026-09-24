---
description: Enforce BP best practices check before starting new work
paths:
  - "CLAUDE.md"
  - "AGENTS.md"
  - ".claude/**"
  - ".agents/**"
  - ".github/**"
  - "Cargo.toml"
  - "Cargo.lock"
  - "crates/*/Cargo.toml"
  - "docs/_toc.yaml"
  - "netlify.toml"
  - "resources/pandamux-orchestrator/**"
  - "winget/**"
---

# RULE 3 Enforcement: Check BP Before Configuration Work

Before creating or modifying infrastructure, tooling, or configuration files matching the paths above, you MUST consult the BP knowledge base to follow proven patterns.

## Required Steps

1. **Fetch the master index:**
   ```
   WebFetch https://raw.githubusercontent.com/BoardPandas/BP/main/llms.txt
   ```

2. **Identify relevant concerns** from the file you're about to write (for example Claude configuration, documentation, GitHub Actions, Cargo workspace structure, versioning, or deployment).

3. **Fetch each relevant concern index:**
   ```
   WebFetch https://raw.githubusercontent.com/BoardPandas/BP/main/practices/<concern>/llms.txt
   ```

4. **Read ALL FOUNDATIONAL entries** for matched concerns.

5. **Read RECOMMENDED entries** whose tech tags match this project's stack.

## When to check

- Setting up or changing repository tooling
- Configuring CI/CD pipelines
- Structuring `.claude/` or `.agents/` configuration
- Changing the Cargo workspace or crate manifests
- Changing release, Netlify, or winget configuration
- Adding versioning or changelog automation

## Do NOT skip this check

- If you already checked BP earlier in this conversation for the same concern, you do not need to re-fetch.
- If no entries are relevant, proceed -- but you must have looked first.
