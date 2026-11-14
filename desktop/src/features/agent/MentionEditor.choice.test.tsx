// @vitest-environment jsdom
import type { ChoiceAnswer } from "./composer/choiceAnswer";
import type { MentionEditorHandle } from "./MentionEditor";
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetModelCatalogRefreshForTests } from "../../integrations/agent/modelCatalogRefresh";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { resetShortcutScopes } from "../keyboard/scopes";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { ComposerControls } from "./composer/ComposerControls";
import { isEditorEmpty, serialize } from "./composer/mentionDom";
import { MentionEditor } from "./MentionEditor";

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => {
    if (command === "list_agent_providers")
      return { builtin: [], custom: [] };
    return [];
  }),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const ANSWER: ChoiceAnswer = {
  mode: "single",
  options: [
    { id: "forest", label: "重画森林图" },
    { id: "flow", label: "补一张实验流程图" },
    { id: "table", label: "表里加一列样本量" },
  ],
  selectedIds: ["forest"],
  title: "接下来做哪一件？",
};

/**
 * The composer's real pairing: the editor that owns the answer, and the real
 * `ComposerControls` whose send button is disabled purely by `inputEmpty` and
 * the attachment count (Composer.tsx wires exactly these two props the same way).
 * The two buttons below stand in for the card row a later step will click.
 */
function Harness({ choice = ANSWER, onChoiceChange }: {
  choice?: ChoiceAnswer;
  onChoiceChange?: (answer: ChoiceAnswer | null) => void;
}) {
  const editorRef = useRef<MentionEditorHandle | null>(null);
  const [inputEmpty, setInputEmpty] = useState(true);
  return (
    <>
      <MentionEditor
        onChoiceChange={onChoiceChange}
        onEmptyChange={setInputEmpty}
        onSubmit={() => {}}
        placeholder="Type a message"
        ref={editorRef}
      />
      <button onClick={() => editorRef.current?.setChoice(choice)} type="button">set-answer</button>
      <button onClick={() => editorRef.current?.setChoice(null)} type="button">clear-answer</button>
      <ComposerControls
        approvalTier="off"
        attachmentCount={0}
        compactionPending={false}
        futureSessionStatus="ready"
        handleAttachFiles={async () => {}}
        inputEmpty={inputEmpty}
        modelOptions={[]}
        recommendPending={false}
        sendPending={false}
      />
    </>
  );
}

