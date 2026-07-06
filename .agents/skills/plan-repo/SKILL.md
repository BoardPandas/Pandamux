---
name: plan-repo
model: opus
effort: high
description: Analyze project requirements and recommend the best tech stack for the current year. Infrastructure (Northflank frontend + backend, Cloudflare R2/CDN, Better Auth, Postgres, Redis) is fixed. Researches languages, frameworks, UI libraries, and tooling, then generates README, design guardrails, and tools reference. Run this before init-repo.
user-invocable: true
argument-hint: [optional: project name or description]
allowed-tools:
  - Read
  - Write
  - Edit
  - Glob
  - Grep
  - Bash
  - WebFetch
  - WebSearch
  - Agent
---

# Plan Repository

You have been asked to plan a new project before initializing it with Codex. This skill recommends the best tech stack based on project requirements and current best practices. Follow these steps exactly.

## Important: Date Awareness

Check the current date FIRST. All recommendations must reflect the state of the ecosystem as of today -- not cached knowledge. Every framework version, every library comparison, every "best practice" must be verified against today's date using WebSearch.

## Important: Fixed Infrastructure

Read `.Codex/references/infrastructure.md` FIRST. The hosting and infrastructure stack is fixed and non-negotiable:

- **Frontend hosting:** Northflank (container-based; SPA static-served or SSR -- the framework choice decides which)
- **Backend hosting:** Northflank (container-based API service)
- **Database:** Northflank Postgres
- **Cache/Queue:** Northflank Redis
- **Object storage:** Cloudflare R2 (public + signed URLs for private)
- **Auth:** Better Auth (hosted within the Northflank API)
- **Cron jobs:** Northflank Cron (standalone container)
- **Email:** Resend
- **CDN:** Cloudflare proxy (orange-cloud) in front of the Northflank frontend (Northflank built-in Fastly CDN as no-WAF fallback)

Do NOT ask about or recommend alternatives for any of the above. These are locked.

Auth methods are also locked: email/password, social (Google, Microsoft), magic link, passkey, two-factor (TOTP).

## Step 1: Gather Project Requirements

Ask the user these questions. Do NOT ask about hosting, database, auth, or infrastructure -- those are decided. Do NOT ask what language or framework they want -- you will recommend that.

### Project Shape
1. What does this project do? (one-sentence description)
2. Who are the users? (developers, end users, internal team, public)
3. What scale are you targeting? (personal project, startup MVP, production at scale, enterprise)

### Functional Requirements
4. Describe the UI (dashboard, marketing site, mobile-first app, real-time collaboration, data visualization, e-commerce, etc.)
5. What data does it work with beyond user accounts? (content/CMS, real-time events, files/media, financial data, etc.)
6. Does it need real-time features? (websockets, live updates, collaborative editing, streaming)
7. Does it integrate with external services beyond Stripe? (list them)

### Constraints
8. Any hard constraints on language or framework? (team only knows X, client requires Y)
9. Is this greenfield or does it need to integrate with an existing codebase?

Do NOT ask about timelines. Planning is phase-based, not date-based.

## Step 2: Research Current Options

Based on answers, spin up parallel `explorer` agents to research the layers that are NOT locked. Each subagent must use WebSearch to get information current as of today's date. Use the custom `explorer` agent (defined in `.Codex/agents/`), never the built-in `Explore` type -- the built-in loads every MCP tool schema and blows the context window.

### Language & Runtime Subagent
"Research the current state of [relevant languages] for a full-stack web app where both the frontend and backend run in containers on Northflank, fronted by a Cloudflare proxy (CDN/WAF) as of today's date. The backend must support Better Auth and connect to Postgres and Redis. Compare: ecosystem maturity, Better Auth SDK support, container build/startup quality, Northflank container support, developer tooling quality. WHY: We need the best language that works with our fixed infrastructure (Northflank containers + Cloudflare CDN/R2 + Better Auth + Postgres + Redis)."

Relevant comparisons:
- **TypeScript (Node)** vs **TypeScript (Bun)** vs **Go** vs **Rust** vs **Python** vs **Elixir**
- Consider: Better Auth has official SDKs for which languages? Which runtimes containerize cleanly for Northflank?

### Frontend Framework Subagent
"Research the current best frontend frameworks that run as a container on Northflank (static-served SPA or Node/Bun SSR server) behind a Cloudflare proxy as of today's date. Compare: ease of producing a production container or static build, SPA vs SSR vs hybrid trade-offs, build speed, ecosystem size, Better Auth client SDK support, developer experience. WHY: The frontend deploys to Northflank (NOT Cloudflare Pages) and integrates with Better Auth client-side; Cloudflare sits in front only as a CDN/WAF proxy."

