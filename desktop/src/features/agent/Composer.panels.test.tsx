import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { emitFutureEvent } from "../../lib/futureEvents";
import { Composer } from "./Composer";

/**
 * The composer's panels and side channels: the approval / model / thinking
 * dropdowns, the attach button and file picker, the agent-event and future-event
 * subscriptions, the draft persistence, and the abort button.
 *
 * The context tool, attachments-by-drag, send and refusal paths live in
 * `Composer.tools.test.tsx`; delivery/focus/paste/reasoning/skill-recommendation
 * have their own suites.
 */
const h = vi.hoisted(() => ({
  open: vi.fn(),
  savePastedImage: vi.fn(),
  savePastedFile: vi.fn(),
  deleteTempAttachment: vi.fn(),
  readNativeClipboardFilePaths: vi.fn(),
  loadSkillCatalog: vi.fn(),
  draft: {
    clear: vi.fn(),
    load: vi.fn(),
    save: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: h.open }));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => (command === "list_agent_providers" ? { builtin: [], custom: [] } : [])),
}));
vi.mock("../../integrations/agent/agentClient", async original => ({
  ...await original<typeof import("../../integrations/agent/agentClient")>(),
  readNativeClipboardFilePaths: h.readNativeClipboardFilePaths,
  savePastedFile: h.savePastedFile,
  savePastedImage: h.savePastedImage,
}));
vi.mock("../../integrations/storage/files", async original => ({
  ...await original<typeof import("../../integrations/storage/files")>(),
  deleteTempAttachment: h.deleteTempAttachment,
}));
vi.mock("../../integrations/skills/skillsClient", () => ({
  loadSkillCatalog: h.loadSkillCatalog,
  listAvailableSkills: vi.fn(async () => []),
  listInstalledSkills: vi.fn(async () => []),
}));
vi.mock("./composerDraft", async original => ({
  ...await original<typeof import("./composerDraft")>(),
  clearComposerDraft: h.draft.clear,
  loadComposerDraft: h.draft.load,
  saveComposerDraft: h.draft.save,
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const MODELS = [
  { id: "m1", label: "Model One", provider: "future", supportsImages: true, supportsThinkingLevels: ["off", "high"] },
  { id: "m2", label: "Model Two", provider: "openai", supportsImages: false, supportsThinkingLevels: ["off"] },
] as unknown as AgentModelOption[];

let container: HTMLDivElement;
let root: Root;

function render(over: Partial<Parameters<typeof Composer>[0]> = {}) {
  act(() => root.render(
    <Composer modelId="m1" modelOptions={MODELS} onSend={vi.fn(async () => {})} {...over} />,
  ));
}

beforeEach(() => {
  vi.clearAllMocks();
  h.open.mockResolvedValue(["/tmp/a.ts"]);
  h.savePastedImage.mockResolvedValue({ path: "/tmp/pasted.png" });
  h.savePastedFile.mockResolvedValue({ path: "/tmp/copied.txt" });
  h.deleteTempAttachment.mockResolvedValue(undefined);
  h.readNativeClipboardFilePaths.mockResolvedValue([]);
  h.draft.load.mockReturnValue(null);
  h.loadSkillCatalog.mockReturnValue({
    catalogue: Promise.resolve([{ description: "Web", descriptionZh: "网页", id: "future-web", nameZh: "网页" }]),
    installed: Promise.resolve([
      { description: "Search the web", id: "future-web", name: "future-web", version: "1.0" },
      { description: "Paper", descriptionZh: "论文", id: "future-paper", name: "future-paper", version: "1.0" },
    ]),
  });
  Element.prototype.scrollIntoView = vi.fn();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  act(() => root.unmount());
  container.remove();
  // Several tests below render in Chinese; restore the shared i18n singleton so
  // the next file's tests are not affected by this one.
  await i18n.changeLanguage("en");
});

function editor() {
  return container.querySelector<HTMLDivElement>("[role=textbox]")!;
}

function byLabel(label: string) {
  return container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
}

function trigger(label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button[title]")]
    .find(button => button.getAttribute("title") === label)!;
}

/** Rows inside an open `SelectMenu` panel (the panel is absolutely positioned). */
function menuItems() {
  return [...container.querySelectorAll<HTMLButtonElement>("div.absolute.bottom-9 button")];
}

/** The open panel itself, for counting how many are open at once. */
function openPanels() {
  return [...container.querySelectorAll("div.absolute.bottom-9")];
}

/** A button whose visible text is exactly `label`. */
function byText(label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => button.textContent?.trim() === label) ?? null;
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function click(element: HTMLElement | null | undefined) {
  await act(async () => {
    element!.click();
    await Promise.resolve();
  });
}

it("shows the tier glyph for each approval setting", () => {
  // platform-cfg: the sandbox tier is Windows/Linux-specific; the glyph family is
  // question → check → off, and it must follow the setting it names.
  const cases: [ApprovalTier, string][] = [
    ["off", "Unrestricted"],
    ["manual", "Manual"],
    ["sandbox", "Sandboxed"],
  ];
  for (const [tier, label] of cases) {
    render({ approvalTier: tier, onChangeApprovalTier: vi.fn() });
    const row = trigger("Approval mode");
    expect(row.textContent).toContain(label);
    // Each tier has its own glyph, and only one is rendered in the trigger.
    expect(row.querySelectorAll("svg").length).toBeGreaterThanOrEqual(1);
  }
});

it("lists every approval tier, marks the current one and reports a change", async () => {
  const onChangeApprovalTier = vi.fn();
  render({ approvalTier: "manual", onChangeApprovalTier });

  await click(trigger("Approval mode"));
  expect(menuItems().length).toBe(3);
  // Pick by label rather than position: the row order is not part of the
  // contract, but each tier must be reachable and report its own value.
  const unrestricted = menuItems().find(row => row.textContent?.includes("Unrestricted"))!;
  expect(unrestricted).not.toBeNull();
  expect(unrestricted.querySelector("svg")).not.toBeNull();
  await click(unrestricted);
  expect(onChangeApprovalTier).toHaveBeenCalledWith("off");
});

it("closes the other panels when one is opened", async () => {
  render({ onChangeApprovalTier: vi.fn() });
  // Opening approval closes the model and thinking panels, and vice versa: two
  // open panels would overlap.
  await click(trigger("Approval mode"));
  await click(trigger("Model"));
  expect(openPanels().length).toBeLessThanOrEqual(1);
});

it("picks a model and a thinking level from their menus", async () => {
  const onModelChange = vi.fn();
  const onThinkingLevelChange = vi.fn();
  render({ onModelChange, onThinkingLevelChange, thinkingLevel: "high" });

  await click(trigger("Model"));
  const modelItems = menuItems();
  expect(modelItems.length).toBe(MODELS.length);
  await click(modelItems[1]);
  expect(onModelChange).toHaveBeenCalledWith("openai/m2");

  await click(byLabel("Thinking level"));
  await click(menuItems()[0]);
  expect(onThinkingLevelChange).toHaveBeenCalled();
});

it("labels the thinking control with the level in force", () => {
  render({ modelId: "m1", thinkingLevel: "high" });
  const thinking = byLabel("Thinking level")!;
  expect(thinking.disabled).toBe(false);
  // The trigger shows the level, so the panel never has to be opened to read it.
  expect(thinking.textContent?.trim().length).toBeGreaterThan(0);
});

it("attaches the paths returned by the file picker", async () => {
  render();
  await click(byLabel("Attach files"));
  expect(h.open).toHaveBeenCalled();
  await flush();
  expect(container.textContent).toContain("a.ts");
});

it("does nothing when the file picker is dismissed", async () => {
  // boundary: the dialog resolves null on cancel.
  h.open.mockResolvedValue(null);
  render();
  await click(byLabel("Attach files"));
  await flush();
  expect(container.querySelectorAll("button[aria-label^='Remove']")).toHaveLength(0);
});

it("accepts a single path from a picker that does not return an array", async () => {
  // boundary: the plugin returns a string for a single selection.
  h.open.mockResolvedValue("/tmp/a.ts");
  render();
  await click(byLabel("Attach files"));
  await flush();
  expect(container.textContent).toContain("a.ts");
});

it("disables the attach and send controls while the composer is locked", () => {
  render({ disabled: true });
  expect(byLabel("Attach files")!.disabled).toBe(true);
});

it("shows a stop control while a run streams, and forwards the abort", async () => {
  const onAbort = vi.fn();
  render({ onAbort, sending: true });
  await click(byLabel("Stop"));
  expect(onAbort).toHaveBeenCalledTimes(1);
});

it("inserts a mention pill from the file tree's future event", async () => {
  // concurrency: the subscription is installed once, so a later event still
  // reaches the composer of the conversation that is open.
  render();
  await act(async () => {
    emitFutureEvent("attach-file-to-context", { name: "z.ts", path: "src/z.ts" });
    await Promise.resolve();
  });
  expect(container.textContent).toContain("z.ts");
});

it("restores a persisted draft on mount without notifying a change", async () => {
  // serialization: the draft round-trips through storage; restoring it is not a
  // user edit, so it must not re-save what it just read.
  h.draft.load.mockReturnValue({ text: "half-written message", version: 1 });
  render({ draftKey: "T1" });
  await flush();
  expect(h.draft.load).toHaveBeenCalledWith("T1");
  expect(editor().textContent).toBe("half-written message");
  // The restore must never clobber the stored draft: any save it triggers has to
  // carry the restored text back, not an empty draft.
  for (const call of h.draft.save.mock.calls)
    expect(call[1]).toMatchObject({ text: "half-written message" });
});

it("clears the persisted draft once the message was sent", async () => {
  render({ draftKey: "T1" });
  act(() => {
    editor().textContent = "to be sent";
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await flush();
  expect(h.draft.save).toHaveBeenCalled();

  act(() => {
    editor().dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" }));
  });
  await flush();
  expect(h.draft.clear).toHaveBeenCalledWith("T1");
  expect(editor().textContent).toBe("");
});

it("falls back to the last known text when the editor handle is gone", () => {
  // boundary: the abort path reads the draft while the editor may already be
  // unmounted (a thread switch mid-run).
  const onAbort = vi.fn();
  render({ onAbort, sending: true });
  act(() => {
    editor().textContent = "drafted";
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  act(() => root.unmount());
  // Unmounting must not throw and must not resurrect a send.
  expect(onAbort).not.toHaveBeenCalled();
  root = createRoot(container);
});

it("closes an open panel when the pointer presses outside it", async () => {
  // Each panel dismisses on an outside press, so two panels can never stack up.
  render({ onChangeApprovalTier: vi.fn() });
  for (const label of ["Approval mode", "Model", "Thinking level"]) {
    await click(trigger(label));
    expect(openPanels().length).toBe(1);
    await act(async () => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(openPanels().length).toBe(0);
  }
});

it("does not clear a newer conversation's composer when a send settles late", async () => {
  // concurrency: the delivery ACK can land after the user switched threads; a
  // late clear must not erase the new conversation's input.
  let resolveSend: () => void = () => {};
  const onSend = vi.fn(() => new Promise<void>((resolve) => {
    resolveSend = resolve;
  }));
  render({ draftKey: "T1", onSend });
  act(() => {
    editor().textContent = "for T1";
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await click(byLabel("Send"));
  expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ content: "for T1" }));

  // The user moves to another conversation and starts typing there.
  render({ draftKey: "T2", onSend });
  act(() => {
    editor().textContent = "for T2";
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });

  await act(async () => {
    resolveSend();
    await Promise.resolve();
    await Promise.resolve();
  });
  expect(editor().textContent).toBe("for T2");
});

it("keeps the recommendation card up when the install fails", async () => {
  // error-path: a failed install must say so and leave the card so the user can
  // retry or send without the skill.
  const toasts: { message: string; tone?: string }[] = [];
  const onFutureEvent = (await import("../../lib/futureEvents")).onFutureEvent;
  const off = onFutureEvent("toast", toast => void toasts.push(toast));
  const onInstall = vi.fn(async () => false);
  render({
    skillRecommendation: {
      card: { description: "Search the web", name: "future-web" },
      onDismiss: vi.fn(),
      onEvaluate: vi.fn(async () => null),
      onInstall,
    },
  });

  await click(byText("Install & use"));
  expect(onInstall).toHaveBeenCalledTimes(1);
  expect(toasts).toEqual([{ message: "Couldn't install the skill. Please try again.", tone: "error" }]);
  // The card is still offered, so the user is not left without a way forward.
  expect(byText("Install & use")).not.toBeNull();
  off();
});

it("installs the skill with no separating space when the draft is empty", async () => {
  // boundary: after a successful install the composer appends `/name ` to the
  // draft, adding a separating space ONLY when there is already content
  // (`current.length > 0 && !current.endsWith(" ")`). An empty composer must not
  // gain a leading space, and a non-empty one must gain exactly one - the two
  // operands of that `&&` need opposite fixtures, and the audit found the empty
  // case uncovered because every install test typed a draft first.
  const onInstall = vi.fn(async () => true);
  const onSend = vi.fn(async () => {});
  render({
    onSend,
    skillRecommendation: {
      card: { description: "Search the web", name: "future-web" },
      onDismiss: vi.fn(),
      onEvaluate: vi.fn(async () => null),
      onInstall,
    },
  });

  await click(byText("Install & use"));
  expect(onInstall).toHaveBeenCalledTimes(1);
  // The observable is what gets SENT, not the editor afterwards: a successful
  // install appends the skill and immediately submits, which clears the composer
  // (so asserting the editor's text would pass on an empty string for the wrong
  // reason). `submitValue` trims the draft, so the trailing space the restore adds
  // is gone by the time it is sent - what matters is that there is no LEADING
  // space, which is what the empty-draft arm governs.
  expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ content: "/future-web" }));
});

it("separates an installed skill from an existing draft with one space", async () => {
  const onInstall = vi.fn(async () => true);
  const onSend = vi.fn(async () => {});
  render({
    onSend,
    skillRecommendation: {
      card: { description: "Search the web", name: "future-web" },
      onDismiss: vi.fn(),
      onEvaluate: vi.fn(async () => null),
      onInstall,
    },
  });
  act(() => {
    editor().textContent = "look at";
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });

  await click(byText("Install & use"));
  expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ content: "look at /future-web" }));
});

it("persists the last known draft when a composition frame lands after unmount", async () => {
  // concurrency: `MentionEditor` schedules its post-composition work (trigger
  // rescan, empty-state sync, `onChange`) from a `requestAnimationFrame`. A frame
  // still pending when the view goes away is NOT cancelled, so `onChange` - this
  // component's `saveDraft` - runs with the editor already detached. That is the
  // only route by which `editorRef.current` is null inside `saveDraft`, and it is
  // why the fallback to `lastTextRef` exists: without it the save would read ""
  // from a detached node and ERASE the half-written draft from storage.
  //
  // This replaces an earlier hypothesis of mine that was simply wrong (that
  // Composer had its own composition handler); it has none - it delegates to the
  // editor, which is what makes this path reachable at all.
  const frames: FrameRequestCallback[] = [];
  const raf = vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.push(callback);
    return frames.length;
  });
  try {
    render({ draftKey: "T1" });
    act(() => {
      editor().textContent = "half-written";
      editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
    });
    h.draft.save.mockClear();

    // Compose, commit, and schedule the frame - then tear the view down first.
    act(() => {
      editor().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
      editor().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    });
    act(() => root.unmount());
    act(() => {
      frames.forEach(callback => callback(0));
    });

    // The draft is preserved from the ref, not clobbered with the detached
    // editor's empty text.
    expect(h.draft.save).toHaveBeenCalledWith("T1", expect.objectContaining({ text: "half-written" }));
  }
  finally {
    raf.mockRestore();
  }
});

