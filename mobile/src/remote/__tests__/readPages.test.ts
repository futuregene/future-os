import type { RemoteClient } from "../client";
import { encodeBase64Url } from "../codec";
import { requestReadPage } from "../readPages";

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
