# PRD: Doppler Secrets Management for Pandamux

**Status**: In progress (Azure Trusted Signing live in CI; winget/Netlify migration pending)
**Owner**: BoardPandas
**Author**: (generated)
**Date**: 2026-07-06 (updated 2026-07-06: signing wired end to end)
**Repo**: github.com/BoardPandas/Pandamux

---

## 1. Summary

Stand up a single Doppler project as the source of truth for every secret this
repository touches (CI release signing, package publishing, GitHub release
uploads, and Netlify site deploys). Secrets today live in GitHub Actions repo
secrets and on individual maintainer machines, with no shared inventory, no
rotation story, and no audit trail. Doppler centralizes them, injects them into
CI and local shells on demand, and gives us versioning plus access control.

This PRD covers the Doppler project design and rollout. It does not change
application runtime behavior: Pandamux is a desktop app and does not
read Doppler at runtime.

## 2. Problem & Motivation

- **Scattered secrets.** Release and publish credentials live in GitHub repo
  secrets (`WINGET_TOKEN`) while the Netlify site deploy is run locally
  (`npx netlify deploy --prod --dir site`) using whatever token the maintainer
  has on disk. There was no single inventory. (Code signing now resolves
  through Doppler; see Section 5.)
- **No rotation or expiry tracking.** Nobody knows which token expires when, or
  who holds a copy.
- **No audit trail.** GitHub repo secrets record who set a value but not who
  read it; local `.env` files record nothing.
- **Onboarding friction.** A new maintainer cannot deploy the site or cut a
  release without someone hand-delivering credentials over a side channel.

## 3. Goals

1. One Doppler project (`pandamux`) holding all repo-related secrets.
2. Environment separation between CI/production automation and local maintainer
   use.
3. GitHub Actions pulls secrets from Doppler via a scoped service token instead
   of raw repo secrets.
4. Local site deploys source Netlify creds from Doppler (`doppler run -- ...`)
   instead of ambient environment.
5. Documented, least-privilege access: service tokens are read-only and
   config-scoped.

## 4. Non-Goals

- Runtime secret injection into the shipped Electron app (there are no
  app-runtime secrets today).
- Managing the future Rust rewrite's secrets (separate PRD once its crates and
  deploy targets exist).
- Migrating unrelated BoardPandas org secrets (this project is repo-scoped).
- Replacing GitHub's automatic `GITHUB_TOKEN` (that stays managed by Actions).

## 5. Scope: Secrets Inventory

Secrets to bring under Doppler, grouped by consumer:

| Secret | Consumer | Status | Notes |
|---|---|---|---|
| `AZURE_TENANT_ID` | Release CI (Azure Trusted Signing) | in `prd` ✅ | Wellforce tenant `cea21578-…-528c` |
| `AZURE_CLIENT_ID` | Release CI | in `prd` ✅ | `pandamux-ci-signing` app registration |
| `AZURE_CLIENT_SECRET` | Release CI | in `prd` ✅ | Expires 2027-07-06 (rotate before) |
| `AZURE_TRUSTED_SIGNING_ENDPOINT` | Release CI | in `prd` ✅ | `https://eus.codesigning.azure.net/` |
| `AZURE_TRUSTED_SIGNING_ACCOUNT_NAME` | Release CI | in `prd` ✅ | `HDBtrustedsigning` (shared account) |
| `AZURE_TRUSTED_SIGNING_CERT_PROFILE_NAME` | Release CI | in `prd` ✅ | `SupportForge` profile, reused |
| `WINGET_TOKEN` | `winget.yml` publish job | GitHub repo secret ⬜ | Migrate into `prd` next |
| `NETLIFY_AUTH_TOKEN` | Local `netlify deploy` for `site/` | Maintainer machine ⬜ | Not yet migrated (PRD-only project) |
| `NETLIFY_SITE_ID` | Local `netlify deploy` for `site/` | Maintainer machine ⬜ | Not yet migrated |

`GITHUB_TOKEN` (the ephemeral Actions token) and `DOPPLER_TOKEN` (the CI service
token that unlocks the above) stay as GitHub-managed repo secrets by design.

## 6. Doppler Project Design

**Project**: `pandamux`

**Configs (environments):** `prd` only. Doppler auto-created `dev`/`stg` on
project creation; both were deleted per the owner's "PRD config only" standard
for this project. All repo secrets (CI signing today, winget/Netlify later)
live in `prd`.

**Service tokens:**

