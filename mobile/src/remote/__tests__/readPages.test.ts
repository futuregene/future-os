import type { RemoteClient } from "../client";
import { encodeBase64Url } from "../codec";
import { requestReadPage } from "../readPages";
import type { RemoteCommand } from "../types";

const SIZE = 192 * 1024;
function parts(value: unknown) {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  return Array.from({ length: Math.ceil(bytes.length / SIZE) }, (_, index) => {
    const offset = index * SIZE;
    const nextOffset = Math.min(bytes.length, offset + SIZE);
    return { readChunk: { id: "read_snapshot", offset, nextOffset, totalBytes: bytes.length,
      data: encodeBase64Url(bytes.slice(offset, nextOffset)) } };
  });
}
function clientFor(pages: unknown[]) {
  const requestRetry = jest.fn();
  for (const data of pages) requestRetry.mockResolvedValueOnce({ success: true, data });
  return { client: { requestRetry } as unknown as RemoteClient, requestRetry };
}

test("reassembles a multi-megabyte Unicode projection without changing its cursor or content", async () => {
  const data = { events: [], projection: { cursor: 99, events: [{ data: "中文\\\"".repeat(180_000) }] } };
  const pages = parts(data);
  const { client, requestRetry } = clientFor(pages);
  const result = await requestReadPage(client, { type: "get_events_since", runId: "r" }, "s");
  expect(result.data).toEqual(data);
  expect(requestRetry).toHaveBeenCalledTimes(pages.length);
  expect(requestRetry.mock.calls[0][0]).toMatchObject({ chunkedRead: true });
  expect(requestRetry.mock.calls[1][0]).toEqual({ type: "get_read_chunk", sessionId: "s", runId: "r", replyId: "read_snapshot", offset: SIZE });
});

// Virtual RPC latency only; not a phone/network/JSON decode benchmark.
test.each([20, 150])("large snapshot at %i ms RPC latency uses bounded parallel reads", async latency => {
  jest.useFakeTimers();
  const data = { text: "x".repeat(SIZE * 12) };
  const pages = parts(data);
  let pending = 0;
  let peakPending = 0;
  const requestRetry = jest.fn(async (command: RemoteCommand) => {
    pending++;
    peakPending = Math.max(peakPending, pending);
    await new Promise(resolve => setTimeout(resolve, latency));
    pending--;
    return { success: true, data: pages[(command.offset ?? 0) / SIZE] };
  });
  try {
    const start = Date.now();
    const read = requestReadPage({ requestRetry } as unknown as RemoteClient, { type: "get_session_entries" }, "s");
    await jest.runAllTimersAsync();
    const result = await read;
    const elapsedMs = Date.now() - start;
    console.warn(JSON.stringify({ benchmark: "large-read-snapshot", rpcLatencyMs: latency,
      chunks: pages.length, peakPending, elapsedMs }));
    expect(result.data).toEqual(data);
    expect(requestRetry).toHaveBeenCalledTimes(pages.length);
    expect(peakPending).toBe(4);
    expect(elapsedMs).toBe(latency * (1 + Math.ceil((pages.length - 1) / 4)));
  } finally {
    jest.useRealTimers();
  }
});

test("out-of-order chunks retain snapshot ownership and assemble at their requested offsets", async () => {
  jest.useFakeTimers();
  const data = { text: "中文🙂".repeat(SIZE), cursor: 123 };
  const pages = parts(data);
  const completed: number[] = [];
  const requestRetry = jest.fn(async (command: RemoteCommand) => {
    const index = (command.offset ?? 0) / SIZE;
    if (index) {
      expect(command).toMatchObject({ type: "get_read_chunk", sessionId: "s", runId: "r",
        bridgeInstanceId: "bridge", replyId: "read_snapshot" });
      await new Promise(resolve => setTimeout(resolve, (5 - ((index - 1) % 4)) * 10));
      completed.push(index);
    }
    return { success: true, data: pages[index] };
  });
  try {
    const read = requestReadPage({ requestRetry } as unknown as RemoteClient,
      { type: "get_events_since", runId: "r", bridgeInstanceId: "bridge" }, "s");
    await jest.runAllTimersAsync();
    expect((await read).data).toEqual(data);
    expect(completed.slice(0, 4)).toEqual([4, 3, 2, 1]);
    expect(requestRetry).toHaveBeenCalledTimes(pages.length);
  } finally {
    jest.useRealTimers();
  }
});

