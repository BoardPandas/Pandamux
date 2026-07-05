# wmux-orchestrator — marketing site

Standalone landing page for the wmux-orchestrator Claude Code plugin.

- **Production**: https://plugin.wmux.org
- **Source**: this directory
- **Parent project**: [wmux](https://wmux.org), the host multiplexer
- **Plugin repo**: https://github.com/BoardPandas/Pandamux (bundled under `resources/wmux-orchestrator/`)

## Stack

Pure static HTML, CSS, and vanilla JavaScript. No framework, no build step. Geist Mono self-hosted in `assets/fonts/`.

- `index.html` — markup + editorial copy
- `styles.css` — full visual layer, single-accent amber palette, editorial typography
- `wave-sim.js` — interactive wave-orchestration simulator
- `hero-panes.js` — live 4-pane wmux mockup in the hero
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

## Deployment — `plugin.wmux.org`

This site is deployed via the same Netlify project as wmux.org (`netlify api → wmux`, project ID `6fb46a25-ad92-4d48-b5ae-ca656dee01e0`). The repo-root `netlify.toml` publishes the entire `site/` directory and uses host-scoped redirects to serve `site/orchestrator/` under `plugin.wmux.org` while `site/` continues to serve under `wmux.org`.

```bash
# From the repo root:
npx netlify deploy --prod --dir site
```

That single command pushes both the wmux.org landing page (`site/index.html`) and the orchestrator marketing site (`site/orchestrator/`) in one deploy.

### How `plugin.wmux.org` resolves

1. **Domain alias**: `plugin.wmux.org` is registered as a `domain_alias` on the Netlify project. Netlify auto-provisions Let's Encrypt SSL.
2. **DNS**: a `NETLIFY` type record in the wmux.org Netlify-managed zone maps `plugin.wmux.org → wmux.netlify.app`.
3. **Redirect rule**: in `netlify.toml`, two `[[redirects]]` blocks with `[redirects.conditions] Host = ["plugin.wmux.org"]` rewrite incoming requests to the `/orchestrator/` subpath. Status `200!` (force) makes the rewrite silent — the URL bar still shows `plugin.wmux.org`.

### How `wmux.org/orchestrator/` is unaffected

The host-scoped redirects only fire when `Host == plugin.wmux.org`. Direct hits on `https://wmux.org/orchestrator/` still resolve to `site/orchestrator/index.html` natively, so the same content is reachable from both URLs. Use `plugin.wmux.org` as the canonical (set in the HTML `<link rel="canonical">`).

## Separation from wmux.org

This site lives at `site/orchestrator/` and shares zero markup, styles, JS, or design tokens with `site/index.html`. The two pages have different visual systems on purpose: wmux.org is the host multiplexer's landing page; this is the plugin's editorial product page.

## License

MIT. Same as the plugin.
