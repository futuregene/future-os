// @vitest-environment jsdom
import type { Root } from "react-dom/client";
import type { TerminalTab } from "./tabs";
import type { TerminalServerHandle } from "./types";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView } from "./TerminalView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Everything xterm was asked to render, in order. */
const written: string[] = [];
/** The sockets the view opened, so a test can play the server. */
const sockets: FakeSocket[] = [];
/** Drives the mocked xterm's write callback (the persistence drain). */
let writeCallback: (() => void) | undefined;
let deferWrites = false;
let dataHandler: ((data: string) => void) | undefined;
let resizeHandler: ((size: { cols: number; rows: number }) => void) | undefined;
let observerCallback: (() => void) | undefined;
let rafCallbacks: FrameRequestCallback[] = [];
let deferRaf = false;
let fitCalls = 0;
let focusCalls = 0;
let disposeCalls = 0;
let scrollTo: number[] = [];

class FakeSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readyState = FakeSocket.OPEN;
  binaryType = "arraybuffer";
  sent: string[] = [];
  closedWith: number | undefined;
  private listeners = new Map<string, Array<(event: unknown) => void>>();
  private autoOpen = true;

  constructor() {
    sockets.push(this);
  }

  addEventListener(type: string, listener: (event: unknown) => void) {
    const list = this.listeners.get(type) ?? [];
    list.push(listener);
    this.listeners.set(type, list);
    if (type === "open" && this.autoOpen && this.readyState !== FakeSocket.CLOSED)
      queueMicrotask(() => this.emit("open", {}));
  }

  send(data: string) {
    if (this.readyState !== FakeSocket.OPEN)
      throw new Error("send on a closed socket");
    this.sent.push(data);
  }

  close(code?: number) {
    this.closedWith = code;
    this.readyState = FakeSocket.CLOSED;
  }

  emit(type: string, event: unknown) {
    for (const listener of this.listeners.get(type) ?? [])
      listener(event);
  }

  /** Deliver one binary frame, exactly as the server would. */
  frame(bytes: Uint8Array) {
    const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    this.emit("message", { data: buffer });
  }

  /** Deliver a text frame (the view renders it rather than dropping it). */
  textFrame(text: string) {
    this.emit("message", { data: text });
  }
}

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    rows = 24;
    cols = 80;
    buffer = { active: { viewportY: 7 } };
    loadAddon() {}
    open() {}
    focus() {
      focusCalls += 1;
    }

    dispose() {
      disposeCalls += 1;
    }

    scrollToLine(line: number) {
      scrollTo.push(line);
    }

    write(data: string, callback?: () => void) {
      written.push(data);
      if (deferWrites) {
        writeCallback = callback;
        return;
      }
      callback?.();
    }

    onData(handler: (data: string) => void) {
      dataHandler = handler;
      return { dispose() {} };
    }

    onResize(handler: (size: { cols: number; rows: number }) => void) {
      resizeHandler = handler;
      return { dispose() {} };
    }

    attachCustomKeyEventHandler() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {
      fitCalls += 1;
    }
  },
}));
vi.mock("@xterm/addon-serialize", () => ({
  SerializeAddon: class {
    serialize() {
      return "serialized screen";
    }
  },
}));

const clientMocks = vi.hoisted(() => {
  class TerminalApiError extends Error {
    readonly code: string;
    readonly status: number;
    constructor(code: string, message: string, status: number) {
      super(message);
      this.code = code;
      this.status = status;
    }
  }
  return {
    TerminalApiError,
    connectTicket: vi.fn(async (_id: string) => "ticket"),
    terminalServer: vi.fn(async () => ({ maxSessions: 32, port: 1, token: "t", url: "http://127.0.0.1:1" })),
    updateTerminal: vi.fn(async (_id: string, _input: unknown) => undefined),
  };
});

vi.mock("./client", () => ({
  TerminalApiError: clientMocks.TerminalApiError,
  connectTicket: (id: string) => clientMocks.connectTicket(id),
  connectUrl: (_server: unknown, id: string, cursor: number | undefined, ticket: string) =>
    `ws://127.0.0.1:1/terminal/${id}/connect?cursor=${cursor ?? "-1"}&ticket=${ticket}`,
  terminalServer: () => clientMocks.terminalServer(),
  updateTerminal: (id: string, input: unknown) => clientMocks.updateTerminal(id, input),
}));

