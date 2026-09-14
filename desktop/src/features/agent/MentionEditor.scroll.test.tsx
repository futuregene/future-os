// @vitest-environment jsdom
import type { MentionEditorHandle } from "./MentionEditor";
import { act, createRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MentionEditor } from "./MentionEditor";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom does not lay out ranges. Exercise the real edit handlers with explicit
// geometry; browser verification separately checks native paste and layout.
describe("mention editor caret scrolling", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;
  let editor: HTMLDivElement;
  let caretRect: DOMRect;
  let originalRangeRect: PropertyDescriptor | undefined;
  const onSubmit = vi.fn();
  const editorHandle = createRef<MentionEditorHandle>();

  beforeEach(async () => {
    originalRangeRect = Object.getOwnPropertyDescriptor(Range.prototype, "getClientRects");
    caretRect = new DOMRect(10, 300, 0, 20);
    Object.defineProperty(Range.prototype, "getClientRects", {
      configurable: true,
      value: () => [caretRect],
    });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(<MentionEditor ref={editorHandle} onSubmit={onSubmit} placeholder="Message" />));
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    await act(async () => {
      editor.textContent = "before after";
    });
    vi.spyOn(editor, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 100, 500, 100));
    Object.defineProperty(editor, "clientHeight", { configurable: true, value: 100 });
    Object.defineProperty(editor, "clientTop", { configurable: true, value: 0 });
    editor.scrollTop = 50;
    editor.focus();
    const range = document.createRange();
    range.setStart(editor.firstChild!, 7);
    range.collapse(true);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
    onSubmit.mockClear();
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
    if (originalRangeRect)
      Object.defineProperty(Range.prototype, "getClientRects", originalRangeRect);
    else
      Reflect.deleteProperty(Range.prototype, "getClientRects");
  });

  function paste(text: string) {
    const event = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", {
      value: { getData: (type: string) => type === "text/plain" ? text : "", items: [] },
    });
    act(() => editor.dispatchEvent(event));
  }

  it("reveals the pasted caret below the viewport without changing the selection or scrolling ancestors", () => {
    container.scrollTop = 23;
    paste("inserted");
    expect(editor.textContent).toBe("before insertedafter");
    expect(editor.scrollTop).toBe(170);
    expect(container.scrollTop).toBe(23);
    const range = window.getSelection()!.getRangeAt(0);
    expect(range.collapsed).toBe(true);
    const prefix = range.cloneRange();
    prefix.setStart(editor, 0);
    expect(prefix.toString()).toBe("before inserted");
  });

  it("reveals an edit above the viewport instead of always jumping to the bottom", () => {
    caretRect = new DOMRect(10, 70, 0, 20);
    paste("inserted");
    expect(editor.scrollTop).toBe(20);
  });

  it("does not move the viewport when the pasted caret is already visible", () => {
    caretRect = new DOMRect(10, 140, 0, 20);
    paste("inserted");
    expect(editor.scrollTop).toBe(50);
  });

  it.each([{ shiftKey: true }, { ctrlKey: true }])("reveals the new line immediately for %o", (modifier) => {
    act(() => editor.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
      ...modifier,
    })));
    expect(editor.textContent).toBe("before \nafter");
    expect(editor.scrollTop).toBe(170);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("keeps a trailing newline measurable when insertNode splits off an empty text node", () => {
    const range = window.getSelection()!.getRangeAt(0);
    range.setStart(editor.firstChild!, editor.textContent!.length);
    range.collapse(true);
    act(() => editor.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter",
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    })));
    expect(editor.textContent).toBe("before after\n\u200B");
    expect(editor.scrollTop).toBe(170);
  });

  it("leaves IME composition to the browser", () => {
    act(() => editor.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter",
      shiftKey: true,
      isComposing: true,
      bubbles: true,
      cancelable: true,
    })));
    expect(editor.textContent).toBe("before after");
    expect(editor.scrollTop).toBe(50);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("uses the full line box and padding for both paste and native soft wraps", () => {
    editor.style.lineHeight = "20px";
    editor.style.padding = "4px 8px";
    caretRect = new DOMRect(10, 180.5, 0, 19);
    paste("inserted");
    expect(editor.scrollTop).toBe(54);

    // A native soft wrap advances the line by 20px; the existing 4px scroll
    // shifts its rectangle up. Reveal exactly one line, without an extra jump.
    caretRect = new DOMRect(10, 196.5, 0, 19);
    act(() => editor.dispatchEvent(new InputEvent("input", { bubbles: true })));
    expect(editor.scrollTop).toBe(74);
    caretRect = new DOMRect(10, 176.5, 0, 19);
    act(() => editor.dispatchEvent(new InputEvent("input", { bubbles: true })));
    expect(editor.scrollTop).toBe(74);
  });

  it("preserves top padding and rounds fractional scroll corrections outward", () => {
    editor.style.lineHeight = "20px";
    editor.style.paddingTop = "4px";
    caretRect = new DOMRect(10, 103.25, 0, 19);
    paste("inserted");
    expect(editor.scrollTop).toBe(48);
  });

  it.each(["long text\n", "long text\n\n"])("renders the last pasted line without changing serialized text: %j", (text) => {
    const range = window.getSelection()!.getRangeAt(0);
    range.setStart(editor.firstChild!, editor.textContent!.length);
    range.collapse(true);
    paste(text);
    expect(editor.textContent).toBe(`before after${text}\u200B`);
    expect(editorHandle.current!.getContent()).toBe(`before after${text}`);
    const caret = window.getSelection()!.getRangeAt(0);
    expect(caret.startContainer.textContent).toBe(`${text}\u200B`);
    expect(caret.startOffset).toBe(text.length);
  });

  it("does not adjust scrolling during IME input, but reveals the committed caret", () => {
    let afterComposition: FrameRequestCallback | undefined;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      afterComposition = callback;
      return 1;
    });
    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    act(() => editor.dispatchEvent(new InputEvent("input", { bubbles: true, isComposing: true })));
    expect(editor.scrollTop).toBe(50);
    act(() => editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
    act(() => afterComposition!(0));
    expect(editor.scrollTop).toBe(170);
  });

  it("does not scroll a selection range or a caret outside this editor on input", () => {
    const range = window.getSelection()!.getRangeAt(0);
    range.setEnd(editor.firstChild!, 10);
    act(() => editor.dispatchEvent(new InputEvent("input", { bubbles: true })));
    expect(editor.scrollTop).toBe(50);
    range.setStart(container, 0);
    range.collapse(true);
    act(() => editor.dispatchEvent(new InputEvent("input", { bubbles: true })));
    expect(editor.scrollTop).toBe(50);
  });

  it("does not scroll for an empty range rectangle", () => {
    caretRect = new DOMRect();
    paste("");
    expect(editor.scrollTop).toBe(50);
  });
});
