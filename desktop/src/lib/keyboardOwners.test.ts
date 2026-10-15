// @vitest-environment jsdom
/**
 * The two shared predicates a global key handler asks before acting: "is the
 * caret here?" and "is a surface holding the keyboard?".
 *
 * Both exist because an *unmodified* key has nothing to make it obviously
 * someone else's, so answering it is a claim about the whole window.
 */
import { act } from "react";
import { describe, expect, it } from "vitest";
import { renderHook } from "../test/renderHook";
import { EMPTY_EDITOR_ATTR, isEditableTarget } from "./editableTarget";
import { hasKeyboardOwner, useKeyboardOwner } from "./keyboardOwners";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("isEditableTarget", () => {
  it("recognises the form controls and the composing editor", () => {
    for (const tag of ["input", "textarea", "select"]) {
      expect(isEditableTarget(document.createElement(tag))).toBe(true);
    }

    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    expect(isEditableTarget(editor)).toBe(true);
  });

  it("recognises a keystroke made inside the editor", () => {
    // A key event's target is the element under the caret — a pill, a text
    // span — never the editor itself. Walking up is what makes the predicate
    // answer for the case it exists for.
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    const pill = document.createElement("span");
    pill.setAttribute("contenteditable", "false");
    editor.append(pill);
    expect(isEditableTarget(pill)).toBe(true);
  });

  it("frees the arrows while the editor holds nothing", () => {
    // The composer autofocuses on mount and after every send, so without this
    // rule the caret would be in a text field at the very moment a reader
    // starts paging a finished reply — and the keyboard would be useless for
    // the feature. An empty editor is not somewhere anyone is typing *in*.
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    editor.setAttribute(EMPTY_EDITOR_ATTR, "");
    expect(isEditableTarget(editor)).toBe(false);

    // …and the marker is what does it: remove it and the caret owns the key
    // again. The walk from a pill inside must reach the marker too.
    const pill = document.createElement("span");
    pill.setAttribute("contenteditable", "false");
    editor.append(pill);
    expect(isEditableTarget(pill)).toBe(false);

    editor.removeAttribute(EMPTY_EDITOR_ATTR);
    expect(isEditableTarget(editor)).toBe(true);
    expect(isEditableTarget(pill)).toBe(true);
  });

  it("keeps a filled textarea's arrows for the caret", () => {
    // The remote composer is a plain textarea, marked the same way: empty → the
    // transcript may page, holding a draft → the caret owns every arrow.
    const field = document.createElement("textarea");
    field.setAttribute(EMPTY_EDITOR_ATTR, "");
    expect(isEditableTarget(field)).toBe(false);

    field.textContent = "a draft";
    field.removeAttribute(EMPTY_EDITOR_ATTR);
    expect(isEditableTarget(field)).toBe(true);
  });

  it("does not read a disabled editor's atoms as typing", () => {
    // The composer's file pills are `contenteditable=false` so the caret jumps
    // over them; a target inside one is not somewhere text can be typed, and
    // the attribute is what distinguishes it from the editor around it.
    const pill = document.createElement("span");
    pill.setAttribute("contenteditable", "false");
    expect(isEditableTarget(pill)).toBe(false);
  });

  it("is false for ordinary content", () => {
    expect(isEditableTarget(document.createElement("div"))).toBe(false);
    expect(isEditableTarget(document.createElement("button"))).toBe(false);
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(new EventTarget())).toBe(false);
  });
});

describe("hasKeyboardOwner", () => {
  it("is false with nothing registered, and true with one", () => {
    expect(hasKeyboardOwner()).toBe(false);

    const layer = renderHook(() => useKeyboardOwner(true));
    expect(hasKeyboardOwner()).toBe(true);

    layer.unmount();
    expect(hasKeyboardOwner()).toBe(false);
  });

  it("registers nothing while inactive", () => {
    const closed = renderHook(() => useKeyboardOwner(false));
    expect(hasKeyboardOwner()).toBe(false);

    closed.rerender();
    expect(hasKeyboardOwner()).toBe(false);
    closed.unmount();
  });

  it("keeps holding while another owner is still up", () => {
    // A menu inside an open dialog is the normal case: releasing one must not
    // hand the keyboard back to the transcript while the other is on screen.
    const dialog = renderHook(() => useKeyboardOwner(true));
    const menu = renderHook(() => useKeyboardOwner(true));

    menu.unmount();
    expect(hasKeyboardOwner()).toBe(true);

    dialog.unmount();
    expect(hasKeyboardOwner()).toBe(false);
  });

  it("releases the owner when the layer closes", () => {
    const world = { active: true };
    const layer = renderHook(() => useKeyboardOwner(world.active));
    expect(hasKeyboardOwner()).toBe(true);

    // The owner is a plain boolean prop, so closing is a re-render with
    // `false` — the case a menu takes when the user clicks elsewhere instead
    // of the item. It must release the keyboard then, not only on unmount.
    world.active = false;
    act(() => layer.rerender());
    expect(hasKeyboardOwner()).toBe(false);
    layer.unmount();
  });
});