const { TerminalApiError, connectTicket, terminalServer, updateTerminal } = clientMocks;

const TAB: TerminalTab = { id: "term-1", title: "Terminal 1", titleNumber: 1 };

let host: HTMLDivElement | undefined;
let root: Root | undefined;
let exits: Array<number | null> = [];
let persisted: Array<Partial<TerminalTab>> = [];
let connected = 0;
let disconnected: string[] = [];
let missing = 0;

beforeEach(() => {
  written.length = 0;
  sockets.length = 0;
  exits = [];
  persisted = [];
  connected = 0;
  disconnected = [];
  missing = 0;
  writeCallback = undefined;
  deferWrites = false;
  dataHandler = undefined;
  resizeHandler = undefined;
  observerCallback = undefined;
  rafCallbacks = [];
  deferRaf = false;
  fitCalls = 0;
  focusCalls = 0;
  disposeCalls = 0;
  scrollTo = [];
  connectTicket.mockClear();
  connectTicket.mockImplementation(async () => "ticket");
  terminalServer.mockClear();
  terminalServer.mockImplementation(async () => ({ maxSessions: 32, port: 1, token: "t", url: "http://127.0.0.1:1" }));
  updateTerminal.mockClear();
  updateTerminal.mockImplementation(async () => undefined);
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) {
      observerCallback = callback;
    }

    observe() {}
    disconnect() {}
  });
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    rafCallbacks.push(callback);
    if (!deferRaf) {
      rafCallbacks = [];
      callback(0);
    }
    return rafCallbacks.length;
  });
  vi.stubGlobal("cancelAnimationFrame", () => {});
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** Render the view, let it attach, and hand back the socket it opened. */
async function connect(
  tab: TerminalTab = TAB,
  props: Partial<Parameters<typeof TerminalView>[0]> = {},
): Promise<FakeSocket> {
  await act(async () => {
    root?.render(createElement(TerminalView, {
      autoFocus: false,
      onConnect: () => {
        connected += 1;
      },
      onDisconnected: (reason: string) => {
        disconnected.push(reason);
      },
      onExit: code => exits.push(code),
      onMissing: () => {
        missing += 1;
      },
      onPersist: patch => persisted.push(patch),
      tab,
      ...props,
    }));
  });
  await flush();
  // Every test that binds the result drives a view whose attach succeeded; the
  // ones that assert `sockets` stayed empty ignore the return value.
  return latestSocket();
}

function latestSocket(): FakeSocket {
  return sockets[sockets.length - 1]!;
}

