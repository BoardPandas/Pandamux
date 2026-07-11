<!-- PAGE_ID: pandamux_02_getting-started -->
<details>
<summary>Relevant source files</summary>

The following files were used as evidence for this page (6 files; this page's TOC entry lists exactly these, all consulted in full):

- [package.json:1-61](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L1-L61)
- [pnpm-workspace.yaml:1-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L1-L27)
- [README.md:152-160](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/README.md#L152-L160)
- [CLAUDE.md:15-40](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L15-L40)
- [tsconfig.json:1-22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/tsconfig.json#L1-L22)
- [tsconfig.node.json:1-18](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/tsconfig.node.json#L1-L18)

</details>

# Getting Started

> **Related Pages**: [Overview](OVERVIEW.md), [Release and Packaging](operations/RELEASE.md)

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_prerequisites -->
## Prerequisites

PandaMUX pins its toolchain rather than accepting a version range, so the versions below are enforced, not merely recommended.

| Tool | Version | Why |
|---|---|---|
| Node.js | `>=24.18.0` (24 LTS) | Runtime for Electron main/renderer builds and the CLI ([package.json:7-10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L7-L10)). |
| pnpm | `>=11.10.0`, pinned via `packageManager` | Sole supported package manager; enable it through corepack so the pinned version is used automatically (`corepack enable pnpm`), since a globally installed pnpm is shadowed by the corepack shim ([package.json:6](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L6), [CLAUDE.md:17](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L17)). |
| Windows 10/11 x64 | n/a | Target platform: node-pty uses ConPTY, and the release flow packages a Windows portable zip ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)). |
| Python / VS Build Tools | Only if adding a non-N-API native dependency | Not needed for a normal install: node-pty ships N-API prebuilds and is never rebuilt from source ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)). |

pnpm's own settings (node linker, allowed build scripts) live in `pnpm-workspace.yaml` rather than `.npmrc`, since pnpm 11 moved workspace/build configuration there ([pnpm-workspace.yaml:1-3](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L1-L3)).

Sources: [package.json:6-10](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L6-L10), [CLAUDE.md:15-30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L15-L30), [pnpm-workspace.yaml:1-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L1-L27)
<!-- END:AUTOGEN pandamux_02_getting-started_prerequisites -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_installation -->
## Installation

Clone the repository and install with pnpm. The install runs `allowBuilds` (approved native/build scripts) plus any postinstall steps ([CLAUDE.md:20](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L20)).

```bash
git clone https://github.com/BoardPandas/Pandamux.git
cd Pandamux
corepack enable pnpm
pnpm install
```

Note: `README.md`'s "From source" section still shows an older `npm install` / `npm run build:main` / `npm run dev` sequence from before the pnpm migration ([README.md:154-160](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/README.md#L154-L160)); CLAUDE.md is the current source of truth and the commands above (pnpm, not npm) are what this repo actually builds with ([CLAUDE.md:17](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L17)).

