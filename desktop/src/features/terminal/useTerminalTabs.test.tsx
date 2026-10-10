// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { resetTerminalServerCache } from "./client";
import { useTerminalTabs } from "./useTerminalTabs";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Props-aware probe: the hook's conversation can change under it. */
function mountHook<P, R>(useHook: (props: P) => R, initialProps: P) {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  let current!: R;
  function Probe({ props }: { props: P }) {
    current = useHook(props);
    return null;
  }
  act(() => {
    root.render(createElement(Probe, { props: initialProps }));
  });
  return {
    get current() {
      return current;
    },
    setProps: (props: P) => {
      act(() => {
        root.render(createElement(Probe, { props }));
      });
    },
    unmount: () => {
      act(() => root.unmount());
      host.remove();
    },
  };
}

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
    expect(created?.body).toMatchObject({ threadId: "thread-1" });

    // Persisted immediately, so a reload restores the tab.
    const stored = JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}");
    expect(stored.all).toHaveLength(1);
    harness.unmount();
  });

  it("reports a create failure and retries on demand", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 400,
        body: { error: { code: "SPAWN_FAILED", message: "SPAWN_FAILED: nope" } },
      };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();

    expect(harness.current.createError?.code).toBe("SPAWN_FAILED");
    expect(harness.current.tabs).toHaveLength(0);

    await harness.current.retry();
    await flush();
    const attempts = calls.filter(call => call.method === "POST");
    expect(attempts).toHaveLength(2);
    expect(attempts[1]?.body).toMatchObject({ threadId: "thread-1" });
    harness.unmount();
  });

  it("does not retry a failed create on its own", async () => {
    installFetch(() => ({ status: 500, body: { error: { code: "SPAWN_FAILED", message: "nope" } } }));
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
        return { status: 404, body: { error: { code: "TERMINAL_NOT_FOUND", message: "gone" } } };
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

  it("persists view state synchronously so an unmount cannot lose the screen", async () => {
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

    // This is what the mounted view does on its way out (collapse, tab switch,
    // webview reload). The write must not depend on React committing a render
    // that the unmount would discard.
    harness.current.save("term_1", { buffer: "serialized screen", cursor: 42, scrollY: 3 });
    const stored = JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}");
    expect(stored.all[0]).toMatchObject({ buffer: "serialized screen", cursor: 42, scrollY: 3 });

    await act(async () => {
      harness.unmount();
    });
    const afterUnmount = JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}");
    expect(afterUnmount.all[0]).toMatchObject({ buffer: "serialized screen", cursor: 42 });
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

  it("ignores a restart for a conversation or tab that is not there", async () => {
    installFetch(() => ({ status: 200, body: [] }));
    const withoutThread = renderHook(() => useTerminalTabs(null));
    await flush();
    await withoutThread.current.restart("term_1");
    await flush();
    expect(calls).toHaveLength(0);
    withoutThread.unmount();

    const unknownTab = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    calls = [];
    await unknownTab.current.restart("term_missing");
    await flush();
    expect(calls).toHaveLength(0);
    unknownTab.unmount();
  });

  it("reports a restart that fails at the transport level", async () => {
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1 }],
    }));
    // The old session is already gone (DELETE 404, swallowed) and the fresh
    // spawn never reaches the server.
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      if (call.method === "DELETE")
        return { status: 404, body: { error: { code: "TERMINAL_NOT_FOUND", message: "gone" } } };
      return { status: 500, body: { error: { code: "SPAWN_FAILED", message: "no pty" } } };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.restart("term_1");
    await flush();
    expect(harness.current.createError?.code).toBe("SPAWN_FAILED");
    // The stored tab is untouched — the restart did not half-apply.
    expect(harness.current.tabs[0]).toMatchObject({ id: "term_1", title: "Terminal 1" });
    harness.unmount();
  });

  it("wraps a non-API create failure in the stable error shape", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      throw new TypeError("Failed to fetch");
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();
    expect(harness.current.createError).toMatchObject({ code: "REQUEST_FAILED", message: "Failed to fetch", status: 0 });
    harness.unmount();
  });

  it("marks a session the server lost and can clear the error banner", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 400, body: { error: { code: "SPAWN_FAILED", message: "nope" } } };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    await harness.current.create();
    await flush();
    expect(harness.current.createError).not.toBeNull();
    harness.current.dismissCreateError();
    await flush();
    expect(harness.current.createError).toBeNull();
    harness.unmount();
  });

  it("records a session the server no longer has as missing", async () => {
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1, buffer: "screen" }],
    }));
    installFetch(() => ({ status: 200, body: [] }));
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    harness.current.markMissing("term_1");
    await flush();
    expect(harness.current.tabs[0]?.missing).toBe(true);
    expect(harness.current.tabs[0]?.buffer).toBe("screen");
    const stored = JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}");
    expect(stored.all[0]?.missing).toBe(true);
    harness.unmount();
  });

  it("switches the active tab and persists the choice", async () => {
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [
        { id: "term_1", title: "Terminal 1", titleNumber: 1 },
        { id: "term_2", title: "Terminal 2", titleNumber: 2 },
      ],
    }));
    installFetch(() => ({ status: 200, body: [] }));
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    expect(harness.current.activeId).toBe("term_1");

    harness.current.setActive("term_2");
    await flush();
    expect(harness.current.activeId).toBe("term_2");
    expect(harness.current.activeTab?.title).toBe("Terminal 2");
    expect(JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}").active).toBe("term_2");
    harness.unmount();
  });

  it("collapses concurrent create requests into one shell", async () => {
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
    // A double-click on the "+" must not start two shells.
    const [first, second] = await Promise.all([harness.current.create(), harness.current.create()]);
    expect(first).toBeUndefined();
    expect(second).toBeUndefined();
    await flush();
    expect(calls.filter(call => call.method === "POST")).toHaveLength(1);
    expect(harness.current.tabs).toHaveLength(1);
    harness.unmount();
  });

  it("adopts live sessions and picks up their exit codes", async () => {
    installFetch((call) => {
      if (call.method === "GET") {
        return {
          status: 200,
          body: [
            {
              id: "term_live",
              threadId: "thread-1",
              title: "",
              command: "/bin/bash",
              args: [],
              cwd: "/tmp",
              status: "running",
              exitCode: null,
              pid: 9,
              cols: 100,
              rows: 30,
            },
            {
              id: "term_done",
              threadId: "thread-1",
              title: "Build shell",
              command: "/bin/bash",
              args: [],
              cwd: "/tmp",
              status: "exited",
              exitCode: 130,
              pid: null,
              cols: 80,
              rows: 24,
            },
          ],
        };
      }
      return { status: 200, body: {} };
    });
    const harness = renderHook(() => useTerminalTabs("thread-1"));
    await flush();
    expect(harness.current.tabs.map(tab => tab.id)).toEqual(["term_live", "term_done"]);
    // A server session with no title gets the numbered default, and a width it
    // reported is remembered for the restore.
    expect(harness.current.tabs[0]).toMatchObject({ cols: 100, rows: 30, title: "Terminal 1", titleNumber: 1 });
    expect(harness.current.tabs[1]).toMatchObject({ exitCode: 130, title: "Build shell" });
    // The adopted list is persisted, so the next mount is warm.
    expect(JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}").all).toHaveLength(2);
    harness.unmount();
  });

  it("clears the stored tabs when the last tab closes even if the server call fails", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      if (call.method === "DELETE")
        throw new TypeError("Failed to fetch");
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
    expect(localStorage.getItem("future.terminal.tabs.v1.thread-1")).not.toBeNull();

    await harness.current.close("term_1");
    await flush();
    expect(harness.current.tabs).toHaveLength(0);
    expect(localStorage.getItem("future.terminal.tabs.v1.thread-1")).toBeNull();
    harness.unmount();
  });

  it("takes the new conversation's stored tabs on a switch and drops the old thread's late answer", async () => {
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_a",
      all: [{ id: "term_a", title: "Terminal 1", titleNumber: 1 }],
    }));
    localStorage.setItem("future.terminal.tabs.v1.thread-2", JSON.stringify({
      active: "term_b",
      all: [{ id: "term_b", title: "Terminal 2", titleNumber: 1 }],
    }));
    let releaseThreadOne!: (value: { body: unknown; status: number }) => void;
    const threadOneGate = new Promise<{ body: unknown; status: number }>((resolve) => {
      releaseThreadOne = resolve;
    });
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      calls.push({ body: undefined, method: "GET", url });
      if (url.includes("threadId=thread-1")) {
        const { body, status } = await threadOneGate;
        return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
      }
      return new Response(JSON.stringify([]), { status: 200, headers: { "content-type": "application/json" } });
    });
    vi.stubGlobal("fetch", fetchMock);

    const harness = mountHook(({ threadId }: { threadId: string }) => useTerminalTabs(threadId), { threadId: "thread-1" });
    expect(harness.current.tabs[0]?.id).toBe("term_a");
    expect(harness.current.ready).toBe(false);

    harness.setProps({ threadId: "thread-2" });
    await flush();
    expect(harness.current.tabs[0]?.id).toBe("term_b");
    expect(harness.current.ready).toBe(true);

    // The first thread's answer arrives late; it must not repopulate the panel.
    releaseThreadOne({ body: [{ id: "term_late", threadId: "thread-1", title: "Late", status: "running", exitCode: null, pid: 2, cols: 80, rows: 24 }], status: 200 });
    await flush();
    expect(harness.current.tabs.map(tab => tab.id)).toEqual(["term_b"]);
    harness.unmount();
  });

  it("does not write view state once there is no conversation", async () => {
    installFetch(() => ({ status: 200, body: [] }));
    const harness = renderHook(() => useTerminalTabs(null));
    await flush();
    expect(() => harness.current.save("term_1", { buffer: "nowhere" })).not.toThrow();
    expect(localStorage.length).toBe(0);
    harness.unmount();
  });

  it("swallows a load failure that arrives after the conversation changed", async () => {
    let failFirstList!: (error: unknown) => void;
    const firstList = new Promise<Response>((_resolve, reject) => {
      failFirstList = reject;
    });
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input).includes("threadId=thread-1"))
        return firstList;
      return new Response(JSON.stringify([]), { status: 200, headers: { "content-type": "application/json" } });
    });
    vi.stubGlobal("fetch", fetchMock);

    const harness = mountHook(({ threadId }: { threadId: string }) => useTerminalTabs(threadId), { threadId: "thread-1" });
    harness.setProps({ threadId: "thread-2" });
    await flush();
    expect(harness.current.ready).toBe(true);

    // The abandoned thread's read fails afterwards; its error must not appear
    // on the panel the user is now looking at.
    await act(async () => {
      failFirstList(new Error("listener down"));
      await Promise.resolve();
    });
    expect(harness.current.error).toBeNull();
    harness.unmount();
  });
});
