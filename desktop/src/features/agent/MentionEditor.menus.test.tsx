import type { Root } from "react-dom/client";
import type { Mock } from "vitest";
// @vitest-environment jsdom
import type { ContextToolOption, MentionEditorHandle, SkillMentionOption } from "./MentionEditor";
import { act, createRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MentionEditor } from "./MentionEditor";

/**
 * The trigger menus (`@` files and `/` skills), pill insertion and serialization,
 * draft restore, and the clipboard routing. Caret scrolling, IME and the
 * placeholder live in `MentionEditor.scroll.test.tsx` / `.placeholder.test.tsx`.
 */
const storage = vi.hoisted(() => ({ searchWorkspaceFiles: vi.fn() }));
vi.mock("../../integrations/storage/threadStore", () => ({
  searchWorkspaceFiles: storage.searchWorkspaceFiles,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const SKILLS: SkillMentionOption[] = [
  { description: "Search the web", name: "future-web", nameZh: "网页搜索" },
  { description: "Write a paper", name: "future-paper" },
];
const TOOLS: ContextToolOption[] = [
  { description: "Compress the context", id: "compact", name: "Compact", searchText: "compress compact 压缩" },
];

let container: HTMLDivElement;
let root: Root;
let editor: HTMLDivElement;
let handle: React.RefObject<MentionEditorHandle | null>;
let onSubmit: Mock;
let onChange: Mock;
let onEmptyChange: Mock;
let onPasteAttachmentPaths: Mock;
let onPasteFiles: Mock;
let onContextToolSelect: Mock;

type EditorProps = Partial<Parameters<typeof MentionEditor>[0]>;

function render(over: EditorProps = {}) {
  act(() => root.render(
    <MentionEditor
      contextTools={TOOLS}
      onContextToolSelect={onContextToolSelect}
      onChange={onChange}
      onEmptyChange={onEmptyChange}
      onPasteAttachmentPaths={onPasteAttachmentPaths}
      onPasteFiles={onPasteFiles}
      onSubmit={onSubmit}
      placeholder="Message"
      ref={handle}
      skills={SKILLS}
      workspaceId="W1"
      {...over}
    />,
  ));
}

const FILES = [
  { name: "alpha.ts", path: "src/alpha.ts" },
  { name: "beta.md", path: "docs/beta.md" },
];

beforeEach(() => {
  vi.useFakeTimers();
  // jsdom implements no layout, and both menus scroll the highlighted row into
  // view; the call itself is asserted where it matters.
  Element.prototype.scrollIntoView = vi.fn();
  storage.searchWorkspaceFiles.mockReset();
  // Filter by query like the real backend, so the menu contents prove the query
  // actually reached the search.
  storage.searchWorkspaceFiles.mockImplementation(async ({ query }: { query: string }) =>
    FILES.filter(file => `${file.name}${file.path}`.includes(query)));
  handle = createRef<MentionEditorHandle>();
  onSubmit = vi.fn();
  onChange = vi.fn();
  onEmptyChange = vi.fn();
  onPasteAttachmentPaths = vi.fn();
  onPasteFiles = vi.fn();
  onContextToolSelect = vi.fn();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

/** Replace the editor's text and park a collapsed caret at `offset`. */
function setText(text: string, offset = text.length) {
  act(() => {
    editor.textContent = text;
    // Keep an addressable text node even for empty content, so a caller can
    // park a caret the way a real (empty) contenteditable does.
    if (!editor.firstChild)
      editor.appendChild(document.createTextNode(""));
    const node = editor.firstChild as Text;
    const range = document.createRange();
    range.setStart(node, offset);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
  });
}

/**
 * Focus the editor *before* parking a caret. `insertMention` focuses first, and
 * jsdom's `focus()` on an already-active element preserves the caret while a
 * foreign selection gets clobbered to offset 0 — so this is what makes a
 * caret-sensitive path testable at all.
 */
function focusThenSetText(text: string, offset = text.length) {
  act(() => editor.focus());
  setText(text, offset);
}

/** A native edit: the DOM changed, then the browser fires `input`. */
function type(text: string) {
  setText(text);
  act(() => editor.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" })));
}

function press(key: string, init: KeyboardEventInit = {}) {
  act(() => {
    editor.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key, ...init }));
  });
}

/** Let the 120ms workspace search resolve. */
async function settleSearch() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(120);
  });
}

function menu() {
  return container.querySelector<HTMLElement>("div.absolute.bottom-full");
}

function menuRows() {
  return [...container.querySelectorAll<HTMLButtonElement>("[data-menu-index]")];
}

function menuLabels() {
  return menuRows().map(row => row.textContent);
}

/** The row the keyboard highlight is on (the class, not the `hover:` variant). */
function highlightedIndex() {
  return menuRows().findIndex(row => row.classList.contains("bg-surface-subtle"));
}

function editorWithHandle() {
  return handle.current!;
}

