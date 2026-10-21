// @vitest-environment jsdom
import type { MentionEditorHandle } from "./MentionEditor";
import { act, createRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { MentionEditor } from "./MentionEditor";

/**
 * The panel's input layer turns the `@` / `/` / `#` menus off (`VIBE.md` §5.3).
 *
 * Why it matters is not cosmetic: a dictation arrives as one inserted run, so a
 * recognized "斜杠" or a file path would open a menu *over the transcript*, and
 * the next utterance's keys would feed that menu instead of the sentence. The text
 * itself must still land — only the popups go.
 */
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom has no layout: the menus scroll the highlighted row into view, which is
// the first thing that throws once a menu is actually up (and, incidentally, what
// makes the "no menu" assertions below non-vacuous).
Element.prototype.scrollIntoView = () => {};

const SKILLS = [{ description: "search", name: "future-web" }];

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let editor: HTMLElement;
const ref = createRef<MentionEditorHandle>();

async function mount(triggers: boolean) {
  await act(async () => root.render(
    <MentionEditor
      onSubmit={() => {}}
      placeholder="Message"
      ref={ref}
      skills={SKILLS}
      triggers={triggers}
      workspaceId="ws"
    />,
  ));
  editor = container.querySelector<HTMLElement>("[role=\"textbox\"]")!;
}

/**
 * Type text the way both a keyboard and a dictation land it: one input event.
 *
 * The caret has to end up inside the **text node** — the trigger detection reads
 * `selection.anchorNode` and gives up on an element boundary (`mentionDom.ts`).
 * Getting this wrong made the "triggers on" control below pass for the wrong
 * reason (no menu because the caret looked wrong, not because of the flag).
 */
function type(text: string) {
  act(() => {
    editor.focus();
    editor.textContent = text;
    const node = editor.firstChild!;
    const range = document.createRange();
    range.setStart(node, (node.textContent ?? "").length);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: text }));
  });
}

/**
 * "Is a popup up": every menu renders its rows as `button[data-menu-index]`
 * (`composer/MentionMenus.tsx`), and nothing else in this component does.
 */
function menuOpen(): boolean {
  return container.querySelectorAll("button[data-menu-index]").length > 0;
}

beforeEach(() => {
  container = document.body.appendChild(document.createElement("div"));
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("triggers off", () => {
  it("keeps a dictated slash as text and opens no menu", async () => {
    await mount(false);
    type("/future-web");
    // The text is there: this is not "typing is disabled", it is "no popups".
    expect(editor.textContent).toBe("/future-web");
    expect(menuOpen()).toBe(false);
  });

  it("keeps a dictated path with a slash as text", async () => {
    await mount(false);
    type("看一下 /work/paper.md 里的结论");
    expect(editor.textContent).toBe("看一下 /work/paper.md 里的结论");
    expect(menuOpen()).toBe(false);
  });

  it("does not open the conversation menu on #", async () => {
    await mount(false);
    type("#上一个会话");
    expect(menuOpen()).toBe(false);
  });
});

describe("triggers on", () => {
  it("opens the slash menu as before", async () => {
    // The control for the tests above: without it, "no menu" could pass because
    // the menu never works in jsdom at all.
    await mount(true);
    type("/");
    expect(menuOpen()).toBe(true);
  });
});
