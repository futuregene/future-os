import type { Root } from "react-dom/client";
// @vitest-environment jsdom
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { NewConversation } from "./NewConversation";

/**
 * The composer, the workspace modal, the banner and the two state machines
 * (`useWorkspaceForm`, `useSkillRecommendation`) have their own behaviour
 * tests. This file is about what `NewConversation` itself decides: which mode is
 * active, what it hands the composer, and what it does with a send / guide
 * start / dismissal.
 */
const h = vi.hoisted(() => ({
  form: {} as Record<string, unknown>,
  formArgs: {} as Record<string, unknown>,
  reco: { dismiss: vi.fn(), evaluate: vi.fn(async () => null) },
  guide: { fetchCoachPrompt: vi.fn() },
  skills: { installRecommendedSkill: vi.fn(async () => {}) },
  windowDrag: { startWindowDrag: vi.fn() },
}));

vi.mock("./Composer", () => ({
  Composer: (props: Record<string, unknown>) => (
    <div
      data-composer=""
      data-disabled={String(props.disabled)}
      data-draft-key={String(props.draftKey)}
      data-workspace-id={props.workspaceId == null ? "" : String(props.workspaceId)}
    >
      <button data-accept="" onClick={() => (props.onDragStateChange as (s: string) => void)("accept")} type="button" />
      <button data-reject="" onClick={() => (props.onDragStateChange as (s: string) => void)("reject")} type="button" />
      <button
        data-install=""
        onClick={() => (props.skillRecommendation as { onInstall: (card: { description: string; name: string }) => void }).onInstall({ description: "web", name: "future-web" })}
        type="button"
      />
      <button
        data-send=""
        onClick={() => {
          void (props.onSend as (p: unknown) => Promise<void> | void)({ attachments: [], content: "hello" });
        }}
        type="button"
      />
    </div>
  ),
}));
vi.mock("./NewConversationWorkspaceForm", () => ({
  WorkspaceModal: (props: { creating: boolean; error: string | null; notice: string | null; onPickFolder: () => void; path: string }) => (
    <div data-creating={String(props.creating)} data-error={props.error ?? ""} data-modal="" data-notice={props.notice ?? ""} data-path={props.path}>
      <button data-pick-folder="" onClick={props.onPickFolder} type="button" />
    </div>
  ),
}));
vi.mock("../skills/SkillGuideBanner", () => ({
  SkillGuideBanner: (props: { onDismiss: () => void; onStart: () => void; starting: boolean }) => (
    <div data-banner="" data-starting={String(props.starting)}>
      <button data-guide-start="" onClick={props.onStart} type="button" />
      <button data-guide-dismiss="" onClick={props.onDismiss} type="button" />
    </div>
  ),
}));
vi.mock("../skills/skillGuidePrompt", () => ({ fetchCoachPrompt: h.guide.fetchCoachPrompt }));
vi.mock("../skills/installRecommendedSkill", () => ({
  installRecommendedSkill: h.skills.installRecommendedSkill,
}));
vi.mock("./useSkillRecommendation", () => ({
  useSkillRecommendation: () => ({ candidates: [], state: { recommendation: null }, ...h.reco }),
}));
vi.mock("./useWorkspaceForm", () => ({
  useWorkspaceForm: (args: Record<string, unknown>) => {
    h.formArgs = args;
    return h.form;
  },
}));
vi.mock("../../components/layout/LeftPanelTitlebarToggle", () => ({
  LeftPanelTitlebarToggle: () => null,
}));
vi.mock("../../lib/windowDrag", () => ({ startWindowDrag: h.windowDrag.startWindowDrag }));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function workspace(id: string, name = id): StoredWorkspace {
  return { id, name, path: `/work/${id}` } as StoredWorkspace;
}

const models = [{ id: "m1", name: "Model" }] as unknown as AgentModelOption[];

let root: Root;
let container: HTMLDivElement;
let props: Parameters<typeof NewConversation>[0];

