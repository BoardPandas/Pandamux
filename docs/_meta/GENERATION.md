# Generation Metadata

- **Commit:** `3accc145c64c10d86785466dbe59b344f6100665`
- **Branch:** `master`
- **Generated:** 2026-07-17
- **Mode:** update (full regeneration)
- **Base commit:** `0ab9e6463a9017a7b8ea98f10b3f847507658ac4`
- **Pages generated:** 15
- **Sections generated:** 79

## Notes

The base commit (`0ab9e64`, 2026-07-05) predates the Electron-to-Rust rewrite. The 69 intervening commits replaced the entire TypeScript/Electron codebase with the Rust workspace, so this run regenerated every page against `pandamux-core`/`pandamux-term`/`pandamux-ui`/`pandamux-app`/`pandamux-cli` rather than performing a section-scoped incremental update. The four Electron-only pages with no Rust successor (MAIN_PROCESS, RENDERER_AND_STATE, BROWSER_CDP, AI_INTEGRATION) were moved to `docs/archive/`.
