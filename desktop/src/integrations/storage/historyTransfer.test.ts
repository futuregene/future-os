import { beforeEach, describe, expect, it, vi } from "vitest";
import { readHistoryTransfer, requestNativeRead } from "./historyTransfer";

const invokeMock = vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock("../tauri/invoke", () => ({ invokeCommand: (command: string, args?: Record<string, unknown>) => invokeMock(command, args) }));
beforeEach(() => {
  invokeMock.mockReset();
});
const chunkBytes = 64 * 1024;
function snapshot(bytes: Uint8Array, fallback = false) {
  invokeMock.mockImplementation(async (command, args) => {
    if (command === "begin_session_history_transfer")
      return { id: "snapshot", totalBytes: bytes.length, chunkBytes };
    if (command === "release_session_history_transfer")
      return;
    if (command !== "read_session_history_transfer")
      throw new Error(`unexpected command ${command}`);
    const offset = args?.offset as number;
    const chunk = bytes.slice(offset, offset + chunkBytes);
    return fallback ? Array.from(chunk) : chunk.buffer;
  });
}
describe("native history transfer", () => {
  it("keeps a small remote reply unchanged and does not open another transfer", async () => {
    const value = { entries: [], hasMore: false };
    invokeMock.mockResolvedValue(value);
    expect(await requestNativeRead("remote_peer_request", { command: { type: "get_session_entries" } })).toBe(value);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("remote_peer_request", { command: { type: "get_session_entries" } });
  });
  it("reads a large remote response descriptor through the same bounded IPC chunks", async () => {
    const value = { events: [{ data: "中🙂".repeat(30000) }], watermark: 5 };
    const bytes = new TextEncoder().encode(JSON.stringify(value));
    snapshot(bytes);
    const transport = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (command, args) => command === "remote_peer_request"
      ? { nativeReadTransfer: { id: "snapshot", totalBytes: bytes.length, chunkBytes } }
      : transport(command, args));
    expect(await requestNativeRead("remote_peer_request", { command: { type: "get_events_since" } })).toEqual(value);
    expect(invokeMock.mock.calls.some(([command]) => command === "begin_session_history_transfer")).toBe(false);
    expect(invokeMock).toHaveBeenLastCalledWith("release_session_history_transfer", { transferId: "snapshot" });
  });
  it.each([false, true])("reassembles binary or fallback chunks losslessly: %s", async (fallback) => {
    const page = { entries: [{ text: "中🙂".repeat(30000) }], history: { sourceKey: "immutable-source" }, nextOffset: 7, hasMore: true };
    const bytes = new TextEncoder().encode(JSON.stringify(page));
    snapshot(bytes, fallback);
    expect(await readHistoryTransfer({ threadId: "thread", before: 42 })).toEqual(page);
    const reads = invokeMock.mock.calls.filter(([command]) => command === "read_session_history_transfer");
    expect(reads.map(([, args]) => args?.offset)).toEqual(Array.from({ length: Math.ceil(bytes.length / chunkBytes) }, (_, index) => index * chunkBytes));
    expect(invokeMock).toHaveBeenLastCalledWith("release_session_history_transfer", { transferId: "snapshot" });
  });
  it("releases a failed transfer and preserves the read error", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "begin_session_history_transfer")
        return { id: "snapshot", totalBytes: 10, chunkBytes };
      throw new Error(command === "read_session_history_transfer" ? "read failed" : "cleanup failed");
    });
    await expect(readHistoryTransfer({})).rejects.toThrow("read failed");
    expect(invokeMock).toHaveBeenLastCalledWith("release_session_history_transfer", { transferId: "snapshot" });
  });
  it("fences a canceled read before the next chunk", async () => {
    let current = true;
    snapshot(new TextEncoder().encode(JSON.stringify("x".repeat(100000))));
    const original = invokeMock.getMockImplementation()!;
    invokeMock.mockImplementation(async (command, args) => {
      const result = await original(command, args);
      if (command === "read_session_history_transfer")
        current = false;
      return result;
    });
    await expect(readHistoryTransfer({}, () => current)).rejects.toThrow("stale_history_transfer");
    expect(invokeMock.mock.calls.filter(([command]) => command === "read_session_history_transfer")).toHaveLength(1);
  });
  it.each([0, 16 * 1024 * 1024 + 1, 1.5])("rejects invalid advertised sizes before allocating: %s", async (totalBytes) => {
    invokeMock.mockImplementation(async command => command === "begin_session_history_transfer" ? { id: "snapshot", totalBytes, chunkBytes } : undefined);
    await expect(readHistoryTransfer({})).rejects.toThrow("invalid_history_transfer");
    expect(invokeMock.mock.calls.some(([command]) => command === "read_session_history_transfer")).toBe(false);
  });
  it.each([new ArrayBuffer(1), [123, -1], [123, 256], [123, 0.5]])("rejects malformed chunk bytes", async (raw) => {
    invokeMock.mockImplementation(async command => command === "begin_session_history_transfer" ? { id: "snapshot", totalBytes: 2, chunkBytes } : raw);
    await expect(readHistoryTransfer({})).rejects.toThrow("invalid_history_transfer_chunk");
  });
});
