import { describe, expect, it } from "vitest";
import enSettings from "../../i18n/locales/en/settings.json";
import en from "../../i18n/locales/en/shortcuts.json";
import zhSettings from "../../i18n/locales/zh/settings.json";
import zh from "../../i18n/locales/zh/shortcuts.json";
import { bindingProblem, parseBinding, serializeBinding } from "../../lib/keybinding";
import { CATEGORIES, COMMANDS, defaultBinding, FIXED_KEYS, isCommandId } from "./catalog";

/** Resolve a dotted key (`commands.chat.new`) against a locale bundle. */
function lookup(bundle: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((node, part) => {
    if (typeof node !== "object" || node === null)
      return undefined;
    return (node as Record<string, unknown>)[part];
  }, bundle);
}

describe("command catalog", () => {
  it("has unique ids", () => {
    const ids = COMMANDS.map(spec => spec.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("keeps every default legal on both platforms", () => {
    for (const spec of COMMANDS) {
      const binding = defaultBinding(spec);
      expect(serializeBinding(binding), spec.id).not.toBe("");
      for (const isMac of [true, false])
        expect(bindingProblem(binding, isMac), `${spec.id} on ${isMac ? "mac" : "other"}`).toBeNull();
    }
  });

  it("gives every command its own default", () => {
    const seen = new Map<string, string>();
    for (const spec of COMMANDS) {
      const text = serializeBinding(defaultBinding(spec));
      expect(seen.get(text), `${spec.id} collides with ${seen.get(text)}`).toBeUndefined();
      seen.set(text, spec.id);
    }
  });

  it("lets the terminal's own toggle in, and nothing else", () => {
    // `Ctrl` is the accelerator on Windows and Linux, and the terminal is a
    // shell: everything else must leave `Ctrl+B`, `Ctrl+F` … to readline.
    expect(COMMANDS.filter(spec => spec.inTerminal).map(spec => spec.id)).toEqual(["terminal.toggle"]);
  });

  it("repeats only the commands that walk a list", () => {
    expect(COMMANDS.filter(spec => spec.repeat).map(spec => spec.id).sort()).toEqual([
      "group.next",
      "group.previous",
      "thread.next",
      "thread.previous",
    ]);
  });

  it("knows its own ids and nothing else", () => {
    expect(isCommandId("chat.new")).toBe(true);
    expect(isCommandId("chat.nope")).toBe(false);
  });

  it("names every command and category in both languages", () => {
    for (const [language, bundle] of [["en", en], ["zh", zh]] as const) {
      for (const spec of COMMANDS)
        expect(lookup(bundle, `commands.${spec.id}`), `${language} commands.${spec.id}`).toEqual(expect.any(String));
      for (const category of CATEGORIES)
        expect(lookup(bundle, `categories.${category}`), `${language} categories.${category}`).toEqual(expect.any(String));
      for (const item of FIXED_KEYS)
        expect(lookup(bundle, item.labelKey), `${language} ${item.labelKey}`).toEqual(expect.any(String));
      for (const key of ["page.hint", "page.search", "page.unbound", "page.reset", "page.resetAll", "page.terminalBlocked", "recorder.prompt", "recorder.reserved", "recorder.conflict", "recorder.hint", "fixed.title", "fixed.description"])
        expect(lookup(bundle, key), `${language} ${key}`).toEqual(expect.any(String));
    }
  });

  it("has a settings tab label for the page it opens", () => {
    expect(lookup(enSettings, "dialog.tabs.shortcuts")).toEqual(expect.any(String));
    expect(lookup(zhSettings, "dialog.tabs.shortcuts")).toEqual(expect.any(String));
  });

  it("writes defaults in the notation the parser reads", () => {
    for (const spec of COMMANDS)
      expect(parseBinding(spec.defaultBinding), spec.id).not.toBeNull();
  });
});
