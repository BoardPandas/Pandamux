<!-- PAGE_ID: pandamux_13_release -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page:

- [CLAUDE.md:155-304](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L155-L304)
- [electron-builder.json:1-53](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L1-L53)
- [.github/workflows/release.yml:1-204](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L1-L204)
- [.github/workflows/winget.yml:1-34](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/winget.yml#L1-L34)
- [src/main/updater.ts:1-135](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L1-L135)
- [src/main/update-checker.ts:1-96](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L1-L96)
- [.claude/rules/commit-changelog.md](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.claude/rules/commit-changelog.md)
- [package.json](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json)

</details>

# Release and Packaging

> **Related Pages**: [Getting Started](../GETTING_STARTED.md), [Overview](../OVERVIEW.md)

---

<!-- BEGIN:AUTOGEN pandamux_13_release_overview -->
## Overview

PandaMUX Everywhere ships as a **portable zip**, not an NSIS installer, because without code-signing, Windows SmartScreen flags installers more aggressively than zip extractions ([CLAUDE.md:157](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L157)). There are two independent ways to produce a release: a fully automated CI pipeline triggered by a `v*` tag push ([release.yml:1-6](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L1-L6)), and a manual ASAR-based packaging flow documented step-by-step in `CLAUDE.md` for local/emergency releases ([CLAUDE.md:159-277](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L159-L277)).

| Path | Trigger | Packaging tool | Output |
|---|---|---|---|
| CI release | Push tag `v*`, or manual `workflow_dispatch` | `electron-builder --win --dir` (unpacked dir, no installer) ([release.yml:46-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L46-L47)) | `pandamux-<version>-win-x64.zip` + `latest.yml` GitHub Release assets |
| Manual release | Developer runs the CLAUDE.md steps locally | `npx asar pack` against a hand-built staging dir ([CLAUDE.md:171-188](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L171-L188)) | Same zip/`latest.yml` naming, published via `gh release create` |

Both paths converge on the same artifact shape (a Windows x64 zip containing `pandamux.exe` plus `resources/`) so `electron-updater` and `winget` can treat either origin identically. The Electron app is otherwise frozen to bug fixes pending a native Rust rewrite, but the release flow itself is actively maintained ([CLAUDE.md:1-11](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L1-L11)).

Sources: [CLAUDE.md:155-304](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L155-L304), [.github/workflows/release.yml:1-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L1-L47)
<!-- END:AUTOGEN pandamux_13_release_overview -->

---

<!-- BEGIN:AUTOGEN pandamux_13_release_build -->
## Build and Staging

Both release paths start from the same two build commands: compile the main/preload/CLI TypeScript, then build the renderer with Vite ([CLAUDE.md:162-164](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L162-L164)). The manual flow then hand-assembles an ASAR staging directory instead of delegating to `electron-builder`'s packager.

| Step | Manual flow command | Purpose |
|---|---|---|
| 1. Build | `pnpm run build:main` then `pnpm exec vite build` | Compile TS to `dist/main`, `dist/preload`, `dist/cli`; build renderer to `dist/renderer` ([CLAUDE.md:162-164](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L162-L164)) |
| 2. Verify compiled code | `grep -c 'your_fix_string' dist/main/index.js` | Confirm the intended fix actually landed in the compiled bundle before packaging ([CLAUDE.md:166-169](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L166-L169)) |
| 3. Create ASAR staging | `mkdir -p .asar-staging build-out`, copy `dist/` and `package.json` in, `pnpm install --prod --ignore-scripts --config.node-linker=hoisted` | Build a minimal production `node_modules` for packaging, always from the project root ([CLAUDE.md:171-181](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L171-L181)) |
| 6. Create release staging | Unzip the previous release zip into `../pandamux-release-staging` | Reuse a known-good directory shape instead of hand-building one ([CLAUDE.md:203-208](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L203-L208)) |
| 7. Copy resources | Copy `app.asar`, `app.asar.unpacked`, icon, themes, sounds, CLI, shell-integration, orchestrator plugin into staging | Reassemble `resources/` around the freshly packed ASAR ([CLAUDE.md:210-220](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L210-L220)) |

A cwd-drift hazard applies throughout step 3: if a `cd .asar-staging` is not paired with returning to the project root, the following `mkdir build-out` lands inside the staging directory, and the next `asar pack` recursively swallows its own prior output, producing a bloated ~188M asar ([CLAUDE.md:172-176](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L172-L176)). Always use subshells (`( cd dir && cmd )`) or absolute paths to avoid this ([CLAUDE.md:299](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L299)).

The CI path skips ASAR staging entirely: it installs dependencies with `pnpm install --frozen-lockfile`, then packages via `pnpm exec electron-builder --win --dir --publish never`, producing an unpacked directory under `release/win-unpacked/` ([release.yml:35-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L35-L47)). It also runs `pip install setuptools` first, since node-gyp on Python 3.12+ needs the removed `distutils` module restored ([release.yml:32-33](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L32-L33)).

Sources: [CLAUDE.md:159-220](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L159-L220), [.github/workflows/release.yml:19-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L19-L47)
<!-- END:AUTOGEN pandamux_13_release_build -->

---

<!-- BEGIN:AUTOGEN pandamux_13_release_asar -->
## ASAR Packaging and Native Modules

node-pty is the only native dependency; it ships N-API prebuilds that are ABI-stable across Node and Electron (verified under Electron 33 / ABI 130 / N-API 9), so the release flow deliberately does not rebuild it from source ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)). `electron-builder.json` encodes this decision directly with `"npmRebuild": false` and a comment explaining why ([electron-builder.json:5-6](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L5-L6)), and unpacks all `.node` binaries from the asar via `asarUnpack: ["**/*.node"]` ([electron-builder.json:25-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L25-L27)).