describe("mentionEditor @ file mentions", () => {
  it("opens the recents list for a bare @ and searches as the query grows", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    expect(menu()).toBeNull();

    // boundary: a bare `@` is an empty query — the menu opens on recents.
    type("@");
    expect(storage.searchWorkspaceFiles).not.toHaveBeenCalled();
    await settleSearch();
    expect(storage.searchWorkspaceFiles).toHaveBeenCalledWith({ limit: 10, query: "", workspaceId: "W1" });
    expect(menu()).not.toBeNull();
    expect(menuLabels()).toEqual(["src/alpha.ts", "docs/beta.md"]);

    type("@bet");
    await settleSearch();
    expect(storage.searchWorkspaceFiles).toHaveBeenLastCalledWith({ limit: 10, query: "bet", workspaceId: "W1" });
  });

  it("lands a picked file as an atomic pill at the caret and serializes it back", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("see @");
    await settleSearch();

    // Mouse pick (mousedown, so the editor keeps focus and its caret).
    act(() => {
      menuRows()[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });

    const pill = editor.querySelector<HTMLElement>("[data-mention]")!;
    expect(pill).not.toBeNull();
    expect(pill.getAttribute("data-mention")).toBe("file");
    expect(pill.getAttribute("data-path")).toBe("./src/alpha.ts");
    expect(pill.getAttribute("contenteditable")).toBe("false");
    expect(pill.textContent).toBe("alpha.ts");
    // The typed `@` is gone, and a gap keeps the caret outside the pill.
    expect(editorWithHandle().getContent()).toBe("see [alpha.ts](./src/alpha.ts) ");
    expect(menu()).toBeNull();
    expect(onChange).toHaveBeenCalled();
  });

  it("walks the results with the arrow keys, wrapping at both ends", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@");
    await settleSearch();
    expect(highlightedIndex()).toBe(0);

    press("ArrowDown");
    expect(highlightedIndex()).toBe(1);

    // boundary: past the last row wraps to the first…
    press("ArrowDown");
    expect(highlightedIndex()).toBe(0);
    // …and before the first wraps to the last.
    press("ArrowUp");
    expect(highlightedIndex()).toBe(1);
  });

  it.each(["Enter", "Tab"])("picks the highlighted row with %s", async (key) => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@bet");
    await settleSearch();
    press(key);

    expect(editorWithHandle().getContent()).toBe("[beta.md](./docs/beta.md) ");
    // A menu-driven Enter must not submit the message.
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("closes the menu on Escape without touching the typed text", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@be");
    await settleSearch();
    expect(menu()).not.toBeNull();

    press("Escape");
    expect(menu()).toBeNull();
    expect(editor.textContent).toBe("@be");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("reports an empty result list instead of an empty box", async () => {
    storage.searchWorkspaceFiles.mockResolvedValue([]);
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@nothing");
    await settleSearch();
    expect(menu()?.textContent).toBe("No matching files.");
    expect(menuRows()).toHaveLength(0);
  });

  it("ignores Enter when the highlighted slash row no longer exists", async () => {
    // concurrency + boundary: `slashItems` is derived from the `contextTools` and
    // `skills` PROPS, but the highlight index is only reset when the `/` QUERY
    // changes (`useEffect(..., [slashQuery])`). So a catalogue that shrinks while the
    // menu is open - the real case is the skills list reloading - leaves
    // `selectedSlashItem` pointing past the end, and `slashItems[selectedSlashItem]`
    // undefined. The `if (item)` guard is what stops `selectSlashItem(undefined)`;
    // its false arm was uncovered because every test keeps the props constant.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/");
    await settleSearch();
    expect(menuRows()).toHaveLength(3); // the context tool plus two skills

    // Highlight the last row, then shrink the catalogue under it.
    press("ArrowDown");
    press("ArrowDown");
    expect(highlightedIndex()).toBe(2);
    render({ skills: [] });
    expect(menuRows()).toHaveLength(1);

    // Enter must be inert rather than dereferencing a missing row. The crash React
    // would raise is asynchronous (it escapes a try/catch around the dispatch), so it
    // is captured with a global listener instead: without the guard the handler throws
    // `Cannot read properties of undefined`, and an unhandled error would fail the RUN
    // without any assertion firing - a test that looks green in isolation while
    // proving nothing. Capturing it turns the guard into assertion evidence.
    const failures: unknown[] = [];
    const capture = (event: ErrorEvent) => void failures.push(event.error ?? event.message);
    window.addEventListener("error", capture);
    press("Enter");
    await settleSearch();
    window.removeEventListener("error", capture);

    expect(failures).toEqual([]);
    expect(onContextToolSelect).not.toHaveBeenCalled();
    expect(editorWithHandle().getContent()).toBe("/");
  });

  it("closes the menu when the search fails", async () => {
    // error-path: a failed workspace read must not leave a stale menu up.
    storage.searchWorkspaceFiles.mockRejectedValue(new Error("index unreadable"));
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@x");
    await settleSearch();
    expect(menu()).toBeNull();
  });

  it("ignores a rejected search that was superseded before it settled", async () => {
    // concurrency: the debounced search's cleanup flips `cancelled`, and a rejection
    // arriving after that must not clear a NEWER query's results. The sibling test
    // above takes the true arm; this is the false one, and it needs the rejection to
    // land after the effect was torn down - which no existing test arranges, because
    // they all reject a promise the effect is still awaiting.
    let rejectFirst!: (reason: Error) => void;
    storage.searchWorkspaceFiles.mockReturnValueOnce(new Promise((_resolve, reject) => {
      rejectFirst = reject;
    }));
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;

    // First query: its search is left pending.
    type("@alph");
    await settleSearch();
    expect(menu()).toBeNull();

    // Second query supersedes it, and its own search resolves with results.
    type("@beta");
    await settleSearch();
    expect(menu()?.textContent).toContain("beta.md");

    // Now the stale search rejects. It must be ignored, not treated as the
    // failure of the query on screen.
    rejectFirst(new Error("index unreadable"));
    await settleSearch();

    expect(menu()).not.toBeNull();
    expect(menu()?.textContent).toContain("beta.md");
  });

  it("does not open for an email, a path or a bare slash before the @", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;

    // boundary: an ASCII char (or `./@`) right before the `@` suppresses the
    // trigger, so these can never open the file menu.
    for (const text of ["foo@bar", "./@x", "a/@b"]) {
      type(text);
      expect(menu(), text).toBeNull();
    }
    expect(storage.searchWorkspaceFiles).not.toHaveBeenCalled();
  });

  it("opens mid-sentence and inside CJK text", async () => {
    // A doubled space before the `@` is one of the shapes the trigger allows.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("请在 @");
    await settleSearch();
    expect(menu()).not.toBeNull();
  });

  it("stays closed with no workspace, while disabled, and for a huge query", async () => {
    render({ workspaceId: null });
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@x");
    await settleSearch();
    expect(menu()).toBeNull();

    act(() => root.unmount());
    root = createRoot(container);
    render({ disabled: true });
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@x");
    await settleSearch();
    expect(menu()).toBeNull();
    expect(storage.searchWorkspaceFiles).not.toHaveBeenCalled();

    // boundary: a very long query is still just a query (no crash, no menu).
    act(() => root.unmount());
    root = createRoot(container);
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type(`@${"x".repeat(5000)}`);
    await settleSearch();
    expect(storage.searchWorkspaceFiles).toHaveBeenCalledTimes(1);
  });

  it("cancels a superseded search so a slow answer cannot replace a newer one", async () => {
    // concurrency: the debounce is re-armed per keystroke; the first request's
    // answer must not overwrite the second's.
    let resolveFirst: (value: unknown) => void = () => {};
    storage.searchWorkspaceFiles
      .mockImplementationOnce(() => new Promise((resolve) => {
        resolveFirst = resolve;
      }))
      .mockResolvedValueOnce([{ name: "second.ts", path: "src/second.ts" }]);
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;

    type("@a");
    await settleSearch();
    type("@ab");
    await settleSearch();
    expect(menuLabels()).toEqual(["src/second.ts"]);

    // The stale first answer lands late and must be dropped.
    await act(async () => {
      resolveFirst([{ name: "stale.ts", path: "src/stale.ts" }]);
      await Promise.resolve();
    });
    expect(menuLabels()).toEqual(["src/second.ts"]);
  });
});

