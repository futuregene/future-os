import type { TerminalTransportHandlers } from "./transport";

import { describe, expect, it, vi } from "vitest";
import { remoteTerminalError, remoteTerminalTransport } from "./remoteTerminal";
import { AttachAborted } from "./transport";

/**
 * The polling transport, driven through a fake host.
 *
 * Three properties are load-bearing and each has a way to be silently wrong:
 * bytes must arrive *before* the control frame that accounts for them, a
 * keystroke must never overtake the one typed before it (the host dispatches
 * commands concurrently), and a refused attach must reject rather than poll
 * forever.
 */

const hoisted = vi.hoisted(() => ({ request: vi.fn() }));

vi.mock("../remote-peer/remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => hoisted.request(...args),
}));

const request = hoisted.request;

/**
 * The wire's bytes dialect: URL-safe, unpadded base64.
 *
 * Deliberately *not* `btoa`: the whole point of the tests below is that this
 * alphabet is not the browser's, and a fixture that quietly used the standard
 * one made the seam look fine while it was broken (`-_` throws in `atob`,
 * `bA==` is refused by the host).
 */
function base64(text: string): string {
  return btoa(text).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** What the host answers with for the two bytes `[0xFB, 0xFF]`. */
const DIALECT_LITERAL = "-_8";

/** One `terminal_read` reply, as the host builds it. */
function page(bytes: string, cursor: number, options: {
  start?: number;
  status?: "running" | "exited";
  exitCode?: number | null;
} = {}) {
  return {
    data: base64(bytes),
    cursor,
    start: options.start ?? cursor - bytes.length,
    status: options.status ?? "running",
    exitCode: options.exitCode ?? null,
  };
}

interface Recorded {
  bytes: string[];
  /** The same pages as raw bytes: a page may be binary and not survive decode. */
  raw: number[][];
  metas: Array<{ cursor: number; start: number; exitCode?: number }>;
  closes: number[];
  missing: number;
  opened: number;
  texts: string[];
}

function handlers(): { handlers: TerminalTransportHandlers; record: Recorded } {
  const record: Recorded = { bytes: [], raw: [], metas: [], closes: [], missing: 0, opened: 0, texts: [] };
  return {
    record,
    handlers: {
      onOpen: () => {
        record.opened += 1;
      },
      onBytes: (bytes: Uint8Array) => {
        record.raw.push(Array.from(bytes));
        record.bytes.push(new TextDecoder().decode(bytes));
      },
      onMeta: (meta: { cursor: number; start: number; exitCode?: number } | undefined) => {
        if (meta)
          record.metas.push(meta);
      },
      onText: (text: string) => {
        record.texts.push(text);
      },
      onClose: (code: number) => {
        record.closes.push(code);
      },
      onMissing: () => {
        record.missing += 1;
      },
    },
  };
}

/** Route `terminal_read` calls through a script of replies. */
function script(reads: unknown[], options: { writes?: unknown[] } = {}) {
  const pending = [...reads];
  const writes = [...(options.writes ?? [])];
  request.mockImplementation(async (_desktopId: string, command: { type: string; terminalCursor?: number }) => {
    if (command.type === "terminal_read") {
      if (pending.length === 0) {
        // The shell is idle: a poll that finds nothing moves no cursor. Scripted
        // replies are consumed first, so a test only writes down what it cares
        // about.
        const at = command.terminalCursor ?? 0;
        return { data: "", cursor: at, start: at, status: "running", exitCode: null };
      }
      const next = pending.shift();
      if (next instanceof Error)
        throw next;
      return next;
    }
    if (command.type === "terminal_write")
      return writes.shift() ?? {};
    if (command.type === "terminal_update")
      return {};
    throw new Error(`unexpected command ${command.type}`);
  });
  return { remaining: () => pending.length };
}

describe("remoteTerminalTransport", () => {
  it("delivers the first page, then keeps polling", async () => {
    vi.useFakeTimers();
    script([
      page("hello", 5),
      page(" world", 11),
    ]);
    const { handlers: options, record } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", 0, options, new AbortController().signal);

    expect(record.bytes).toEqual(["hello"]);
    expect(record.metas[0]).toMatchObject({ cursor: 5, start: 0 });
    expect(record.opened).toBe(1);

    await vi.advanceTimersByTimeAsync(50);
    expect(record.bytes).toEqual(["hello", " world"]);
    expect(record.metas[1]).toMatchObject({ cursor: 11, start: 5 });

    link.close();
    vi.useRealTimers();
  });

  it("asks from the cursor it was given, never from zero", async () => {
    vi.useFakeTimers();
    script([page("", 40, { start: 40 })]);
    const { handlers: options } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", 40, options, new AbortController().signal);
    expect(request.mock.calls[0]![1]).toMatchObject({ type: "terminal_read", terminalCursor: 40 });
    link.close();
    vi.useRealTimers();
  });

  it("reports an exited shell once, with its code", async () => {
    vi.useFakeTimers();
    script([page("bye", 3, { status: "exited", exitCode: 7 })]);
    const { handlers: options, record } = handlers();
    await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);
    expect(record.bytes).toEqual(["bye"]);
    expect(record.metas[0]).toMatchObject({ exitCode: 7 });
    expect(record.closes).toEqual([1000]);
    vi.useRealTimers();
  });

  it("rejects an attach the host refuses, instead of polling a dead shell", async () => {
    const reads = script([new Error("TERMINAL_NOT_FOUND: that terminal is no longer on this machine")]);
    const { handlers: options, record } = handlers();
    await expect(
      remoteTerminalTransport("desk-b").open("term-1", 0, options, new AbortController().signal),
    ).rejects.toMatchObject({ code: "TERMINAL_NOT_FOUND" });
    expect(reads.remaining()).toBe(0);
    expect(record.opened).toBe(0);
  });

  it("marks a shell that disappears mid-session as missing", async () => {
    vi.useFakeTimers();
    script([
      page("", 0, { start: 0 }),
      new Error("TERMINAL_NOT_FOUND: that terminal is no longer on this machine"),
    ]);
    const { handlers: options, record } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);
    await vi.advanceTimersByTimeAsync(300);
    expect(record.missing).toBe(1);
    link.close();
    vi.useRealTimers();
  });

  it("batches keystrokes into one write and keeps their order", async () => {
    vi.useFakeTimers();
    script([page("", 0, { start: 0 })]);
    const { handlers: options } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);

    link.write("l");
    link.write("s");
    link.write("\r");
    expect(request.mock.calls.filter(call => call[1].type === "terminal_write")).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(20);
    const writes = request.mock.calls.filter(call => call[1].type === "terminal_write");
    expect(writes).toHaveLength(1);
    expect(atob(writes[0]![1].terminalData)).toBe("ls\r");

    link.close();
    vi.useRealTimers();
  });

  it("never has two writes in flight, so the host cannot reorder them", async () => {
    vi.useFakeTimers();
    script([page("", 0, { start: 0 })]);
    let release!: () => void;
    const inFlight = new Promise<void>((resolve) => {
      release = resolve;
    });
    const base = request.getMockImplementation()!;
    request.mockImplementation(async (desktopId: string, command: { type: string }) => {
      if (command.type === "terminal_write") {
        await inFlight;
        return {};
      }
      return base(desktopId, command);
    });
    const { handlers: options } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);

    link.write("a");
    await vi.advanceTimersByTimeAsync(20);
    link.write("b");
    await vi.advanceTimersByTimeAsync(50);
    // The first batch is stuck; the second must not be sent behind its back.
    expect(request.mock.calls.filter(call => call[1].type === "terminal_write")).toHaveLength(1);

    release();
    await vi.advanceTimersByTimeAsync(200);
    const writes = request.mock.calls.filter(call => call[1].type === "terminal_write");
    expect(writes.map(call => atob(call[1].terminalData))).toEqual(["a", "b"]);

    link.close();
    vi.useRealTimers();
  });

  it("abandons an attach that was aborted while its first page was in flight", async () => {
    const controller = new AbortController();
    script([page("hi", 2)]);
    const { handlers: options, record } = handlers();
    controller.abort();
    await expect(
      remoteTerminalTransport("desk-b").open("term-1", 0, options, controller.signal),
    ).rejects.toBeInstanceOf(AttachAborted);
    expect(record.opened).toBe(0);
  });
});