```json
"npmRebuild": false,
...
"files": [
  "dist/**/*",
  "package.json",
  "node_modules/**/*",
  "!**/*.ts",
  "!**/*.tsx",
  "!src/**/*",
  "!docs/**/*",
  "!tests/**/*"
],
"asarUnpack": [
  "**/*.node"
]
```

(electron-builder.json:6-27)

For the manual flow, the equivalent unpack step is done with the `asar` CLI directly:

```bash
npx asar pack .asar-staging build-out/app.asar --unpack-dir "node_modules/node-pty/prebuilds"
```

(CLAUDE.md:188)

This must use `--unpack-dir` (a path) and never `--unpack "**/*.node"` (a glob): the glob form is silently eaten by Git Bash for Windows, so `asar` produces a packed asar but creates no `.unpacked/` directory and reports no error ([CLAUDE.md:183-188](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L183-L188), [CLAUDE.md:298](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L298)). Before packing, `.asar-staging/node_modules/node-pty/build` must also be removed so the loader is forced onto the prebuilds path: `conpty.dll` (via `useConptyDll`) resolves relative to whichever `conpty.node` actually loads, and only `prebuilds/win32-x64/` ships the matching `conpty/` directory next to it ([CLAUDE.md:181](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L181)).

After packing, verify both the unpack and the fix content:

| Check | Command | Expected |
|---|---|---|
| Natives unpacked | `ls build-out/app.asar.unpacked/node_modules/node-pty/prebuilds/win32-x64/` | Contains `conpty.node`, `conpty_console_list.node`, `pty.node` ([CLAUDE.md:190-192](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L190-L192)) |
| ASAR size sanity | inspect `build-out/app.asar` size | ~24M is normal; 80M+ means natives weren't moved out; 180M+ means staging got polluted by cwd drift ([CLAUDE.md:193-194](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L193-L194)) |
| Fix content present | extract to `/tmp/asar-verify` with `npx asar extract`, then `grep -c 'your_fix_marker' ...` | Non-zero match count in the extracted `dist/` files ([CLAUDE.md:196-201](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L196-L201)) |

Extraction is routed to `/tmp` rather than piped through stdout because `asar extract-file`'s stdout piping is unreliable on Windows ([CLAUDE.md:197](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L197)). The manual ASAR should never be packed directly to the live `resources/app.asar` while pandamux may be running; it is packed to `build-out/` first and copied into staging afterward ([CLAUDE.md:187](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L187), [CLAUDE.md:300](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L300)).

Sources: [electron-builder.json:1-53](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L1-L53), [CLAUDE.md:171-201](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L171-L201)
<!-- END:AUTOGEN pandamux_13_release_asar -->

---

<!-- BEGIN:AUTOGEN pandamux_13_release_metadata -->
## Icon and Metadata (rcedit)

Both release paths embed the application icon and Windows version-string metadata into `pandamux.exe` using `rcedit`, since `electron-builder`'s `--dir` target does not do this itself. `electron-builder.json` supplies the icon path for its own Windows target config (`win.icon`), separate from the manual rcedit call ([electron-builder.json:28-30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L28-L30)).

`rcedit` exports a **named** export, `{ rcedit }`, not a default function. Calling `const rcedit = require('rcedit')` and then invoking `rcedit(...)` throws `"rcedit is not a function"` ([CLAUDE.md:223-225](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L223-L225), [CLAUDE.md:297](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L297)). CI destructures correctly:

