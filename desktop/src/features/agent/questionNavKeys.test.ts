// @vitest-environment jsdom
/**
 * The transcript's keyboard half.
 *
 * The whole value of this policy is what it *refuses*: a key belongs to a text
 * field, to a tab switch, to an open menu — and every one of those is a case
 * where answering it would take the key away from the user. So most of these
 * cases assert null, and two pin the page keys that do move the transcript. The
 * bare arrows are asserted *null* on purpose: they are the reader's ordinary line
 * scroll, the finer control on a long reply, and the walk must not own them.
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
  it("maps the two page keys to the two directions", () => {
    expect(navDirectionForKey(key("PageUp"), { editable: false, owned: false })).toBe("previous");
    expect(navDirectionForKey(key("PageDown"), { editable: false, owned: false })).toBe("next");
  });

  it("leaves the bare arrows to the page's own scroll", () => {
    // ↑/↓ scrolls a container a line at a time, which is what a reader pressing
    // them asked for — not a jump of a whole turn.
    expect(navDirectionForKey(key("ArrowUp"), { editable: false, owned: false })).toBeNull();
    expect(navDirectionForKey(key("ArrowDown"), { editable: false, owned: false })).toBeNull();
  });

  it("ignores every other key", () => {
    for (const other of ["ArrowLeft", "ArrowRight", "Home", "End", "Space", "j", "Enter"]) {
      expect(navDirectionForKey(key(other), { editable: false, owned: false })).toBeNull();
    }
  });

  it("leaves a modified page key to the platform", () => {
    // Ctrl+Page Up/Page Down switches tabs, Shift+Page Up extends a selection,
    // Option/Alt and Cmd are the platform's own motions. None of them is a
    // request to page the transcript, and taking them would break a tab-switch
    // shortcut the app (or the webview around it) already has.
    for (const modifier of ["altKey", "ctrlKey", "metaKey", "shiftKey"] as const) {
      expect(navDirectionForKey(key("PageUp", { [modifier]: true }), { editable: false, owned: false })).toBeNull();
      expect(navDirectionForKey(key("PageDown", { [modifier]: true }), { editable: false, owned: false })).toBeNull();
    }
  });

  it("leaves a composing press to the input method", () => {
    expect(navDirectionForKey(
      key("PageDown", { isComposing: true }),
      { editable: false, owned: false },
    )).toBeNull();
  });

  it("leaves the keys to a text field", () => {
    // The composer is a contentEditable the reader is usually sitting in: a page
    // key there scrolls the field, and jumping the transcript would be the app
    // fighting the user for every keystroke they type.
    expect(navDirectionForKey(key("PageUp"), { editable: true, owned: false })).toBeNull();
    expect(navDirectionForKey(key("PageDown"), { editable: true, owned: false })).toBeNull();
  });

  it("leaves the keys to whatever surface is on top", () => {
    // A modal, or a menu whose rows the page keys walk.
    expect(navDirectionForKey(key("PageUp"), { editable: false, owned: true })).toBeNull();
    expect(navDirectionForKey(key("PageDown"), { editable: false, owned: true })).toBeNull();
  });
});