describe("terminalView connection", () => {
  it("attaches, reports the first size, and forwards typed input", async () => {
    const socket = await connect(TAB, { autoFocus: true });
    expect(socket).toBeInstanceOf(FakeSocket);
    expect(connected).toBe(1);
    expect(focusCalls).toBe(1);
    // The first measurement goes out immediately (no debounce) so the shell
    // starts at the geometry the user sees.
    expect(updateTerminal).toHaveBeenCalledWith("term-1", { cols: 80, rows: 24 });

    await act(async () => {
      dataHandler?.("ls\r");
    });
    expect(socket.sent).toEqual(["ls\r"]);
  });

  it("drops keystrokes while the socket is gone", async () => {
    await connect();
    const socket = latestSocket();
    socket.readyState = FakeSocket.CLOSED;
    await act(async () => {
      dataHandler?.("ignored");
    });
    expect(socket.sent).toEqual([]);
  });

  it("restores a stored screen and resumes from its cursor", async () => {
    const tab: TerminalTab = { ...TAB, buffer: "restored screen", cols: 100, cursor: 42, rows: 30, scrollY: 12 };
    const socket = await connect(tab);
    expect(written[0]).toBe("restored screen");
    expect(scrollTo).toEqual([12]);
    // `seek` is the cursor the screen already contains, not a tail.
    expect(connectTicket).toHaveBeenCalledWith("term-1");
    expect(socket).toBeDefined();
    expect(fitCalls).toBeGreaterThan(0);
  });

  it("tails the stream when a restored screen has no cursor", async () => {
    const tab: TerminalTab = { ...TAB, buffer: "screen without cursor" };
    const socket = await connect(tab);
    // A restored screen with no stored cursor must not be replayed.
    expect(socket).toBeDefined();
    expect(written[0]).toBe("screen without cursor");
  });

  it("renders a text frame instead of dropping the output", async () => {
    const socket = await connect();
    await act(async () => {
      socket.textFrame("plain text frame");
    });
    expect(written.join("")).toContain("plain text frame");
  });

  it("ignores an empty binary frame", async () => {
    const socket = await connect();
    await act(async () => {
      socket.frame(new Uint8Array([]));
    });
    expect(written).toEqual([]);
  });

  it("says so when the server no longer retains the detached output", async () => {
    // seek = 0 for a fresh view; a replay starting later means a hole.
    const socket = await connect(TAB);
    const meta = new Uint8Array([0, ...new TextEncoder().encode("{\"cursor\":99,\"start\":50}")]);
    await act(async () => {
      socket.frame(meta);
    });
    expect(written.join("")).toContain("output was truncated while this view was detached");
  });

  it("reports an exit code carried by the control frame, including a null one", async () => {
    const socket = await connect();
    await act(async () => {
      socket.frame(new Uint8Array([0, ...new TextEncoder().encode("{\"cursor\":5,\"exitCode\":null}")]));
    });
    expect(exits).toEqual([null]);
  });

  it("drops an unparseable control frame without losing the stream", async () => {
    const socket = await connect();
    await act(async () => {
      socket.frame(new Uint8Array([0, ...new TextEncoder().encode("{not json")]));
    });
    expect(written).toEqual([]);

    await act(async () => {
      socket.frame(new TextEncoder().encode("after"));
    });
    expect(written.join("")).toBe("after");
  });
});

