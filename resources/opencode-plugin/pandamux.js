// pandamux-plugin-version: 2
// pandamux OpenCode plugin — bridges OpenCode hooks/events to the pandamux sidebar.
// Auto-installed by pandamux to ~/.config/opencode/plugin/pandamux.js.
// No-ops entirely outside pandamux (PANDAMUX !== '1').
const { execFile } = require("node:child_process");

function pandamux(args) {
  // Fire-and-forget; never block or throw into OpenCode.
  try {
    const cli = process.env.PANDAMUX_CLI;
    const file = cli ? process.execPath : "pandamux";
    const argv = cli ? [cli, ...args] : args;
    execFile(file, argv, { windowsHide: true }, () => {});
  } catch {}
}

export const PandaMUXPlugin = async () => {
  if (process.env.PANDAMUX !== "1") return {};
  const surface = process.env.PANDAMUX_SURFACE_ID;
  if (!surface) return {};

  // message.part.updated fires per streaming delta (many per second). Throttle
  // the "active" pings so we don't spawn a CLI process for every token.
  let lastActivePing = 0;
  const pingActive = () => {
    const now = Date.now();
    if (now - lastActivePing < 1000) return;
    lastActivePing = now;
    pandamux(["agent-activity", "--surface", surface, "--active"]);
  };
  const activeTool = (input) => {
    const tool = String((input && input.tool) || "");
    const args = ["agent-activity", "--surface", surface, "--active"];
    if (tool) args.push("--tool", tool);
    pandamux(args);
  };

  return {
    "tool.execute.after": async (input) => {
      const tool = String((input && input.tool) || "");
      if (tool) pandamux(["hook", "--event", "PostToolUse", "--tool", tool]);
      activeTool(input);
    },
    "tool.execute.before": async (input) => {
      activeTool(input);
    },
    event: async ({ event }) => {
      if (!event || !event.type) return;
      if (event.type === "session.idle") {
        pandamux(["agent-activity", "--surface", surface, "--done"]);
      } else if (event.type === "session.error") {
        pandamux(["agent-activity", "--surface", surface, "--done"]);
      } else if (event.type === "message.part.updated") {
        pingActive();
      }
    },
    "shell.env": async (input, output) => {
      output.env.PANDAMUX = "1";
      output.env.PANDAMUX_SURFACE_ID = surface;
      if (process.env.PANDAMUX_PIPE) output.env.PANDAMUX_PIPE = process.env.PANDAMUX_PIPE;
      if (process.env.PANDAMUX_CLI) output.env.PANDAMUX_CLI = process.env.PANDAMUX_CLI;
    },
  };
};
