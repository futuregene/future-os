import type { Root } from "react-dom/client"; // @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Composer } from "./Composer";

/**
 * The composer's non-text surface: the `/compact` context tool, attachment
 * add/reject/remove through the path classifier, the vision-capability warning,
 * the drag-and-drop verdict, and the send/disabled state machine. The plain-text
 * delivery, focus, paste and skill-recommendation paths have their own suites.
 */
const h = vi.hoisted(() => ({
  classifyAttachment: vi.fn(),
  dragHandlers: [] as ((event: { payload: { paths?: string[]; type: string } }) => void)[],
  listAvailableSkills: vi.fn(async () => []),
  listInstalledSkills: vi.fn(async () => []),
  savePastedImage: vi.fn(async () => ({ path: "/tmp/pasted.png" })),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (handler: (event: { payload: { paths?: string[]; type: string } }) => void) => {
      h.dragHandlers.push(handler);
      return () => {
        const index = h.dragHandlers.indexOf(handler);
        if (index >= 0)
          h.dragHandlers.splice(index, 1);
      };
    },
  }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => (command === "list_agent_providers" ? { builtin: [], custom: [] } : [])),
}));
vi.mock("./attachments", async original => ({
  ...await original<typeof import("./attachments")>(),
  classifyAttachment: h.classifyAttachment,
}));
vi.mock("../../integrations/agent/agentClient", async original => ({
  ...await original<typeof import("../../integrations/agent/agentClient")>(),
  savePastedImage: h.savePastedImage,
}));
vi.mock("../../integrations/skills/skillsClient", () => ({
  listAvailableSkills: h.listAvailableSkills,
  listInstalledSkills: h.listInstalledSkills,
  loadSkillCatalog: () => ({
    catalogue: h.listAvailableSkills(),
    installed: h.listInstalledSkills(),
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

function render(over: Partial<Parameters<typeof Composer>[0]> = {}) {
  const onSend = over.onSend ?? vi.fn(async () => {});
  act(() => root.render(
    <Composer modelOptions={[{ id: "m1", name: "Model", supportsImages: true }] as never} onSend={onSend} {...over} />,
  ));
  return onSend;
}

beforeEach(() => {
  h.classifyAttachment.mockReset();
  h.classifyAttachment.mockImplementation(async (path: string) => ({ kind: "file", reason: null, path }));
  h.dragHandlers.length = 0;
  h.listAvailableSkills.mockResolvedValue([]);
  h.listInstalledSkills.mockResolvedValue([]);
  h.savePastedImage.mockResolvedValue({ path: "/tmp/pasted.png" });
  // jsdom implements no layout; the menus scroll the highlighted row into view.
  Element.prototype.scrollIntoView = vi.fn();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function editor() {
  return container.querySelector<HTMLDivElement>("[role=textbox]")!;
}

function type(text: string) {
  act(() => {
    editor().textContent = text;
    const node = editor().firstChild as Text;
    const range = document.createRange();
    range.setStart(node, text.length);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
}

function slashItem(label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("[data-menu-index]")]
    .find(row => row.textContent?.includes(label));
}

/** Attachment chips are the only remove buttons. */
function chips() {
  return [...container.querySelectorAll<HTMLButtonElement>("button[aria-label^='Remove']")];
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** A drag-drop event from the webview. */
async function drag(payload: { paths?: string[]; type: string }) {
  await act(async () => {
    h.dragHandlers.forEach(handler => handler({ payload }));
    await Promise.resolve();
  });
}

it("offers the /compact tool only when the session can compact, and runs it once", async () => {
  const onCompactContext = vi.fn(async () => {});
  render({ onCompactContext });
  await flush();

  type("/compact");
  const row = slashItem("Compact");
  expect(row).toBeDefined();
  act(() => row!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
  await flush();
  expect(onCompactContext).toHaveBeenCalledTimes(1);

  // concurrency: while the compaction is in flight the tool is withdrawn, so no
  // second request can be queued from the menu.
  act(() => root.unmount());
  root = createRoot(container);
  let release: () => void = () => {};
  const pending = vi.fn(() => new Promise<void>((resolve) => {
    release = resolve;
  }));
  render({ onCompactContext: pending });
  await flush();
  type("/compact");
  act(() => slashItem("Compact")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
  await flush();
  expect(pending).toHaveBeenCalledTimes(1);

  // The menu is closed and the tool is gone while pending.
  type("/compact");
  expect(slashItem("Compact")).toBeUndefined();
  release();
});

it("withdraws the /compact tool while a run is streaming", async () => {
  const onCompactContext = vi.fn(async () => {});
  render({ onCompactContext, sending: true });
  await flush();
  type("/compact");
  expect(container.querySelector("[data-menu-index]")).toBeNull();
});

it("tolerates a compaction call that returns nothing", async () => {
  // boundary: the handler may be synchronous with no promise to await.
  const onCompactContext = vi.fn(() => undefined as unknown as Promise<void>);
  render({ onCompactContext });
  await flush();
  type("/compact");
  act(() => slashItem("Compact")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
  await flush();
  expect(onCompactContext).toHaveBeenCalledTimes(1);
  // Still offered afterwards: nothing was left pending.
  type("/compact");
  expect(slashItem("Compact")).toBeDefined();
});

it("reports a failed compaction request without leaving the tool pending", async () => {
  // error-path: a rejected compaction must not wedge the menu shut.
  const onCompactContext = vi.fn(() => Promise.reject(new Error("agent is busy")));
  render({ onCompactContext });
  await flush();
  type("/compact");
  act(() => slashItem("Compact")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true })));
  await flush();
  type("/compact");
  expect(slashItem("Compact")).toBeDefined();
});

it("attaches a dropped path and clears the drag verdict", async () => {
  render();
  await flush();
  expect(h.dragHandlers).toHaveLength(1);

  await drag({ paths: ["/tmp/a.ts"], type: "enter" });
  const drop = container.firstElementChild!;
  expect(drop.className).toContain("ring-focus");

  await drag({ paths: ["/tmp/a.ts"], type: "drop" });
  await flush();
  expect(h.classifyAttachment).toHaveBeenCalledWith("/tmp/a.ts");
  expect(container.textContent).toContain("a.ts");
  expect(container.firstElementChild!.className).not.toContain("ring-focus");
});

it("rejects a drag that carries no local path", async () => {
  render();
  await flush();
  await drag({ paths: [], type: "enter" });
  expect(container.firstElementChild!.className).toContain("ring-danger-line");

  // boundary: an `over` without a verdict keeps the one decided on `enter`.
  await drag({ paths: [], type: "over" });
  expect(container.firstElementChild!.className).toContain("ring-danger-line");

  await drag({ type: "leave" });
  expect(container.firstElementChild!.className).not.toContain("ring-danger-line");
});

it("keeps the drag verdict through an `over` that carries no paths", async () => {
  render();
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "enter" });
  await drag({ type: "over" });
  expect(container.firstElementChild!.className).toContain("ring-focus");
});

it("ignores a drag event whose type it does not recognise", async () => {
  // boundary: the webview chain is enter/over/leave/drop, and anything else now falls
  // past the last `else if`. Such a payload must leave the drop zone alone rather
  // than being treated as a drop (attaching a file) or as a leave (clearing the
  // verdict the user is looking at). Only the four known types were ever dispatched,
  // so the final branch's false arm never ran.
  render();
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "enter" });
  expect(container.firstElementChild!.className).toContain("ring-focus");

  await drag({ paths: ["/tmp/a.ts"], type: "mystery" });
  await flush();

  // Nothing attached, and the verdict is still the one the `enter` decided.
  expect(h.classifyAttachment).not.toHaveBeenCalled();
  expect(container.textContent).not.toContain("a.ts");
  expect(container.firstElementChild!.className).toContain("ring-focus");
});

it("accepts the drag when the very first event is an `over`", async () => {
  // boundary: the webview can deliver `over` before any `enter` (the pointer
  // entered the window already inside the drop zone). There is then no recorded
  // verdict, and the `dragStateRef.current ?? "accept"` fallback decides - a
  // branch the other tests never reach because each sends `enter` first, so
  // without it a first-`over` drag would be treated as... whatever the initial
  // ref happens to be.
  render();
  await flush();
  await drag({ type: "over" });
  expect(container.firstElementChild!.className).toContain("ring-focus");
  expect(container.firstElementChild!.className).not.toContain("ring-danger-line");
});

it("renders a dotless attachment without an empty extension span", async () => {
  // boundary: `splitFileName` returns `ext: ""` for a name with no dot, and the
  // chip renders the extension span only when `ext` is truthy. Asserted on the
  // DOM rather than the text, because an empty `<span>` contributes no text - so
  // a text-only assertion could not tell the two branches apart.
  render();
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "drop" });
  await flush();
  await drag({ paths: ["/tmp/README"], type: "drop" });
  await flush();

  expect(chips()).toHaveLength(2);
  // The chip container is the remove button's parent (`span.inline-flex max-w-64`);
  // the extension is rendered as a `span.shrink-0` inside it (the icon is an `svg`
  // and the remove control is a `button`, so neither is counted).
  expect(chips()[0]!.parentElement!.querySelectorAll("span.shrink-0")).toHaveLength(1);
  expect(chips()[1]!.parentElement!.querySelectorAll("span.shrink-0")).toHaveLength(0);
});

it("ignores a drop with no paths", async () => {
  render();
  await flush();
  await drag({ paths: [], type: "drop" });
  await flush();
  expect(h.classifyAttachment).not.toHaveBeenCalled();
});

it("does not listen for drags while the composer is disabled", async () => {
  render({ disabled: true });
  await flush();
  expect(h.dragHandlers).toHaveLength(0);
});

it("stops listening for drags once the composer unmounts", async () => {
  // concurrency: the listener is registered asynchronously and removed on
  // teardown, so a drop after unmount cannot attach anything.
  render();
  await flush();
  expect(h.dragHandlers).toHaveLength(1);
  act(() => root.unmount());
  await flush();
  expect(h.dragHandlers).toHaveLength(0);
  // Recreate the root so the shared `afterEach` unmount stays valid.
  root = createRoot(container);
});

it("reports each rejected attachment with its reason and keeps the accepted ones", async () => {
  // error-path: a directory or an unreadable file is refused, not silently
  // dropped, and the good path in the same batch still attaches.
  h.classifyAttachment.mockImplementation(async (path: string) =>
    path.endsWith("dir") ? { kind: null, reason: "Folders are not supported" } : { kind: "file", path });
  render();
  await flush();
  await drag({ paths: ["/tmp/dir", "/tmp/good.ts"], type: "drop" });
  await flush();

  expect(container.textContent).toContain("good.ts");
  // The refusal names the path and its reason instead of failing silently.
  expect(container.textContent).toContain("Folders are not supported");
  expect(chips()).toHaveLength(1);
});

it("attaches the same path only once", async () => {
  // boundary: a duplicate drop must not create a second chip.
  render();
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "drop" });
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "drop" });
  await flush();

  expect(chips()).toHaveLength(1);
});

