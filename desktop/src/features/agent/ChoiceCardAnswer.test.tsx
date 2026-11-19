// @vitest-environment jsdom
import type { ChoiceCardAnswer } from "../markdown/MarkdownContent";
import type { ComposerHandle, ComposerSendPayload } from "./Composer";
import type { ChoiceAnswer, ChoiceCardQuestion } from "./composer/choiceAnswer";
import { act, useCallback, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetModelCatalogRefreshForTests } from "../../integrations/agent/modelCatalogRefresh";
import { resetShortcutScopes } from "../keyboard/scopes";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { Composer } from "./Composer";
import { loadComposerDraft } from "./composerDraft";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const DRAFT_KEY = "thread-1";

/** A reply whose closing block is the question the card asks. */
const REPLY = [
  "先把这一轮收个口。",
  "",
  "```futureos-choice",
  "title: 接下来做哪一件？",
  "option: forest | 重画森林图 | 按 Dreher 2025 的样本量重算",
  "option: flow | 补一张实验流程图",
  "option: table | 表里加一列样本量",
  "```",
].join("\n");

/**
 * The production wiring, in one component: the transcript (which owns the card)
 * and the composer (which owns the answer) talking through exactly the two
 * channels `AgentThread` gives them — the answer mirrored out of the composer,
 * and the pick written back through its handle. Nothing is stubbed, so a click
 * here travels the same path a click in the app does.
 */
function Harness({ onDismiss, onSend }: {
  /**
   * What the card's "都不做" click hands back to the caller — in the app,
   * `AgentThread`'s store write.
   */
  onDismiss?: () => void;
  onSend: (payload: ComposerSendPayload) => void | Promise<void>;
}) {
  const composerRef = useRef<ComposerHandle | null>(null);
  const [answer, setAnswer] = useState<ChoiceAnswer | null>(null);
  const handleSelect = useCallback((optionId: string, card: ChoiceCardQuestion) => {
    composerRef.current?.selectChoice(optionId, card);
  }, []);
  // The two halves of a dismissal, exactly as `AgentThread` splits them: the draft
  // is this side's to clear, the mark is the store's.
  const handleDismiss = useCallback(() => {
    composerRef.current?.clearChoice();
    onDismiss?.();
  }, [onDismiss]);
  const surface = useMemo<ChoiceCardAnswer>(() => ({
    onDismiss: onDismiss ? handleDismiss : undefined,
    onSelect: handleSelect,
    selectedIds: answer?.selectedIds ?? [],
  }), [answer, handleDismiss, handleSelect, onDismiss]);
  return (
    <>
      <MarkdownContent choiceAnswer={surface} content={REPLY} />
      <Composer
        draftKey={DRAFT_KEY}
        modelOptions={[]}
        onChoiceChange={setAnswer}
        onSend={onSend}
        ref={composerRef}
      />
    </>
  );
}

