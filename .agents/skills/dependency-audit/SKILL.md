---
name: dependency-audit
model: sonnet
effort: medium
description: Audit project dependencies for outdated versions, known vulnerabilities, and unused packages. Use periodically or before releases.
user-invocable: true
argument-hint: (no arguments needed)
allowed-tools:
  - Read
  - Glob
  - Grep
  - Bash
  - WebFetch
---

# Dependency Audit

You have been asked to audit this project's dependencies. Follow these steps.

## Step 1: Identify Package Managers

Search for dependency manifests in the repo root and subdirectories:

- `package.json` (npm/yarn/pnpm)
- `requirements.txt`, `Pipfile`, `pyproject.toml` (Python)
- `go.mod` (Go)
- `Cargo.toml` (Rust)
- `Gemfile` (Ruby)
- `pom.xml`, `build.gradle` (Java/Kotlin)
- `composer.json` (PHP)

Read each manifest found.

## Step 2: Check for Vulnerabilities

For each package manager detected, run the appropriate audit command:

- **npm:** `npm audit --json` (if package-lock.json exists)
- **yarn:** `yarn audit --json` (if yarn.lock exists)
- **pip:** `pip audit` (if installed)
- **cargo:** `cargo audit` (if installed)
- **go:** `govulncheck ./...` (if installed)

If the audit tool is not installed, note it and continue. Do not install tools without asking.

## Step 3: Check for Outdated Packages

For each package manager:

- **npm:** `npm outdated --json`
- **pip:** `pip list --outdated --format=json`
- **cargo:** `cargo outdated` (if installed)
- **go:** Check go.mod versions against latest

Categorize updates as:
- **Patch:** Bug fixes, safe to update
- **Minor:** New features, backward-compatible
- **Major:** Breaking changes, requires migration

## Step 4: Detect Unused Dependencies

1. Read the dependency manifest.
2. For each dependency listed, use Grep to search the codebase for imports or references.
3. If a dependency has zero references, flag it as potentially unused.
4. Note: some dependencies are used indirectly (plugins, presets, peer deps). Check for these cases before flagging.

## Step 5: Produce Report

```
# Dependency Audit Report

## Summary
- Manifests found: <list>
- Vulnerabilities: <count by severity>
- Outdated packages: <count by type>
- Potentially unused: <count>

## Vulnerabilities
[CRITICAL/HIGH/MEDIUM/LOW] <package>@<version>
  CVE: <id if available>
  Description: <what the vulnerability is>
  Fix: Update to <safe version>

## Outdated Packages
[MAJOR] <package> <current> -> <latest> (breaking changes likely)
[MINOR] <package> <current> -> <latest>
[PATCH] <package> <current> -> <latest>

## Potentially Unused
- <package> -- no imports found in codebase

## Recommendations
1. <prioritized list of actions>
```
