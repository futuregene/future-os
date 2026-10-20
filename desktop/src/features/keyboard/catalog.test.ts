import { describe, expect, it } from "vitest";
import enSettings from "../../i18n/locales/en/settings.json";
import en from "../../i18n/locales/en/shortcuts.json";
import zhSettings from "../../i18n/locales/zh/settings.json";
import zh from "../../i18n/locales/zh/shortcuts.json";
import { bindingProblem, parseBinding, serializeBinding } from "../../lib/keybinding";
import { CATEGORIES, COMMANDS, defaultBindingTexts, everyDefaultBindingText, FIXED_KEYS, isCommandId, scopeOf } from "./catalog";

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
      // A bare key is legal only inside a scope: the global keymap must be
      // modifier chords, because the app has no way to know what a bare key was
      // aimed at outside a mode.
      const allowBare = scopeOf(spec) !== "global";
      for (const text of everyDefaultBindingText(spec)) {
        const binding = parseBinding(text)!;
        expect(serializeBinding(binding), `${spec.id} (${text})`).not.toBe("");
        for (const isMac of [true, false])
          expect(bindingProblem(binding, isMac, { allowBare }), `${spec.id} (${text}) on ${isMac ? "mac" : "other"}`).toBeNull();
      }
    }
  });

  it("gives every command its own default within its scope", () => {
    // Uniqueness is per keymap, not per app: a mode is allowed to put a key where a
    // global command already has one (that is how the panel takes over `←`), and
    // two commands in *one* keymap still may not share a key.
    const seen = new Map<string, string>();
    for (const spec of COMMANDS) {
      // Aliases count: a remote control's button is still a key in that keymap,
      // and two commands answering to it would be as ambiguous as two chords.
      for (const text of everyDefaultBindingText(spec)) {
        const key = `${scopeOf(spec)}|${serializeBinding(parseBinding(text)!)}`;
        expect(seen.get(key), `${spec.id} collides with ${seen.get(key)} in ${scopeOf(spec)}`).toBeUndefined();
        seen.set(key, spec.id);
      }
    }
  });

  it("scopes the panel's bare keys and nothing else", () => {
    const bare = COMMANDS
      .filter(spec => defaultBindingTexts(spec).every(text => !/mod|ctrl|alt/.test(text)))
      .map(spec => spec.id)
      .sort();
    expect(bare).toEqual([
      "vibe.back",
      "vibe.block.next",
      "vibe.block.previous",
      "vibe.cell",
      "vibe.close",
      "vibe.focus.down",
      "vibe.focus.left",
      "vibe.focus.right",
      "vibe.focus.up",
      "vibe.home",
      "vibe.input.close",
      "vibe.input.open",
      "vibe.input.send",
      "vibe.open",
      "vibe.zoom.next",
      "vibe.zoom.previous",
    ]);
    for (const id of bare)
      expect(scopeOf(COMMANDS.find(spec => spec.id === id)!), id).not.toBe("global");
  });

  it("runs over the editor only in the layer that operates it", () => {
    // A bare key is the text field's — except in the input layer, where `↩`
    // sending *is* how the field is operated. Nothing else may do it.
    expect(COMMANDS.filter(spec => spec.overEditable).map(spec => spec.id).sort()).toEqual([
      "vibe.home",
      "vibe.input.close",
      "vibe.input.send",
      "vibe.menu",
    ]);
    for (const spec of COMMANDS.filter(one => one.overEditable)) {
      // Reaching over an editor is only meaningful for a key the panel binds
      // itself; a global command must never take a keystroke from a text field.
      expect(scopeOf(spec), spec.id).not.toBe("global");
    }
  });

  it("keeps the device's keys on their command", () => {
    // These are the remote control's buttons: rebinding the chord, or clearing the
    // binding entirely, must not take them away — that is what `aliases` is for.
    const aliased = COMMANDS
      .filter(spec => (spec.aliases?.length ?? 0) > 0)
      .map(spec => `${spec.id}: ${spec.aliases!.join(", ")}`)
      .sort();
    expect(aliased).toEqual([
      "vibe.back: browserback",
      "vibe.close: browserback",
      "vibe.input.close: browserback",
      "vibe.menu: contextmenu",
    ]);
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
      "vibe.block.next",
      "vibe.block.previous",
      "vibe.focus.down",
      "vibe.focus.left",
      "vibe.focus.right",
      "vibe.focus.up",
      "vibe.zoom.next",
      "vibe.zoom.previous",
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
