// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from "vitest";
import { fetchRemoteReasoning, forgetRemoteReasonings } from "./remoteReasoning";

/**
 * One reasoning block's text, read from the host when its row is opened.
 *
 * The trimmed lane sends a reasoning row's boundary and not its body, so this is
 * the read `ThinkingBlock` makes on the reader's first expand. What is asserted
 * is the request identity (the block's index in the run), that a transcript's
 * cache behaves — one request per block, a miss remembered — and that a block
 * still being written is never cached, because it keeps growing.
 */

const request = vi.fn<(desktopId: string, command: Record<string, unknown>, lane: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: Parameters<typeof request>) => request(...args),
}));

const BLOCK = {
  desktopId: "desktop_a",
  sessionId: "sess_1",
  runId: "run_1",
  index: 2,
};

beforeEach(() => {
  request.mockReset();
  forgetRemoteReasonings("desktop_a", "sess_1");
});

it("reads one block by its index in the run", async () => {
  request.mockResolvedValue({ runId: "run_1", index: 2, text: "why it broke" });

  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBe("why it broke");
  expect(request).toHaveBeenCalledWith(
    "desktop_a",
    { type: "get_reasoning", sessionId: "sess_1", runId: "run_1", offset: 2 },
    "sess_1",
  );
});

it("asks once for a settled block, however many times the row is drawn", async () => {
  request.mockResolvedValue({ text: "why it broke" });

  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBe("why it broke");
  // A collapsed-then-reopened row, or a transcript re-render, is not a request.
  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBe("why it broke");
  expect(request).toHaveBeenCalledTimes(1);
});

it("does not cache a block that is still being written", async () => {
  request.mockResolvedValue({ text: "half a thought" });

  await expect(fetchRemoteReasoning({ ...BLOCK, settled: false })).resolves.toBe("half a thought");
  await expect(fetchRemoteReasoning({ ...BLOCK, settled: false })).resolves.toBe("half a thought");
  expect(request).toHaveBeenCalledTimes(2);
});

it("remembers a miss instead of asking again", async () => {
  request.mockResolvedValue({ runId: "run_1", index: 2, text: null });

  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBeNull();
  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBeNull();
  expect(request).toHaveBeenCalledTimes(1);
});

it("treats a host that cannot answer as 'not readable', not as an error", async () => {
  request.mockRejectedValue(new Error("Unsupported command: get_reasoning"));

  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBeNull();
  // The row shows that once; a transcript that retried per render would worse.
  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBeNull();
  expect(request).toHaveBeenCalledTimes(1);
});

it("never serves one conversation's block to another", async () => {
  request.mockResolvedValue({ text: "session a thought" });
  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBe("session a thought");

  request.mockResolvedValueOnce({ text: "session b thought" });
  await expect(fetchRemoteReasoning({ ...BLOCK, sessionId: "sess_2" })).resolves.toBe(
    "session b thought",
  );
  expect(request).toHaveBeenCalledTimes(2);
});

it("drops a conversation's cache when it is left", async () => {
  request.mockResolvedValue({ text: "why it broke" });
  await expect(fetchRemoteReasoning(BLOCK)).resolves.toBe("why it broke");

  forgetRemoteReasonings("desktop_a", "sess_1");
  await fetchRemoteReasoning(BLOCK);
  expect(request).toHaveBeenCalledTimes(2);
});