Relevant comparisons:
- **Next.js** vs **SvelteKit** vs **Nuxt** vs **Astro** vs **Remix** vs **Solid Start** vs **React (SPA with Vite)**
- Key factor: which frameworks containerize cleanly on Northflank, and whether SPA (static) or SSR (Node/Bun server) better fits the project.

### Backend Framework Subagent
"Research the current best backend/API frameworks for [recommended language] that run in containers on Northflank as of today's date. The framework must support Better Auth middleware, Postgres connections via an ORM, Redis connections, and Cloudflare R2 S3-compatible API. Compare: performance, middleware ecosystem, Better Auth integration, container startup time. WHY: The API runs on Northflank containers and must host Better Auth endpoints alongside application logic."

Relevant comparisons (TypeScript):
- **Hono** vs **Express** vs **Fastify** vs **Elysia** vs **tRPC** (as API layer on top of one of the above)

### UI Library Subagent (always runs -- all projects have UI)
"Research the current best UI component libraries and styling approaches for [frontend framework] as of today's date. Compare: component quality, accessibility out-of-box, theming/customization, bundle size, maintenance activity, design system maturity. WHY: We need a UI approach that gives the best developer experience and end-user quality for [project type]."

Relevant comparisons:
- **Component libraries:** shadcn/ui vs Radix vs Ark UI vs Mantine vs MUI vs Chakra vs Park UI
- **Styling:** Tailwind CSS vs CSS Modules vs vanilla-extract vs Panda CSS vs UnoCSS
- **Animation:** Framer Motion vs Motion One vs GSAP vs CSS-only

### ORM & Data Layer Subagent
"Research the current best ORM/query builder options for [language] connecting to Postgres as of today's date. Compare: type safety, migration tooling, query performance, connection pooling, Northflank Postgres compatibility. WHY: The ORM must work with Northflank-hosted Postgres and support Better Auth's database adapter."

Relevant comparisons (TypeScript):
- **Drizzle** vs **Prisma** vs **Kysely** vs **TypeORM**
- Key factor: which ORMs have a Better Auth database adapter?

### Tooling Subagent
"Research the current recommended developer tooling for [language] projects as of today's date. Compare: speed, reliability, ecosystem compatibility. WHY: We need to populate tools.md with the fastest and most reliable tools for this stack."

Relevant comparisons:
- **Package managers:** npm vs pnpm vs yarn vs bun
- **Bundlers:** Vite vs Turbopack vs esbuild vs Rspack
- **Linters:** ESLint vs Biome vs oxc-lint
- **Formatters:** Prettier vs Biome vs dprint
- **Test runners:** Vitest vs Jest vs Bun test vs Playwright vs Cypress
- **Monorepo (if needed):** Turborepo vs Nx vs moon

## Step 3: Produce Stack Recommendation

Synthesize all subagent results into a recommendation. The infrastructure section is stated as fact (locked). The research-based sections present trade-offs.

```markdown
# Stack Recommendation

## Infrastructure (Locked)
| Layer | Choice |
|-------|--------|
| Frontend hosting | Northflank (container; SPA or SSR) |
| Backend hosting | Northflank (container) |
| Database | Northflank Postgres |
| Cache/Queue | Northflank Redis |
| Object storage | Cloudflare R2 |
| Auth | Better Auth |
| Cron | Northflank Cron |
| Email | Resend |
| CDN | Cloudflare proxy in front of Northflank |

## Language & Runtime
**Recommended:** <choice> <version>
**Why:** <2-3 sentences specific to this project + infrastructure>
**Runner-up:** <choice> -- <why it lost>

## Frontend Framework
**Recommended:** <choice> <version>
**Why:** <2-3 sentences, must reference Northflank container hosting and whether it ships as a static-served SPA or an SSR server>
**Runner-up:** <choice> -- <trade-off>

## Backend Framework
**Recommended:** <choice> <version>
**Why:** <2-3 sentences, must reference Northflank + Better Auth>
**Runner-up:** <choice> -- <trade-off>

## UI Approach
**Component library:** <choice> -- <why>
**Styling:** <choice> -- <why>
**Rationale:** <how these choices work together>

## ORM / Data Layer
**ORM:** <choice> -- <why, must reference Better Auth adapter support>

## Developer Tooling
**Package manager:** <choice>
**Bundler:** <choice>
**Linter + Formatter:** <choice>
**Test runner:** <choice>

## Full Stack Summary
| Layer | Choice | Version | Why |
|-------|--------|---------|-----|
| Language | ... | ... | ... |
| Frontend framework | ... | ... | ... |
| Backend framework | ... | ... | ... |
| UI library | ... | ... | ... |
| Styling | ... | ... | ... |
| ORM | ... | ... | ... |
| Package mgr | ... | ... | ... |
| Linter | ... | ... | ... |
| Test runner | ... | ... | ... |
```

