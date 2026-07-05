<!-- pandamux:start — AUTO-MANAGED BY PandaMUX Everywhere. Do not edit this section manually. -->

# PandaMUX Everywhere

You are running inside PandaMUX Everywhere, a terminal multiplexer with a browser panel on the right side that the user can see in real-time.

## Browser

For any web browsing task, use the `pandamux browser` commands so the user can watch in the browser panel. Do NOT use Playwright, Firecrawl, or WebSearch — they open invisible windows the user cannot see. If the user explicitly asks for one of those tools, use it.

```bash
pandamux browser open <url>          # navigate
pandamux browser snapshot            # get accessibility tree with @eN refs
pandamux browser click @eN           # click element
pandamux browser type @eN <text>     # type into element
pandamux browser fill @eN <value>    # set input value
pandamux browser get-text            # get page text
pandamux browser screenshot          # capture screenshot
pandamux browser eval <js>           # run JavaScript
pandamux browser back                # go back
pandamux browser forward             # go forward
pandamux browser reload              # reload page
```

Workflow: `browser open <url>` → `browser snapshot` → read tree → `browser click/type @eN` → `browser snapshot` again.

Refs (`@e1`, `@e2`...) expire after page changes — always re-snapshot.

## Markdown

To let the user review a markdown document — your plan-mode plan, a spec, a design doc, a README — open it in a read-only markdown view (like the diff view) instead of dumping it into the terminal:

```bash
pandamux markdown <file>             # open a .md/.markdown/.mdx/.txt/.rst file in a new markdown view
pandamux markdown set <id> --content "# Title\n..."   # set content of an existing markdown surface
pandamux markdown set <id> --file <path>              # load a file into an existing markdown surface
```

Relative paths resolve against your current working directory. Only text/markdown files up to 5 MB are accepted. Prefer this over pasting long markdown into the terminal so the user can read it comfortably in a pane.

<!-- pandamux:end -->
