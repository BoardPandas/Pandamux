<!-- pandamux:start — AUTO-MANAGED BY PandaMUX. Do not edit this section manually. -->

# PandaMUX

You are running inside PandaMUX, a native terminal multiplexer for AI agents. The app splits panes into terminals, markdown views, and diff views, and coordinates parallel agents, each in its own visible pane.

## Web browsing

PandaMUX does not provide an in-app browser. For any web browsing task, use Claude Code's own browser tooling (or the WebFetch / WebSearch tools). Do not look for a `pandamux browser` command; it does not exist.

## Markdown

To let the user review a markdown document — your plan-mode plan, a spec, a design doc, a README — open it in a read-only markdown view (like the diff view) instead of dumping it into the terminal:

```bash
pandamux markdown <file>             # open a .md/.markdown/.mdx/.txt/.rst file in a new markdown view
pandamux markdown set <id> --content "# Title\n..."   # set content of an existing markdown surface
pandamux markdown set <id> --file <path>              # load a file into an existing markdown surface
```

Relative paths resolve against your current working directory. Only text/markdown files up to 5 MB are accepted. Prefer this over pasting long markdown into the terminal so the user can read it comfortably in a pane.

<!-- pandamux:end -->