test.each(["cancel", "transport", "malformed"])("%s during a parallel batch stops further reads without publishing partial data", async mode => {
  jest.useFakeTimers();
  const pages = parts({ text: "x".repeat(SIZE * 8) });
  let current = true;
  const requestRetry = jest.fn(async (command: RemoteCommand) => {
    const index = (command.offset ?? 0) / SIZE;
    if (index) {
      await new Promise(resolve => setTimeout(resolve, index * 10));
      if (index === 1) {
        if (mode === "cancel") current = false;
        if (mode === "transport") throw new Error("remote_read_expired");
        if (mode === "malformed") return { success: true, data: {} };
      }
      // Another outstanding rejection must be handled even after the batch
      // has failed. No new requests may be launched by its late siblings.
      if (index === 4) throw new Error("late_transport_failure");
    }
    return { success: true, data: pages[index] };
  });
  try {
    const read = requestReadPage({ requestRetry } as unknown as RemoteClient,
      { type: "get_session_entries" }, "s", () => current);
    const rejected = expect(read).rejects.toThrow(mode === "cancel" ? "stale_sync_lane"
      : mode === "transport" ? "remote_read_expired" : "remote_read_invalid_chunk");
    await jest.runAllTimersAsync();
    await rejected;
    expect(requestRetry).toHaveBeenCalledTimes(5);
    expect(requestRetry.mock.calls.slice(1).map(([command]) => command.offset)).toEqual([SIZE, SIZE * 2, SIZE * 3, SIZE * 4]);
  } finally {
    jest.useRealTimers();
  }
});

test("ordinary and old Desktop replies remain compatible", async () => {
  const { client, requestRetry } = clientFor([{ entries: [] }]);
  expect((await requestReadPage(client, { type: "get_session_entries" }, "s")).data).toEqual({ entries: [] });
  expect(requestRetry).toHaveBeenCalledTimes(1);
});

test("a cancelled lane does not continue reading snapshot chunks", async () => {
  let current = true;
  const page = parts({ text: "x".repeat(SIZE * 2) })[0];
  const requestRetry = jest.fn(async () => { current = false; return { success: true, data: page }; });
  const client = { requestRetry } as unknown as RemoteClient;
  await expect(requestReadPage(client, { type: "get_session_entries" }, "s", () => current)).rejects.toThrow("stale_sync_lane");
  expect(requestRetry).toHaveBeenCalledTimes(1);
});

test.each(["owner", "offset", "total"])("rejects a changed %s between chunks", async field => {
  const pages = parts({ text: "x".repeat(SIZE * 2) });
  const second = pages[1]!.readChunk;
  if (field === "owner") second.id = "another_snapshot";
  if (field === "offset") second.offset++;
  if (field === "total") second.totalBytes++;
  const { client } = clientFor(pages);
  await expect(requestReadPage(client, { type: "get_events_since" }, "s")).rejects.toThrow("remote_read_chunk_changed");
});

test.each([
  { id: "r", offset: 0, nextOffset: 1, totalBytes: 17 * 1024 * 1024, data: "eA" },
  { id: "r", offset: 0, nextOffset: 0, totalBytes: 1, data: "" },
  { id: "r", offset: 0, nextOffset: 1, totalBytes: 2, data: "eA" },
  { id: "r", offset: 0, nextOffset: 2, totalBytes: 2, data: "eA" },
  { id: "r", offset: 0, nextOffset: 1, totalBytes: 1, data: "!" },
])("rejects malformed or over-budget chunks without issuing another request", async readChunk => {
  const { client, requestRetry } = clientFor([{ readChunk }]);
  await expect(requestReadPage(client, { type: "get_session_entries" }, "s")).rejects.toThrow("remote_read_invalid_chunk");
  expect(requestRetry).toHaveBeenCalledTimes(1);
});