```javascript
const { rcedit } = require('rcedit');
rcedit('release/win-unpacked/pandamux.exe', {
  icon: 'resources/icons/icon.ico',
  'version-string': {
    ProductName: 'PandaMUX Everywhere',
    FileDescription: 'PandaMUX Everywhere',
    CompanyName: 'BoardPandas',
    InternalName: 'pandamux',
    OriginalFilename: 'pandamux.exe',
    LegalCopyright: 'Copyright (c) 2025 BoardPandas'
  },
  'file-version': process.env.VER,
  'product-version': process.env.VER
}).then(() => console.log('rcedit done')).catch(e => { console.error(e); process.exit(1); });
```

(.github/workflows/release.yml:131-146)

`FileDescription` matters beyond cosmetics: Windows taskbar pinning uses the PE `FileDescription` field for the shortcut name, so it must read "PandaMUX Everywhere" ([CLAUDE.md:302](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L302)). Separately, `AppUserModelId` is set to `com.pandamux.app` in `src/main/index.ts` for correct taskbar grouping ([CLAUDE.md:303](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L303)), matching the `appId` in `electron-builder.json` ([electron-builder.json:2](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L2)).

`rcedit` cannot modify a running exe. The manual flow always operates on the copy inside `../pandamux-release-staging/`, never on a `pandamux.exe` in the project root that might currently be running ([CLAUDE.md:242-243](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L242-L243), [CLAUDE.md:296](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L296)). After rcedit, the manual flow zips the staging directory with `Compress-Archive` and, separately, downloaded release zips carry an NTFS Mark-of-the-Web (`Zone.Identifier` stream) that must be cleared with `Get-ChildItem -Recurse | Unblock-File` before the extracted exe will run cleanly ([CLAUDE.md:301](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L301)).