beforeEach(() => {
  vi.clearAllMocks();
  h.guide.fetchCoachPrompt.mockResolvedValue("coach prompt");
  h.form = {
    begin: vi.fn(),
    cancel: vi.fn(),
    creating: false,
    displayName: "",
    error: null,
    mode: null,
    notice: null,
    path: "",
    pickFolder: vi.fn(async () => {}),
    setDisplayName: vi.fn(),
    submit: vi.fn(async () => {}),
  };
  props = {
    approvalTier: "manual",
    futureBalance: 10,
    futureSessionStatus: "authenticated",
    leftPanelExpanded: true,
    modelId: "m1",
    modelOptions: models,
    onAddWorkspace: vi.fn(async () => null),
    onChangeApprovalTier: vi.fn(),
    onDismissSkillGuide: vi.fn(),
    onModelChange: vi.fn(),
    onStart: vi.fn(),
    onThinkingLevelChange: vi.fn(),
    onToggleLeftPanel: vi.fn(),
    skillGuideDismissed: false,
    skillRecommend: false,
    thinkingLevel: "high",
    workspaces: [workspace("w1", "Alpha"), workspace("w2", "Beta")],
  };
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(over: Partial<Parameters<typeof NewConversation>[0]> = {}) {
  act(() => root.render(<NewConversation {...props} {...over} />));
}

function rerender(over: Partial<Parameters<typeof NewConversation>[0]> = {}) {
  render(over);
}

async function click(selector: string) {
  await act(async () => {
    const element = container.querySelector<HTMLButtonElement>(selector);
    if (!element)
      throw new Error(`no element matching ${selector}`);
    element.click();
    await Promise.resolve();
  });
}

function composer() {
  return container.querySelector<HTMLElement>("[data-composer]")!;
}

function headerSubtitle() {
  return container.querySelector("header .text-xs")?.textContent ?? "";
}

/** A button inside the open workspace menu, matched by its visible label. */
function menuItem(label: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("div.absolute button")]
    .find(button => button.textContent?.includes(label));
}

describe("newConversation mode and workspace selection", () => {
  it("starts in workspace mode when a workspace exists, and chat mode when none does", () => {
    render();
    expect(headerSubtitle()).toBe("Alpha");
    expect(composer().getAttribute("data-workspace-id")).toBe("w1");
    expect(composer().getAttribute("data-draft-key")).toBe("new");

    // boundary: no workspaces at all — nothing to run in, so chat.
    rerender({ workspaces: [] });
    expect(headerSubtitle()).toBe("Chat");
    expect(composer().getAttribute("data-workspace-id")).toBe("");
  });

  it("honours an explicit initial mode, and an explicit workspace over that mode", () => {
    render({ initialMode: "chat" });
    expect(headerSubtitle()).toBe("Chat");

    // An explicitly requested workspace is a stronger signal than the mode: it
    // both selects the workspace and switches to workspace mode.
    rerender({ initialMode: "chat", initialWorkspaceId: "w2", workspaces: [workspace("w1", "Alpha"), workspace("w2", "Beta")] });
    expect(headerSubtitle()).toBe("Beta");
    expect(h.formArgs.workspaces).toEqual(props.workspaces);
  });

  it("adopts the requested workspace once it appears in the options", () => {
    // The options can lag the mount (AppShell loads them asynchronously).
    render({ initialWorkspaceId: "w2", workspaces: [] });
    expect(headerSubtitle()).toBe("Chat");

    rerender({ initialWorkspaceId: "w2", workspaces: [workspace("w1", "Alpha"), workspace("w2", "Beta")] });
    expect(headerSubtitle()).toBe("Beta");
    expect(composer().getAttribute("data-workspace-id")).toBe("w2");
  });

  it("does not snap a user's choice back to the requested workspace on a later render", () => {
    // concurrency: AppShell re-renders every poll tick with a new array
    // identity; the adoption effect must run exactly once or it would fight the
    // user mid-composition.
    const list = [workspace("w1", "Alpha"), workspace("w2", "Beta")];
    render({ initialWorkspaceId: "w2", workspaces: list });
    expect(headerSubtitle()).toBe("Beta");

    // The user switches to Alpha through the menu…
    act(() => {
      container.querySelector<HTMLButtonElement>("button[title]")!.click();
    });
    act(() => menuItem("Alpha")!.click());
    expect(headerSubtitle()).toBe("Alpha");

    // …and a later render with a fresh array must not undo it.
    rerender({ initialWorkspaceId: "w2", workspaces: [...list] });
    expect(headerSubtitle()).toBe("Alpha");
  });

  it("falls back to the first workspace when the selected one disappears", () => {
    // boundary: the workspace was deleted in another window while composing.
    render();
    expect(headerSubtitle()).toBe("Alpha");

    rerender({ workspaces: [workspace("w3", "Gamma")] });
    expect(headerSubtitle()).toBe("Gamma");
  });
});

describe("newConversation workspace menu", () => {
  it("opens on the workspace chip, lists the options and selects one", async () => {
    render();
    expect(container.querySelector("div.absolute")).toBeNull();

    await click("button[title]");
    const items = [...container.querySelectorAll<HTMLButtonElement>("div.absolute button")];
    expect(items.map(item => item.textContent)).toEqual([
      "Alpha/work/w1",
      "Beta/work/w2",
      "Open workspace",
      "Chat",
    ]);

    act(() => menuItem("Beta")!.click());
    expect(headerSubtitle()).toBe("Beta");
    expect(container.querySelector("div.absolute")).toBeNull();
  });

  it("explains an empty workspace list instead of showing nothing", async () => {
    render({ workspaces: [] });
    await click("button[title]");
    expect(container.querySelector("div.absolute")?.textContent).toContain("No workspaces yet.");
  });

  it("opens the workspace modal from the menu and closes the menu", async () => {
    render();
    await click("button[title]");
    act(() => menuItem("Open workspace")!.click());

    expect(h.form.begin).toHaveBeenCalledTimes(1);
    // The hook owns `mode`; a truthy value renders the modal over the view.
    h.form = { ...h.form, mode: "open" };
    rerender();
    expect(container.querySelector("[data-modal]")).not.toBeNull();
  });

  it("switches to chat from both the menu entry and the chat chip", async () => {
    render();
    await click("button[title]");
    act(() => menuItem("Chat")!.click());
    expect(headerSubtitle()).toBe("Chat");
    expect(container.querySelector("div.absolute")).toBeNull();

    // Back to a workspace, then leave it through the dedicated chat chip.
    await click("button[title]");
    act(() => menuItem("Alpha")!.click());
    expect(headerSubtitle()).toBe("Alpha");
    const chatChip = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .filter(button => !button.closest("div.absolute"))
      .find(button => button.textContent?.trim() === "Chat")!;
    act(() => chatChip.click());
    expect(headerSubtitle()).toBe("Chat");
  });

  it("re-pins the mode to workspace when the hook asks for it", async () => {
    render();
    // The workspace form's `cancel`/`submit` drive the parent's mode through
    // these callbacks.
    act(() => {
      (h.formArgs.onModeChange as (mode: string) => void)("chat");
    });
    expect(headerSubtitle()).toBe("Chat");
    act(() => {
      (h.formArgs.onModeChange as (mode: string) => void)("workspace");
    });
    expect(headerSubtitle()).toBe("Alpha");
  });

  it("lets the workspace hook close the menu too", async () => {
    // `begin`/`cancel`/`submit` in the form all report back through onCloseMenu.
    render();
    await click("button[title]");
    expect(container.querySelector("div.absolute")).not.toBeNull();

    act(() => {
      (h.formArgs.onCloseMenu as () => void)();
    });
    expect(container.querySelector("div.absolute")).toBeNull();
  });

  it("closes the menu on a pointer press outside it or on Escape", async () => {
    render();
    await click("button[title]");
    expect(container.querySelector("div.absolute")).not.toBeNull();

    // A press anywhere outside the menu's own layer dismisses it.
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(container.querySelector("div.absolute")).toBeNull();

    await click("button[title]");
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Escape" }));
    });
    expect(container.querySelector("div.absolute")).toBeNull();
  });
});