it("caps the number of images in one turn", async () => {
  // boundary: images carry a per-message ceiling (`MAX_IMAGES_PER_TURN`, 4); a
  // fifth is refused with a reason rather than silently dropped.
  h.classifyAttachment.mockImplementation(async (path: string) => ({ kind: "image", path }));
  render();
  await flush();
  const paths = Array.from({ length: 5 }, (_, index) => `/tmp/shot-${index}.png`);
  await drag({ paths, type: "drop" });
  await flush();

  expect(chips()).toHaveLength(4);
  expect(container.textContent).toContain("shot-4.png");
});

it("removes an attachment from its chip", async () => {
  render();
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "drop" });
  await flush();
  expect(chips()).toHaveLength(1);

  act(() => chips()[0]!.click());
  expect(chips()).toHaveLength(0);
});

it("warns about an image the current model cannot see", async () => {
  // platform-cfg: capability is a per-model flag, so the chip must follow it.
  h.classifyAttachment.mockImplementation(async (path: string) => ({ kind: "image", path }));
  render({ modelId: "text-only", modelOptions: [{ id: "text-only", name: "Text", supportsImages: false }] as never });
  await flush();
  await drag({ paths: ["/tmp/shot.png"], type: "drop" });
  await flush();

  const chip = chips()[0]!.closest("span")!;
  expect(chip.getAttribute("title")).toContain("not");

  // A vision model shows the plain chip instead.
  act(() => root.unmount());
  root = createRoot(container);
  render({ modelId: "vision", modelOptions: [{ id: "vision", name: "Vision", supportsImages: true }] as never });
  await flush();
  await drag({ paths: ["/tmp/shot.png"], type: "drop" });
  await flush();
  expect(chips()[0]!.closest("span")!.getAttribute("title")).toBe("shot.png");
});

