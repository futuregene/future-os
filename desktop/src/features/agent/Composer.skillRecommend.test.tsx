// @vitest-environment jsdom
import type { SkillRecommendationProp } from "./Composer";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Wires a recommendation prop whose `onEvaluate` resolves to `card`.
 *
 * `skillRecommendation` is present on every composer that can recommend, so the
 * no-card path is the common case and must still send.
 */
function recommendationProps(
  evaluate: SkillRecommendationProp["onEvaluate"],
  card: SkillRecommendationProp["card"] = null,
): SkillRecommendationProp {
  return {
    card,
    onEvaluate: evaluate,
    onInstall: vi.fn(async () => true),
    onDismiss: vi.fn(),
  };
}

async function mount(host: HTMLElement, skillRecommendation: SkillRecommendationProp, onSend: (payload: { content: string }) => void | Promise<void>) {
  const root = createRoot(host);
  await act(async () => root.render(<Composer onSend={onSend} modelOptions={[]} skillRecommendation={skillRecommendation} />));
}

function typeInto(host: HTMLElement, text: string) {
  const editor = host.querySelector<HTMLElement>("[role=textbox]")!;
  act(() => {
    editor.textContent = text;
    editor.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return editor;
}

function submit(host: HTMLElement) {
  return act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
}

/** The content of the first recorded send, narrowed for assertions. */
function sentContent(onSend: ReturnType<typeof vi.fn>): string {
  const [first] = onSend.mock.calls as { content: string }[][];
  return first?.[0]?.content ?? "";
}

/**
 * The regression this guards: the intercept used to re-enter `submitValue` from
 * the evaluate callback while the in-flight guard was still set, so a message
 * with no recommendation was silently swallowed and Send looked broken.
 */
it("sends when the recommender finds nothing", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  try {
    await mount(host, recommendationProps(vi.fn(async () => null)), onSend);
    typeInto(host, "please search the web for this");
    await submit(host);
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(sentContent(onSend)).toContain("search the web");
  }
  finally {
    act(() => host.remove());
  }
});

it("sends when the recommender errors", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  try {
    const failing = vi.fn(async () => {
      throw new Error("agent down");
    });
    await mount(host, recommendationProps(failing), onSend);
    typeInto(host, "please search the web for this");
    await submit(host);
    expect(onSend).toHaveBeenCalledTimes(1);
  }
  finally {
    act(() => host.remove());
  }
});

it("holds the draft and shows the card when a skill is recommended", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  const card = { name: "future-web", description: "search the web" };
  try {
    await mount(host, recommendationProps(vi.fn(async () => card), card), onSend);
    typeInto(host, "please search the web for this");
    await submit(host);
    // Nothing is sent while the card is up, and the draft survives.
    expect(onSend).not.toHaveBeenCalled();
    expect(host.textContent).toContain("future-web");
    expect(host.querySelector<HTMLElement>("[role=textbox]")!.textContent).toContain("search the web");
  }
  finally {
    act(() => host.remove());
  }
});

it("does not ask the recommender twice for the same draft", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  const evaluate = vi.fn(async () => null);
  try {
    await mount(host, recommendationProps(evaluate), onSend);
    typeInto(host, "please search the web for this");
    // Two submissions of an unchanged draft: the first sends, the second is a
    // no-op because the composer is now empty. Either way the recommender must
    // have been asked exactly once.
    await submit(host);
    await submit(host);
    expect(evaluate).toHaveBeenCalledTimes(1);
  }
  finally {
    act(() => host.remove());
  }
});

it("locks the input and spins the send button while the recommender is asked", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  // An evaluate we resolve by hand, so the pending window can be inspected.
  let release!: (card: null) => void;
  const pending = new Promise<null>((resolve) => {
    release = resolve;
  });
  try {
    await mount(host, recommendationProps(() => pending), onSend);
    const editor = typeInto(host, "please search the web for this");
    // Fire submit without awaiting: the composer is now inside the wait.
    act(() => {
      host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });

    // The editor is locked (the message that will be sent is the one that was
    // evaluated) and the draft survives.
    expect(editor.getAttribute("contenteditable")).toBe("false");
    expect(editor.textContent).toContain("search the web");
    // The send button cannot be pressed and says why.
    const send = host.querySelector<HTMLButtonElement>("button[type=submit]")!;
    expect(send.disabled).toBe(true);
    expect(send.getAttribute("aria-label")).toBe("Finding a skill…");
    expect(host.querySelector("[role=status]")?.textContent).toBe("Finding a skill…");
    // Nothing went out during the wait.
    expect(onSend).not.toHaveBeenCalled();

    // Once the answer arrives (here: no skill), the draft is sent and the box
    // is usable again.
    await act(async () => release(null));
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(host.querySelector<HTMLElement>("[role=textbox]")!.getAttribute("contenteditable")).toBe("true");
  }
  finally {
    act(() => host.remove());
  }
});

it("installs, appends the slash command and sends on 安装并使用", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  const card = { name: "future-web", description: "search the web" };
  try {
    const props = recommendationProps(vi.fn(async () => card), card);
    await mount(host, props, onSend);
    typeInto(host, "please search the web for this");
    await submit(host);
    const installButton = Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Install & use"))!;
    await act(async () => installButton.click());
    expect(props.onInstall).toHaveBeenCalledWith(card);
    // The sent message carries the original draft plus the slash command,
    // appended after a space.
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(sentContent(onSend)).toBe("please search the web for this /future-web");
    // A successful send clears the composer.
    expect(host.querySelector<HTMLElement>("[role=textbox]")!.textContent).toBe("");
  }
  finally {
    act(() => host.remove());
  }
});

it("sends the original draft unchanged on dismiss", async () => {
  const host = document.createElement("div");
  document.body.append(host);
  const onSend = vi.fn();
  const card = { name: "future-web", description: "search the web" };
  try {
    const props = recommendationProps(vi.fn(async () => card), card);
    await mount(host, props, onSend);
    typeInto(host, "please search the web for this");
    await submit(host);
    const dismissButton = Array.from(host.querySelectorAll("button")).find(b => b.textContent?.includes("Send without it"))!;
    await act(async () => dismissButton.click());
    expect(props.onDismiss).toHaveBeenCalled();
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(sentContent(onSend)).toBe("please search the web for this");
  }
  finally {
    act(() => host.remove());
  }
});
