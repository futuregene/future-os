/**
 * Built-in help screen rendering.  A pure function: takes a terminal width
 * and returns the formatted help card lines (ANSI-styled).  The command list
 * mirrors the slash commands handled by `app.ts` (dispatch + autocomplete).
 */
import { fg, bold } from "./theme.js";
import { visibleWidth, truncateToWidth } from "./utils.js";

interface HelpEntry {
  key: string;
  desc: string;
}

const SHORTCUTS: HelpEntry[] = [
  { key: "ctrl+c", desc: "interrupt" },
  { key: "ctrl+p", desc: "cycle model" },
  { key: "ctrl+r", desc: "browse sessions" },
  { key: "ctrl+t", desc: "cycle thinking" },
  { key: "tab", desc: "autocomplete" },
  { key: "\u2191\u2193", desc: "scroll / navigate" },
  { key: "enter", desc: "submit / accept" },
  { key: "escape", desc: "close popup" },
];

const COMMANDS: HelpEntry[] = [
  { key: "/model [name]", desc: "select model" },
  { key: "/new", desc: "start a new session" },
  { key: "/sessions", desc: "browse and switch sessions" },
  { key: "/compact", desc: "compress conversation context" },
  { key: "/scoped-models", desc: "configure model enable/disable list" },
  { key: "/clone", desc: "clone the current session" },
  { key: "/fork", desc: "fork the current session" },
  { key: "/tree", desc: "session tree with fork/clone hierarchy" },
  { key: "/name [n]", desc: "set the session name" },
  { key: "/status", desc: "session state, token usage, cost" },
  { key: "/stop", desc: "abort current generation" },
  { key: "/cwd", desc: "change the working directory" },
  { key: "/approve", desc: "approve pending tool execution" },
  { key: "/reject", desc: "reject pending tool execution" },
  { key: "/cancel <run-id>", desc: "cancel a queued run" },
  { key: "/reload", desc: "reload skills and context" },
  { key: "/help", desc: "show all commands and shortcuts" },
];

export function renderHelp(W: number): string[] {
  const dim_ = (t: string) => fg(245, t);
  const acc = (t: string) => fg(151, t);
  const bold_ = (t: string) => fg(252, bold(t));

  const innerW = W - 4; // card body width: 2 borders + 2-space gutter

  const lines: string[] = [];
  // Push one card row: border + gutter + content + pad to body width + border.
  const push = (row: string) => {
    const clipped = visibleWidth(row) > innerW ? truncateToWidth(row, innerW) : row;
    lines.push(dim_("\u2502") + "  " + clipped + " ".repeat(Math.max(0, innerW - visibleWidth(clipped))) + dim_("\u2502"));
  };

  lines.push(dim_("\u250c" + "\u2500".repeat(Math.max(0, W - 2)) + "\u2510"));
  lines.push(
    dim_("\u2502") + "  " + bold_("future-tui") + "  " + dim_("Terminal UI Help") + " ".repeat(Math.max(0, innerW - 28)) + dim_("\u2502"),
  );
  lines.push(dim_("\u251c" + "\u2500".repeat(Math.max(0, W - 2)) + "\u2524"));

  push(acc("Shortcuts:"));
  for (const { key, desc } of SHORTCUTS) {
    push(dim_(`${key.padEnd(8)} ${desc}`));
  }

  push("");
  push(acc("/commands:"));

  const keyW = Math.max(...COMMANDS.map((c) => visibleWidth(c.key)));
  for (const { key, desc } of COMMANDS) {
    push(dim_(`${key.padEnd(keyW + 2)}${desc}`));
  }

  lines.push(dim_("\u2514" + "\u2500".repeat(Math.max(0, W - 2)) + "\u2518"));
  return lines;
}