describe("the bytes dialect", () => {
  it("decodes the host's URL-safe unpadded pages", async () => {
    vi.useFakeTimers();
    // `-_8` is how the host spells [0xFB, 0xFF]; `atob` rejects it outright, and
    // a page containing those bytes used to stop the stream dead.
    const bytes = Uint8Array.from([0xFB, 0xFF]);
    script([{
      data: DIALECT_LITERAL,
      cursor: 2,
      start: 0,
      status: "running",
      exitCode: null,
    }]);
    const { handlers: options, record } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", 0, options, new AbortController().signal);
    expect(record.raw).toEqual([[0xFB, 0xFF]]);
    expect(bytes.length).toBe(2);
    link.close();
    vi.useRealTimers();
  });

  it("writes input in that same alphabet, unpadded", async () => {
    vi.useFakeTimers();
    script([page("", 0, { start: 0 })]);
    const { handlers: options } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);

    // One character: standard base64 would be `bA==`, which the host refuses.
    link.write("l");
    await vi.advanceTimersByTimeAsync(20);
    const write = request.mock.calls.find(call => call[1].type === "terminal_write");
    expect(write?.[1].terminalData).toBe("bA");

    link.write("ÿ");
    await vi.advanceTimersByTimeAsync(20);
    const writes = request.mock.calls.filter(call => call[1].type === "terminal_write");
    expect(writes[writes.length - 1]?.[1].terminalData).toBe("w78");

    link.close();
    vi.useRealTimers();
  });
});

