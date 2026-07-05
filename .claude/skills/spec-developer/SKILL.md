---
name: spec-developer
model: opus
effort: high
description: Interview-driven spec generation. Asks 20+ clarifying questions, then produces a detailed implementation plan saved to /tasks. Use for any feature larger than a single file change.
user-invocable: true
argument-hint: <feature name or description>
disable-model-invocation: true
allowed-tools:
  - Read
  - Write
  - Glob
  - Grep
  - Agent
  - WebSearch
  - WebFetch
---

# Spec Developer

You have been asked to develop a detailed specification for a feature. Follow these steps exactly.

## Important: Plan in This Session, Execute in Another

This skill produces a plan ONLY. It does not implement anything. The user will start a fresh session to execute the plan, keeping context clean.

## Step 1: Explore the Codebase (Spec Developer Explorer variant)

Before asking questions, spin up 3-5 parallel `explorer` agents to understand the codebase. Use the custom `explorer` agent (defined in `.claude/agents/`), never the built-in `Explore` type -- the built-in loads every MCP tool schema and blows the context window before it can do any work.

1. **Architecture subagent:** "Understand the project's architecture and module boundaries. WHY: We need to know where this feature fits and what it can depend on."
2. **Patterns subagent:** "Find existing patterns for similar features (routing, state, data fetching, error handling). WHY: The new feature must follow established patterns to maintain consistency."
3. **Dependencies subagent:** "List external dependencies and their capabilities. WHY: We need to know what's already available before adding new dependencies."
4. **Tests subagent:** "Understand the test setup, patterns, and coverage gaps. WHY: The spec must include a test plan that matches the existing test infrastructure."
5. **Data flow subagent (if applicable):** "Trace how data flows from input to storage to display for a similar existing feature. WHY: The new feature's data flow must integrate with existing patterns."

Wait for all subagents to complete before proceeding.

## Step 2: Ask Clarifying Questions

Based on the codebase exploration, ask the user 20+ non-obvious clarifying questions. Group them by category:

### Behavior
- What is the exact input/output contract?
- What happens on partial success?
- What are the error states and how should each be handled?
- Are there rate limits or throttling requirements?
- What is the expected data volume?

### Integration
- Which existing modules does this touch?
- Does this replace or extend existing behavior?
- Are there migration steps for existing data?
- Does this affect any public API contracts?

### Edge Cases
- What happens with empty/null/missing input?
- What happens when external services are unavailable?
- What happens under concurrent access?
- Are there timezone, locale, or encoding concerns?

### UX (if applicable)
- What loading states are needed?
- What does the error state look like?
- Is optimistic UI appropriate here?
- What accessibility requirements apply?

### Security
- What authorization is required?
- Is there sensitive data that needs encryption or redaction?
- Are there audit logging requirements?

Adapt questions to the specific feature. Skip irrelevant categories. Add domain-specific questions based on what the subagents found.

Wait for user answers before proceeding.

## Step 3: Generate the Spec

Produce a detailed implementation plan (target: 500-700 lines) covering:

### 1. Overview
- Feature name and one-sentence description
- Phase assignment (Foundation, Core, Polish, Ship)
- Dependencies on other features or tasks

### 2. Architecture
- Which modules/files are affected
- New files to create (with purpose)
- Data flow diagram (mermaid syntax)
- State management approach (if applicable)

### 3. Implementation Steps
Numbered steps in implementation order. For each step:
- File to create or modify
- What to add or change (specific, not vague)
- Patterns to follow from existing code (reference specific files)
- Potential gotchas

### 4. Data Model (if applicable)
- Schema changes
- Migration steps
- Seed data requirements

### 5. API Contract (if applicable)
- Endpoint definitions (method, path, request, response)
- Error response format
- Authentication/authorization requirements

### 6. Test Plan
- Unit tests needed (list specific test cases)
- Integration tests needed
- Edge case tests
- Test data requirements

### 7. Error Handling
- Exhaustive list of failure modes
- Recovery strategy for each
- User-facing error messages

### 8. Rollback Plan
- How to undo this feature without affecting other work
- Database rollback steps (if applicable)

### 9. Learning Lessons / Gotchas

After implementation, capture here:
- [ ] New patterns discovered -- add to `.claude/agent-memory/patterns.md`
- [ ] Gotchas encountered -- add to `.claude/agent-memory/debugging.md`
- [ ] Workflow improvements -- update CLAUDE.md or agent memory
- [ ] Failed approaches -- document what was tried and why it failed

*Fill in during/after implementation. Route generalizable lessons to the appropriate agent-memory file.*

## Step 4: Save the Spec

Save to `tasks/<feature-slug>.md` with the complete spec.

If a `tasks/` folder does not exist, create it.

## Step 5: Report

Print a summary of the spec, then tell the user:

> Spec saved to `tasks/<feature-slug>.md`. Start a fresh session to implement -- this keeps context clean. In the new session, say: "Implement the spec in tasks/<feature-slug>.md"

> **Reminder:** After implementing this spec, review the "Learning Lessons / Gotchas" section and route discoveries to `.claude/agent-memory/debugging.md`.

## Document Failed Attempts

If this spec is a retry after a failed implementation, ask the user to describe what went wrong. Add a "Previous Attempts" section to the spec documenting:
- What was tried
- Why it failed
- What to avoid this time

This prevents the implementation session from repeating dead ends.