Sources: [CLAUDE.md:222-244](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L222-L244), [.github/workflows/release.yml:126-148](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L126-L148), [electron-builder.json:2](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/electron-builder.json#L2)
<!-- END:AUTOGEN pandamux_13_release_metadata -->

---

<!-- BEGIN:AUTOGEN pandamux_13_release_updater -->
## Auto-Update and latest.yml

PandaMUX Everywhere has two independent, deliberately separate update paths. `src/main/updater.ts` uses `electron-updater` to actually download and (with confirmation) install updates; `src/main/update-checker.ts` is a lightweight, notify-only poll of the GitHub `/releases/latest` API that only surfaces a badge and opens the release page in the OS browser ([update-checker.ts:4-11](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L4-L11)).

`electron-updater` requires `latest.yml` at the root of the latest GitHub Release; without it, every launch produces a 404 against the update feed (tracked as issue #68) ([CLAUDE.md:248-250](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L248-L250)). Both release paths generate it independently: CI computes it with a Node script that hashes the produced zip with SHA-512 and writes `version`, `files[].url/sha512/size`, and top-level `path`/`sha512`/`releaseDate` fields ([release.yml:157-183](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L157-L183)), and the manual flow uses an equivalent standalone Node script ([CLAUDE.md:251-262](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L251-L262)).

```javascript
const crypto = require('crypto'); const fs = require('fs');
const version = '<VERSION>';
const zip = '../pandamux-' + version + '-win-x64.zip';
const data = fs.readFileSync(zip);
const sha512 = crypto.createHash('sha512').update(data).digest('base64');
const yaml = ['version: ' + version, 'files:', '  - url: pandamux-' + version + '-win-x64.zip',
  '    sha512: ' + sha512, '    size: ' + data.length, 'path: pandamux-' + version + '-win-x64.zip',
  'sha512: ' + sha512, 'releaseDate: ' + JSON.stringify(new Date().toISOString()), ''].join('\n');
fs.writeFileSync('../latest.yml', yaml);
```

(CLAUDE.md:251-261)

`updater.ts` layers a quarantine window on top of `electron-updater` (tracked as issue #29): a release must be publicly visible for at least `PANDAMUX_MIN_RELEASE_AGE_DAYS` (default 3) days, computed from GitHub's server-side `published_at` rather than the attacker-writable `latest.yml releaseDate`, before it is downloaded ([updater.ts:5-30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L5-L30)). `autoDownload` and `autoInstallOnAppQuit` are both disabled, so install always requires an explicit user click in a confirmation dialog ([updater.ts:72-73](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L72-L73), [updater.ts:104-117](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L104-L117)). The updater can be disabled entirely for air-gapped/corporate environments via `PANDAMUX_DISABLE_UPDATER=1` ([updater.ts:58-68](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L58-L68)), and a missing `latest.yml` (`ERR_UPDATER_CHANNEL_FILE_NOT_FOUND`) is treated as an expected condition rather than an error, since it can happen after a manual/partial release ([updater.ts:48-56](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L48-L56), [updater.ts:120-129](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L120-L129)).

| Config | Default | Purpose |
|---|---|---|
| `PANDAMUX_MIN_RELEASE_AGE_DAYS` | 3 days | Quarantine window before a discovered update is downloaded ([updater.ts:21-30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L21-L30)) |
| `PANDAMUX_DISABLE_UPDATER` | unset (updater enabled) | Set to `1` to fully disable auto-update ([updater.ts:58-60](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L58-L60)) |
| Re-check interval | 6 hours (`RECHECK_INTERVAL_MS`) | Both `updater.ts` and `update-checker.ts` re-poll on this cadence ([updater.ts:22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L22), [update-checker.ts:15](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L15)) |

`update-checker.ts` also reuses `fetchLatestRelease()` from `updater.ts`'s age computation, and independently compares semantic versions field-by-field before broadcasting `UPDATE_AVAILABLE` to renderer windows ([update-checker.ts:31-41](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L31-L41), [update-checker.ts:73-91](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L73-L91)). Draft and prerelease GitHub releases are skipped by this check ([update-checker.ts:75](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L75)).

Signature verification, Authenticode signing, and build provenance are explicitly out of scope for the current mitigation and are tracked as follow-ups on issue #29, since they require offline signing keys and CI changes rather than code changes alone ([updater.ts:18-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L18-L19)).

Sources: [src/main/updater.ts:1-135](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/updater.ts#L1-L135), [src/main/update-checker.ts:1-96](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/src/main/update-checker.ts#L1-L96), [CLAUDE.md:248-262](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L248-L262)
<!-- END:AUTOGEN pandamux_13_release_updater -->

---

<!-- BEGIN:AUTOGEN pandamux_13_release_ci -->
## CI Workflows

Two GitHub Actions workflows drive automated distribution: `release.yml` builds and publishes the Windows release, and `winget.yml` republishes each released version to the Windows Package Manager.

`release.yml` runs on `windows-latest`, triggered by a `v*` tag push or manually via `workflow_dispatch` with an optional `tag` input ([release.yml:1-10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L1-L10)). It requires `permissions: contents: write` to publish the release ([release.yml:12-13](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L12-L13)). pnpm is installed before `actions/setup-node` specifically so `setup-node`'s pnpm cache can find it, with the pnpm version sourced from the `packageManager` field in `package.json` as the single source of truth ([release.yml:22-30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L22-L30)).

| Job step | Command | Notes |
|---|---|---|
| Install pnpm | `pnpm/action-setup@v4` | Runs before `setup-node` for cache compatibility ([release.yml:24-25](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L24-L25)) |
| Setup Node | `actions/setup-node@v4`, node 24.18.0, `cache: pnpm` | Matches `CLAUDE.md`'s pinned Node LTS ([release.yml:27-30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L27-L30)) |
| Fix Python distutils | `pip install setuptools` | node-gyp needs the removed `distutils` module on Python 3.12+ ([release.yml:32-33](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L32-L33)) |
| Install deps | `pnpm install --frozen-lockfile` | Lockfile-exact install ([release.yml:35-36](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L35-L36)) |
| Build main/preload/CLI | `pnpm run build:main` | ([release.yml:39-40](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L39-L40)) |
| Build renderer | `pnpm exec vite build` | ([release.yml:42-43](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L42-L43)) |
| Package | `pnpm exec electron-builder --win --dir --publish never` | Unpacked directory, no installer, no auto-publish ([release.yml:46-47](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L46-L47)) |
| Extract version/tag | Node one-liner reading `package.json`, or `github.ref_name`/dispatch input | Feeds later step outputs ([release.yml:49-61](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L49-L61)) |
| Embed icon/metadata | `rcedit` (see above) | ([release.yml:127-148](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L127-L148)) |
| Create zip | `Compress-Archive` (pwsh) | ([release.yml:151-155](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L151-L155)) |
| Generate latest.yml | Node crypto/fs script | ([release.yml:157-183](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L157-L183)) |
| Upload workflow artifact | `actions/upload-artifact@v4` | Zip + `latest.yml` retained as CI artifacts too ([release.yml:185-191](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L185-L191)) |
| Publish GitHub Release | `softprops/action-gh-release@v2`, `generate_release_notes: true` | Not a draft; auto-generates release notes ([release.yml:194-204](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L194-L204)) |

Authenticode signing via SignPath is fully wired into the workflow (submit, poll, download signed exe) but commented out pending SignPath's OSS quota approval ([release.yml:63-124](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L63-L124)); until it is enabled, the shipped `pandamux.exe` is unsigned, which is the reason SmartScreen sensitivity drives the zip-not-installer decision described in Overview.

`winget.yml` runs on `ubuntu-latest`, triggered whenever a GitHub Release transitions to `released` (or manually), and is scoped to only run in the `BoardPandas/Pandamux` repository ([winget.yml:14-23](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/winget.yml#L14-L23)). It uses `vedantmgoyal9/winget-releaser@v2` to open a PR against `microsoft/winget-pkgs`, matching release assets by the regex `pandamux-.*-win-x64\.zip$` ([winget.yml:25-31](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/winget.yml#L25-L31)). This requires a one-time manual bootstrap (submit `winget/*.yaml` to `microsoft/winget-pkgs` once) plus a classic PAT with `public_repo` scope stored as the `WINGET_TOKEN` repo secret, since `winget-releaser` can only update an already-existing package, not create the first version ([winget.yml:1-11](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/winget.yml#L1-L11)). If `WINGET_TOKEN` is unset, the publish step is skipped silently rather than failing the run ([winget.yml:26](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/winget.yml#L26)).

Sources: [.github/workflows/release.yml:1-204](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/release.yml#L1-L204), [.github/workflows/winget.yml:1-34](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.github/workflows/winget.yml#L1-L34)
<!-- END:AUTOGEN pandamux_13_release_ci -->

---

<!-- BEGIN:AUTOGEN pandamux_13_release_checklist -->
## Release Checklist

`CLAUDE.md` maintains an explicit manual-release checklist, since the manual ASAR flow has no CI to enforce these invariants automatically.

| Check | Why it matters |
|---|---|
| `pnpm run build:main` succeeds | Base requirement before packaging can start ([CLAUDE.md:281](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L281)) |
| `pnpm exec vite build` succeeds | Renderer bundle must be current ([CLAUDE.md:282](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L282)) |
| Compiled code verified (grep `dist/` for the intended change) | Catches a build that silently didn't pick up the fix ([CLAUDE.md:283](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L283)) |
| ASAR packed with `--unpack-dir node_modules/node-pty/prebuilds` (not `--unpack` glob) | The glob form silently fails on Git Bash for Windows ([CLAUDE.md:284](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L284)) |
| ASAR size is ~24M | 80M+ means natives weren't unpacked; 180M+ means staging got polluted ([CLAUDE.md:285](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L285)) |
| node-pty natives present under `app.asar.unpacked/node_modules/node-pty/prebuilds/win32-x64/` | Confirms conpty/pty binaries actually shipped ([CLAUDE.md:286](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L286)) |
| PR-specific markers grep-confirmed inside the packed ASAR (extracted to /tmp) | Verifies the shipped bits actually contain the intended fixes ([CLAUDE.md:287](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L287)) |
| pandamux-orchestrator plugin copied to release staging | Plugin is bundled with every release, not built separately ([CLAUDE.md:288](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L288)) |
| rcedit applied (icon + version metadata), `{ rcedit }` destructured | Prevents the "rcedit is not a function" failure mode ([CLAUDE.md:289](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L289)) |
| `latest.yml` generated (sha512 + size of the final zip) and uploaded as a release asset | electron-updater 404s on every launch without it (issue #68) ([CLAUDE.md:290](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L290)) |
| Zip created and uploaded to the GitHub release | Final publish step ([CLAUDE.md:291](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L291)) |
| Mark of the Web: remind user to right-click > Unblock after download | Downloaded zips carry an NTFS `Zone.Identifier` stream that can prevent clean extraction/execution ([CLAUDE.md:292](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L292), [CLAUDE.md:301](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L301)) |

After tagging and publishing, the final manual step is a cleanup pass removing the temporary staging directories: `rm -rf .asar-staging build-out /tmp/asar-verify ../pandamux-release-staging` ([CLAUDE.md:275-276](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L275-L276)). Version bumps and changelog updates that precede a release commit follow the separate pre-commit convention in `.claude/rules/commit-changelog.md`, which requires updating `CHANGELOG.md` and bumping `package.json`'s SemVer before every commit, not just release commits ([.claude/rules/commit-changelog.md](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.claude/rules/commit-changelog.md)).

Sources: [CLAUDE.md:279-304](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L279-L304), [.claude/rules/commit-changelog.md](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/.claude/rules/commit-changelog.md)
<!-- END:AUTOGEN pandamux_13_release_checklist -->

---