describe("input", () => {
  it("splits a paste at the host's per-write cap instead of losing it", async () => {
    vi.useFakeTimers();
    script([page("", 0, { start: 0 })]);
    const { handlers: options } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);

    // Larger than one write may be: the host refuses above 256 KiB, and this
    // client sends at most 32 KiB per request.
    link.write("x".repeat(100 * 1024));
    await vi.advanceTimersByTimeAsync(60);
    const writes = request.mock.calls.filter(call => call[1].type === "terminal_write");
    expect(writes.length).toBeGreaterThan(1);
    const total = writes.reduce(
      (sum, call) => sum + atob(String(call[1].terminalData).replace(/-/g, "+").replace(/_/g, "/")).length,
      0,
    );
    expect(total).toBe(100 * 1024);

    link.close();
    vi.useRealTimers();
  });

  it("re-attaches instead of silently dropping keystrokes the host refused", async () => {
    vi.useFakeTimers();
    script([page("", 0, { start: 0 })]);
    const base = request.getMockImplementation()!;
    request.mockImplementation(async (desktopId: string, command: { type: string }) => {
      if (command.type === "terminal_write")
        throw new Error("PTY_IO_FAILED: write: broken pipe");
      return base(desktopId, command);
    });
    const { handlers: options, record } = handlers();
    const link = await remoteTerminalTransport("desk-b").open("term-1", -1, options, new AbortController().signal);

    link.write("a");
    await vi.advanceTimersByTimeAsync(30);
    expect(record.closes).toEqual([4408]);

    link.close();
    vi.useRealTimers();
  });
});

describe("remoteTerminalError", () => {
  it("splits the host's coded sentence into the code and the message", () => {
    const error = remoteTerminalError(new Error("CAPACITY_EXCEEDED: at most 8 terminals"));
    expect(error.code).toBe("CAPACITY_EXCEEDED");
    expect(error.message).toBe("at most 8 terminals");
  });

  it("keeps an uncoded failure readable rather than inventing a code", () => {
    const error = remoteTerminalError(new Error("failed to send request"));
    expect(error.code).toBe("REQUEST_FAILED");
    expect(error.message).toBe("failed to send request");
  });
});
