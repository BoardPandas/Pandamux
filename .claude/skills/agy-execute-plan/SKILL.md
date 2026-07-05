---
name: agy-execute-plan
effort: high
model: opus
description: Hand an existing Claude-written plan to the Antigravity CLI (agy) for autonomous end-to-end execution, then independently verify the result against the plan via tests and git diff, fix whatever AGY left incomplete or broke, and report an honest blocked/partial/complete status. Use this whenever the user wants Antigravity (AGY) to execute, build, implement, or "knock out" a plan, spec, or task file and then have the work checked, including casual phrasings like "have AGY run the plan", "kick off agy on tasks/x.md", "let Antigravity build this", "run the plan end-to-end with antigravity", "agy the plan then double-check it", or "have antigravity do this and verify it after"; and whenever a plan in the tasks/ folder is meant to be carried out by AGY rather than by Claude directly. Trigger even if the user does not say the word "skill". Do NOT use this when the user wants Claude itself to implement the plan, only wants a plan written/spec'd, only wants a diff reviewed or tests run with no AGY execution, is asking what agy is or how to install it, or wants agy for a one-off research question or a cron/scheduling setup rather than executing a plan file.
user-invocable: true
argument-hint: <path to plan file, or leave blank to pick from tasks/>
allowed-tools:
  - Read
  - Write
  - Edit
  - Glob
  - Grep
  - Bash
  - Agent
---

# AGY Execute Plan

You orchestrate a handoff: an existing plan (written earlier by Claude, usually in `tasks/`) is executed end-to-end by the Antigravity CLI (`agy`), and then you verify that the plan was actually carried out, repair what AGY got wrong, and report anything that blocked either AGY or you.

You are the supervisor, not the implementer-of-first-resort. AGY does the building. Your value is in catching what AGY missed or botched, because AGY runs unattended and cannot be trusted to grade its own work.

## Why this skill is shaped the way it is

Three facts about `agy` (verified against v1.0.8) drive every decision below:

1. **A clean exit means AGY ran, not that the plan is done.** Exit codes are reliable at the process level: a non-zero exit is a genuine failure (auth, crash, killed) and is a hard blocker. But exit 0 only means AGY ran and called the model. It does NOT mean the plan was fully or correctly implemented. AGY can finish "successfully" having left a phase half-done, skipped a phase, or broken an existing test, and its own log may even claim "all phases complete, tests pass" when that is false. So you judge completion from the filesystem: the git diff, the plan's acceptance criteria, and a real test/build run, never from the exit code or AGY's self-report.
2. **You do not get AGY's answer on stdout, and you do not need it.** In print mode (`-p`), AGY renders its response to the terminal only; when stdout is redirected or piped (any headless context) the captured stream comes back empty even though the run succeeded. This skill sidesteps that entirely: AGY's job is to change files in the repo, and you grade those file changes directly (git diff + tests), not its prose. Pass `--log-file` to capture the operational log (it records the run and the conversation id as `Print mode: conversation=<id>`); if you ever genuinely need AGY's written answer, it is stored in the latest SQLite DB under `~/.gemini/antigravity-cli/conversations/<id>.db`.
3. **AGY is interactive-first and will hang unattended.** Run non-interactively without an empty stdin, AGY waits forever on input/permission prompts (even `agy models` returns nothing until killed). So you always (a) give stdin an immediate EOF and (b) auto-approve permissions, making a human wait physically impossible.

When something genuinely blocks progress, halt and report immediately with full context. Do not paper over a hard blocker or push past it. A clear "here is exactly where it stopped and why" is more useful than a half-finished run dressed up as success.

## Step 0: Preflight (fail fast before spending an AGY run)

Do all of these before invoking AGY. Any failure here is a blocker: stop and report.

1. **Confirm AGY is available.** Check with `Get-Command agy` (PowerShell) or `command -v agy` (bash). Do NOT "test" it by running `agy models` or a bare prompt: those hang without an empty stdin (see fact 3). If it does not resolve, a freshly installed `agy` may just need PATH reloaded in this shell before you give up; on Windows PowerShell:
   ```powershell
   $env:Path = [System.Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [System.Environment]::GetEnvironmentVariable('Path','User')
   ```
   If it still does not resolve, halt and tell the user AGY is not installed or not on PATH. Do not silently fall back to implementing the plan yourself unless the user asks.
