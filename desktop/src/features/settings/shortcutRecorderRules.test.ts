import type { TFunction } from "i18next";
import type { CommandId } from "../keyboard/catalog";
import { describe, expect, it } from "vitest";
import { heldModifiers, interpretPress, refusalText } from "./shortcutRecorderRules";

/**
 * The recorder's rules, without a DOM.
 *
 * The page's own tests drive these through the real control; what they cannot reach
 * is what a *particular platform* does (the page reads the running one) and the two
 * presses real hardware produces but a synthetic event does not: AltGr, which types
 * `{` `[` `@` on Windows and Linux, and a key the parser cannot name at all. Both
 * have to be refused rather than recorded.
 */
type Press = Parameters<typeof interpretPress>[0];

function press(overrides: Partial<Press> = {}): Press {
  return { altKey: false, ctrlKey: false, key: "", metaKey: false, shiftKey: false, ...overrides };
}

const passthrough = ((key: string) => key) as unknown as TFunction<"shortcuts">;

describe("interpretPress", () => {
  it("keeps waiting while only a modifier is held", () => {
    // The escape hatch the caller uses to show the held glyphs.
    expect(interpretPress(press({ ctrlKey: true, key: "Control" }), "chat.new", false)).toBeNull();
    expect(interpretPress(press({ key: "Shift", shiftKey: true }), "chat.new", true)).toBeNull();
  });

  it("refuses AltGr, which is how the layout types punctuation", () => {
    // `Ctrl+Alt+7` is `{` on a German layout: a command there would fire mid-word.
    const altGraph = press({ altKey: true, ctrlKey: true, getModifierState: key => key === "AltGraph", key: "7" });
    expect(interpretPress(altGraph, "chat.new", false)).toEqual({ problem: "unsupported" });
  });

  it("refuses a press it cannot name", () => {
    // A dead key or an unidentified press: there is no key to store.
    expect(interpretPress(press({ ctrlKey: true, key: "Unidentified" }), "chat.new", false)).toEqual({ problem: "unsupported" });
  });

  it("refuses a bare key, because that is ordinary typing", () => {
    expect(interpretPress(press({ key: "k" }), "chat.new", false)).toEqual({ problem: "needsModifier" });
    // Shift alone is how everyone types a capital.
    expect(interpretPress(press({ key: "K", shiftKey: true }), "chat.new", false)).toEqual({ problem: "needsModifier" });
  });

  it("allows a bare key for a command that lives in a mode's keymap", () => {
    // The Vibe panel owns `←` / `]` while it is open, so its rows may record them.
    expect(interpretPress(press({ key: "]" }), "vibe.menu", false, true)).toEqual({ binding: "BracketRight" });
  });

  it("refuses the system's own combinations", () => {
    expect(interpretPress(press({ ctrlKey: true, key: "c" }), "chat.new", false)).toEqual({ problem: "reserved" });
  });

  it("names the command a press would collide with", () => {
    const result = interpretPress(press({ ctrlKey: true, key: "f" }), "chat.new", false);
    expect(result && "problem" in result && result.problem).toBe("conflict");
    expect(result && "conflict" in result ? result.conflict?.id : undefined).toBe("thread.find");
  });

  it("records what it accepts, in the platform's notation", () => {
    expect(interpretPress(press({ ctrlKey: true, key: "K", shiftKey: true }), "chat.new", false)).toEqual({
      binding: "mod+shift+KeyK",
    });
    // On macOS the accelerator is ⌘, so a held ⌃ is a literal ctrl.
    expect(interpretPress(press({ ctrlKey: true, key: "K" }), "chat.new", true)).toEqual({ binding: "ctrl+KeyK" });
    expect(interpretPress(press({ key: "K", metaKey: true }), "chat.new", true)).toEqual({ binding: "mod+KeyK" });
  });
});

describe("heldModifiers", () => {
  it("prints Apple's glyphs, in Apple's order", () => {
    expect(heldModifiers({ altKey: true, ctrlKey: true, metaKey: true, shiftKey: true }, true)).toBe("⌃⌥⇧⌘");
    expect(heldModifiers({ altKey: false, ctrlKey: true, metaKey: false, shiftKey: false }, true)).toBe("⌃");
    expect(heldModifiers({ altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }, true)).toBe("");
  });

  it("prints the other platforms' modifier names", () => {
    expect(heldModifiers({ altKey: true, ctrlKey: false, metaKey: false, shiftKey: false }, false)).toBe("Alt+");
    expect(heldModifiers({ altKey: false, ctrlKey: true, metaKey: false, shiftKey: true }, false)).toBe("Ctrl+Shift+");
    // ⌘ means nothing away from macOS and is not shown.
    expect(heldModifiers({ altKey: false, ctrlKey: false, metaKey: true, shiftKey: false }, false)).toBe("");
  });
});

describe("refusalText", () => {
  it("says what to do about each refusal", () => {
    const command = { id: "thread.find" } as never;
    expect(refusalText(passthrough, { problem: "needsModifier" }, false)).toBe("recorder.needsModifierOther");
    expect(refusalText(passthrough, { problem: "needsModifier" }, true)).toBe("recorder.needsModifierMac");
    expect(refusalText(passthrough, { problem: "reserved" }, false)).toBe("recorder.reserved");
    expect(refusalText(passthrough, { conflict: command, problem: "conflict" }, false)).toBe("recorder.conflict");
    expect(refusalText(passthrough, { problem: "unsupported" }, false)).toBe("recorder.unsupported");
  });

  it("does not name a command when the conflict carries none", () => {
    // A conflict recorded without the spec still has to read as a sentence rather
    // than as "already used by undefined".
    const calls: unknown[] = [];
    const t = ((key: string, options?: unknown) => {
      calls.push([key, options]);
      return key;
    }) as unknown as TFunction<"shortcuts">;
    expect(refusalText(t, { problem: "conflict" }, false)).toBe("recorder.conflict");
    expect(calls).toEqual([["recorder.conflict", { command: "" }]]);
  });

  it("is typed by the command id when there is one", () => {
    const t = ((key: string) => key) as unknown as TFunction<"shortcuts">;
    expect(refusalText(t, { conflict: { id: "thread.find" } as never, problem: "conflict" }, false)).toBe(
      "recorder.conflict",
    );
  });
});

describe("the command id namespace", () => {
  it("accepts a mode command as the recording target", () => {
    // A guard against the two vocabularies drifting apart: a `CommandId` the rules
    // cannot name is one the recorder would refuse for the wrong reason.
    const ids: CommandId[] = ["chat.new", "thread.find"];
    for (const id of ids)
      expect(interpretPress(press({ ctrlKey: true, key: "K", shiftKey: true }), id, false)).not.toBeNull();
  });
});
