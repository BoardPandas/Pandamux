# PandaMUX Orchestrator — marketing site

Standalone landing page for the PandaMUX Orchestrator Claude Code plugin.

- **Production**: https://plugin.pandamux.boardpandas.ai
- **Source**: this directory
- **Parent project**: [PandaMUX Everywhere](https://pandamux.boardpandas.ai), the host multiplexer
- **Plugin repo**: https://github.com/BoardPandas/Pandamux (bundled under `resources/pandamux-orchestrator/`)

## Stack

Pure static HTML, CSS, and vanilla JavaScript. No framework, no build step. Geist Mono self-hosted in `assets/fonts/`.

- `index.html` — markup + editorial copy
- `styles.css` — full visual layer, single-accent amber palette, editorial typography
- `wave-sim.js` — interactive wave-orchestration simulator
- `hero-panes.js` — live 4-pane PandaMUX mockup in the hero
- `motion.js` — IntersectionObserver reveals, scroll progression, keyboard shortcuts
- `ambient.js` — cursor glow, grain overlay, faux-live activity rail

## Preview locally

Any static file server works. Examples:

```bash
# Python
cd site/orchestrator && python -m http.server 4200

# Node
cd site/orchestrator && npx serve -l 4200
```

Then visit http://localhost:4200.

## Deployment — `plugin.pandamux.boardpandas.ai`

This site is deployed via the same Netlify project as pandamux.boardpandas.ai (`netlify api → pandamux`, project ID `6fb46a25-ad92-4d48-b5ae-ca656dee01e0`). The repo-root `netlify.toml` publishes the entire `site/` directory and uses host-scoped redirects to serve `site/orchestrator/` under `plugin.pandamux.boardpandas.ai` while `site/` continues to serve under `pandamux.boardpandas.ai`.

```bash
# From the repo root:
npx netlify deploy --prod --dir site
```

That single command pushes both the pandamux.boardpandas.ai landing page (`site/index.html`) and the orchestrator marketing site (`site/orchestrator/`) in one deploy.

### How `plugin.pandamux.boardpandas.ai` resolves

1. **Domain alias**: `plugin.pandamux.boardpandas.ai` is registered as a `domain_alias` on the Netlify project. Netlify auto-provisions Let's Encrypt SSL.
2. **DNS**: a `NETLIFY` type record in the pandamux.boardpandas.ai Netlify-managed zone maps `plugin.pandamux.boardpandas.ai → pandamux.netlify.app`.
3. **Redirect rule**: in `netlify.toml`, two `[[redirects]]` blocks with `[redirects.conditions] Host = ["plugin.pandamux.boardpandas.ai"]` rewrite incoming requests to the `/orchestrator/` subpath. Status `200!` (force) makes the rewrite silent — the URL bar still shows `plugin.pandamux.boardpandas.ai`.

### How `pandamux.boardpandas.ai/orchestrator/` is unaffected

The host-scoped redirects only fire when `Host == plugin.pandamux.boardpandas.ai`. Direct hits on `https://pandamux.boardpandas.ai/orchestrator/` still resolve to `site/orchestrator/index.html` natively, so the same content is reachable from both URLs. Use `plugin.pandamux.boardpandas.ai` as the canonical (set in the HTML `<link rel="canonical">`).

## Separation from pandamux.boardpandas.ai

This site lives at `site/orchestrator/` and shares zero markup, styles, JS, or design tokens with `site/index.html`. The two pages have different visual systems on purpose: pandamux.boardpandas.ai is the host multiplexer's landing page; this is the plugin's editorial product page.

## License

MIT. Same as the plugin.