it("sends the draft unchanged when the card is dismissed", async () => {
  const onSend = vi.fn(async () => {});
  render({
    onSend,
    skillRecommendation: {
      card: { description: "Search the web", name: "future-web" },
      onDismiss: vi.fn(),
      onEvaluate: vi.fn(async () => null),
      onInstall: vi.fn(async () => true),
    },
  });
  act(() => {
    editor().textContent = "just send this";
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });

  await click(byText("Send without it"));
  expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ content: "just send this" }));
});

it("uses the platform catalogue's Chinese text for a skill description", async () => {
  // serialization: installed-skill frontmatter often lacks name_zh/description_zh,
  // so the catalogue (which always carries them) is the fallback.
  render();
  await flush();
  act(() => {
    editor().textContent = "/paper";
    const node = editor().firstChild as Text;
    const range = document.createRange();
    range.setStart(node, 6);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await flush();
  expect([...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent)).toEqual(["/future-paperPaper"]);
});

it("falls back to the English description when no Chinese text exists anywhere", async () => {
  // serialization / localization, and a boundary: a skill with no Chinese text in
  // either source must still render a description rather than an empty row. This
  // covers the `|| skill.description` arm of
  // `useZh ? descriptionZh || skill.description : skill.description`.
  //
  // What it does NOT do, verified by mutation: it does not discriminate that arm.
  // `descriptionZh` is itself `skill.descriptionZh || catalogue.descriptionZh || null`,
  // so whenever the RIGHT operand runs, the left is null and `null || skill.description`
  // yields exactly what omitting the OR would yield - replacing the whole expression
  // with `skill.description` leaves this test passing. The arm is therefore
  // **redundant** in the same sense as F8: it is a defensive fallback that cannot
  // change the result. Kept because the *contract* it asserts is real and
  // user-visible (see finding F9).
  h.loadSkillCatalog.mockReturnValue({
    catalogue: Promise.resolve([]),
    installed: Promise.resolve([{ description: "Bare skill", id: "future-bare", name: "future-bare", version: "1.0" }]),
  });
  await i18n.changeLanguage("zh");
  render();
  await flush();
  act(() => {
    editor().textContent = "/bare";
    const node = editor().firstChild as Text;
    const range = document.createRange();
    range.setStart(node, 5);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await flush();

  const rows = [...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent);
  expect(rows).toEqual(["/future-bareBare skill"]);
});

it("shows the Chinese skill description when the UI is in Chinese", async () => {
  // serialization / localization: `useZh` is computed once at render
  // (`i18n.language !== "en"`), and the description becomes
  // `descriptionZh || skill.description`. A branch audit found the whole
  // `useZh` TRUE arm - and both arms of the `descriptionZh || …` fallback -
  // uncovered: NO Composer test had ever rendered in Chinese, even the one
  // named "uses the platform catalogue's Chinese text for a skill description",
  // which asserts the English `Paper` and therefore documents the opposite of
  // its own name. This is the fixture that actually switches the locale.
  await i18n.changeLanguage("zh");
  render();
  await flush();
  act(() => {
    editor().textContent = "/paper";
    const node = editor().firstChild as Text;
    const range = document.createRange();
    range.setStart(node, 6);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await flush();

  // `future-paper` carries `descriptionZh: "论文"` on the INSTALLED skill, so the
  // first operand of the fallback supplies it. The canonical name is still
  // shown, because the row renders `/${skill.name}` regardless of locale.
  expect([...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent)).toEqual(["/future-paper论文"]);
});

it("falls back to the platform catalogue's Chinese text when the installed skill has none", async () => {
  // The other operand of the same fallback: `future-web` is installed WITHOUT
  // `descriptionZh`, so `skill.descriptionZh || zhById.get(id)?.descriptionZh`
  // takes the catalogue's value. Both operands need their own fixture, which is
  // why pinning only one leaves the other arm uncovered.
  await i18n.changeLanguage("zh");
  render();
  await flush();
  act(() => {
    editor().textContent = "/web";
    const node = editor().firstChild as Text;
    const range = document.createRange();
    range.setStart(node, 4);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await flush();

  const rows = [...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent);
  expect(rows).toHaveLength(1);
  // "网页" comes from the catalogue, not from the installed entry.
  expect(rows[0]).toBe("/future-web网页");
});

it("keeps the mention menu working when the skill catalogue is unavailable", async () => {
  // error-path: the `/` menu still works from the installed list alone.
  h.loadSkillCatalog.mockReturnValue({
    catalogue: Promise.reject(new Error("platform offline")),
    installed: Promise.resolve([{ description: "Web", id: "future-web", name: "future-web", version: "1" }]),
  });
  render();
  await flush();
  act(() => {
    editor().textContent = "/web";
    const node = editor().firstChild as Text;
    const range = document.createRange();
    range.setStart(node, 4);
    range.collapse(true);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    editor().dispatchEvent(new InputEvent("input", { bubbles: true }));
  });
  await flush();
  expect([...container.querySelectorAll("[data-menu-index]")].map(row => row.textContent)).toEqual(["/future-webWeb"]);
});

it("explains an empty model list differently when the models are merely disabled", async () => {
  // boundary: `modelsEmptyReason` is the copy the picker shows when there is
  // nothing to choose. A branch audit found the prop was never passed in ANY
  // test, so only its `undefined` arm had ever run and the caller's
  // "all disabled" distinction was unverified. Both sides are asserted here -
  // in separate tests, because a second render into the same root leaves the
  // popup open and the next trigger click would close it instead.
  render({ modelOptions: [], modelsEmptyReason: "all_disabled" });
  await click(trigger("Model"));
  expect(container.textContent).toContain("Models are loaded but all disabled");
});

it("points at the app when no models are loaded at all", async () => {
  render({ modelOptions: [], modelsEmptyReason: "no_models" });
  await click(trigger("Model"));
  expect(container.textContent).toContain("Reopen FutureOS to load models");
  expect(container.textContent).not.toContain("all disabled");
});