describe("mentionEditor / slash menu", () => {
  it("lists the context tool above the skills, with a separator between the groups", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/");
    expect(menu()).not.toBeNull();

    // Context tools always precede skills, and the separator names the section.
    expect(menuLabels()[0]).toContain("Compact");
    expect(menuLabels()[1]).toContain("/future-web");
    expect(menu()!.textContent).toContain("Skills");
  });

  it("narrows by the localized name, the Chinese alias and the hidden search text", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;

    type("/web");
    expect(menuLabels()).toEqual(["/future-webSearch the web"]);

    // A Chinese catalogue name matches even though it is never displayed.
    type("/网页");
    expect(menuLabels()).toEqual(["/future-webSearch the web"]);

    // A tool's hidden bilingual alias matches too.
    type("/压缩");
    expect(menuLabels()).toEqual(["CompactCompress the context"]);

    // boundary: `pa` matches the tool's NAME ("comPAct") as well as the skill,
    // so both groups show — the separator is what distinguishes them.
    type("/pa");
    expect(menuLabels()).toEqual(["CompactCompress the context", "/future-paperWrite a paper"]);
    expect(menu()!.textContent).toContain("Skills");
  });

  it("reports no matches, and hides the menu entirely with nothing to offer", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/nothingmatches");
    expect(menu()!.textContent).toBe("No matching tools or skills.");

    // boundary: no skills and no tools means no `/` menu at all.
    act(() => root.unmount());
    root = createRoot(container);
    render({ contextTools: [], skills: [] });
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/");
    expect(menu()).toBeNull();
  });

  it("inserts a skill as an atomic pill and serializes it back to a /token", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("please /paper");
    // Only the skill matches this query, so row 0 is the skill.
    expect(menuLabels()).toEqual(["/future-paperWrite a paper"]);
    act(() => {
      menuRows()[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });

    const pill = editor.querySelector<HTMLElement>("[data-mention='skill']")!;
    expect(pill.getAttribute("data-skill")).toBe("future-paper");
    expect(pill.textContent).toBe("/future-paper");
    expect(pill.getAttribute("contenteditable")).toBe("false");
    expect(editorWithHandle().getContent()).toBe("please /future-paper ");
  });

  it("runs a context tool by removing the typed slash and reporting the id", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("run /comp");

    act(() => {
      menuRows()[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });

    expect(onContextToolSelect).toHaveBeenCalledWith("compact");
    // The typed `/comp` is consumed rather than sent as text.
    expect(editorWithHandle().getContent()).toBe("run ");
    expect(menu()).toBeNull();
    expect(onChange).toHaveBeenCalled();
  });

  it.each([
    ["ArrowDown", 1],
    // boundary: 3 rows (1 tool + 2 skills), so ArrowUp from the first wraps to
    // the last rather than stopping at index -1.
    ["ArrowUp", 2],
  ])("moves and wraps the slash highlight with %s", (key, expected) => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/");
    press(key);
    expect(highlightedIndex()).toBe(expected);
  });

  it.each(["Enter", "Tab"])("selects the highlighted slash row with %s", (key) => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/fut");
    press(key);
    expect(onSubmit).not.toHaveBeenCalled();
    expect(editorWithHandle().getContent()).toContain("/future-web");
  });

  it("closes the slash menu on Escape and reopens it while typing", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/fu");
    expect(menu()).not.toBeNull();
    press("Escape");
    expect(menu()).toBeNull();

    // boundary: a slash not preceded by whitespace/start is a path, not a trigger.
    type("run /fu");
    expect(menu()).not.toBeNull();
    type("a/b");
    expect(menu()).toBeNull();
  });

  it("resets the highlight when the query changes", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/");
    press("ArrowDown");
    expect(highlightedIndex()).toBe(1);

    // A new query starts again from the first row.
    type("/fut");
    expect(highlightedIndex()).toBe(0);
  });

  it("switches the menu from files to skills when the trigger changes", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@al");
    await settleSearch();
    expect(menu()).not.toBeNull();
    expect(storage.searchWorkspaceFiles).toHaveBeenCalledTimes(1);

    type("/fut");
    expect(menuLabels()).toEqual(["/future-webSearch the web", "/future-paperWrite a paper"]);
  });
});