- `pandamux-ci-release` — read-only, `prd`-scoped service token. **Created** and
  stored as the single GitHub Actions repo secret `DOPPLER_TOKEN`. The release
  workflow fetches secrets from it via `dopplerhq/secrets-fetch-action`
  (`inject-env-vars: true`), so the individual `AZURE_*` values need no
  per-secret repo entry.

## 7. Integration Plan

### 7.1 GitHub Actions

1. ✅ Stored one repo secret: `DOPPLER_TOKEN` = the `pandamux-ci-release`
   service token.
2. ✅ `release.yml` now fetches secrets with `dopplerhq/secrets-fetch-action@v2`
   (`inject-env-vars: true`) and signs `pandamux.exe` with
   `azure/trusted-signing-action@v2`, placed after the rcedit step so the
   signature is not invalidated. Untested until the next tagged release
   exercises it.
3. ⬜ `winget.yml` still reads `WINGET_TOKEN` directly; migrate it to Doppler and
   delete the raw repo secret after a green run.

### 7.2 Local maintainer workflow

1. `doppler login` then `doppler setup` (project `pandamux`, config `dev`) in
   the repo root.
2. Site deploy becomes: `doppler run -- npx netlify deploy --prod --dir site`.
3. Update `docs/` (Website section) and the repo `CLAUDE.md` deploy note to show
   the `doppler run` form.

### 7.3 `.gitignore` / hygiene

- Confirm no `.env` is tracked (none today). Add `.doppler*` local artifacts to
  `.gitignore` if the CLI writes any.

## 8. Access Control & Rotation

- CI service token: read-only, `prd` config only, revocable independently of
  human access.
- Human access: via Doppler workplace membership and project roles, not shared
  tokens.
- Rotation: the rotation-sensitive items are `AZURE_CLIENT_SECRET` (expires
  **2027-07-06**; rotate via Graph `addPassword` on the `pandamux-ci-signing`
  app registration, then update Doppler) and `WINGET_TOKEN`. The Doppler service
  token itself is non-expiring; revoke and re-mint to rotate it.
- On maintainer offboarding: revoke Doppler membership; no local `.env` copies
  to chase.

## 9. Rollout Plan

1. ✅ **Created** the `pandamux` Doppler project, `prd` config only.
2. ✅ **Seeded** the six `AZURE_*` signing values into `prd`; provisioned the
   `pandamux-ci-signing` service principal (Wellforce) and granted it the
   Artifact Signing Certificate Profile Signer role on the `SupportForge`
   profile of the `HDBtrustedsigning` account.
3. ✅ **Minted** the `pandamux-ci-release` read-only service token; added
   `DOPPLER_TOKEN` to GitHub.
4. ✅ **Wired** `release.yml` to Doppler + Azure Trusted Signing. ⬜ `winget.yml`
   still pending. ⬜ Verify on the next real tagged release.
5. ⬜ **Cut over** local site deploy to Doppler (`NETLIFY_*` into `prd`, run via
   `doppler run -- npx netlify deploy`); update docs.
6. ⬜ **Delete** migrated GitHub repo secrets once CI is green (only
   `WINGET_TOKEN` remains to migrate; signing never used raw repo secrets, so
   nothing to delete there).
7. ⬜ **Document** the setup in `docs/operations/`.

## 10. Success Criteria

- A fresh maintainer can deploy the site and cut a release with only Doppler
  access (no side-channel credential handoff).
- `release.yml` and `winget.yml` reference exactly one GitHub secret
  (`DOPPLER_TOKEN`); all other secrets resolve through Doppler.
- Every managed secret has an owner and rotation note in Doppler.
- No secret values are committed to the repo or stored in tracked files.

## 11. Risks & Open Questions

- **CI coupling to Doppler availability.** A Doppler outage blocks releases.
  Mitigation: releases are infrequent and manually triggered; acceptable.
- **First signed release is unverified.** The Azure Trusted Signing step is
  wired but has not run yet; the next tagged release is the first real exercise,
  and fresh app registrations can take ~15 min to propagate. If signing fails,
  the unsigned zip is still produced by the prior steps.
- **Shared signing account.** `HDBtrustedsigning` / `SupportForge` is shared
  with other apps. The CI identity is scoped to that one profile (least
  privilege), but signing throughput quota is shared across consumers.
- **Rust rewrite.** The frozen Electron app is the only consumer today; the Rust
  workspace will need its own config set once it has deploy targets. Out of
  scope here.
- **Resolved:** No `dev` config. This project is `prd`-only per the owner's
  standard; local/maintainer secrets (Netlify) will also live in `prd` when
  migrated, or stay on maintainer machines until then.
