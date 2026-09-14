import type { Root } from "react-dom/client";
/**
 * The replay and the control frame that follows it are two WebSocket messages,
 * and a 64 KiB replay chunk can end in the middle of a multi-byte character
 * (CJK, emoji). Decoding the control payload with a *non-streaming* call on the
 * decoder that holds those pending bytes flushes them out as U+FFFD, so the
 * control frame fails to parse: the view loses the cursor it must resume from
 * (and `start`/`exitCode` with it) exactly when the output is non-ASCII.
 *
 * The frames here are a real WebSocket exchange: raw bytes for output, a
 * `0x00`-prefixed JSON control frame after the replay.
 */
// @vitest-environment jsdom
import type { TerminalTab } from "./tabs";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView } from "./TerminalView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Everything xterm was asked to render, in order. */
const written: string[] = [];
/** The sockets the view opened, so a test can play the server. */
const sockets: FakeSocket[] = [];

class FakeSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readyState = FakeSocket.OPEN;
  binaryType = "arraybuffer";
  sent: string[] = [];
  private listeners = new Map<string, Array<(event: unknown) => void>>();

  constructor() {
    sockets.push(this);
  }

  addEventListener(type: string, listener: (event: unknown) => void) {
    const list = this.listeners.get(type) ?? [];
    list.push(listener);
    this.listeners.set(type, list);
    if (type === "open")
      queueMicrotask(() => this.emit("open", {}));
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
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
}

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    rows = 24;
    cols = 80;
    buffer = { active: { viewportY: 0 } };
    loadAddon() {}
    open() {}
    focus() {}
    dispose() {}
    scrollToLine() {}
    write(data: string, callback?: () => void) {
      written.push(data);
      callback?.();
    }

    onData() {
      return { dispose() {} };
    }

    onResize() {
      return { dispose() {} };
    }

    attachCustomKeyEventHandler() {}
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  },
}));
vi.mock("@xterm/addon-serialize", () => ({
  SerializeAddon: class {
    serialize() {
      return "";
    }
  },
}));
vi.mock("./client", () => ({
  TerminalApiError: class TerminalApiError extends Error {},
  connectTicket: async () => "ticket",
  connectUrl: () => "ws://127.0.0.1:1/terminal/term-1/connect",
  terminalServer: async () => ({ url: "http://127.0.0.1:1", token: "t", port: 1, maxSessions: 32 }),
  updateTerminal: async () => undefined,
}));

const TAB: TerminalTab = { id: "term-1", title: "Terminal 1", titleNumber: 1 };

let host: HTMLDivElement | undefined;
let root: Root | undefined;
let exits: Array<number | null> = [];

beforeEach(() => {
  written.length = 0;
  sockets.length = 0;
  exits = [];
  vi.stubGlobal("WebSocket", Object.assign(FakeSocket, { OPEN: 1, CLOSED: 3, CLOSING: 2, CONNECTING: 0 }));
  vi.stubGlobal("ResizeObserver", class {
    observe() {}
    disconnect() {}
  });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  vi.unstubAllGlobals();
});

/** Mount and wait for the view to open its socket. */
async function connect(): Promise<FakeSocket> {
  await act(async () => {
    root?.render(createElement(TerminalView, {
      tab: TAB,
      onExit: code => exits.push(code),
      onPersist: () => {},
    }));
  });
  await act(async () => {
    await Promise.resolve();
  });
  const socket = sockets[sockets.length - 1];
  expect(socket).toBeDefined();
  return socket as FakeSocket;
}

describe("terminal frames", () => {
  it("keeps a character that a replay chunk split, and still reads the control frame", async () => {
    const socket = await connect();
    const bytes = new TextEncoder().encode("中");

    // The replay chunk stops after two of the three bytes of 中.
    await act(async () => {
      socket.frame(bytes.subarray(0, 2));
    });
    // Then the control frame — ASCII JSON, but the decoder still holds bytes.
    const meta = new Uint8Array([0, ...new TextEncoder().encode("{\"cursor\":3,\"start\":0,\"exitCode\":7}")]);
    await act(async () => {
      socket.frame(meta);
    });
    // …and the rest of the character arrives with the live stream.
    await act(async () => {
      socket.frame(bytes.subarray(2));
    });

    expect(written.join("")).toBe("中");
    expect(exits).toEqual([7]);
  });
});