describe("mentionEditor serialization round-trip", () => {
  it("round-trips text, file pills and skill pills through restore and getContent", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;

    const draft = "hello [alpha.ts](./src/alpha.ts) and /future-web end";
    act(() => editorWithHandle().restore(draft));
    expect(editorWithHandle().getContent()).toBe(draft);

    // The pills are real atomic spans, not raw markup text.
    expect(editor.querySelectorAll("[data-mention='file']")).toHaveLength(1);
    expect(editor.querySelectorAll("[data-mention='skill']")).toHaveLength(1);
    expect(editor.textContent).not.toContain("](");
    // restore() must not fire onChange — it is not a user edit.
    expect(onChange).not.toHaveBeenCalled();
  });

  it("parks the caret at the end of a restored draft", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("first line"));
    const range = window.getSelection()!.getRangeAt(0);
    expect(range.collapsed).toBe(true);
    // At the end: everything before the caret is the restored text.
    const prefix = range.cloneRange();
    prefix.selectNodeContents(editor);
    prefix.setEnd(range.startContainer, range.startOffset);
    expect(prefix.toString()).toBe("first line");
  });

  it("restores a plain string verbatim when no skill name matches the token", () => {
    // boundary: `/foo` is not an installed skill, so it stays literal text.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("a /notaskill b"));
    expect(editor.querySelectorAll("[data-mention]")).toHaveLength(0);
    expect(editorWithHandle().getContent()).toBe("a /notaskill b");
  });

  it("restores a /token as a pill once the skill is installed", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("/future-web"));
    expect(editor.querySelector("[data-mention='skill']")).not.toBeNull();
    expect(editorWithHandle().getContent()).toBe("/future-web");
  });

  it("clears with an empty restore", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("something"));
    expect(editorWithHandle().getContent()).toBe("something");
    act(() => editorWithHandle().restore(""));
    expect(editorWithHandle().getContent()).toBe("");
    expect(editor.textContent).toBe("");
  });

  it("angle-wraps a path that holds whitespace or parentheses", () => {
    // boundary: a bare `)` in the path would close the markdown link early and
    // truncate the mention downstream, so those paths are `<...>`-wrapped.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().insertMention({ name: "a b.txt", path: "docs/a b.txt" }));
    expect(editorWithHandle().getContent()).toBe("[a b.txt](<./docs/a b.txt>) ");

    act(() => editorWithHandle().clear());
    act(() => editorWithHandle().insertMention({ name: "x(1).txt", path: "docs/x(1).txt" }));
    expect(editorWithHandle().getContent()).toBe("[x(1).txt](<./docs/x(1).txt>) ");
  });

  it("neutralizes brackets in a file label so they cannot break the link", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().insertMention({ name: "a[b]c.ts", path: "src/a[b]c.ts" }));
    expect(editorWithHandle().getContent()).toBe("[a(b)c.ts](./src/a[b]c.ts) ");
  });

  it("serializes browser-inserted block wrappers as line breaks", () => {
    // boundary: a paste or IME can leave WebKit's own <div>/<p> wrappers behind.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("one"));
    act(() => {
      editor.innerHTML = "one<div>two</div><p>three</p>four<br>five";
    });
    expect(editorWithHandle().getContent()).toBe("onetwo\nthree\nfour\nfive");
  });

  it("strips the zero-width padding a trailing newline needs", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("line"));
    act(() => {
      const range = window.getSelection()!.getRangeAt(0);
      range.setStart(editor.firstChild!, 4);
      range.collapse(true);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
    });
    press("Enter", { shiftKey: true });
    expect(editor.textContent).toBe("line\n\u200B");
    // The ZWSP is a rendering aid, never part of the message.
    expect(editorWithHandle().getContent()).toBe("line\n");
  });
});

