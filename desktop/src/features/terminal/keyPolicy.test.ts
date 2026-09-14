import type { KeyPolicyEvent } from "./keyPolicy";
import { describe, expect, it } from "vitest";
import { terminalKeyPolicy } from "./keyPolicy";

/**
 * Event shapes below are the ones the real WebKitGTK webview + ibus-libpinyin
 * delivered (captured with the WebDriver harness in the bug report): the
 * composing Backspace arrives with `keyCode 0`, the composing Enter with `229`
 * on its first keydown, and everything typed after the commit with
 * `isComposing: false`.
 */
function key(overrides: Partial<KeyPolicyEvent> = {}): KeyPolicyEvent {
  return { key: "j", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, isComposing: false, ...overrides };
}

describe("terminalKeyPolicy", () => {
  it("leaves ordinary terminal input to xterm", () => {
    expect(terminalKeyPolicy(key({ key: "a" }), false)).toBe(true);
    expect(terminalKeyPolicy(key({ key: "c", ctrlKey: true }), false)).toBe(true);
    expect(terminalKeyPolicy(key({ key: "Enter" }), false)).toBe(true);
    expect(terminalKeyPolicy(key({ key: "\x7F" }), false)).toBe(true);
  });

  it("releases the panel shortcut so the window listener can collapse the panel", () => {
    expect(terminalKeyPolicy(key({ ctrlKey: true }), false)).toBe(false);
    expect(terminalKeyPolicy(key({ metaKey: true }), true)).toBe(false);
  });

  it("stands down for the keys an IME is composing, whatever their keyCode", () => {
    // A composing Backspace: WebKit reports keyCode 0, so xterm's `229` check
    // does not recognise it and used to commit the half-typed composition.
    expect(terminalKeyPolicy(key({ key: "\u0008", isComposing: true }), false)).toBe(false);
    // A composing Enter, after which the terminal must not also run the line.
    expect(terminalKeyPolicy(key({ key: "Enter", isComposing: true }), false)).toBe(false);
    // The pinyin letters themselves.
    expect(terminalKeyPolicy(key({ key: "n", isComposing: true }), false)).toBe(false);
    // Candidate navigation, should an engine deliver it.
    expect(terminalKeyPolicy(key({ key: "ArrowDown", isComposing: true }), false)).toBe(false);
  });

  it("handles the terminal again as soon as the composition ends", () => {
    // The Enter that actually submits arrives after `compositionend`.
    expect(terminalKeyPolicy(key({ key: "Enter", isComposing: false }), false)).toBe(true);
    expect(terminalKeyPolicy(key({ key: "\x7F", isComposing: false }), false)).toBe(true);
  });

  it("lets the app's command through even mid-composition", () => {
    // Contract of the policy: the app's command is not text input, so it is
    // released before the composing check. Whether the browser presents a
    // recognisable `j` during a composition is up to the engine — measured on
    // WebKitGTK + ibus, the composing `j` arrives as `key: "Unidentified"` with
    // keyCode 229 (the IME consumes it and cancels the preedit), so the
    // shortcut needs a second press there. The ordering only decides the case
    // where the key *is* reported normally.
    expect(terminalKeyPolicy(key({ ctrlKey: true, isComposing: true }), false)).toBe(false);
    expect(terminalKeyPolicy(key({ metaKey: true, isComposing: true }), true)).toBe(false);
  });
});
