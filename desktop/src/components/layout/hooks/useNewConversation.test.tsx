// @vitest-environment jsdom
import type { NewConversationStart } from "../../../features/agent/NewConversation";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { clearComposerDraft, loadComposerDraft } from "../../../features/agent/composerDraft";
import { clearPromptSendInFlight, markPromptSendInFlight } from "../../../features/agent/pendingPromptSends";
import { onFutureEvent } from "../../../lib/futureEvents";
import { renderHook } from "../../../test/renderHook";
import { useNewConversation } from "./useNewConversation";

/**
 * The new-conversation flow's promise to the panel.
 *
 * The panel's "new conversation" row means "create one *and* keep it": it arms
 * `pinCreated` before handing over, and this hook asks it once, at the moment the thread
 * first exists — so a pin cannot be lost between two screens. A pin that fails is worth
 * saying and not worth failing the send for, which is the second half of the contract.
 *
 * `createThread`/`pinThread` are the store's own entry points, so they are the seam this
 * file stands on; the composer's side (`Composer.test.tsx`) owns the draft.
 */
const mocks = vi.hoisted(() => ({
  createThread: vi.fn(async () => ({ id: "t_new", mode: "chat" })),
  pinThread: vi.fn(async () => ({})),
}));

vi.mock("../../../integrations/storage/threadStore", () => ({
  createThread: (...args: unknown[]) => mocks.createThread(...(args as [])),
  pinThread: (...args: unknown[]) => mocks.pinThread(...(args as [])),
}));

const start: NewConversationStart = {
  attachments: [],
  content: "first message",
  mode: "chat",
  modelId: "future/m1",
  thinkingLevel: "default",
} as NewConversationStart;

let toasts: Array<{ message: string; tone?: string }>;
let dispose: (() => void) | null;

beforeEach(() => {
  mocks.createThread.mockClear();
  mocks.createThread.mockResolvedValue({ id: "t_new", mode: "chat" } as never);
  mocks.pinThread.mockClear();
  mocks.pinThread.mockResolvedValue({} as never);
  // The draft store is session-wide (sessionStorage): a fixed thread id here
  // would otherwise inherit the previous test's restored draft.
  clearComposerDraft("t_first");
  clearComposerDraft("t_second");
  toasts = [];
  dispose = onFutureEvent("toast", (payload) => {
    toasts.push(payload as { message: string; tone?: string });
  });
});

afterEach(() => {
  dispose?.();
});

function mount(pinCreated?: () => boolean) {
  return renderHook(() => useNewConversation({
    pinCreated,
    refreshStore: vi.fn(async () => {}),
    setCenterMode: vi.fn(),
    setSection: vi.fn(),
    syncSelection: vi.fn(),
  }));
}

describe("a conversation the panel asked for", () => {
  it("pins what it created, once, through the store", async () => {
    const pinCreated = vi.fn(() => true);
    const hook = mount(pinCreated);
    await act(async () => {
      await hook.current.startNewConversation(start);
    });

    expect(mocks.createThread).toHaveBeenCalledTimes(1);
    expect(pinCreated).toHaveBeenCalledTimes(1);
    // The pin is the store's own call, on the id the creation returned.
    expect(mocks.pinThread).toHaveBeenCalledWith({ pinned: true, threadId: "t_new" });
    expect(toasts).toEqual([]);
    hook.unmount();
  });

  it("does not pin when the flow was not asked to, and never asks twice", async () => {
    const notAsked = vi.fn(() => false);
    const first = mount(notAsked);
    await act(async () => {
      await first.current.startNewConversation(start);
    });
    expect(notAsked).toHaveBeenCalledTimes(1);
    expect(mocks.pinThread).not.toHaveBeenCalled();
    first.unmount();

    // The Panel arms the flag for one creation only; the hook asks once per creation
    // either way, which is what keeps "pin the next one" from leaking to the one after.
    const armed = vi.fn(() => true);
    const second = mount(armed);
    await act(async () => {
      await second.current.startNewConversation(start);
    });
    expect(armed).toHaveBeenCalledTimes(1);
    second.unmount();
  });

  it("says why a pin failed, and still opens the conversation it created", async () => {
    // The thread exists and is about to be shown; a pin the store refused is a toast,
    // not a failed send — losing the message would be the worse answer.
    mocks.pinThread.mockRejectedValueOnce(new Error("store is locked"));
    const hook = mount(() => true);

    await act(async () => {
      await hook.current.startNewConversation(start);
    });

    expect(toasts).toHaveLength(1);
    expect(toasts[0]!.tone).toBe("error");
    // The message is the failure's, not a generic one, and it is staged for the thread.
    expect(toasts[0]!.message).toContain("store is locked");
    expect(hook.current.pendingPrompt).toMatchObject({ content: "first message", targetThreadId: "t_new" });
    hook.unmount();
  });
});

describe("a first message the flow never delivered", () => {
  it("stages one prompt at a time, and returns the replaced message to its own draft", async () => {
    // Two new conversations in a row before the first prompt was picked up. The
    // flow holds one staged prompt, so the second creation used to replace the
    // first message outright: that conversation kept no session, no message and
    // no workspace folder — the text existed nowhere at all.
    mocks.createThread
      .mockResolvedValueOnce({ id: "t_first", mode: "chat" } as never)
      .mockResolvedValueOnce({ id: "t_second", mode: "chat" } as never);
    const hook = mount();

    await act(async () => {
      await hook.current.startNewConversation(start);
    });
    expect(hook.current.pendingPrompt).toMatchObject({ targetThreadId: "t_first" });

    await act(async () => {
      await hook.current.startNewConversation({ ...start, content: "second message" });
    });

    // The newest message goes to the conversation it was composed for…
    expect(hook.current.pendingPrompt).toMatchObject({
      content: "second message",
      targetThreadId: "t_second",
    });
    // …and the replaced one is still a draft of its own conversation, one
    // explicit resend away instead of lost.
    expect(loadComposerDraft("t_first")).toMatchObject({ text: "first message" });
    hook.unmount();
  });

  it("leaves a prompt a send already owns to that send", async () => {
    // Ownership moves in two steps: the thread view latches the prompt, the
    // Agent accepts it. Reclaiming a latched prompt would put the same message
    // in the draft *and* send it — a duplicate nobody asked for.
    mocks.createThread
      .mockResolvedValueOnce({ id: "t_first", mode: "chat" } as never)
      .mockResolvedValueOnce({ id: "t_second", mode: "chat" } as never);
    const hook = mount();

    await act(async () => {
      await hook.current.startNewConversation(start);
    });
    const inFlight = hook.current.pendingPrompt!.id;
    markPromptSendInFlight(inFlight);

    await act(async () => {
      await hook.current.startNewConversation({ ...start, content: "second message" });
    });

    expect(loadComposerDraft("t_first")).toBeNull();
    clearPromptSendInFlight(inFlight);
    hook.unmount();
  });
});