describe("mentionEditor insertion and clearing", () => {
  it("inserts a mention at the caret when the caret is inside the editor", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("one two", 7);
    act(() => editorWithHandle().insertMention({ name: "f.ts", path: "src/f.ts" }));
    expect(editorWithHandle().getContent()).toBe("one two[f.ts](./src/f.ts) ");
  });

  it("inserts a mention mid-text when the caret sits in the middle", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("one two", 4);
    act(() => editorWithHandle().insertMention({ name: "f.ts", path: "src/f.ts" }));
    expect(editorWithHandle().getContent()).toBe("one [f.ts](./src/f.ts) two");
  });

  it("appends with a leading space when the caret is not in the editor", () => {
    // concurrency/boundary: the file tree can insert while focus is elsewhere.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("existing text"));
    act(() => {
      const outside = document.createElement("div");
      outside.textContent = "elsewhere";
      document.body.append(outside);
      const range = document.createRange();
      range.setStart(outside.firstChild!, 1);
      range.collapse(true);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
      outside.remove();
    });
    act(() => editorWithHandle().insertMention({ name: "f.ts", path: "src/f.ts" }));
    expect(editorWithHandle().getContent()).toBe("existing text [f.ts](./src/f.ts) ");
  });

  it("avoids a doubled space when the editor is empty and the caret is outside", () => {
    // boundary: the "no caret" arm of `insertMention` has two shapes - append to an
    // existing draft (the leading space above) and insert into an EMPTY editor,
    // where the separator space would land before the pill's own trailing space.
    // The name promised this case but the body never moved the caret out of the
    // editor, so it passed through the caret-inside path instead and the arm stayed
    // uncovered. The detached-node selection below is what makes the caret foreign.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    expect(editorWithHandle().getContent()).toBe("");
    // The editor must already be the active element: `insertMention` focuses it
    // itself, and jsdom's `focus()` on a *not-yet-active* contenteditable parks a
    // fresh caret inside it (so the caret-inside path would run instead), while on
    // an already-active element it leaves a foreign selection alone. That is the
    // whole reason the existing "caret is not in the editor" test works.
    act(() => editor.focus());
    act(() => {
      const outside = document.createElement("div");
      outside.textContent = "elsewhere";
      document.body.append(outside);
      const range = document.createRange();
      range.setStart(outside.firstChild!, 1);
      range.collapse(true);
      window.getSelection()!.removeAllRanges();
      window.getSelection()!.addRange(range);
      outside.remove();
    });
    act(() => editorWithHandle().insertMention({ name: "f.ts", path: "src/f.ts" }));
    expect(editorWithHandle().getContent()).toBe("[f.ts](./src/f.ts) ");
  });

  it("clear() empties the editor, closes either menu and reports the change", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@al");
    await settleSearch();
    expect(menu()).not.toBeNull();

    act(() => editorWithHandle().clear());
    expect(editor.innerHTML).toBe("");
    expect(menu()).toBeNull();
    expect(editorWithHandle().getContent()).toBe("");
    expect(onEmptyChange).toHaveBeenLastCalledWith(true);
  });

  it("focus() moves the caret into the editor", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    editor.blur();
    act(() => editorWithHandle().focus());
    expect(document.activeElement).toBe(editor);
  });

  it("closes the menu when the caret leaves the trigger", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@al");
    await settleSearch();
    expect(menu()).not.toBeNull();

    // Typing a space ends the mention.
    type("@al ");
    await settleSearch();
    expect(menu()).toBeNull();
  });

  it("ignores a stale row click once the caret is gone", async () => {
    // concurrency: the row outlives the caret it was rendered for (the webview
    // can drop the selection between the mousedown and the handler). Nothing may
    // be inserted or thrown.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@al");
    await settleSearch();
    expect(menuRows().length).toBeGreaterThan(0);

    act(() => window.getSelection()!.removeAllRanges());
    onChange.mockClear();
    act(() => {
      menuRows()[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });
    expect(editor.querySelectorAll("[data-mention]")).toHaveLength(0);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("inserts nothing when a mention sits at the caret with no room for a query", async () => {
    // boundary: a bare `@` at the very start is a valid (empty) query, so the
    // menu can open; picking still replaces just the `@`.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@");
    await settleSearch();
    act(() => {
      menuRows()[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });
    expect(editorWithHandle().getContent()).toBe("[alpha.ts](./src/alpha.ts) ");
  });

  it("re-opens the mention menu after a pill, bounding the query at the pill", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@");
    await settleSearch();
    act(() => {
      menuRows()[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });

    // The pill is an atomic node, so a second `@` after it is a fresh query.
    const pill = editor.querySelector("[data-mention]")!;
    act(() => {
      const after = pill.nextSibling as Text;
      after.data = " @be";
      const range = document.createRange();
      range.setStart(after, after.data.length);
      range.collapse(true);
      const selection = window.getSelection()!;
      selection.removeAllRanges();
      selection.addRange(range);
      editor.dispatchEvent(new InputEvent("input", { bubbles: true }));
    });
    await settleSearch();
    expect(menuLabels()).toEqual(["docs/beta.md"]);
  });
});

describe("mentionEditor stale-row clicks with no caret", () => {
  it("ignores a slash skill row click once the caret is gone", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/paper");
    const rows = menuRows();
    expect(rows.length).toBeGreaterThan(0);

    act(() => window.getSelection()!.removeAllRanges());
    act(() => rows[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
    expect(editor.querySelectorAll("[data-mention]")).toHaveLength(0);
    expect(onContextToolSelect).not.toHaveBeenCalled();
  });

  it("ignores a context-tool row click once the caret is gone", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/compact");
    const rows = menuRows();
    expect(rows.length).toBe(1);

    act(() => window.getSelection()!.removeAllRanges());
    act(() => rows[0]!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
    expect(onContextToolSelect).not.toHaveBeenCalled();
  });
});

describe("mentionEditor serialization edge nodes", () => {
  it("skips a non-text, non-element node instead of serializing it", () => {
    // boundary: a comment (or any other node type) can end up in a
    // contenteditable; it must contribute nothing to the message.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => editorWithHandle().restore("kept"));
    act(() => {
      editor.appendChild(document.createComment("annotation"));
    });
    expect(editorWithHandle().getContent()).toBe("kept");
  });

  it("does not crash a newline with no caret in the document", () => {
    // error-path: the selection can be gone when the key arrives.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    setText("text");
    act(() => window.getSelection()!.removeAllRanges());
    press("Enter", { shiftKey: true });
    expect(editor.textContent).toBe("text");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("does not crash a paste with no caret in the document", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    setText("text");
    act(() => window.getSelection()!.removeAllRanges());
    const event = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", {
      value: { getData: (type: string) => (type === "text/plain" ? "pasted" : ""), items: [] },
    });
    act(() => editor.dispatchEvent(event));
    expect(editor.textContent).toBe("text");
  });
});

describe("mentionEditor keyboard submission", () => {
  it("submits on Enter and leaves Shift/Ctrl+Enter as a newline", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    setText("hello");

    press("Enter", { ctrlKey: true });
    expect(onSubmit).not.toHaveBeenCalled();

    press("Enter", { shiftKey: true });
    expect(onSubmit).not.toHaveBeenCalled();

    press("Enter");
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("hands every keystroke to the IME while composing", () => {
    // platform-cfg: with a Chinese IME, Enter commits the candidate. `isComposing`
    // and the legacy keyCode 229 are both honoured.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    setText("正在输入");

    press("Enter", { isComposing: true } as KeyboardEventInit);
    expect(onSubmit).not.toHaveBeenCalled();

    press("Escape", { isComposing: true } as KeyboardEventInit);
    press("Enter", { keyCode: 229 } as unknown as KeyboardEventInit);
    expect(onSubmit).not.toHaveBeenCalled();

    // The composition flag from the event sequence is honoured too.
    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    press("Enter");
    expect(onSubmit).not.toHaveBeenCalled();
    act(() => editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
  });

  it("ignores keys that are not ours", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    setText("hello");
    press("a");
    press("ArrowLeft");
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("does not submit a whitespace-only draft", () => {
    // boundary: the parent gets the call; the editor must still not treat a
    // Space as Enter, and the placeholder logic keeps the box "empty".
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("   ");
    press("Enter");
    expect(onSubmit).toHaveBeenCalledTimes(1);
    // The parent owns the blank-draft guard; the editor reports emptiness so it
    // can be disabled there.
    expect(editorWithHandle().getContent()).toBe("   ");
  });

  it("submits a very long draft and CJK text unchanged", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    const long = `单细胞测序 ${"x".repeat(20_000)} 结束`;
    type(long);
    press("Enter");
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(editorWithHandle().getContent()).toBe(long);
  });
});

describe("mentionEditor clipboard routing", () => {
  function paste(data: { files?: File[]; text?: string; uriList?: string }) {
    const items = (data.files ?? []).map(file => ({ getAsFile: () => file, kind: "file" }));
    const event = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", {
      value: {
        getData: (type: string) => (type === "text/plain" ? (data.text ?? "") : (data.uriList ?? "")),
        items,
      },
    });
    act(() => editor.dispatchEvent(event));
  }

  it("hands a local file URI to the parent as an attachment rather than copying bytes", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    paste({ uriList: "file:///tmp/report.pdf\r\n" });
    expect(onPasteAttachmentPaths).toHaveBeenCalledWith(["/tmp/report.pdf"]);
    expect(onPasteFiles).not.toHaveBeenCalled();
    // The URI must not also land in the draft as text.
    expect(editor.textContent).toBe("");
  });

  it("hands clipboard files with no local URI to the parent for copying", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    const file = new File(["x"], "shot.png", { type: "image/png" });
    paste({ files: [file] });
    expect(onPasteFiles).toHaveBeenCalledWith([file]);
    expect(editor.textContent).toBe("");
  });

  it("prefers the editable text when the clipboard carries both an image and text", () => {
    // platform-cfg: PowerPoint on macOS publishes both for a copied text box.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("");
    paste({ files: [new File(["x"], "shot.png", { type: "image/png" })], text: "the words" });
    expect(onPasteFiles).not.toHaveBeenCalled();
    expect(editor.textContent).toBe("the words");
  });

  it("forces plain text so pasted rich markup cannot reach the draft", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("");
    paste({ text: "<b>bold</b>\n<script>x</script>" });
    // The plain text lands verbatim (no ZWSP: it does not end in a newline) and
    // no element from the rich form is ever created.
    expect(editor.textContent).toBe("<b>bold</b>\n<script>x</script>");
    expect(editor.querySelector("b, script")).toBeNull();
  });

  it("pads a pasted trailing newline so the last line is measurable", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("");
    paste({ text: "line\n" });
    expect(editor.textContent).toBe("line\n\u200B");
    expect(editorWithHandle().getContent()).toBe("line\n");
  });

  it("re-checks for a trigger after the paste", async () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("");
    paste({ text: "@al" });
    await settleSearch();
    expect(menu()).not.toBeNull();
    // The query really reached the backend: only the matching file is offered.
    expect(menuLabels()).toEqual(["src/alpha.ts"]);
  });

  it("leaves an empty paste alone", () => {
    // boundary: a clipboard with no text, URIs or files must be a no-op.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    focusThenSetText("");
    paste({});
    expect(editor.textContent).toBe("");
    expect(onPasteFiles).not.toHaveBeenCalled();
    expect(onPasteAttachmentPaths).not.toHaveBeenCalled();
  });
});