describe("newConversation sending", () => {
  it("refuses to send until the model catalog has loaded", async () => {
    // error-path: the initial modelId is empty and would silently fall back to
    // the agent default, so the composer is disabled instead.
    render({ modelOptions: [] });
    expect(composer().getAttribute("data-disabled")).toBe("true");

    await click("[data-send]");
    expect(props.onStart).not.toHaveBeenCalled();
  });

  it("starts a workspace conversation with the active workspace", async () => {
    render();
    await click("[data-send]");

    expect(props.onStart).toHaveBeenCalledWith({
      attachments: [],
      content: "hello",
      mode: "workspace",
      modelId: "m1",
      thinkingLevel: "high",
      workspace: { id: "w1", label: "Alpha", path: "/work/w1" },
    });
  });

  it("starts a chat conversation with no workspace attached", async () => {
    render({ initialMode: "chat" });
    await click("[data-send]");

    expect(props.onStart).toHaveBeenCalledWith({
      attachments: [],
      content: "hello",
      mode: "chat",
      modelId: "m1",
      thinkingLevel: "high",
    });
  });

  it("falls back to the agent's default model while the id is still empty", async () => {
    // boundary: the catalog loaded but the user has not picked a model yet.
    render({ modelId: "" });
    await click("[data-send]");
    expect(props.onStart).toHaveBeenCalledWith(expect.objectContaining({ modelId: "" }));
  });

  it("applies the same default-model fallback on the chat send path", async () => {
    // boundary: the workspace branch and the chat branch each repeat
    // `modelId || defaultAgentModelId`, and the test above only ever reaches the
    // workspace one (it renders with the default mode). A branch audit found the
    // chat line's right-hand arm at zero hits. `defaultAgentModelId` is `""`, so
    // the observable result equals the empty input - which is exactly why the
    // old test could not tell the two branches apart.
    render({ initialMode: "chat", modelId: "" });
    await click("[data-send]");

    expect(props.onStart).toHaveBeenCalledWith(expect.objectContaining({ mode: "chat", modelId: "" }));
  });

  it("keeps the composer's promise tied to the thread creation", async () => {
    // The composer must not clear the draft until the thread really exists.
    let resolveStart: () => void = () => {};
    const onStart = vi.fn(() => new Promise<void>((resolve) => {
      resolveStart = resolve;
    }));
    render({ onStart });
    await click("[data-send]");

    expect(onStart).toHaveBeenCalledTimes(1);
    resolveStart();
    await act(async () => {
      await Promise.resolve();
    });
  });
});

