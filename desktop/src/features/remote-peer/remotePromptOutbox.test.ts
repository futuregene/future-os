// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getRemoteOutbox,
  recoverRemoteOutbox,
  resetRemoteOutbox,
  sendRemoteContinuation,
  sendRemotePrompt,
} from "./remotePromptOutbox";

/**
 * The durable outbox's contract: a prompt or continuation that could not reach
 * the host is persisted, retried on reconnect, and never sent twice because the
 * host's receipt is checked first.
 */

const prompt = vi.fn<
  (
    desktopId: string,
    sessionId: string,
    message: string,
    attachments?: string[],
    workspaceId?: string | null,
    fresh?: { modelId?: string | null; thinkingLevel?: string | null } | null,
    commandId?: string,
  ) => Promise<{ sessionId: string; threadId: string }>
>();
const continuation = vi.fn<
  (
    desktopId: string,
    sessionId: string,
    runId: string,
    commandId?: string,
  ) => Promise<unknown>
>();
const getPromptReceipt = vi.fn<
  (desktopId: string, commandId: string) => Promise<{ sessionId: string; threadId: string; runId: string } | null>
>();

vi.mock("./remotePeerClient", () => ({
  promptRemoteConversation: (...args: Parameters<typeof prompt>) => prompt(...args),
  getRemotePromptReceipt: (...args: Parameters<typeof getPromptReceipt>) => getPromptReceipt(...args),
  continueRemoteRun: (...args: Parameters<typeof continuation>) => continuation(...args),
}));

beforeEach(() => {
  localStorage.clear();
  resetRemoteOutbox();
  prompt.mockReset().mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });
  continuation.mockReset().mockResolvedValue(undefined);
  getPromptReceipt.mockReset().mockResolvedValue(null);
});

describe("sendRemotePrompt", () => {
  it("persists the prompt and clears it after the host accepts", async () => {
    await expect(
      sendRemotePrompt("desktop_a", "sess_1", {
        text: "hello",
        attachments: [],
        workspaceId: null,
        freshModelId: null,
        freshThinkingLevel: null,
      }),
    ).resolves.toEqual({ sessionId: "sess_1", threadId: "thread_1" });

    expect(prompt).toHaveBeenCalledTimes(1);
    const call = prompt.mock.calls[0]!;
    expect(call[0]).toBe("desktop_a");
    expect(call[1]).toBe("sess_1");
    expect(call[2]).toBe("hello");
    expect(call[6]).toEqual(expect.any(String));
    expect(getRemoteOutbox("desktop_a", "sess_1")).toEqual({ prompt: null, continuation: null });
  });

  it("leaves the prompt queued when the link is down", async () => {
    prompt.mockRejectedValue(new Error("peer_not_connected"));

    await expect(
      sendRemotePrompt("desktop_a", "sess_1", {
        text: "hello",
        attachments: [],
        workspaceId: null,
        freshModelId: null,
        freshThinkingLevel: null,
      }),
    ).rejects.toThrow("peer_not_connected");

    const outbox = getRemoteOutbox("desktop_a", "sess_1");
    expect(outbox.prompt).not.toBeNull();
    expect(outbox.prompt!.text).toBe("hello");
    expect(outbox.prompt!.commandId).toEqual(expect.any(String));
  });

  it("checks the receipt before retry and does not send twice when already accepted", async () => {
    prompt.mockRejectedValue(new Error("peer_not_connected"));
    await expect(
      sendRemotePrompt("desktop_a", "sess_1", {
        text: "hello",
        attachments: [],
        workspaceId: null,
        freshModelId: null,
        freshThinkingLevel: null,
      }),
    ).rejects.toThrow("peer_not_connected");

    // The host accepted the command while we were disconnected.
    getPromptReceipt.mockReset().mockResolvedValue({
      sessionId: "sess_1",
      threadId: "thread_1",
      runId: "run_1",
    });
    prompt.mockReset();
    prompt.mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });

    await recoverRemoteOutbox("desktop_a", "sess_1");

    expect(getPromptReceipt).toHaveBeenCalledTimes(1);
    expect(prompt).not.toHaveBeenCalled();
    expect(getRemoteOutbox("desktop_a", "sess_1").prompt).toBeNull();
  });

  it("retries after reconnect and then clears the queue", async () => {
    prompt.mockRejectedValue(new Error("peer_not_connected"));
    await expect(
      sendRemotePrompt("desktop_a", "sess_1", {
        text: "hello",
        attachments: [],
        workspaceId: null,
        freshModelId: null,
        freshThinkingLevel: null,
      }),
    ).rejects.toThrow("peer_not_connected");

    prompt.mockReset().mockResolvedValue({ sessionId: "sess_1", threadId: "thread_1" });

    await recoverRemoteOutbox("desktop_a", "sess_1");

    expect(prompt).toHaveBeenCalledTimes(1);
    expect(getRemoteOutbox("desktop_a", "sess_1").prompt).toBeNull();
  });

  it("keeps per-desktop+session queues isolated", async () => {
    prompt.mockRejectedValue(new Error("peer_not_connected"));
    await expect(
      sendRemotePrompt("desktop_a", "sess_a", {
        text: "for a",
        attachments: [],
        workspaceId: null,
        freshModelId: null,
        freshThinkingLevel: null,
      }),
    ).rejects.toThrow("peer_not_connected");

    const outboxA = getRemoteOutbox("desktop_a", "sess_a");
    const outboxB = getRemoteOutbox("desktop_a", "sess_b");
    expect(outboxA.prompt?.text).toBe("for a");
    expect(outboxB.prompt).toBeNull();

    const outboxC = getRemoteOutbox("desktop_b", "sess_a");
    expect(outboxC.prompt).toBeNull();
  });
});

describe("sendRemoteContinuation", () => {
  it("persists a continuation and clears it after the host accepts", async () => {
    await expect(sendRemoteContinuation("desktop_a", "sess_1", "run_1")).resolves.toBeUndefined();
    expect(continuation).toHaveBeenCalledTimes(1);
    expect(getRemoteOutbox("desktop_a", "sess_1")).toEqual({ prompt: null, continuation: null });
  });

  it("leaves the continuation queued on recoverable failure", async () => {
    continuation.mockRejectedValue(new Error("remote_peer_disconnected"));
    await expect(sendRemoteContinuation("desktop_a", "sess_1", "run_1")).rejects.toThrow("remote_peer_disconnected");
    expect(getRemoteOutbox("desktop_a", "sess_1").continuation).not.toBeNull();
  });
});
