/**
 * Built-in help screen rendering.  A pure function: takes a terminal width
 * and returns the formatted two-column help card lines (ANSI-styled).
 */
import { fg, bold } from "./theme.js";
import { visibleWidth } from "./utils.js";

export function renderHelp(W: number): string[] {
  const dim_ = (t: string) => fg(245, t);
  const acc = (t: string) => fg(151, t);
  const bold_ = (t: string) => fg(252, bold(t));

  const innerW = W - 4;
  const leftCol = [
    acc("Shortcuts:"),
    dim_("  ctrl+c  interrupt"),
    dim_("  ctrl+p  cycle model"),
    dim_("  ctrl+r  browse sessions"),
    dim_("  ctrl+t  cycle thinking"),
    dim_("  tab     autocomplete"),
    dim_("  \u2191\u2193    scroll / navigate"),
    dim_("  enter   submit / accept"),
    dim_("  escape  close popup"),
  ];
  const rightCol = [
    acc("/commands:"),
    dim_("  /model [name]  select model"),
    dim_("  /sessions   browse sessions"),
    dim_("  /new       new session"),
    dim_("  /scoped-models  configure model scope"),
    dim_("  /compact   compact context"),
    dim_("  /clone     clone session"),
    dim_("  /fork      fork session"),
    dim_("  /tree      session tree"),
    dim_("  /name [n]  set session name"),
    dim_("  /help"),
  ];

  const lines: string[] = [];
  const colW = Math.floor(innerW / 2);
  const maxRows = Math.max(leftCol.length, rightCol.length);

  lines.push(dim_("\u250c" + "\u2500".repeat(W - 2) + "\u2510"));
  lines.push(dim_("\u2502") + "  " + bold_("future-tui") + "  " + dim_("Terminal UI Help") + " ".repeat(Math.max(0, W - 24)) + dim_("\u2502"));
  lines.push(dim_("\u251c" + "\u2500".repeat(W - 2) + "\u2524"));

  for (let i = 0; i < maxRows; i++) {
    const l = leftCol[i] || "";
    const r = rightCol[i] || "";
    const lPad = colW - visibleWidth(l);
    const rPad = colW - visibleWidth(r);
    lines.push(dim_("\u2502") + "  " + l + " ".repeat(Math.max(1, lPad)) + r + " ".repeat(Math.max(1, rPad)) + dim_("\u2502"));
  }

  lines.push(dim_("\u2514" + "\u2500".repeat(W - 2) + "\u2518"));
  return lines;
}