`pnpm-workspace.yaml` requires `nodeLinker: hoisted`, a flat symlink-free `node_modules`, because node-pty's native addon resolution and the ASAR packaging step both assume a hoisted tree ([pnpm-workspace.yaml:10-13](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L10-L13)). It also whitelists the packages allowed to run install/build scripts under pnpm 11's supply-chain hardening: `node-pty` (native ConPTY addon), `electron` (postinstall downloads the Electron binary), and `esbuild` (Vite's postinstall binary); `electron-winstaller` is explicitly disabled since this repo never builds the NSIS/Squirrel installer ([pnpm-workspace.yaml:19-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L19-L27)).

Sources: [CLAUDE.md:17-20](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L17-L20), [pnpm-workspace.yaml:1-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L1-L27), [README.md:152-160](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/README.md#L152-L160)
<!-- END:AUTOGEN pandamux_02_getting-started_installation -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_dev -->
## Development Workflow

The `dev` script runs Vite and Electron concurrently: Vite serves the renderer on port 5199, and `wait-on` holds Electron until that port answers before launching the app with hot-reload ([package.json:16](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L16)).

```bash
pnpm run dev           # Vite (port 5199) + Electron hot-reload
```

For faster iteration on main-process, preload, or CLI code without rebuilding the renderer, run only the TypeScript compile step:

```bash
pnpm run build:main    # tsc main/preload/cli only (fast iteration)
```

`build:main` compiles under `tsconfig.node.json`: CommonJS output, `outDir: dist`, `rootDir: src`, covering `src/main`, `src/preload`, `src/shared`, and `src/cli` ([tsconfig.node.json:1-18](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/tsconfig.node.json#L1-L18)). The renderer instead compiles under `tsconfig.json` with `moduleResolution: bundler`, JSX, DOM libs, and the `@renderer/*` / `@shared/*` path aliases, covering `src/renderer` and `src/shared` ([tsconfig.json:1-22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/tsconfig.json#L1-L22)).

Sources: [package.json:16](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L16), [tsconfig.json:1-22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/tsconfig.json#L1-L22), [tsconfig.node.json:1-18](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/tsconfig.node.json#L1-L18)
<!-- END:AUTOGEN pandamux_02_getting-started_dev -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_build -->
## Build Scripts

All scripts are declared in `package.json` and invoked with `pnpm run <script>` (root scripts also run bare, e.g. `pnpm test`, since `pnpm-workspace.yaml` marks the repo root as the sole workspace package) ([pnpm-workspace.yaml:4-8](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L4-L8)).

| Script | Command | Purpose |
|---|---|---|
| `dev` | `concurrently "vite --port 5199" "wait-on http://localhost:5199 && electron ."` | Renderer dev server plus Electron hot-reload ([package.json:16](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L16)). |
| `build` | `tsc -p tsconfig.node.json && vite build && electron-builder` | Full production build: main/preload/cli compile, renderer bundle, then electron-builder packaging ([package.json:17](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L17)). |
| `build:main` | `tsc -p tsconfig.node.json` | Compiles main, preload, and CLI only, for fast iteration ([package.json:18](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L18)). |
| `build:renderer` | `vite build` | Production Vite build of the renderer only ([package.json:19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L19)). |
| `test` | `vitest run` | Runs the Vitest suite once ([package.json:20](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L20)). |
| `test:watch` | `vitest` | Vitest in watch mode ([package.json:21](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L21)). |
| `lint` | `eslint src/` | Lints the `src/` tree ([package.json:22](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L22)). |

Note that the official release process does not use the `build` script's `electron-builder` packaging step for the final artifact; it uses manual ASAR staging plus a portable zip instead (see [Release and Packaging](operations/RELEASE.md)) ([CLAUDE.md:38](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L38)).

Sources: [package.json:15-23](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L15-L23), [CLAUDE.md:38](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L38)
<!-- END:AUTOGEN pandamux_02_getting-started_build -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_testing -->
## Running Tests

Unit tests run under Vitest, either as a full one-shot run, in watch mode, or scoped to a single file.

```bash
pnpm test                   # Run all unit tests
pnpm run test:watch         # Watch mode
pnpm exec vitest run tests/unit/pty-manager.test.ts  # Single file
```

Test files live under `tests/unit/` and currently cover: `agent-manager`, `cdp-bridge`, `config-loader`, `notification-slice`, `pipe-server`, `port-scanner`, `pty-manager`, `session-persistence`, `shell-detector`, and `split-tree` ([CLAUDE.md:452](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L452)). The underlying `vitest` and `test`/`test:watch` scripts are the same ones defined for the toolchain upgrade in `package.json` ([package.json:20-21](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L20-L21), [package.json:58](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L58)).

Sources: [CLAUDE.md:444-452](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L444-L452), [package.json:20-21](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/package.json#L20-L21)
<!-- END:AUTOGEN pandamux_02_getting-started_testing -->

---

<!-- BEGIN:AUTOGEN pandamux_02_getting-started_native -->
## Native Dependencies and Gotchas

node-pty is the only native dependency in this repo, and it ships N-API prebuilds that are ABI-stable across both Node and Electron; the project has verified them loading under Node 24 and under Electron 33 / ABI 130 / N-API 9 ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)).

Deliberate choices that follow from trusting the prebuilds:

- There is no `install-app-deps` postinstall step, and `electron-builder.json` sets `"npmRebuild": false`, so node-pty is never rebuilt from source ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)).
- This avoids node-pty's flaky legacy winpty gyp build and means a normal `pnpm install` needs no Python or Visual Studio Build Tools toolchain ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)).
- If a future dependency needs a non-N-API native build, a rebuild step must be reintroduced; on Python 3.12+, `pip install setuptools` is required first, since node-gyp expects the `distutils` module that Python removed ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)).
- `pnpm-workspace.yaml`'s `allowBuilds` explicitly permits `node-pty` (compiles/fetches its native addon) and `electron` (postinstall downloads the Electron binary), while `esbuild` is allowed for Vite, and `electron-winstaller` is explicitly set to `false` since this repo never builds the Windows installer ([pnpm-workspace.yaml:19-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L19-L27)).

### Known build gotcha: paths with spaces

The original checkout for this project lived under a OneDrive path containing spaces, which broke `npm link` / `node-gyp` (unable to build node-pty) and `electron-builder`'s winCodeSign step (symlink errors) ([CLAUDE.md:34-36](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L34-L36)). A checkout path with no spaces (e.g. `D:\Dev\Repos\Pandamux`) avoids both problems; either way, the actual release flow uses ASAR-based manual packaging rather than `electron-builder` for the final artifact, sidestepping winCodeSign entirely ([CLAUDE.md:38](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L38)).

| Symptom | Cause | Fix |
|---|---|---|
| `node-gyp` / `npm link` fails building node-pty | Checkout path contains spaces (e.g. under OneDrive) ([CLAUDE.md:34-35](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L34-L35)) | Use a checkout path without spaces; a normal `pnpm install` does not rebuild node-pty at all since it ships N-API prebuilds ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)). |
| `electron-builder` winCodeSign symlink errors | Checkout path contains spaces ([CLAUDE.md:36](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L36)) | Move the checkout to a path without spaces, or skip `electron-builder` and follow the ASAR-based release flow instead ([CLAUDE.md:38](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L38)). |
| `node-gyp` cannot find `distutils` when building a native dependency | Python 3.12+ removed the `distutils` module that node-gyp expects ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)) | Run `pip install setuptools` before installing, only relevant if you add a non-N-API native dependency ([CLAUDE.md:30](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30)). |
| `pnpm install` fails or skips a native build script | Package not listed in `pnpm-workspace.yaml`'s `allowBuilds` (pnpm 11 blocks dependency build scripts by default) ([pnpm-workspace.yaml:15-19](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L15-L19)) | Add the package to `allowBuilds: true` in `pnpm-workspace.yaml` if its build script is genuinely required. |

Sources: [CLAUDE.md:30-38](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/CLAUDE.md#L30-L38), [pnpm-workspace.yaml:10-27](https://github.com/BoardPandas/Pandamux/blob/0ab9e6463a9017a7b8ea98f10b3f847507658ac4/pnpm-workspace.yaml#L10-L27)
<!-- END:AUTOGEN pandamux_02_getting-started_native -->

---