describe("mentionEditor composition and disabled state", () => {
  it("re-checks the trigger after an IME composition commits", () => {
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    const frames: FrameRequestCallback[] = [];
    const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });

    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    setText("请在 @al");
    act(() => editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
    act(() => frames.forEach(callback => callback(0)));

    expect(storage.searchWorkspaceFiles).not.toHaveBeenCalled();
    raf.mockRestore();
  });

  it("skips the post-composition re-check when the next composition has begun", () => {
    // concurrency: `compositionend` schedules its re-check one frame later, and a
    // user resuming typing in the IME starts a new composition inside that frame.
    // The frame re-reads the ref rather than a value captured at schedule time, so
    // the re-check must be skipped - running it would resolve a trigger against a
    // half-composed buffer and report the draft as changed.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    const frames: FrameRequestCallback[] = [];
    const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });

    type("@al");
    expect(onChange).toHaveBeenCalledTimes(1);
    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    act(() => editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
    // The user resumes composing before the scheduled frame runs.
    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    act(() => frames.forEach(callback => callback(0)));

    // The frame contributed nothing: no second change report and no search kicked off.
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(storage.searchWorkspaceFiles).not.toHaveBeenCalled();
    raf.mockRestore();
  });

  it("skips the caret reveal when a composition commits with no caret left", () => {
    // boundary: the frame re-reads `window.getSelection()` because the selection
    // can change inside the frame. With the ranges cleared (the editor blurred as
    // the composed text landed) there is no caret to reveal, and `getRangeAt(0)`
    // on an empty selection throws - the guard is what keeps the frame quiet. The
    // throw would be raised inside a rAF callback, escaping any try/catch, so it is
    // captured with a global listener; without the guard the error event fires and
    // the assertion below is what fails.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    const frames: FrameRequestCallback[] = [];
    const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      frames.push(callback);
      return frames.length;
    });
    const failures: unknown[] = [];
    const capture = (event: ErrorEvent) => void failures.push(event.error ?? event.message);
    window.addEventListener("error", capture);

    act(() => editor.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    setText("hello @al");
    act(() => editor.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
    act(() => window.getSelection()!.removeAllRanges());
    act(() => frames.forEach(callback => callback(0)));
    window.removeEventListener("error", capture);

    expect(failures).toEqual([]);
    // The rest of the frame still ran: the composed text is reported as a change.
    expect(onChange).toHaveBeenCalled();
    raf.mockRestore();
  });

  it("is not editable while disabled but still shows the placeholder", () => {
    render({ disabled: true });
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    expect(editor.getAttribute("contenteditable")).toBe("false");
    expect(container.textContent).toContain("Message");
  });

  it("does not prepend a space when Enter is pressed in an empty editor", () => {
    // boundary: `insertNewline` adds a leading space only when the editor already
    // holds something (`if (!isEditorEmpty(editor))`) - otherwise pressing Enter
    // on a blank composer would insert a stray space before the new line. A
    // branch audit found the empty case at zero hits: every other newline test
    // types text first.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    press("Enter");

    const content = editorWithHandle().getContent();
    expect(content).not.toMatch(/^ /);
    expect(content.trim()).toBe("");
  });

  it("serializes a non-block wrapper element without inventing a newline", () => {
    // boundary: the serializer appends "\n" only for a browser-inserted block
    // wrapper (DIV/P), so a SPAN — which pasted rich text produces — must NOT
    // gain a line break. Both operands of that `||` were uncovered because the
    // tests only ever exercised text nodes and DIV/P wrappers.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => {
      editor.innerHTML = "<span>inline</span>";
    });

    expect(editorWithHandle().getContent()).toBe("inline");
  });

  it("survives an input event with no selection at all", async () => {
    // error-path: a webview can report an empty Selection (the range was
    // dropped, e.g. after the node under the caret was replaced). Both
    // `selection.rangeCount > 0` guards must tolerate it rather than throwing on
    // `getRangeAt(0)`; the audit found both at zero hits because every helper
    // here parks a caret first.
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    act(() => {
      editor.textContent = "@";
      window.getSelection()!.removeAllRanges();
      editor.dispatchEvent(new InputEvent("input", { bubbles: true }));
    });
    await settleSearch();

    // The observed contract: with no caret there is no trigger to resolve, so
    // the menu does not open and nothing throws. (The text alone is not a
    // trigger - the editor reads the caret's preceding text, which a dropped
    // selection cannot supply.) The search is not issued either.
    expect(menu()).toBeNull();
    expect(storage.searchWorkspaceFiles).not.toHaveBeenCalled();
  });

  it("tolerates the imperative handle being used after unmount", () => {
    // concurrency: a parent can hold the ref past the child's unmount (an async
    // send settling after the view is gone) and call the handle's entry points.
    // Each must no-op rather than touch a detached editor. `insertMention` is the
    // fourth member of the documented handle - the one the file tree calls - and
    // was the only one this test did not drive, so its `if (editor)` arm stayed
    // uncovered while the test read as if it covered them all.
    render();
    const ref = editorWithHandle();
    act(() => root.unmount());

    expect(() => ref.clear()).not.toThrow();
    expect(() => ref.restore("text")).not.toThrow();
    expect(() => ref.insertMention({ name: "f.ts", path: "src/f.ts" })).not.toThrow();
    expect(ref.getContent()).toBe("");
  });

  it("opens the slash menu with no skills prop at all", async () => {
    // boundary: `skills` is an OPTIONAL prop (`skills?: SkillMentionOption[]`), so a
    // caller that has not loaded a catalogue renders without it. The `skills ?? []`
    // arm was uncovered because every test passes `skills={SKILLS}`. Without the
    // fallback `buildSlashMenuGroups` would receive `undefined` and throw on
    // `.filter`, taking the whole composer down.
    render({ skills: undefined });
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/");
    await settleSearch();

    // The menu still opens, offering the context tool and no skills. (A context
    // tool row renders its name plus description, without the `/` prefix that
    // skill rows carry.)
    expect(menuRows().map(row => row.textContent)).toEqual(["CompactCompress the context"]);
  });

  it("renders a root-level file row without a directory label", async () => {
    // boundary: the row shows the file's directory when it has one. For a file at
    // the workspace root the computed `dir` is the empty string, so the label must
    // be omitted rather than rendering an empty span - the `dir ? ... : null` arm,
    // uncovered because every fixture file lived under a directory.
    storage.searchWorkspaceFiles.mockImplementation(async () => [{ name: "README.md", path: "README.md" }]);
    render();
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("@README");
    await settleSearch();

    const row = menuRows()[0]!;
    expect(row.textContent).toBe("README.md");
    // No directory span was emitted (the icon is an `svg`, the name a `span`).
    expect(row.querySelectorAll("span.text-ink-muted")).toHaveLength(0);
  });

  it("renders a skill row with no description", async () => {
    // boundary: a skill may carry no description, in which case the row shows only
    // its name - the description ternary's null arm, uncovered because both fixture
    // skills had descriptions.
    render({ skills: [{ description: "", name: "future-bare" }] });
    editor = container.querySelector<HTMLDivElement>("[role=textbox]")!;
    type("/bare");
    await settleSearch();

    const row = menuRows()[0]!;
    expect(row.textContent).toBe("/future-bare");
    expect(row.querySelectorAll("span.text-xs")).toHaveLength(0);
  });
});