**Present this to the user for approval before proceeding.** They may override specific choices -- accept overrides and adjust dependent choices if needed.

## Step 4: Generate README

Create a `README.md` with:

1. Project name and one-line description
2. Tech stack summary (locked infra + recommended stack)
3. Architecture diagram (the one from infrastructure.md, adapted with chosen framework names)
4. Prerequisites (required tools and versions)
5. Getting started (clone, install, run)
6. Project structure (planned folder layout based on framework conventions)
7. Environment variables needed (Postgres, Redis, R2, Better Auth, Resend, Stripe)
8. Development phases:
   - Phase 1: Foundation (project setup, auth, database schema, deployment pipeline)
   - Phase 2: Core features (primary functionality)
   - Phase 3: Polish (error handling, edge cases, testing)
   - Phase 4: Ship (production deployment, monitoring, documentation)
9. Deployment section (Northflank frontend + API specifics, plus the Cloudflare proxy/CDN setup in front of the Northflank frontend -- DNS CNAME, Full (Strict) TLS, ACME/proxy ordering)

## Step 5: Generate Design Guardrails

Create `.Codex/references/design-guardrails.md` with rules specific to the chosen UI library and styling approach:

1. **Component rules:** Max component size, composition patterns, prop conventions for <chosen library>
2. **Styling rules:** Conventions for <chosen styling approach>, responsive breakpoints, dark mode strategy
3. **Accessibility:** WCAG AA minimum, required ARIA patterns, keyboard navigation, focus management
4. **Performance:** Bundle size budget, image optimization (WebP/AVIF), lazy loading rules, Core Web Vitals targets
5. **Auth UI patterns:** Better Auth sign-in/sign-up flow, social login buttons, magic link flow, passkey enrollment, 2FA setup
6. **Consistency:** Typography scale, spacing scale, color system usage per the chosen design approach

## Step 6: Generate Tools Reference

Create or update `.Codex/references/tools.md` with the exact CLI tools for the chosen stack. **Important constraints:**

- There is NO local Docker, no local Postgres, no local Redis. All databases and services run remotely on Northflank and Cloudflare.
- Development connects to remote services via environment variables or Northflank CLI port-forwarding.
- Do NOT add docker, docker-compose, psql, redis-cli, or any local infrastructure tools.

Tools to include:

- Package manager commands
- Build and dev commands
- Framework CLI commands (frontend + backend)
- ORM/migration commands (connecting to remote Northflank Postgres)
- Wrangler commands (Cloudflare R2 + DNS/CDN -- NOT Pages; the frontend deploys to Northflank)
- Northflank CLI commands (frontend + backend deploy, addon management, port-forwarding)
- Linter and formatter commands
- Test runner commands

For each tool: name, install command, version check command, common usage patterns.

Also preserve the **Available MCP Servers** section in tools.md -- it documents all MCP integrations available to Codex (Cloudflare, GitHub, Slack, Gmail, Google Calendar, Notion, Northflank, Railway, Doppler, NinjaOne, Zendesk, browser automation). Do not remove or overwrite this section.

## Step 7: Plan the Hierarchical AGENTS.md Structure

Standard structure for this infrastructure:

- Root `AGENTS.md` -- Project-wide rules, full stack summary, shared conventions
- `frontend/AGENTS.md` (or `apps/web/AGENTS.md`) -- Northflank frontend (SPA/SSR container) conventions, UI component rules, styling rules
- `api/AGENTS.md` (or `apps/api/AGENTS.md`) -- Northflank API conventions, Better Auth integration rules, database patterns

Plan these but do NOT create subfolder AGENTS.md files until the folders exist.

## Step 8: Save the Plan

Save the complete plan to `tasks/plan-repo.md` with:

1. Project requirements (user's answers)
2. Infrastructure (locked -- from infrastructure.md)
3. Stack recommendation (approved version)
4. Research findings summary (key data points that drove decisions)
5. Planned file structure
6. Planned AGENTS.md hierarchy
7. Design guardrails summary
8. Phase-based development plan
9. Environment variables needed
10. Tools required
11. Learning Lessons / Gotchas checklist (to be filled during/after implementation)

## Step 9: Report and Next Step

Print a summary of everything planned, then tell the user:

> Your project plan is saved to `tasks/plan-repo.md`. To initialize the project with Codex, say **"initialize repo"**. The init-repo skill will read your plan and use it to configure everything.

> **Reminder:** The plan includes a Learning Lessons / Gotchas section. After each phase, route discoveries to `.Codex/agent-memory/debugging.md`.
