// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { resetTerminalServerCache } from "./client";
import { useTerminalTabs } from "./useTerminalTabs";

const invokeCommand = vi.fn();
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

const SERVER = { url: "http://127.0.0.1:41234", token: "t".repeat(64), port: 41234, maxSessions: 32 };

interface FetchCall {
  url: string;
  method: string;
  body: unknown;
}

let calls: FetchCall[] = [];

function installFetch(handler: (call: FetchCall) => { status: number; body: unknown }) {
  calls = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: FetchCall = {
      url: String(input),
      method: init?.method ?? "GET",
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    };
    calls.push(call);
    const { status, body } = handler(call);
    return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  });
  vi.stubGlobal("fetch", fetchMock);
}

beforeEach(() => {
  vi.unstubAllGlobals();
  localStorage.clear();
  invokeCommand.mockReset();
  invokeCommand.mockResolvedValue(SERVER);
  resetTerminalServerCache();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

async function flush() {
  // The controller awaits the browser's `fetch` microtasks plus React's state
  // commit; two macrotask turns is enough for both.
  await new Promise(resolve => setTimeout(resolve, 0));
  await new Promise(resolve => setTimeout(resolve, 0));
}

describe("useTerminalTabs", () => {
  it("starts a shell and records the tab", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 200,
        body: {
          id: "term_1",
          threadId: "thread-1",
          title: "Terminal 1",
          command: "/bin/bash",
          args: [],
          cwd: "/tmp",
          status: "running",
          exitCode: null,
          pid: 1,
          cols: 80,
          rows: 24,
        },
      };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    expect(harness.current.ready).toBe(true);

    await harness.current.create();
    await flush();
    expect(harness.current.tabs).toHaveLength(1);
    expect(harness.current.activeId).toBe("term_1");
    const created = calls.find(call => call.method === "POST");
    expect(created?.body).toMatchObject({ threadId: "thread-1", cwdPolicy: "thread" });

    // Persisted immediately, so a reload restores the tab.
    const stored = JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}");
    expect(stored.all).toHaveLength(1);
    harness.unmount();
  });

  it("reports a bad working directory with the home fallback available", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 400,
        body: { error: { code: "CWD_INVALID", message: "CWD_INVALID: /nope does not exist", allowsHomeFallback: true } },
      };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();

    expect(harness.current.createError?.code).toBe("CWD_INVALID");
    expect(harness.current.createError?.allowsHomeFallback).toBe(true);
    expect(harness.current.tabs).toHaveLength(0);

    // The confirmed fallback is the only retry that changes the policy.
    await harness.current.retryInHome();
    await flush();
    const attempts = calls.filter(call => call.method === "POST");
    expect(attempts).toHaveLength(2);
    expect(attempts[1]?.body).toMatchObject({ cwdPolicy: "homeConfirmed" });
    harness.unmount();
  });

  it("does not retry a failed create on its own", async () => {
    installFetch(() => ({ status: 500, body: { error: { code: "SPAWN_FAILED", message: "nope", allowsHomeFallback: false } } }));
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();
    const posts = calls.filter(call => call.method === "POST").length;
    await flush();
    expect(calls.filter(call => call.method === "POST").length).toBe(posts);
    harness.unmount();
  });

  it("keeps the final screen when the session is gone and offers a restart", async () => {
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1, buffer: "old screen", cursor: 10 }],
    }));
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      if (call.method === "DELETE")
        return { status: 404, body: { error: { code: "TERMINAL_NOT_FOUND", message: "gone", allowsHomeFallback: false } } };
      return {
        status: 200,
        body: {
          id: "term_2",
          threadId: "thread-1",
          title: "Terminal 1",
          command: "/bin/bash",
          args: [],
          cwd: "/tmp",
          status: "running",
          exitCode: null,
          pid: 2,
          cols: 80,
          rows: 24,
        },
      };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    expect(harness.current.tabs[0]?.missing).toBe(true);
    expect(harness.current.tabs[0]?.buffer).toBe("old screen");

    await harness.current.restart("term_1");
    await flush();
    expect(harness.current.tabs[0]?.id).toBe("term_2");
    expect(harness.current.tabs[0]?.missing).toBe(false);
    // The restart asks for a fresh shell with the same label.
    const created = calls.find(call => call.method === "POST");
    expect(created?.body).toMatchObject({ title: "Terminal 1" });
    harness.unmount();
  });

  it("closing a tab removes it and asks the server to stop the shell", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      if (call.method === "DELETE")
        return { status: 200, body: { removed: true } };
      return {
        status: 200,
        body: {
          id: "term_1",
          threadId: "thread-1",
          title: "Terminal 1",
          command: "/bin/bash",
          args: [],
          cwd: "/tmp",
          status: "running",
          exitCode: null,
          pid: 1,
          cols: 80,
          rows: 24,
        },
      };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();
    await harness.current.close("term_1");
    await flush();
    expect(harness.current.tabs).toHaveLength(0);
    expect(calls.some(call => call.method === "DELETE" && call.url.endsWith("/terminal/term_1"))).toBe(true);
    expect(localStorage.getItem("future.terminal.tabs.v1.thread-1")).toBeNull();
    harness.unmount();
  });

  it("records the server's exit code on the tab", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 200,
        body: {
          id: "term_1",
          threadId: "thread-1",
          title: "Terminal 1",
          command: "/bin/bash",
          args: [],
          cwd: "/tmp",
          status: "running",
          exitCode: null,
          pid: 1,
          cols: 80,
          rows: 24,
        },
      };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();
    harness.current.markExit("term_1", 7);
    await flush();
    expect(harness.current.tabs[0]?.exitCode).toBe(7);
    harness.unmount();
  });

  it("reports a transport failure without inventing tabs", async () => {
    installFetch(() => ({ status: 500, body: {} }));
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    expect(harness.current.error).not.toBeNull();
    expect(harness.current.tabs).toHaveLength(0);
    harness.unmount();
  });

  it("does nothing without a conversation", async () => {
    installFetch(() => ({ status: 200, body: [] }));
    const harness = renderHook(() => useTerminalTabs(null));
    await flush();
    await harness.current.create();
    await flush();
    expect(harness.current.tabs).toHaveLength(0);
    expect(calls).toHaveLength(0);
    harness.unmount();
  });
});
