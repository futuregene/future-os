/**
 * Regression tests for the help screen:
 * - Lists every slash command dispatched by `app.ts` (was missing
 *   /status /stop /cwd /approve /reject /cancel /reload, and /compact
 *   from autocomplete)
 * - Every card row is exactly the requested terminal width (the original
 *   two-column card overflowed the width)
 */

import { describe, it, expect } from "bun:test";
import { renderHelp } from "../help-screen.js";
import { visibleWidth, stripAnsiCodes } from "../utils.js";

const EXPECTED_COMMANDS = [
  "/model [name]",
  "/new",
  "/sessions",
  "/compact",
  "/scoped-models",
  "/clone",
  "/fork",
  "/tree",
  "/name [n]",
  "/status",
  "/stop",
  "/cwd",
  "/approve",
  "/reject",
  "/cancel <run-id>",
  "/reload",
  "/help",
];

describe("renderHelp", () => {
  it("lists every slash command handled by the TUI", () => {
    const text = renderHelp(80).map(stripAnsiCodes).join("\n");
    for (const cmd of EXPECTED_COMMANDS) {
      expect(text).toContain(cmd);
    }
  });

  it("renders every row at exactly the requested width", () => {
    for (const width of [40, 60, 80, 120]) {
      const rows = renderHelp(width).map(visibleWidth);
      expect(new Set(rows).size).toBe(1);
      expect(rows[0]!).toBe(width);
    }
  });

  it("keeps ANSI codes intact (no dangling escapes after truncation)", () => {
    for (const width of [40, 80]) {
      for (const line of renderHelp(width)) {
        expect(stripAnsiCodes(line)).not.toContain("\x1b");
      }
    }
  });
});
