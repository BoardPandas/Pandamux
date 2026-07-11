---
name: pandamux-detect
description: Detect if PandaMUX terminal multiplexer is running. Used internally by orchestrate skill to decide between PandaMUX multi-pane mode and degraded subagent mode.
---

# PandaMUX Detection

First, resolve the plugin root (not available as env var in main session):

```bash
PLUGIN_ROOT=$(find "$HOME/.claude/plugins/cache/pandamux-orchestrator" -name "plugin.json" -path "*/.claude-plugin/*" 2>/dev/null | sort -V | tail -1 | sed 's|/.claude-plugin/plugin.json||')
```

Run the detection script to check if PandaMUX is available:

```bash
bash "$PLUGIN_ROOT/scripts/detect-pandamux.sh"
```

**If output is "available":**
- PandaMUX is running and the named pipe is accessible
- The orchestrator can use `pandamux split`, `pandamux agent spawn`, `pandamux markdown set` etc.
- Full multi-pane visual experience is available

**If output is "unavailable":**
- PandaMUX is not running or not installed
- Fall back to Claude Code's native `Agent` tool for parallel workers
- No visual dashboard — use text summaries in the terminal instead
- Log: "PandaMUX not detected. Running in degraded mode — agents will use Claude Code's native subagent system. Install PandaMUX for the full multi-pane experience: https://pandamux.boardpandas.ai"

Store the detection result so other skills can check it without re-running:

```bash
export PANDAMUX_AVAILABLE=$( bash "$PLUGIN_ROOT/scripts/detect-pandamux.sh" 2>/dev/null && echo "true" || echo "false" )
```

**ENFORCEMENT:**
- When `PANDAMUX_AVAILABLE=true`: ALL agents MUST be spawned via `pandamux agent spawn`. Do NOT use Claude Code's `Agent` tool. The Agent tool creates invisible subagents — the user chose PandaMUX specifically to SEE agents in panes.
- When `PANDAMUX_AVAILABLE=false`: Use Claude Code's `Agent` tool with `subagent_type: "pandamux-orchestrator:pandamux-worker"`.
- Never mix modes within an orchestration.