describe("the choice card's answer, from the card to the draft", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    resetAllCommandBindings();
    resetShortcutScopes();
    resetModelCatalogRefreshForTests();
    sessionStorage.clear();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    sessionStorage.clear();
  });

  async function mount(
    onSend: (payload: ComposerSendPayload) => void | Promise<void> = () => {},
    onDismiss?: () => void,
  ) {
    await act(async () => root.render(<Harness onDismiss={onDismiss} onSend={onSend} />));
    return {
      card: () => container.querySelector<HTMLElement>("[data-choice-card]")!,
      dismiss: () => container.querySelector<HTMLElement>("[data-choice-dismiss]"),
      editor: () => container.querySelector<HTMLDivElement>("[role=\"textbox\"]")!,
      pill: () => container.querySelector<HTMLElement>("[data-mention=\"choice\"]"),
      row: (id: string) => container.querySelector<HTMLElement>(`[data-choice-option="${id}"]`)!,
      sendButton: () => container.querySelector<HTMLButtonElement>("button[type=\"submit\"]")!,
      submit: async () => {
        await act(async () => {
          container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
        });
      },
      click: async (element: HTMLElement) => {
        await act(async () => element.click());
      },
    };
  }

  /** The marked row of the card — the answer read back where the reader sees it. */
  const markedRow = (card: HTMLElement) =>
    [...card.querySelectorAll<HTMLElement>("[data-choice-option]")]
      .filter(row => row.getAttribute("data-choice-selected") === "true")
      .map(row => row.getAttribute("data-choice-option"));

  it("turns a clicked row into the editor's pill, which alone arms the send button", async () => {
    const view = await mount();
    // Nothing typed and no answer: the send button is grey.
    expect(view.sendButton().disabled).toBe(true);
    expect(view.pill()).toBeNull();
    expect(markedRow(view.card())).toEqual([]);

    await view.click(view.row("forest"));

    // The answer is a pill *inside* the editor — the same element `isEditorEmpty`
    // and `ComposerControls`' `disabled` expression read, which is why the button
    // lights up without either of them being touched.
    expect(view.pill()).not.toBeNull();
    expect(view.editor().contains(view.pill())).toBe(true);
    expect(view.pill()!.textContent).toContain("重画森林图");
    expect(view.sendButton().disabled).toBe(false);
    // The card marks the row it was told about, and only that row.
    expect(markedRow(view.card())).toEqual(["forest"]);

    // The answer carries the question it came from — title, mode and every row —
    // because the pill's pencil re-offers them from the editor alone.
    const payload = JSON.parse(view.pill()!.getAttribute("data-choice")!);
    expect(payload.title).toBe("接下来做哪一件？");
    expect(payload.mode).toBe("single");
    expect(payload.options.map((option: { id: string }) => option.id)).toEqual(["forest", "flow", "table"]);
    expect(payload.selectedIds).toEqual(["forest"]);
  });

  it("writes the answer into the draft beside the text, not into it", async () => {
    const view = await mount();
    await view.click(view.row("forest"));

    const draft = loadComposerDraft(DRAFT_KEY);
    expect(draft?.choice?.selectedIds).toEqual(["forest"]);
    expect(draft?.choice?.title).toBe("接下来做哪一件？");
    // The answer lives in `choice` alone: a text carrying its `[title → …]` form
    // too would rebuild it twice on the next load. What is left is the empty
    // caret space the pill keeps beside it, never the answer.
    expect(draft?.text ?? "").not.toContain("重画森林图");
    expect((draft?.text ?? "").trim()).toBe("");
  });

  it("brings the answer back after a reload, as a pill and a marked row", async () => {
    const view = await mount();
    await view.click(view.row("flow"));
    expect(view.sendButton().disabled).toBe(false);

    // A reload: a new tree over the same session storage.
    act(() => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    const reloaded = await mount();

    expect(reloaded.pill()).not.toBeNull();
    expect(reloaded.pill()!.textContent).toContain("补一张实验流程图");
    expect(reloaded.sendButton().disabled).toBe(false);
    expect(markedRow(reloaded.card())).toEqual(["flow"]);
  });

  it("keeps reading version-1 drafts, answer included (no DRAFT_VERSION bump)", async () => {
    // A draft exactly as version 1 stored it: the same envelope, the answer as an
    // optional field. Bumping the version would have made this unreadable — and
    // every draft in every open window with it.
    sessionStorage.setItem("composer-draft:thread-1", JSON.stringify({
      version: 1,
      text: "顺手把图例挪到右下",
      choice: {
        mode: "single",
        options: [{ id: "forest", label: "重画森林图" }, { id: "flow", label: "补一张实验流程图" }],
        selectedIds: ["flow"],
        title: "接下来做哪一件？",
      },
    }));

    const view = await mount();
    expect(view.editor().textContent).toContain("顺手把图例挪到右下");
    expect(view.pill()).not.toBeNull();
    expect(view.pill()!.textContent).toContain("补一张实验流程图");
    expect(markedRow(view.card())).toEqual(["flow"]);
    expect(view.sendButton().disabled).toBe(false);
  });

  it("sends the answer as plain text and then leaves the draft without it", async () => {
    const onSend = vi.fn();
    const view = await mount(onSend);
    await view.click(view.row("forest"));
    await view.submit();

    // The answer rides in both forms: the plain text that is sent, and the
    // structured pick — the queue needs the latter to offer the other options
    // later (§5.8.3), and it must not have to re-parse the sentence.
    expect(onSend).toHaveBeenCalledWith({
      attachments: [],
      choice: {
        mode: "single",
        options: [
          { id: "forest", label: "重画森林图" },
          { id: "flow", label: "补一张实验流程图" },
          { id: "table", label: "表里加一列样本量" },
        ],
        selectedIds: ["forest"],
        title: "接下来做哪一件？",
      },
      content: "[接下来做哪一件？ → 重画森林图]",
    });
    // Requirement: the next message must not inherit this conversation's pick.
    expect(loadComposerDraft(DRAFT_KEY)).toBeNull();
    expect(view.pill()).toBeNull();
    expect(markedRow(view.card())).toEqual([]);
  });

  it("does not carry the answer into the next message's draft", async () => {
    const view = await mount();
    await view.click(view.row("forest"));
    await view.submit();
    expect(loadComposerDraft(DRAFT_KEY)).toBeNull();

    // The next message is written into the same slot: the answer must not ride
    // along with it (that is where a pick would leak into the following turn).
    act(() => {
      view.editor().appendChild(document.createTextNode("再补一个对照组"));
      view.editor().dispatchEvent(new Event("input", { bubbles: true }));
    });
    const next = loadComposerDraft(DRAFT_KEY);
    expect(next?.text).toBe("再补一个对照组");
    expect(next?.choice).toBeUndefined();
    expect(view.pill()).toBeNull();
  });

  it("re-points the answer, and the card's marked row, from the pill's pencil", async () => {
    const view = await mount();
    await view.click(view.row("forest"));

    await view.click(view.pill()!.querySelector<HTMLElement>("[data-choice-action=\"change\"]")!);
    const popover = container.querySelector<HTMLElement>("[data-choice-popover]")!;
    expect(popover).not.toBeNull();
    await act(async () => {
      popover.querySelector<HTMLElement>("[data-choice-popover-option=\"table\"]")!
        .dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });

    expect(view.pill()!.textContent).toContain("表里加一列样本量");
    expect(markedRow(view.card())).toEqual(["table"]);
    expect(loadComposerDraft(DRAFT_KEY)?.choice?.selectedIds).toEqual(["table"]);
    // Changing the answer never arms or disarms the send path behind the reader.
    expect(view.sendButton().disabled).toBe(false);
  });

  /**
   * "都不做" (§5.9.4): the way out of a question the reader does not want to
   * answer. It sends nothing — that is the whole point — and it must not leave the
   * draft claiming an answer to a question that was just dismissed, so a pick made
   * before the dismissal goes with it.
   */
  it("closes the card without answering: the draft's pick goes and nothing is sent", async () => {
    const onSend = vi.fn();
    const onDismiss = vi.fn();
    const view = await mount(onSend, onDismiss);
    await view.click(view.row("forest"));
    expect(view.pill()).not.toBeNull();
    expect(view.dismiss()).not.toBeNull();

    await view.click(view.dismiss()!);

    // The pill, the marked row and the stored draft all let go of the answer.
    expect(view.pill()).toBeNull();
    expect(markedRow(view.card())).toEqual([]);
    expect(loadComposerDraft(DRAFT_KEY)).toBeNull();
    // Nothing to send any more: the button the pill had armed goes grey again.
    expect(view.sendButton().disabled).toBe(true);
    // The click travels to the caller (the store write that clears the mark), and
    // the agent hears nothing at all.
    expect(onDismiss).toHaveBeenCalledTimes(1);
    expect(onSend).not.toHaveBeenCalled();
  });

  it("offers no way out where no mark owner is wired", async () => {
    // A surface without a dismiss handler (a preview, a reading cell) must not
    // draw a button whose click would go nowhere.
    const view = await mount();
    expect(view.card()).not.toBeNull();
    expect(view.dismiss()).toBeNull();
  });

  it("drops the answer from the card and the draft when the pill's cross removes it", async () => {
    const view = await mount();
    await view.click(view.row("forest"));
    await view.click(view.pill()!.querySelector<HTMLElement>("[data-choice-action=\"remove\"]")!);

    expect(view.pill()).toBeNull();
    expect(markedRow(view.card())).toEqual([]);
    expect(view.sendButton().disabled).toBe(true);
    // Nothing is left to persist, so the empty draft clears its slot outright.
    expect(loadComposerDraft(DRAFT_KEY)).toBeNull();
  });
});
