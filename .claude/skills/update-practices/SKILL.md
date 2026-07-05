---
name: update-practices
model: opus
effort: high
description: Fetch latest Claude Code best practices and update the .claude/ folder configuration. Safe to run repeatedly.
user-invocable: true
argument-hint: (no arguments needed)
allowed-tools:
  - Read
  - Write
  - Edit
  - Glob
  - Grep
  - WebFetch
  - WebSearch
  - Agent
---

# Update Best Practices

You have been asked to update this repository's Claude Code configuration to the latest best practices. Follow these steps exactly.

## Important: Date Awareness

Check the current date FIRST. All best practices must be verified as current as of today's date. Do not rely on cached knowledge -- use WebSearch to confirm that recommended versions, tools, and patterns are still current.

## Step 1: Read Current Configuration

1. Read `.claude/references/source-urls.md` to get the list of URLs to fetch.
2. Read `CLAUDE.md` in the repo root. Note its contents and version references.
3. Read `agents.md` in the repo root. Note registered agents.
4. Scan `.claude/skills/` using Glob. List all existing skills.
5. Scan `.claude/agents/` using Glob. List all existing agents.
6. Read `.claude/settings.json`. Note current settings.
7. Read `.claude/references/tools.md` (if it exists). Note current tools.
8. Read `.claude/references/design-guardrails.md` (if it exists). Note current guardrails.
9. Scan `.claude/rules/` using Glob. List all existing path-scoped rules.
10. Scan `.claude/agent-memory/` using Glob. List all existing memory files.

## Step 2: Fetch Latest Practices

Spin up parallel `explorer` agents to fetch and analyze sources. Use the custom `explorer` agent (defined in `.claude/agents/`), never the built-in `Explore` type -- the built-in loads every MCP tool schema and blows the context window.

1. **Official sources subagent:** "Fetch all official Anthropic sources from the source URL registry. Extract: current Claude Code version, new features, deprecated features, new recommended settings/skills/agents/hooks, folder structure changes, new frontmatter fields for agents and skills, new hook events, new settings options. WHY: We need to know what changed officially to update the config accurately."

2. **Community sources subagent:** "Fetch all community sources from the source URL registry. Extract: new practical patterns, updated skill examples, agent configurations, workflow improvements, path-scoped rule patterns, agent memory patterns, HTTP hook patterns. WHY: Community sources capture battle-tested patterns ahead of official docs."

3. **Stack freshness subagent:** "Check the project's detected stack (from CLAUDE.md or dependency manifests) against current versions and best practices as of today's date. WHY: We need to ensure tools.md and design guardrails reflect the latest stable versions."

Wait for all subagents, then proceed.

## Step 2b: Sync from Bootstrap Template

The `.claude/` folder in this repo was originally scaffolded from the **claude-code-bootstrap** template. This step checks the template for any new or updated files that should be pulled in.

### Fetch the template file tree

```
WebFetch https://api.github.com/repos/BoardPandas/claude-code-bootstrap/git/trees/main?recursive=1
```

Parse the JSON response. Extract all paths under `.claude/` — these are the canonical template files. Ignore files outside `.claude/` (CLAUDE.md, README.md, package.json, etc.) since those are project-specific.

### Categorize each template file

For each `.claude/` path in the template tree:

1. **Check if it exists locally** using Glob or Read.
2. If it does NOT exist locally → mark as **TEMPLATE-NEW**.
3. If it exists locally → fetch the template version:
   ```
   WebFetch https://raw.githubusercontent.com/BoardPandas/claude-code-bootstrap/main/<path>
   ```
   **Read the FULL local file and the FULL template file and compare the actual text content**, not just file existence, size, or frontmatter. Do a real line-by-line comparison of the body:
   - If the text is byte-for-byte identical → mark as **TEMPLATE-CURRENT** (no action needed).
   - If the text differs in a few lines/sections (incremental edits) → mark as **TEMPLATE-UPDATED**.
   - If the body has been substantially rewritten (most steps/sections changed, reordered, or replaced — e.g., the `add-lesson` skill was rewritten upstream) → mark as **TEMPLATE-REWRITTEN**. A rewrite is when a merge would produce a Frankenstein file; the template's version is the new canonical source.

   Do not rely on quick heuristics like "the file exists, so it's current." A skill that exists locally can still be stale if its instructions were rewritten in the template. Always diff the prose.

