// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { EMPTY_EDITOR_ATTR, isEditableTarget } from "./editableTarget";

/**
 * A jsdom element that answers `isContentEditable` the way the engines that
 * only expose the property do: `true`, with no `contenteditable` attribute to
 * walk to. jsdom itself leaves the property `undefined`, so this is the only
 * way to exercise the property arm — and the elements below deliberately carry
 * no attribute, which is what makes the assertion fail if the property is not
 * consulted.
 */
function propertyOnlyEditor(tag = "div"): HTMLElement {
  const element = document.createElement(tag);
  Object.defineProperty(element, "isContentEditable", { value: true, configurable: true });
  return element;
}

describe("isEditableTarget", () => {
  it("treats a form control as somewhere the user is typing", () => {
    expect(isEditableTarget(document.createElement("input"))).toBe(true);
    expect(isEditableTarget(document.createElement("textarea"))).toBe(true);
    expect(isEditableTarget(document.createElement("select"))).toBe(true);
  });

  it("reads a target that reports contentEditable only through the property", () => {
    const editor = propertyOnlyEditor();
    expect(editor.hasAttribute("contenteditable")).toBe(false);
    expect(isEditableTarget(editor)).toBe(true);
  });

  it("ignores a non-element target", () => {
    expect(isEditableTarget(null)).toBe(false);
    expect(isEditableTarget(document.createTextNode("x"))).toBe(false);
  });

  it("walks up to the editor that holds the caret", () => {
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "true");
    const pill = document.createElement("span");
    editor.append(pill);
    expect(isEditableTarget(pill)).toBe(true);
  });

  it("does not claim an explicitly non-editable ancestor", () => {
    const editor = document.createElement("div");
    editor.setAttribute("contenteditable", "false");
    const child = document.createElement("span");
    editor.append(child);
    expect(isEditableTarget(child)).toBe(false);
  });

  it("hands an empty editor's keys back to the surrounding view", () => {
    // The composer autofocuses into an empty editor; the marker it sets while
    // empty is what frees PageUp/PageDown for the transcript.
    const editor = propertyOnlyEditor();
    editor.setAttribute(EMPTY_EDITOR_ATTR, "");
    expect(isEditableTarget(editor)).toBe(false);

    const attributeEditor = document.createElement("div");
    attributeEditor.setAttribute("contenteditable", "true");
    attributeEditor.setAttribute(EMPTY_EDITOR_ATTR, "");
    expect(isEditableTarget(attributeEditor)).toBe(false);

    attributeEditor.removeAttribute(EMPTY_EDITOR_ATTR);
    expect(isEditableTarget(attributeEditor)).toBe(true);
  });
});
