# PandaMUX Everywhere config file

PandaMUX Everywhere reads `~/.pandamux/config.toml` on startup (Windows: `%USERPROFILE%\.pandamux\config.toml`).
The file is optional — if it isn't present, built-in defaults apply.

Edit it, then run `pandamux reload-config` (or restart pandamux) to pick up changes.

## Full example

```toml
[terminal]
font-family      = "Cascadia Mono"
font-size        = 14
cursor-style     = "block"        # block | underline | bar
cursor-blink     = true
scrollback-lines = 10000

[terminal.colors]
# Default scheme for every new pane. Any bundled theme name works
# (see `pandamux list-themes`), or the key of a user-defined scheme below.
default = "Dracula"

# User-defined named schemes — override individual fields of the base theme.
# Invoke them with:   pandamux split --color-scheme prod
[terminal.colors.schemes.prod]
background = "#2b0b0b"
foreground = "#ffdddd"
cursor     = "#ff5555"

[terminal.colors.schemes.staging]
background = "#2b1f0b"
foreground = "#ffeecc"
cursor     = "#ffaa44"

[terminal.colors.schemes.dev]
background = "#0b1f0b"
foreground = "#ccffcc"
cursor     = "#55ff55"

# Full palette override (up to 16 ANSI colors) — optional.
[terminal.colors.schemes.mono]
background = "#000000"
foreground = "#ffffff"
palette = [
  "#000000", "#ff0000", "#00ff00", "#ffff00",
  "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
  "#555555", "#ff5555", "#55ff55", "#ffff55",
  "#5555ff", "#ff55ff", "#55ffff", "#ffffff",
]
```

## Precedence

1. Built-in defaults
2. Settings UI values (persisted to Zustand / localStorage)
3. **`config.toml`** — applied over 1 and 2 at startup and on `reload-config`
4. Per-pane overrides (e.g. `pandamux split --color-scheme prod`) — always win for that pane

"File wins at startup, app wins at runtime": if you tweak a value in the Settings
UI after PandaMUX Everywhere booted, your tweak sticks until the next reload.

## CLI helpers

```bash
pandamux config path      # print the config file path
pandamux config show      # dump the parsed config (useful for debugging syntax)
pandamux config reload    # re-read the file and apply to running surfaces
pandamux reload-config    # alias of `config reload`
pandamux list-themes      # print all valid `default`/`--color-scheme` names
```

## Notes

- Keys can be written either `kebab-case` or `camelCase`
  (`font-family` and `fontFamily` both work).
- `cursor` inside a scheme is the cursor color; use `cursor-style` (under `[terminal]`)
  for the shape.
- A parse error in one key is reported in `pandamux config show` but never
  aborts loading — the rest of the file still applies.
- Per-pane overrides via `pandamux split --color-scheme NAME` or
  `pandamux set-color-scheme [id] NAME` always take precedence for that surface.