### Files to sync (whitelist)

Only sync these categories of `.claude/` files from the template:

| Category | Path pattern | Sync strategy |
|----------|-------------|---------------|
| Skills | `.claude/skills/*/SKILL.md` | Sync new skills entirely. For **TEMPLATE-UPDATED** skills, merge new steps/sections but preserve project-specific customizations (e.g., custom matchers, stack-specific checks). For **TEMPLATE-REWRITTEN** skills, **replace the body wholesale with the template version**, then re-apply only the genuinely project-specific additions (custom stack checks, custom paths). Do not try to merge a rewrite line-by-line. |
| Agents | `.claude/agents/*.md` | Sync new agents entirely. For **TEMPLATE-UPDATED** agents, update frontmatter fields and instructions but preserve project-specific `context` or `skills` overrides. For **TEMPLATE-REWRITTEN** agents, replace the body with the template version, preserving only project-specific `context`/`skills`/`tools` overrides. |
| Rules | `.claude/rules/*.md` | Sync new rules entirely. For **TEMPLATE-UPDATED** rules, update content but preserve custom `paths:` frontmatter if the project has different file structure. For **TEMPLATE-REWRITTEN** rules, replace the body with the template version, preserving only the project's custom `paths:` frontmatter. |
| Scripts | `.claude/scripts/*.sh` | Sync new scripts entirely. For existing scripts, replace with template version unless local version has project-specific logic (check for project-specific paths, env vars, or tool references). |
| References | `.claude/references/source-urls.md` | Merge: add any new URLs from template that aren't already present. Never remove existing URLs. |
| References | `.claude/references/hooks-and-settings.md` | Generic catalog, not project-specific. Replace with the template version unless the local copy has project-specific notes appended, in which case merge new rows in. |
| References | `.claude/references/infrastructure.md` | Do NOT sync — infrastructure is project-specific. |
| References | `.claude/references/tools.md` | Do NOT sync — tools depend on project stack. |
| References | `.claude/references/design-guardrails.md` | Do NOT sync — guardrails depend on project stack. |
| Settings | `.claude/settings.json` | Deep-merge: add new hooks, permissions, and env vars from template. Never remove existing entries. Preserve project-specific matchers and custom hooks. |
| Settings | `.claude/settings.local.json.example` | Replace with template version (it's just an example file). |

### Files to NEVER sync

- `.claude/agent-memory/*` — project-specific memory, never overwrite.
- Any file not in the `.claude/` directory.
- `CLAUDE.md`, `agents.md`, `instructions.md` — these are project-tailored.

### Apply template changes

For each **TEMPLATE-NEW** file:
- Create it locally with the template content.

For each **TEMPLATE-UPDATED** file:
- Apply the sync strategy from the table above.
- When merging, use the non-destructive rules: never remove project-specific content, append/merge rather than replace.

For each **TEMPLATE-REWRITTEN** file:
- The upstream body is the new canonical source. Replace the local body with the template version per the sync strategy above.
- Before replacing, scan the local file for genuinely project-specific content (custom stack checks, custom paths, custom `paths:`/`context`/`skills` frontmatter). Re-apply ONLY those after pulling in the rewrite.
- Do not attempt a line-by-line merge of a rewrite — that produces an incoherent hybrid. Take the template body whole, then graft back the small project-specific pieces.

### Track template version

After syncing, note the latest commit SHA from the template repo (available from the tree API response). Log it in the final report so future runs can detect if the template has changed.

## Step 3: Compare and Identify Changes

Categorize findings as:

- **NEW:** Features or patterns not yet reflected in the current config.
- **UPDATED:** Patterns that exist but need modification to match current best practices.
- **DEPRECATED:** Patterns in use that are no longer recommended.
- **CURRENT:** Patterns that already match best practices (no action needed).

Check each of these areas explicitly:

### Core files
- Skills (all template skills present and current)
- Agents (all template agents present and current)
- Settings (permissions, hooks, env)
- Tools reference
- Design guardrails

### Path-scoped rules (.claude/rules/*.md)
- Are existing rules still valid for the current stack?
- Are there new path patterns that should have rules (e.g., new source directories added)?
- Do rule frontmatter `paths:` patterns still match the actual file structure?
- Are there new best-practice rule templates from official/community sources?

### Agent memory (.claude/agent-memory/)
- Does the directory exist? If not, it should be created.
- Are the standard files present (README.md, patterns.md, decisions.md, debugging.md)?
- Is the README still accurate about conventions?
- Have any memory entries become stale or contradicted by current code?
- Does `debugging.md` have the standard gotchas structure? If empty or unstructured, initialize with the template from init-repo.

### Agent frontmatter
Review each agent for new frontmatter fields:
- `background` — Should any agents run in the background?
- `isolation` — Should security agent use isolation?
- `context` — Should any agents have injected context?
- `skills` — Should any agents be bound to specific skills?
- `memory` — Should any agents read agent-memory files on startup?

### Skill frontmatter
Review each skill for new frontmatter fields:
- `context: fork` — Should any skills run in isolated context?
- `agent` — Should any skills be bound to a specific agent?
- Are `model` assignments still optimal?

### Hook events
- Are all recommended hooks configured?
- Are there new hook events available that should be adopted?
- Are any configured hooks using deprecated event names or syntax?
- Should HTTP hooks be added for team workflows?
- Are matchers using the correct syntax?

The full event catalog, hook types, and matcher syntax live in `.claude/references/hooks-and-settings.md` — check configured hooks against it. When the fetched sources reveal a hook event or type not yet in that reference, add it there (not into a skill body), so the catalog stays in one place.

### Settings
Check configured settings against the optional-settings catalog in `.claude/references/hooks-and-settings.md` (`attribution.*`, `autoUpdatesChannel`, `sandbox.*`, `worktree.*`, `language`, `allowedHttpHookUrls`, `alwaysThinkingEnabled`, `disableAllHooks`). If the latest Claude Code version introduces a new setting, add it to that reference too.

### Cost / token efficiency

Audit the config for token-efficiency patterns. Do NOT recommend disabling the 1M context window (the 200K default is intentionally not used in this template).

- **Per-prompt effort:** Are skills setting `effort:` in frontmatter where appropriate? Mechanical, step-by-step skills should use `low` or `medium`; analysis skills `medium`; orchestration/planning `high` or `max`. Flag skills missing `effort` where a non-default would save tokens.
- **Model routing:** Are skills and agents assigned the cheapest model that fits? Step-by-step → haiku, analysis → sonnet, orchestration/planning → opus. Flag any opus assignment that could be sonnet/haiku.
- **Cache preservation:** Skills and agents should not switch model mid-session unnecessarily (model switches invalidate the prompt cache). Flag skills that change model partway through a multi-step flow.
- **Input format swaps:** Where the project ingests PDFs, web pages, or screenshots, prefer cheaper extractors (`pdftotext` for PDFs, an agent-browser / DOM read over screenshot capture, code knowledge graphs over raw repo dumps). Note any tool reference in `tools.md` that should be added.
- **Subagent delegation:** Long mechanical work (log scans, repo-wide searches, doc fetches) should be delegated to subagents with cheaper models so the main session's cache stays intact.

## Step 4: Implement Changes

For each NEW or UPDATED item:

1. Determine which file(s) need to change.
2. Make the change. Follow the non-destructive merge rules:
   - Never remove custom project-specific content.
   - Append new sections rather than replacing existing ones.
   - For JSON files, deep-merge -- preserve existing keys.
   - For agent-memory files, never overwrite -- only add missing files.
   - For rules, preserve existing rules -- only add new ones or update paths.
3. For DEPRECATED items: update the pattern to the recommended alternative.

Ensure these skills still exist and are current:
- plan-repo, init-repo, update-practices
- spec-developer, security-scan
- performance-review, dependency-audit, test-scaffold
- doc-sync, mermaid-diagram

Ensure these agents still exist and are current:
- architect, reviewer, security, performance, explorer

Update `.claude/references/tools.md` if any tools have new versions or new tools should be added.

Update `.claude/references/design-guardrails.md` if UI best practices have changed.

Review skill frontmatter and update `model`, `disable-model-invocation`, `context`, and `agent` fields if recommendations have changed.

Review agent frontmatter and update `background`, `isolation`, `context`, `skills`, and `memory` fields if recommendations have changed.

Review hook configuration:
- Verify all hook events are still valid.
- Add new recommended hooks.
- Update matchers if file paths have changed.
- Add HTTP hooks if `allowedHttpHookUrls` is configured and team webhooks are in use.

Review settings:
- Check `attribution`, `autoUpdatesChannel`, `sandbox`, `language`, `allowedHttpHookUrls`, `alwaysThinkingEnabled` against best practices.
- Ensure `settings.local.json.example` exists if it should.

## Step 5: Prune CLAUDE.md Files

Review all CLAUDE.md files in the hierarchy. Remove:
- Advice the model now handles natively (check against current model capabilities)
- Outdated version references
- Redundant rules that duplicate parent CLAUDE.md content

Keep each CLAUDE.md focused and under 200 lines.

## Step 6: Prune and Validate Rules

Review `.claude/rules/*.md` files:
- Remove rules that duplicate CLAUDE.md content (rules should be path-specific, not general).
- Verify `paths:` patterns still match actual files in the project.
- Update rules if stack conventions have changed.
- Remove rules for deleted source directories.

## Step 7: Update Documentation

1. Update `CLAUDE.md` if skill or agent inventory changed.
2. Update `agents.md` if agent inventory changed.
3. Update `instructions.md` if usage patterns, available features, or configuration options changed. Ensure it documents:
   - Path-scoped rules (`.claude/rules/*.md`)
   - Agent memory (`.claude/agent-memory/`)
   - All agent and skill frontmatter fields
   - All hook events and types (command, http, prompt, agent)
   - All settings options including settings.local.json overrides

## Step 8: Report

Print a diff-style summary:

```
BOOTSTRAP TEMPLATE SYNC:
  Template repo: BoardPandas/claude-code-bootstrap
  Template commit: <SHA from tree API>
  [TEMPLATE-NEW] Added <path> -- <description>
  [TEMPLATE-UPDATED] Updated <path> -- <what changed (incremental merge)>
  [TEMPLATE-REWRITTEN] Replaced body of <path> -- <upstream rewrite; project-specific bits re-applied: ...>
  [TEMPLATE-CURRENT] No changes needed: <list>
  [TEMPLATE-SKIPPED] Skipped <path> -- <reason, e.g., project-specific>

CHANGES APPLIED:
  [NEW] Added skill: <name> -- <reason>
  [NEW] Added rule: <path> -- <scope description>
  [NEW] Added agent-memory file: <path> -- <purpose>
  [NEW] Added hook: <event>/<matcher> -- <reason>
  [NEW] Added setting: <key> -- <value> -- <reason>
  [UPDATED] Modified .claude/settings.json -- <what changed>
  [UPDATED] Modified agent <name> frontmatter -- <fields added/changed>
  [UPDATED] Modified skill <name> frontmatter -- <fields added/changed>
  [UPDATED] Modified rule <path> -- <what changed>
  [DEPRECATED] Replaced <old pattern> with <new pattern> in <file>
  [CURRENT] No changes needed for: <list>
  [PRUNED] Removed from <file>: <what was removed and why>

FEATURES IN USE:
  - Path-scoped rules: <count> rules in .claude/rules/
  - Agent memory: <count> files in .claude/agent-memory/
  - Hook events: <list of configured events>
  - Hook types: <command|http|prompt|agent|mcp_tool>
  - Advanced agent frontmatter: <agents using background/isolation/context/skills/memory>
  - Advanced skill frontmatter: <skills using context/agent>
  - Settings: <list of configured optional settings>

FEATURES AVAILABLE BUT NOT CONFIGURED:
  <list any features that could be enabled but aren't, with instructions>

CLAUDE CODE VERSION: <version found>
CURRENT DATE: <today's date>
SOURCES CHECKED: <count> of <total> fetched successfully
```

## Idempotency

Running this skill twice in a row must produce no changes the second time. Every change must be conditional -- only apply if the current state differs from the target state.