2. **Locate the plan.**
   - If the user passed a path, use it.
   - Otherwise list `tasks/*.md` and pick the obvious one. If more than one is plausible, ask the user which plan to run rather than guessing.
   - Read the plan in full.
3. **Extract the acceptance checklist.** Walk the plan and pull out every phase and every concrete, verifiable outcome (files to create, functions to wire, endpoints, migrations, tests to add). Write this list down in your working notes; it is what you grade against in Step 3. If the plan has a "Lessons Learned / Gotchas" section, note it so AGY's known pitfalls inform your review.
4. **Record the git baseline.** Confirm you are in a git repo. Capture `git rev-parse HEAD` and `git status --porcelain`. This baseline is how you isolate exactly what AGY changed. If the working tree is already dirty, record what was already modified so you do not later credit (or blame) AGY for it. Recommend the user start from a clean tree; proceed if they accept the noise.
5. **Detect the project commands** you will need in Step 3: the test command, the build/typecheck command, and the lint command. Read `package.json` scripts (or the equivalent for the stack) and the project's CLAUDE.md. If you cannot find them, note that and plan to verify by other means.

## Step 1: Hand the plan to AGY

Do not try to stuff the plan's full text into `-p`. There is no `--prompt-file` flag, and a long multi-line plan will break Windows argument-length and quoting. Instead, give AGY a short instruction that points it at the plan file and tells it to execute everything. AGY is an agent with file tools; it will read the plan itself.

**The handoff prompt** (adapt the path; keep it this direct):

> Read the plan at `<relative/path/to/plan.md>` and execute it end to end. Complete every phase and every task in order. Make all required code changes in this repository. Run the project's tests as you go and fix what you break. Do not stop to ask for confirmation; you are running unattended. Only stop early if you hit something you genuinely cannot resolve, and if so, write a short note to `AGY_BLOCKED.md` in the repo root explaining what blocked you and at which phase/task (your terminal output is not captured, so a file is the only way the message survives).

**The invocation.** The critical piece is the empty-stdin redirect, which is what actually prevents the hang. PowerShell has no `< NUL` redirect, so on Windows invoke through `cmd /c`:

```
cmd /c "agy --dangerously-skip-permissions --print-timeout 3600s --log-file ""tasks/.agy-run.log"" -p ""<handoff prompt>"" < NUL"
```

The POSIX equivalent (bash):

```
agy --dangerously-skip-permissions --print-timeout 3600s --log-file tasks/.agy-run.log -p "<handoff prompt>" < /dev/null
```

Notes:
- `< NUL` (Windows) / `< /dev/null` (POSIX) gives stdin an immediate EOF so AGY cannot hang waiting on a prompt. This is the single most important flag-equivalent; do not omit it.
- `--dangerously-skip-permissions` auto-approves AGY's tool requests. Required for unattended runs; surface to the user that AGY will modify files without prompting.
- Size `--print-timeout` to the plan (a few phases: `1800s`; a large plan: `3600s`+) and add a job-side kill as a backstop. Too short and AGY gets killed mid-task, which looks like a blocker.
- Do not bother capturing stdout: it is empty when redirected (fact 2). You judge by the repo, and `--log-file` records the run and the conversation id.
- There is no `--output json`, `--no-color`, `--yes`, `--prompt-file`, or `run` subcommand in this version; those error with "unknown flag." Do not design around blog posts that use them.
- This is long-running. Consider running it in the background so you are not blocked, then read the log and the repo when it finishes.

## Step 2: Triage the AGY run (the first blocker gate)

Before reviewing the work, decide whether there is any work to review. Note AGY's exit code, read `tasks/.agy-run.log`, and run `git status --porcelain` against the baseline.

- **Non-zero exit** (halt and report now): a genuine failure (auth/login failure, rate-limit/quota, crash, killed by timeout). The log states the cause; common case is `ANTIGRAVITY_API_KEY` not set (interactive OAuth does not work non-interactively). There is nothing to review. Report the log excerpt, the phase it reached, and what is needed to unblock. If the log alone does not explain the stop, the full assistant transcript is in the latest `~/.gemini/antigravity-cli/conversations/<id>.db` (the id is in the log as `Print mode: conversation=<id>`).
- **Exit 0 but no file changes** (halt and report): AGY ran but did nothing to the repo. Treat as a blocker and report it; do not silently implement the plan yourself.
- **Exit 0 with file changes**: there is work to grade. Do NOT assume it is correct or complete, even if the log says "all phases complete, tests pass" (it may be lying). Continue to Step 3, which is exactly where you find out.