describe("newConversation skill guide", () => {
  it("starts the guide with the default model when no model is chosen yet", async () => {
    // boundary: the guide's own `onStart` repeats `modelId || defaultAgentModelId`
    // a third time, and a branch audit found its right-hand arm at zero hits -
    // every other guide test renders with the default `modelId: "m1"`.
    render({ modelId: "" });
    await click("[data-guide-start]");

    expect(props.onStart).toHaveBeenCalledWith(expect.objectContaining({
      content: "coach prompt",
      mode: "chat",
      modelId: "",
    }));
  });

  it("shows the banner until it is dismissed, then a short-lived notice", async () => {
    vi.useFakeTimers();
    try {
      render();
      expect(container.querySelector("[data-banner]")).not.toBeNull();
      expect(container.textContent).not.toContain("Tutorial entry hidden");

      // Dismissing forwards to the setting, pulses the rail and shows the notice.
      await click("[data-guide-dismiss]");
      expect(props.onDismissSkillGuide).toHaveBeenCalledTimes(1);

      // boundary: the notice only renders once the parent reports the dismissal.
      rerender({ skillGuideDismissed: true });
      expect(container.querySelector("[data-banner]")).toBeNull();
      expect(container.textContent).toContain("Tutorial entry hidden");

      // It clears itself after four seconds.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000);
      });
      expect(container.textContent).not.toContain("Tutorial entry hidden");
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("announces the dismissal so the left rail can point at the Skills page", async () => {
    const events: unknown[] = [];
    const off = onFutureEvent("skill-guide-dismissed", () => void events.push(undefined));
    render();
    await click("[data-guide-dismiss]");
    expect(events).toHaveLength(1);
    off();
  });

  it("shows nothing at all once the guide was already dismissed earlier", () => {
    render({ skillGuideDismissed: true });
    expect(container.querySelector("[data-banner]")).toBeNull();
    expect(container.textContent).not.toContain("Tutorial entry hidden");
  });

  it("starts a chat conversation from the guide's prompt", async () => {
    render();
    await click("[data-guide-start]");

    expect(h.guide.fetchCoachPrompt).toHaveBeenCalledTimes(1);
    expect(props.onStart).toHaveBeenCalledWith({
      content: "coach prompt",
      mode: "chat",
      modelId: "m1",
      thinkingLevel: "high",
    });
  });

  it("toasts a failed prompt fetch and creates nothing", async () => {
    const toasts: { message: string; tone?: string }[] = [];
    const off = onFutureEvent("toast", toast => void toasts.push(toast));
    h.guide.fetchCoachPrompt.mockRejectedValue(new Error("offline"));
    render();

    await click("[data-guide-start]");

    expect(props.onStart).not.toHaveBeenCalled();
    expect(toasts).toEqual([{ message: "Failed to load tutorial configuration: offline", tone: "error" }]);
    // The banner stays usable for a retry.
    expect(container.querySelector("[data-banner]")?.getAttribute("data-starting")).toBe("false");
    off();
  });

  it("does not start the guide twice while the first attempt is in flight", async () => {
    // concurrency: a second click after React committed `guideStarting` must not
    // create a second thread.
    let resolvePrompt: (value: string) => void = () => {};
    h.guide.fetchCoachPrompt.mockImplementation(() => new Promise((resolve) => {
      resolvePrompt = resolve;
    }));
    render();

    await click("[data-guide-start]");
    expect(h.guide.fetchCoachPrompt).toHaveBeenCalledTimes(1);
    expect(container.querySelector("[data-banner]")?.getAttribute("data-starting")).toBe("true");

    await click("[data-guide-start]");
    expect(h.guide.fetchCoachPrompt).toHaveBeenCalledTimes(1);

    resolvePrompt("coach prompt");
    await act(async () => {
      await Promise.resolve();
    });
    expect(props.onStart).toHaveBeenCalledTimes(1);
    expect(container.querySelector("[data-banner]")?.getAttribute("data-starting")).toBe("false");
  });

  it("keeps the guide usable when the thread could not be created", async () => {
    // error-path: the start path already toasted; this must not throw.
    render({ onStart: vi.fn(() => Promise.reject(new Error("thread creation failed"))) });
    await click("[data-guide-start]");
    expect(container.querySelector("[data-banner]")?.getAttribute("data-starting")).toBe("false");
  });

  it("ignores the guide while the catalog is still loading", async () => {
    render({ modelOptions: [] });
    await click("[data-guide-start]");
    expect(h.guide.fetchCoachPrompt).not.toHaveBeenCalled();
  });
});

describe("newConversation drag verdict and skill install", () => {
  it("wraps the card in an accept or reject ring for a drag", async () => {
    render();
    const card = container.querySelector("div.max-w-3xl.rounded-lg")!;
    expect(card.className).not.toContain("ring-2");

    await click("[data-accept]");
    expect(container.querySelector("div.max-w-3xl.rounded-lg")!.className).toContain("ring-focus");

    await click("[data-reject]");
    expect(container.querySelector("div.max-w-3xl.rounded-lg")!.className).toContain("ring-danger-line");
  });

  it("installs the recommended skill at its latest version", async () => {
    render();
    await click("[data-install]");
    expect(h.skills.installRecommendedSkill).toHaveBeenCalledWith("future-web");
  });

  it("passes the workspace modal every piece of the form state", () => {
    h.form = {
      ...h.form,
      creating: true,
      displayName: "Draft",
      error: "Choose a workspace directory.",
      mode: "open",
      notice: "already exists",
      path: "/tmp/x",
    };
    render();

    const modal = container.querySelector<HTMLElement>("[data-modal]")!;
    expect(modal.getAttribute("data-creating")).toBe("true");
    expect(modal.getAttribute("data-error")).toBe("Choose a workspace directory.");
    expect(modal.getAttribute("data-notice")).toBe("already exists");
    expect(modal.getAttribute("data-path")).toBe("/tmp/x");

    act(() => container.querySelector<HTMLButtonElement>("[data-pick-folder]")!.click());
    expect(h.form.pickFolder).toHaveBeenCalledTimes(1);
  });

  it("starts the window drag from the header", () => {
    render();
    act(() => {
      container.querySelector("header")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(h.windowDrag.startWindowDrag).toHaveBeenCalledTimes(1);
  });
});