describe("the choice answer in the composer editor", () => {
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;

  beforeEach(() => {
    resetAllCommandBindings();
    resetShortcutScopes();
    resetModelCatalogRefreshForTests();
    vi.mocked(invokeCommand).mockClear();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  async function mount(
    onChoiceChange?: (answer: ChoiceAnswer | null) => void,
    choice?: ChoiceAnswer,
  ) {
    await act(async () => root.render(<Harness choice={choice} onChoiceChange={onChoiceChange} />));
    return {
      editor: container.querySelector<HTMLDivElement>("[role=\"textbox\"]")!,
      sendButton: () => container.querySelector<HTMLButtonElement>("button[type=\"submit\"]")!,
      click: async (label: string) => {
        const button = [...container.querySelectorAll("button")].find(candidate => candidate.textContent === label);
        expect(button, `the ${label} button`).toBeDefined();
        await act(async () => button!.click());
      },
    };
  }

  const pill = () => container.querySelector<HTMLElement>("[data-mention=\"choice\"]");
  const popover = () => container.querySelector<HTMLElement>("[data-choice-popover]");

  it("§5.2.1: send is disabled when empty, enabled by the pill alone, disabled again once it is removed", async () => {
    const { editor, sendButton, click } = await mount();
    expect(sendButton().disabled).toBe(true);
    expect(isEditorEmpty(editor)).toBe(true);

    await click("set-answer");
    // Requirement (d): nothing typed, only the answer — and the button is live.
    expect(isEditorEmpty(editor)).toBe(false);
    expect(sendButton().disabled).toBe(false);
    // The answer sits *in* the editor, not in an attachment row: `isEditorEmpty`
    // (mentionDom.ts) and the `disabled` expression in ComposerControls are both
    // reading this one element.
    expect(pill()).not.toBeNull();
    expect(editor.contains(pill())).toBe(true);
    expect(pill()!.closest("[contenteditable=\"true\"]")).toBe(editor);
    expect(container.querySelectorAll("[data-mention=\"choice\"]")).toHaveLength(1);

    await click("clear-answer");
    expect(pill()).toBeNull();
    expect(isEditorEmpty(editor)).toBe(true);
    expect(sendButton().disabled).toBe(true);
  });

  it("serializes the answer as the plain text of §7, ahead of whatever was typed", async () => {
    const { editor, click } = await mount();
    await click("set-answer");
    // The pill owns one trailing space, so the caret has somewhere editable to
    // land and typing continues beside the answer rather than inside it.
    expect(serialize(editor)).toBe("[接下来做哪一件？ → 重画森林图] ");

    // Requirement (c): the pill and typed text share one flow; the pill comes
    // first because it was picked first.
    act(() => {
      editor.appendChild(document.createTextNode("顺便把图例挪到右下"));
    });
    expect(serialize(editor)).toBe("[接下来做哪一件？ → 重画森林图] 顺便把图例挪到右下");
  });

  it("renders a multi-choice answer as the labels joined in pick order", async () => {
    const multi: ChoiceAnswer = { ...ANSWER, mode: "multi", selectedIds: ["forest", "flow"] };
    const { editor, click } = await mount(undefined, multi);
    await click("set-answer");
    expect(pill()!.textContent).toContain("重画森林图、补一张实验流程图");
    expect(serialize(editor).trim()).toBe("[接下来做哪一件？ → 重画森林图、补一张实验流程图]");

    // Picking an already-picked row takes it back out — multi toggles (§6 b).
    await act(async () => pill()!.querySelector<HTMLElement>("[data-choice-action=\"change\"]")!.click());
    const forest = popover()!.querySelector<HTMLElement>("[data-choice-popover-option=\"forest\"]")!;
    await act(async () => {
      forest.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(serialize(editor).trim()).toBe("[接下来做哪一件？ → 补一张实验流程图]");
  });

  it("is idempotent: setting the same answer twice leaves one pill and one serialized answer", async () => {
    const { editor, click } = await mount();
    await click("set-answer");
    await click("set-answer");
    expect(container.querySelectorAll("[data-mention=\"choice\"]")).toHaveLength(1);
    expect(serialize(editor).trim()).toBe("[接下来做哪一件？ → 重画森林图]");
  });

  it("re-opens the question from the pill's pencil and rewrites it on a new pick", async () => {
    const changed: Array<ChoiceAnswer | null> = [];
    const { editor, sendButton, click } = await mount(answer => changed.push(answer));
    await click("set-answer");

    const pencil = pill()!.querySelector<HTMLElement>("[data-choice-action=\"change\"]")!;
    await act(async () => pencil.click());
    expect(popover()).not.toBeNull();
    expect(popover()!.querySelectorAll("[data-choice-popover-option]")).toHaveLength(3);
    expect(popover()!.querySelector("[data-choice-popover-option=\"forest\"]")!.getAttribute("aria-checked")).toBe("true");

    const flow = popover()!.querySelector<HTMLElement>("[data-choice-popover-option=\"flow\"]")!;
    await act(async () => {
      flow.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(popover()).toBeNull();
    expect(pill()!.textContent).toContain("补一张实验流程图");
    expect(serialize(editor).trim()).toBe("[接下来做哪一件？ → 补一张实验流程图]");
    expect(changed[changed.length - 1]?.selectedIds).toEqual(["flow"]);
    // Changing the answer never breaks the send path (§5.3).
    expect(sendButton().disabled).toBe(false);
  });

  it("closes the popover on Esc without touching the answer", async () => {
    const changed: Array<ChoiceAnswer | null> = [];
    const { editor, click } = await mount(answer => changed.push(answer));
    await click("set-answer");
    await act(async () => pill()!.querySelector<HTMLElement>("[data-choice-action=\"change\"]")!.click());
    expect(popover()).not.toBeNull();

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
    });
    expect(popover()).toBeNull();
    expect(serialize(editor).trim()).toBe("[接下来做哪一件？ → 重画森林图]");
    expect(changed).toHaveLength(0);
  });

  it("removes the answer from the pill's cross and tells the draft it is gone", async () => {
    const changed: Array<ChoiceAnswer | null> = [];
    const { editor, sendButton, click } = await mount(answer => changed.push(answer));
    await click("set-answer");

    await act(async () => pill()!.querySelector<HTMLElement>("[data-choice-action=\"remove\"]")!.click());
    expect(pill()).toBeNull();
    expect(serialize(editor)).toBe("");
    expect(sendButton().disabled).toBe(true);
    expect(changed[changed.length - 1]).toBeNull();
  });

  it("names both pill actions for assistive technology (§9)", async () => {
    await mount();
    await act(async () => {
      [...container.querySelectorAll("button")].find(button => button.textContent === "set-answer")!.click();
    });
    expect(pill()!.querySelector("[data-choice-action=\"change\"]")!.getAttribute("aria-label"))
      .toBe("Change the answer to \"接下来做哪一件？\"");
    expect(pill()!.querySelector("[data-choice-action=\"remove\"]")!.getAttribute("aria-label"))
      .toBe("Remove the answer to \"接下来做哪一件？\"");
  });
});