describe("terminalView resizing", () => {
  it("debounces a resize and sends the settled geometry once", async () => {
    vi.useFakeTimers();
    await connect();
    updateTerminal.mockClear();

    await act(async () => {
      resizeHandler?.({ cols: 120, rows: 40 });
      resizeHandler?.({ cols: 140, rows: 44 });
    });
    expect(updateTerminal).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(updateTerminal).toHaveBeenCalledTimes(1);
    expect(updateTerminal).toHaveBeenCalledWith("term-1", { cols: 140, rows: 44 });
  });

  it("does not send a size that did not change", async () => {
    vi.useFakeTimers();
    await connect();
    updateTerminal.mockClear();

    await act(async () => {
      resizeHandler?.({ cols: 80, rows: 24 });
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(updateTerminal).not.toHaveBeenCalled();
  });

  it("refits when the window resizes", async () => {
    await connect();
    const before = fitCalls;
    await act(async () => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(fitCalls).toBe(before + 1);
  });

  it("refits when the container itself is resized", async () => {
    await connect();
    const before = fitCalls;
    await act(async () => {
      observerCallback?.();
    });
    expect(fitCalls).toBe(before + 1);
  });

  it("coalesces overlapping fit requests into one animation frame", async () => {
    deferRaf = true;
    await connect();
    const before = fitCalls;

    // Two triggers before the frame runs must not queue two fits.
    await act(async () => {
      window.dispatchEvent(new Event("resize"));
      observerCallback?.();
    });
    expect(rafCallbacks).toHaveLength(1);

    await act(async () => {
      const [frame] = rafCallbacks;
      rafCallbacks = [];
      frame?.(0);
    });
    expect(fitCalls).toBe(before + 1);

    // A later trigger schedules again.
    await act(async () => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(rafCallbacks).toHaveLength(1);
  });

  it("cancels a pending resize on unmount", async () => {
    vi.useFakeTimers();
    await connect();
    updateTerminal.mockClear();
    await act(async () => {
      resizeHandler?.({ cols: 200, rows: 60 });
    });

    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(updateTerminal).not.toHaveBeenCalled();
  });

  it("tolerates a rejected size update", async () => {
    updateTerminal.mockRejectedValue(new Error("terminal is gone"));
    await connect();
    await act(async () => {
      await Promise.resolve();
    });
    // The failure is swallowed: a size the server refuses must not break the
    // view (the terminal keeps working at the geometry it has).
    expect(written).toEqual([]);
    expect(disconnected).toEqual([]);
  });
});

describe("terminalView reconnect", () => {
  it("resumes from the applied cursor after the server dropped a lagging viewer", async () => {
    vi.useFakeTimers();
    const socket = await connect();
    const meta = new Uint8Array([0, ...new TextEncoder().encode("{\"cursor\":512,\"start\":0}")]);
    await act(async () => {
      socket.frame(meta);
    });

    await act(async () => {
      socket.emit("close", { code: 4408 });
    });
    // No disconnect is reported for a lagged resume, and the retry is immediate.
    expect(disconnected).toEqual([]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(sockets).toHaveLength(2);
    expect(connected).toBe(2);
  });

  it("reports a drop and retries with backoff", async () => {
    vi.useFakeTimers();
    const socket = await connect();
    await act(async () => {
      socket.emit("close", { code: 1006 });
    });
    expect(disconnected).toEqual(["terminal connection closed (1006)"]);
    expect(sockets).toHaveLength(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(249);
    });
    expect(sockets).toHaveLength(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(sockets).toHaveLength(2);
    expect(connected).toBe(2);
  });

  it("backs off further on each successive failure and stops at the ceiling", async () => {
    vi.useFakeTimers();
    // No attachment ever succeeds, so `tries` never resets: 250, 500, 1000,
    // 2000, 4000, then the ceiling for every later attempt.
    connectTicket.mockRejectedValue(new Error("listener not bound"));
    await connect();
    expect(disconnected).toEqual(["listener not bound"]);
    expect(sockets).toHaveLength(0);

    const delays = [250, 500, 1000, 2000, 4000, 4000];
    for (const delay of delays) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(delay - 1);
      });
      const attemptsBefore = connectTicket.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
      expect(connectTicket.mock.calls.length).toBe(attemptsBefore + 1);
    }
    expect(disconnected).toHaveLength(7);
  });

  it("treats a normal close as the shell exiting, with no reconnect", async () => {
    vi.useFakeTimers();
    const socket = await connect();
    await act(async () => {
      socket.emit("close", { code: 1000 });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(sockets).toHaveLength(1);
    expect(disconnected).toEqual([]);
  });

  it("marks the tab missing when the server no longer has the session", async () => {
    vi.useFakeTimers();
    connectTicket.mockRejectedValueOnce(new TerminalApiError("TERMINAL_NOT_FOUND", "gone", 404));
    await connect();
    expect(missing).toBe(1);
    expect(sockets).toHaveLength(0);
    // A gone session is not retried: only a restart can help.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(connectTicket).toHaveBeenCalledTimes(1);
  });

  it("retries when only the connect ticket fails", async () => {
    vi.useFakeTimers();
    connectTicket.mockRejectedValueOnce(new Error("listener not bound"));
    await connect();
    expect(disconnected).toEqual(["listener not bound"]);
    expect(sockets).toHaveLength(0);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(sockets).toHaveLength(1);
  });

  it("retries when the server endpoint cannot be resolved", async () => {
    vi.useFakeTimers();
    terminalServer.mockRejectedValueOnce(new Error("no terminal server"));
    await connect();
    expect(disconnected).toEqual(["no terminal server"]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(sockets).toHaveLength(1);
  });

  it("schedules only one retry when the socket drops twice in a row", async () => {
    vi.useFakeTimers();
    const socket = await connect();
    await act(async () => {
      socket.emit("close", { code: 1006 });
      socket.emit("close", { code: 1006 });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(sockets).toHaveLength(2);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(sockets).toHaveLength(2);
  });

  it("abandons a connection whose ticket arrives after unmount", async () => {
    let settleTicket!: (value: string) => void;
    connectTicket.mockImplementation(() => new Promise<string>((resolve) => {
      settleTicket = resolve;
    }));
    await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);

    await act(async () => {
      settleTicket("late-ticket");
      await Promise.resolve();
    });
    expect(sockets).toHaveLength(0);
  });

  it("abandons a connection whose ticket fails after unmount", async () => {
    let failTicket!: (error: unknown) => void;
    connectTicket.mockImplementation(() => new Promise<string>((_resolve, reject) => {
      failTicket = reject;
    }));
    await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);

    await act(async () => {
      failTicket(new Error("too late"));
      await Promise.resolve();
    });
    expect(disconnected).toEqual([]);
    expect(sockets).toHaveLength(0);
  });

  it("abandons a connection whose endpoint arrives after unmount", async () => {
    let settleServer!: (value: TerminalServerHandle) => void;
    terminalServer.mockImplementation(() => new Promise((resolve) => {
      settleServer = resolve;
    }));
    await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);

    await act(async () => {
      settleServer({ maxSessions: 32, port: 1, token: "t", url: "http://127.0.0.1:1" });
      await Promise.resolve();
    });
    expect(sockets).toHaveLength(0);
  });

  it("swallows an endpoint failure that lands after unmount", async () => {
    let failServer!: (error: unknown) => void;
    terminalServer.mockImplementation(() => new Promise((_resolve, reject) => {
      failServer = reject;
    }));
    await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);

    await act(async () => {
      failServer(new Error("listener gone"));
      await Promise.resolve();
    });
    // No disconnect notice and no retry for a view that is gone.
    expect(disconnected).toEqual([]);
  });
});

describe("terminalView teardown", () => {
  it("closes the socket, persists the applied screen and disposes xterm", async () => {
    const socket = await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);
    expect(socket.closedWith).toBe(1000);
    expect(persisted).toEqual([{ buffer: "serialized screen", cols: 80, cursor: undefined, rows: 24, scrollY: 7 }]);
    // The terminal is released twice: once through the effect's disposables and
    // once after the final screen has been drained (xterm's dispose is
    // idempotent).
    expect(disposeCalls).toBe(2);
  });

  it("drains queued output before serializing the screen", async () => {
    deferWrites = true;
    const socket = await connect();
    socket.emit("message", { data: new TextEncoder().encode("last chunk").buffer });

    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);
    // Nothing is persisted until the pending write has been applied.
    expect(persisted).toEqual([]);
    await act(async () => {
      writeCallback?.();
    });
    expect(persisted).toHaveLength(1);
    expect(disposeCalls).toBe(2);
  });

  it("cancels a scheduled reconnect when the view unmounts", async () => {
    vi.useFakeTimers();
    const socket = await connect();
    await act(async () => {
      socket.emit("close", { code: 1006 });
    });
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(sockets).toHaveLength(1);
  });

  it("ignores frames and events that arrive after unmount", async () => {
    const socket = await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);
    const writesBefore = written.length;
    await act(async () => {
      socket.frame(new TextEncoder().encode("late"));
      socket.emit("close", { code: 1006 });
      socket.emit("open", {});
    });
    expect(written).toHaveLength(writesBefore);
    expect(connected).toBe(1);
  });

  it("does not persist a second time when the write queue was already empty", async () => {
    await connect();
    await act(async () => {
      root?.unmount();
    });
    root = createRoot(host!);
    expect(persisted).toHaveLength(1);
    expect(written).toEqual([]);
  });

  it("persists without a buffer when the screen cannot be serialized", async () => {
    vi.doMock("@xterm/addon-serialize", () => ({
      SerializeAddon: class {
        serialize() {
          throw new Error("serializer exploded");
        }
      },
    }));
    vi.resetModules();
    const { TerminalView: FreshView } = await import("./TerminalView");
    const freshHost = document.createElement("div");
    document.body.append(freshHost);
    const freshRoot = createRoot(freshHost);
    try {
      await act(async () => {
        freshRoot.render(createElement(FreshView, {
          onExit: () => {},
          onPersist: patch => persisted.push(patch),
          tab: TAB,
        }));
      });
      await flush();
      await act(async () => {
        freshRoot.unmount();
      });
      // The cursor and geometry are still persisted; only the screen is lost.
      expect(persisted).toHaveLength(1);
      expect(persisted[0]).not.toHaveProperty("buffer");
      expect(persisted[0]).toMatchObject({ cols: 80, rows: 24 });
    }
    finally {
      freshHost.remove();
      vi.doUnmock("@xterm/addon-serialize");
      vi.resetModules();
    }
  });
});
