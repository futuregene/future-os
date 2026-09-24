// @vitest-environment jsdom
import type { MentionEditorHandle } from "./MentionEditor";
import { act, createRef, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MentionEditor } from "./MentionEditor";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("mention editor placeholder", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;
  let editor: HTMLDivElement;
  const ref = createRef<MentionEditorHandle>();
  const onEmptyChange = vi.fn();
  const onChange = vi.fn();

  beforeEach(() => {
    onEmptyChange.mockClear();
    onChange.mockClear();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root.render(
      <StrictMode>
        <MentionEditor ref={ref} onSubmit={() => {}} onEmptyChange={onEmptyChange} onChange={onChange} placeholder="Message" />
      </StrictMode>,
    ));
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  function placeholderElement() {
    return Array.from(container.querySelectorAll("div")).find(element =>
      element !== editor && element.textContent === "Message" && element.childElementCount === 0,
    );
  }

  function placeholderVisible() {
    return Boolean(placeholderElement());
  }

  it("sizes the hint to the whole editor box so the empty-state repaint clears stale text", () => {
    // WKWebView on macOS keeps removed text painted in the composer unless the
    // whole box is dirtied (see the report behind this test: after a send the
    // DOM was empty, the hint was up, and the last line was still on screen).
    // Mounting the hint is what dirties it, so it must cover the box rather
    // than just the text's own width.
    const hint = placeholderElement()!;
    expect(hint.className).toContain("inset-0");
    // Same padding as the editor, so the hint sits exactly where a first line
    // would — widening the box must not move the text.
    expect(hint.className).toContain("px-2");
    expect(hint.className).toContain("py-1");
    // Must stay click-through: the hint overlays the editor now.
    expect(hint.className).toContain("pointer-events-none");
  });

  it("tracks DOM edits even without an input event, without replacing the text or caret", async () => {
    expect(placeholderVisible()).toBe(true);
    const text = document.createTextNode("Native edit");
    await act(async () => {
      editor.append(text);
      const range = document.createRange();
      range.setStart(text, 3);
      range.collapse(true);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
    });
    expect(placeholderVisible()).toBe(false);
    expect(onEmptyChange.mock.calls).toEqual([[false]]);
    expect(editor.firstChild).toBe(text);
    expect(window.getSelection()!.anchorNode).toBe(text);
    expect(window.getSelection()!.anchorOffset).toBe(3);

    // Character-data changes and browser-inserted empty blocks must also sync.
    await act(async () => {
      text.data = "";
    });
    expect(placeholderVisible()).toBe(true);
    await act(async () => {
      editor.innerHTML = "<div><br></div>";
    });
    expect(placeholderVisible()).toBe(true);
    expect(onEmptyChange.mock.calls).toEqual([[false], [true]]);
  });

  it("syncs composing text without submitting or rewriting the composition node", async () => {
    const text = document.createTextNode("");
    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    await act(async () => {
      editor.append(text);
    });
    await act(async () => {
      text.data = "正在输入";
    });
    expect(placeholderVisible()).toBe(false);
    expect(editor.firstChild).toBe(text);
    expect(ref.current!.getContent()).toBe("正在输入");
  });

  it("keeps restore, clear and mention insertion synchronized without duplicate notifications", async () => {
    await act(async () => {
      ref.current!.restore("Saved draft");
    });
    expect(placeholderVisible()).toBe(false);
    expect(onChange).not.toHaveBeenCalled();
    expect(onEmptyChange.mock.calls).toEqual([[false]]);

    await act(async () => {
      ref.current!.clear();
    });
    expect(placeholderVisible()).toBe(true);
    await act(async () => {
      ref.current!.insertMention({ name: "file.ts", path: "file.ts" });
    });
    expect(placeholderVisible()).toBe(false);
    expect(ref.current!.getContent()).toBe("[file.ts](./file.ts) ");
    expect(onEmptyChange.mock.calls).toEqual([[false], [true], [false]]);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("does not notify twice when both input and the observer see the same edit", async () => {
    await act(async () => {
      editor.textContent = "Typed text";
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
    });
    expect(placeholderVisible()).toBe(false);
    expect(onEmptyChange.mock.calls).toEqual([[false]]);
    expect(onChange).toHaveBeenCalledTimes(1);
    await act(async () => {
      editor.replaceChildren();
      editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "deleteContentBackward" }));
    });
    expect(placeholderVisible()).toBe(true);
    expect(onEmptyChange.mock.calls).toEqual([[false], [true]]);
  });
});
