import { describe, expect, it } from "vitest";
import enSettings from "../../i18n/locales/en/settings.json";
import en from "../../i18n/locales/en/shortcuts.json";
import zhSettings from "../../i18n/locales/zh/settings.json";
import zh from "../../i18n/locales/zh/shortcuts.json";
import { bindingProblem, parseBinding, serializeBinding } from "../../lib/keybinding";
import { CATEGORIES, COMMANDS, defaultBinding, defaultBindingLabel, defaultBindings, defaultBindingTexts, everyDefaultBindingText, FIXED_KEYS, isCommandId, scopeOf, sectionCommands, SECTIONS } from "./catalog";

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
      "vibe.cell",
      "vibe.close",
      "vibe.focus.down",
      "vibe.focus.left",
      "vibe.focus.right",
      "vibe.focus.up",
      "vibe.home",
      "vibe.input.close",
      "vibe.input.open",
      "vibe.open",
      "vibe.scroll.down",
      "vibe.scroll.up",
      "vibe.zoom.next",
      "vibe.zoom.previous",
      "vibe.zoom.scroll.down",
      "vibe.zoom.scroll.up",
    ]);
    for (const id of bare)
      expect(scopeOf(COMMANDS.find(spec => spec.id === id)!), id).not.toBe("global");
  });

  it("runs over the editor only in the layer that operates it", () => {
    // A bare key is the text field's — except in the input layer, where `↩`
    // sending *is* how the field is operated. Nothing else may do it.
    // `⏎` sending is *not* on this list: that key is the composer's own
    // (`FIXED_KEYS`), routed through `MentionEditor.onSubmit` rather than the
    // command table, so the two owners never share one press.
    expect(COMMANDS.filter(spec => spec.overEditable).map(spec => spec.id).sort()).toEqual([
      "vibe.home",
      "vibe.input.close",
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
      "rail.device.next",
      "thread.next",
      "thread.previous",
      "vibe.focus.down",
      "vibe.focus.left",
      "vibe.focus.right",
      "vibe.focus.up",
      "vibe.scroll.down",
      "vibe.scroll.up",
      "vibe.zoom.next",
      "vibe.zoom.previous",
      "vibe.zoom.scroll.down",
      "vibe.zoom.scroll.up",
    ]);
  });

  it("knows its own ids and nothing else", () => {
    expect(isCommandId("chat.new")).toBe(true);
    expect(isCommandId("chat.nope")).toBe(false);
  });

  it("lists every command under exactly one section, and no empty one", () => {
    // The settings page and the cheat sheet both walk `SECTIONS`, so a category
    // missing from it would drop its commands from both screens — silently, since
    // neither page can see a command it never asked for.
    const sectioned = SECTIONS.flatMap(section => section.categories);
    expect(new Set(sectioned).size).toBe(sectioned.length);
    for (const spec of COMMANDS)
      expect(sectioned, spec.id).toContain(spec.category);
    for (const section of SECTIONS)
      expect(sectionCommands(section).length, section.id).toBeGreaterThan(0);
  });

  it("names every command and category in both languages", () => {
    for (const [language, bundle] of [["en", en], ["zh", zh]] as const) {
      for (const spec of COMMANDS)
        expect(lookup(bundle, `commands.${spec.id}`), `${language} commands.${spec.id}`).toEqual(expect.any(String));
      for (const category of CATEGORIES)
        expect(lookup(bundle, `categories.${category}`), `${language} categories.${category}`).toEqual(expect.any(String));
      for (const section of SECTIONS)
        expect(lookup(bundle, `sections.${section.id}`), `${language} sections.${section.id}`).toEqual(expect.any(String));
      for (const item of FIXED_KEYS)
        expect(lookup(bundle, item.labelKey), `${language} ${item.labelKey}`).toEqual(expect.any(String));
      for (const key of ["page.hint", "page.search", "page.unbound", "page.reset", "page.resetAll", "recorder.prompt", "recorder.reserved", "recorder.conflict", "recorder.hint", "fixed.title", "fixed.description"])
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
  it("parses a command's default and labels it for the platform", () => {
    // `defaultBinding` / `defaultBindingLabel` are the table's own accessors: the
    // settings page and the cheat sheet read a command's default through them, so a
    // broken default has to surface here rather than as a missing label in the UI.
    const spec = COMMANDS.find(spec => spec.id === "chat.new")!;
    // The accessor agrees with the text it was written from — `n` and `KeyN` are the
    // same physical key, which is exactly what canonicalising is for.
    expect(serializeBinding(defaultBinding(spec))).toBe(serializeBinding(parseBinding(spec.defaultBinding)!));
    expect(defaultBindingLabel(spec, false)).toBe("Ctrl+N");
    expect(defaultBindingLabel(spec, true)).toBe("⌘N");
  });

  it("refuses to build a binding from a default the parser cannot read", () => {
    // The defaults are literals in this file; a null here means the table itself was
    // broken by an edit, which must fail loudly instead of producing a command with
    // no key.
    const broken = { ...COMMANDS[0]!, defaultBinding: "not a binding" } as (typeof COMMANDS)[number];
    expect(() => defaultBindings(broken)).toThrow(/Invalid default binding for chat\.new: not a binding/);
  });
});