Also check for an `AGY_BLOCKED.md` file in the repo root. If AGY wrote one, it stopped early on purpose: read it, treat it as a blocker, and fold it into the report (still run Step 3 on whatever partial work exists so you can describe how far it got).

## Step 3: Review what AGY actually did

Grade AGY's output against four lenses. This is heavy read-and-run work, so offload the independent parts to subagents in parallel and always tell each one WHY (per project convention). Reserve the fixing for the main session.

Spawn in parallel:
- A **reviewer** subagent on the diff: "Review `git diff <baseline>` for correctness bugs, half-finished work, leftover stubs/TODOs, accidental deletions, and deviations from the plan at `<path>`. WHY: AGY ran unattended and we cannot trust it to have completed or verified its own work."
- A **tester** subagent: "Run the project's test suite and build/typecheck (`<commands>`) and report pass/fail with failure output. WHY: we are verifying an autonomous agent's changes before accepting them."

Then do the two checks that need the plan in mind yourself:

1. **Plan acceptance criteria.** Walk the checklist from Step 0. For each item, verify the described artifact or behavior actually exists in the code (read/grep the files). Mark each: done, partial, or missing.
2. **Standards and lint.** Run the project linter/formatter check and confirm the changes follow the project's coding standards (CLAUDE.md, rules/). Note violations.

Collate everything into a single issue list: missing acceptance criteria, diff-review findings, failing tests/build, lint/standards violations.

## Step 4: Fix directly, then re-verify

Work the issue list. Fix the problems yourself: complete missing tasks, repair broken code, finish stubs, resolve failing tests. After each meaningful fix, re-run the relevant tests/build so you know the fix landed. Iterate until the checks are green.

Stop and report immediately (do not keep grinding) when you hit a hard blocker: an issue you cannot resolve, an ambiguity that needs the user's decision, a fix that would require a destructive or risky change, or a missing external dependency. Capture exactly where you stopped and why. Per the user's policy, blockers halt the process rather than getting silently skipped.

## Step 5: Final report

Always use this structure so the user can scan it fast:

```
# AGY Execution Report: <plan name>

## Run summary
- Plan: <path>
- AGY command: <the invocation used>
- Duration / timeout: <...>  | Exit: <code>  | Log: tasks/.agy-run.log
- Net status: Complete | Complete with fixes | Blocked

## Plan acceptance (phase by phase)
- Phase 1 / <task>: done | partial | missing  (<one-line note>)
- ...

## What AGY changed
<git diff --stat summary>

## Verification
- Tests: <pass/fail + key failures>
- Build/typecheck: <pass/fail>
- Lint/standards: <pass/fail + notable items>

## Issues I found and fixed
- <issue> -> <fix> -> <re-verified how>

## Blockers (if any)
- <what> at <phase/task>: <why blocked> -> <what is needed to proceed>
```

Be honest in "Net status." If anything is blocked or a test still fails, say so plainly; do not report success you did not verify.

## Lessons Learned / Gotchas

After the run, route any reusable discoveries to LL-G via `/add-lesson` (not a local file). Verified AGY (v1.0.8) facts and gotchas:
- AGY exited 0 but the plan was not met, or its log claimed "all phases complete / tests pass" when they were not. Verify via git diff + a real test run, never the exit code or the log's self-report.
- AGY hangs forever when run non-interactively without an empty stdin. The fix is `< NUL` (Windows, via `cmd /c`) or `< /dev/null` (POSIX), plus `--dangerously-skip-permissions`.
- Print-mode stdout is empty when redirected/piped. This skill does not need it (judge by the repo); AGY's actual answer, if ever needed, is in `~/.gemini/antigravity-cli/conversations/<id>.db`.
- After install, `agy` may not resolve in an already-open shell until PATH is reloaded.
- The `--output json`, `--yes`, `--no-color`, `--prompt-file`, and `antigravity run` flags from third-party blogs do not exist on v1.0.8; they error with unknown-flag (exit 2).
- `--print-timeout` too short kills AGY mid-task and looks like a blocker; pair it with a job-side kill.
- Auth: unattended runs need `ANTIGRAVITY_API_KEY` set rather than the interactive OAuth flow.
- For overlapping cron/CI invocations, guard with a lock file and write result artifacts to a unique per-run filename (verify, then move into place) rather than appending to a fixed file.
