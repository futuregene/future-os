import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import en from "../../i18n/locales/en/terminal.json";
import { terminalPanelLabel } from "./panelLabel";

/**
 * The terminal panel's toggle, in the reader's words.
 *
 * Two affordances collapse the same panel — the header button and the panel's own ✕
 * — so the sentence lives in one place; and a user who removed the binding must not
 * read "Hide terminal ()", which is why the shortcut is only mentioned when there is
 * one to mention.
 */
const t = ((key: string, options?: Record<string, unknown>) => {
  const values = Object.entries(options ?? {})
    .map(([name, value]) => `${name}=${String(value)}`)
    .join(",");
  return values ? `${key}(${values})` : key;
}) as unknown as TFunction<"terminal">;

describe("terminalPanelLabel", () => {
  it("names the key that collapses or expands the panel", () => {
    expect(terminalPanelLabel(t, true, "Ctrl+J")).toBe("collapse(shortcut=Ctrl+J)");
    expect(terminalPanelLabel(t, false, "Ctrl+J")).toBe("expand(shortcut=Ctrl+J)");
  });

  it("says the same thing without a key when the binding was removed", () => {
    // The unbound pair exists in the locale bundle: an empty parenthesis would read
    // as a rendering bug rather than as "this has no shortcut".
    expect(terminalPanelLabel(t, true, "")).toBe("collapseNoShortcut");
    expect(terminalPanelLabel(t, false, "")).toBe("expandNoShortcut");
    expect(en).toMatchObject({
      collapseNoShortcut: expect.any(String),
      expandNoShortcut: expect.any(String),
    });
  });
});