it("sends the draft with its attachments and clears the box", async () => {
  const onSend = vi.fn(async () => {});
  render({ onSend });
  await flush();
  await drag({ paths: ["/tmp/a.ts"], type: "drop" });
  await flush();
  type("hello");

  act(() => {
    editor().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" }));
  });
  await flush();

  expect(onSend).toHaveBeenCalledWith({ attachments: [expect.objectContaining({ path: "/tmp/a.ts" })], content: "hello" });
  expect(editor().textContent).toBe("");
});

it("keeps the draft when the send is refused", async () => {
  // error-path: a refused send must not wipe the composed message.
  const onSend = vi.fn(() => Promise.reject(new Error("already running")));
  render({ onSend });
  await flush();
  type("keep me");
  act(() => {
    editor().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" }));
  });
  await flush();
  expect(editor().textContent).toBe("keep me");
});

it("falls back to the platform catalogue's Chinese text for a skill description", async () => {
  // serialization: installed-skill frontmatter often lacks name_zh/description_zh,
  // so the catalogue (which always carries them) is the fallback.
  h.listInstalledSkills.mockResolvedValue([
    { description: "Search the web", id: "future-web", name: "future-web", version: "1.0" },
  ] as never);
  h.listAvailableSkills.mockResolvedValue([] as never);
  render();
  await flush();
  type("/web");
  expect([...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent)).toEqual(["/future-webSearch the web"]);
});

it("survives a skill catalogue that cannot be read", async () => {
  // error-path: the / menu must still work from the installed list alone.
  h.listInstalledSkills.mockResolvedValue([
    { description: "Write a paper", id: "future-paper", name: "future-paper", version: "1.0" },
  ] as never);
  h.listAvailableSkills.mockRejectedValue(new Error("platform offline"));
  render();
  await flush();
  type("/paper");
  expect([...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent)).toEqual(["/future-paperWrite a paper"]);
});
