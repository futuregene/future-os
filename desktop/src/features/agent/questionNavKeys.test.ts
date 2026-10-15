// @vitest-environment jsdom
/**
 * The transcript's keyboard half.
 *
 * The whole value of this policy is what it *refuses*: an unmodified arrow key
 * belongs to the caret, to an IME candidate list, to an open menu, or to an
 * ordinary line scroll — and every one of those is a case where answering it
 * would take the key away from the user. So most of these cases assert null, and
 * the two that don't pin the pair of keys that do move the transcript.
 */
import { describe, expect, it } from "vitest";
import { navDirectionForKey } from "./questionNavKeys";

type KeyInit = Partial<Parameters<typeof navDirectionForKey>[0]>;

function key(k: string, init: KeyInit = {}) {
  return {
    altKey: false,
    ctrlKey: false,
    isComposing: false,
    key: k,
    metaKey: false,
    shiftKey: false,
    ...init,
  };
}

describe("navDirectionForKey", () => {
  it("maps the two arrows to the two directions", () => {
    expect(navDirectionForKey(key("ArrowUp"), { editable: false, owned: false })).toBe("previous");
    expect(navDirectionForKey(key("ArrowDown"), { editable: false, owned: false })).toBe("next");
  });

  it("ignores every other key", () => {
    for (const other of ["ArrowLeft", "ArrowRight", "PageUp", "PageDown", "Home", "End", "j", "Enter"]) {
      expect(navDirectionForKey(key(other), { editable: false, owned: false })).toBeNull();
    }
  });

  it("leaves a modified arrow to the platform", () => {
    // Shift extends a selection, Option/Alt moves word-wise, Cmd/Ctrl is the
    // platform's own line motion. None of them is a request to page the
    // transcript, and taking them would break text interaction everywhere.
    for (const modifier of ["altKey", "ctrlKey", "metaKey", "shiftKey"] as const) {
      expect(navDirectionForKey(key("ArrowUp", { [modifier]: true }), { editable: false, owned: false })).toBeNull();
      expect(navDirectionForKey(key("ArrowDown", { [modifier]: true }), { editable: false, owned: false })).toBeNull();
    }
  });

  it("leaves a composing arrow to the input method", () => {
    expect(navDirectionForKey(
      key("ArrowDown", { isComposing: true }),
      { editable: false, owned: false },
    )).toBeNull();
  });

  it("leaves the caret's arrows to the caret", () => {
    // The composer is a contentEditable the reader is usually sitting in: there
    // an arrow moves the caret, and jumping the transcript would be the app
    // fighting the user for every keystroke they type.
    expect(navDirectionForKey(key("ArrowUp"), { editable: true, owned: false })).toBeNull();
    expect(navDirectionForKey(key("ArrowDown"), { editable: true, owned: false })).toBeNull();
  });

  it("leaves the arrows to whatever surface is on top", () => {
    // A modal, or a menu whose rows the arrows walk.
    expect(navDirectionForKey(key("ArrowUp"), { editable: false, owned: true })).toBeNull();
    expect(navDirectionForKey(key("ArrowDown"), { editable: false, owned: true })).toBeNull();
  });
});
