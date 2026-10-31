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

  /** Two transfers that hold both slots until the test releases them. */
  function holdSlots() {
    let release = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    invokeMock.mockImplementation(async (command) => {
      if (command === "begin_session_history_transfer")
        await held;
      return undefined;
    });
    // These never finish on their own; the caller releases them and swallows the
    // rejection a released transfer produces from its bogus descriptor.
    const held_reads = [readHistoryTransfer({}).catch(() => {}), readHistoryTransfer({}).catch(() => {})];
    return {
      async finish() {
        release();
        await Promise.all(held_reads);
      },
      held_reads,
    };
  }

  it("refuses to queue a 33rd transfer instead of piling up unbounded", async () => {
    // Two transfers hold the slots and never answer, so every later caller parks.
    // Refusing the 33rd waiter is what keeps a burst of reads from becoming memory
    // the app never gets back.
    const slots = holdSlots();
    await Promise.resolve();
    // 32 callers park behind the two running transfers; the next one is refused
    // before it queues at all.
    const started = Array.from({ length: 33 }, () => readHistoryTransfer({}).catch(error => (error as Error).message));
    await Promise.resolve();
    // The refusal is decided synchronously in `admit`, so it has already settled.
    await expect(started[32]!).resolves.toBe("history_transfer_queue_full");
    await slots.finish();
    // The parked callers all proceed once the slots free up.
    const outcomes = await Promise.all(started);
    expect(outcomes.filter(outcome => outcome === "history_transfer_queue_full")).toEqual(["history_transfer_queue_full"]);
  });

  it("gives up waiting for a slot instead of blocking for ever", async () => {
    vi.useFakeTimers();
    try {
      const slots = holdSlots();
      await vi.advanceTimersByTimeAsync(1);
      const waiting = readHistoryTransfer({});
      const outcome = expect(waiting).rejects.toThrow("history_transfer_admission_timeout");
      await vi.advanceTimersByTimeAsync(6000);
      await outcome;
      await vi.advanceTimersByTimeAsync(1);
      await slots.finish();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("refuses a waiter that was cancelled while it queued", async () => {
    vi.useFakeTimers();
    try {
      const slots = holdSlots();
      await vi.advanceTimersByTimeAsync(1);
      let current = true;
      const cancelled = readHistoryTransfer({}, () => current);
      const outcome = expect(cancelled).rejects.toThrow("stale_history_transfer");
      current = false;
      await vi.advanceTimersByTimeAsync(1);
      await outcome;
      expect(invokeMock.mock.calls.filter(([command]) => command === "read_session_history_transfer")).toHaveLength(0);
      await slots.finish();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("refuses a read that was cancelled before its slot was granted", async () => {
    // `isCurrent` is asked again after admission: a read cancelled in the window
    // between queueing and starting must never open a transfer or allocate.
    invokeMock.mockResolvedValue(undefined);
    await expect(readHistoryTransfer({}, () => false)).rejects.toThrow("stale_history_transfer");
    expect(invokeMock.mock.calls.some(([command]) => command === "begin_session_history_transfer")).toBe(false);
  });

  it("refuses a small remote reply that arrived after the reader was cancelled", async () => {
    // The reply was small enough to come back inline, but the view that asked for it
    // has moved on: handing it over would repaint a page nobody is looking at.
    // Admission says yes (the reader was current when it asked); the cancel lands
    // while the reply is in flight.
    let asked = 0;
    invokeMock.mockResolvedValue({ entries: [], hasMore: false });
    await expect(requestNativeRead("remote_peer_request", {}, () => asked++ === 0)).rejects.toThrow("stale_history_transfer");
  });

  it("refuses a descriptor whose id is not an opaque token", async () => {
    // The id is handed to the next IPC call, so anything that is not a plain token
    // is refused before it is used as one.
    invokeMock.mockImplementation(async command => command === "begin_session_history_transfer"
      ? { chunkBytes, id: "not an id", totalBytes: 12 }
      : undefined);
    await expect(readHistoryTransfer({})).rejects.toThrow("invalid_history_transfer");
    expect(invokeMock.mock.calls.some(([command]) => command === "read_session_history_transfer")).toBe(false);
  });

  it("refuses a read cancelled after admission but before the first chunk", async () => {
    // One yes for admission, then the reader is gone: the buffer is never allocated
    // and no chunk is asked for.
    let asked = 0;
    invokeMock.mockImplementation(async command => command === "begin_session_history_transfer"
      ? { chunkBytes, id: "snapshot", totalBytes: 12 }
      : undefined);
    await expect(readHistoryTransfer({}, () => asked++ === 0)).rejects.toThrow("stale_history_transfer");
    expect(invokeMock.mock.calls.some(([command]) => command === "read_session_history_transfer")).toBe(false);
  });

  it("refuses a read cancelled between the pre-allocation check and the first chunk", async () => {
    // The last moment a cancel can be honoured is just before the loop asks for a
    // chunk; the transfer is released either way.
    let asked = 0;
    invokeMock.mockImplementation(async command => command === "begin_session_history_transfer"
      ? { chunkBytes, id: "snapshot", totalBytes: 12 }
      : undefined);
    await expect(readHistoryTransfer({}, () => asked++ < 2)).rejects.toThrow("stale_history_transfer");
    expect(invokeMock.mock.calls.filter(([command]) => command === "read_session_history_transfer")).toHaveLength(0);
    expect(invokeMock).toHaveBeenLastCalledWith("release_session_history_transfer", { transferId: "snapshot" });
  });
});
