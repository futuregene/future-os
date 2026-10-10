// @vitest-environment jsdom
import type { TerminalPanelController } from "./useTerminalPanel";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetTerminalServerCache } from "./client";
import { TerminalPanel } from "./TerminalPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const invokeCommand = vi.fn();
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

// The real view mounts a GPU-backed xterm; the panel's notice wiring is what is
// under test, so stand in a stub that can fire each view callback.
let throwOnRender = false;
vi.mock("./TerminalView", () => ({
  TerminalView: (props: {
    onConnect?: () => void;
    onDisconnected?: (reason: string) => void;
    onExit: (code: number | null) => void;
    onMissing?: () => void;
    onPersist: (patch: Record<string, unknown>) => void;
    tab: { id: string };
  }) => {
    if (throwOnRender)
      throw new Error("chunk failed to load");
    return createElement("div", { "data-testid": "terminal-view" }, [
      createElement("button", { "data-testid": "fire-connect", "key": "c", "onClick": () => props.onConnect?.(), "type": "button" }, "connect"),
      createElement("button", { "data-testid": "fire-disconnect", "key": "d", "onClick": () => props.onDisconnected?.("socket dropped"), "type": "button" }, "disconnect"),
      createElement("button", { "data-testid": "fire-exit", "key": "e", "onClick": () => props.onExit(130), "type": "button" }, "exit"),
      createElement("button", { "data-testid": "fire-missing", "key": "m", "onClick": () => props.onMissing?.(), "type": "button" }, "missing"),
      createElement("button", { "data-testid": "fire-persist", "key": "p", "onClick": () => props.onPersist({ buffer: "screen", cursor: 3 }), "type": "button" }, "persist"),
    ]);
  },
}));

const SERVER = { maxSessions: 32, port: 41234, token: "t".repeat(64), url: "http://127.0.0.1:41234" };

interface FetchCall {
  body: unknown;
  method: string;
  url: string;
}

let calls: FetchCall[] = [];

function installFetch(handler: (call: FetchCall) => { body: unknown; status: number }) {
  calls = [];
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: FetchCall = {
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
      method: init?.method ?? "GET",
      url: String(input),
    };
    calls.push(call);
    const { body, status } = handler(call);
    return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  }));
}

function running(id: string, overrides: Record<string, unknown> = {}) {
  return {
    args: [],
    cols: 80,
    command: "/bin/bash",
    cwd: "/tmp",
    exitCode: null,
    id,
    pid: 1,
    rows: 24,
    status: "running",
    threadId: "thread-1",
    title: `Terminal ${id.slice(-1)}`,
    ...overrides,
  };
}

function controller(overrides: Partial<TerminalPanelController> = {}): TerminalPanelController {
  return {
    enabled: true,
    height: 300,
    maxHeight: 600,
    open: true,
    setHeight: vi.fn(),
    setOpen: vi.fn(),
    shortcut: "Ctrl+J",
    toggle: vi.fn(),
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
  await new Promise(resolve => setTimeout(resolve, 0));
}

async function render(panel: TerminalPanelController) {
  await act(async () => {
    root.render(createElement(TerminalPanel, { panel, threadId: "thread-1" }));
  });
  await flush();
}

async function fire(testId: string) {
  await act(async () => {
    container.querySelector<HTMLButtonElement>(`[data-testid='${testId}']`)?.click();
  });
  await flush();
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

function tabButtons(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>("button[role='tab']")];
}

beforeEach(() => {
  localStorage.clear();
  invokeCommand.mockReset();
  invokeCommand.mockResolvedValue(SERVER);
  resetTerminalServerCache();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.unstubAllGlobals();
});

describe("terminalPanel notices", () => {
  it("shows the reconnecting badge after a drop and clears it on reconnect", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    await render(controller());
    expect(tabButtons()).toHaveLength(1);
    expect(container.textContent).not.toContain("Reconnecting…");

    await fire("fire-disconnect");
    expect(container.textContent).toContain("Reconnecting…");

    await fire("fire-connect");
    expect(container.textContent).not.toContain("Reconnecting…");
  });

  it("marks a session the server lost and restarts it from the notice", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { body: [], status: 200 };
      if (call.method === "DELETE")
        return { body: { removed: true }, status: 200 };
      return { body: running("term_2"), status: 200 };
    });
    await render(controller());
    await fire("fire-missing");

    expect(container.textContent).toContain("This terminal is no longer running");
    expect(tabButtons()[0]?.textContent).toContain("ended");
    const restart = buttonByText("Restart")!;
    await act(async () => {
      restart.click();
    });
    await flush();
    expect(calls.some(call => call.method === "DELETE")).toBe(true);
    expect(calls.some(call => call.method === "POST")).toBe(true);
  });

  it("does not claim a shell exited while it is still running", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    await render(controller());
    // A running tab gets no exit bar at all (and no stray "code undefined").
    expect(container.textContent).not.toContain("The shell exited");
    expect(container.textContent).not.toContain("undefined");

    // A tab the server no longer has shows the session-ended notice instead of
    // the exit bar, even when it carries an exit code from a previous life.
    await fire("fire-exit");
    expect(container.textContent).toContain("The shell exited with code 130.");
    await fire("fire-missing");
    expect(container.textContent).not.toContain("The shell exited");
    expect(container.textContent).toContain("This terminal is no longer running");
  });

  it("shows the exit bar after an exit and restarts the shell", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { body: [], status: 200 };
      if (call.method === "DELETE")
        return { body: { removed: true }, status: 200 };
      return { body: running("term_2"), status: 200 };
    });
    await render(controller());
    await fire("fire-exit");

    expect(container.textContent).toContain("The shell exited with code 130.");
    const restart = buttonByText("Restart")!;
    await act(async () => {
      restart.click();
    });
    await flush();
    expect(calls.some(call => call.method === "POST")).toBe(true);
  });

  it("persists the view state the mounted view reports", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    await render(controller());
    await fire("fire-persist");
    const stored = JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}");
    expect(stored.all[0]).toMatchObject({ buffer: "screen", cursor: 3 });
  });
});

describe("terminalPanel chrome", () => {
  it("switches the active tab and starts a new one from the strip", async () => {
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [
        { id: "term_1", title: "Terminal 1", titleNumber: 1 },
        { id: "term_2", title: "Terminal 2", titleNumber: 2 },
      ],
    }));
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_3"), status: 200 }));
    await render(controller());

    await act(async () => {
      tabButtons()[1]?.click();
    });
    await flush();
    expect(tabButtons()[1]?.getAttribute("aria-selected")).toBe("true");
    expect(JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}").active).toBe("term_2");

    await act(async () => {
      container.querySelector<HTMLButtonElement>("button[aria-label='New terminal']")?.click();
    });
    await flush();
    expect(calls.some(call => call.method === "POST")).toBe(true);
  });

  it("does not auto-create a shell while the panel is closed", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    await render(controller({ open: false }));
    expect(calls.some(call => call.method === "POST")).toBe(false);
    expect(container.textContent).toContain("No terminal open");
  });

  it("offers a new terminal from the empty state", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    await render(controller());
    // One tab exists from the auto-create; close it to reach the empty state.
    await act(async () => {
      container.querySelector<HTMLButtonElement>("button[aria-label^='Close Terminal']")?.click();
    });
    await flush();
    expect(container.textContent).toContain("No terminal open");
    expect(container.textContent).toContain("Run commands in this conversation's working directory.");

    const start = buttonByText("New terminal");
    expect(start).toBeTruthy();
    await act(async () => {
      start?.click();
    });
    await flush();
    expect(calls.filter(call => call.method === "POST").length).toBeGreaterThanOrEqual(2);
  });

  it("retries and dismisses a failed create from the error bar", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { body: [], status: 200 };
      return { body: { error: { code: "SPAWN_FAILED", message: "SPAWN_FAILED: nope" } }, status: 400 };
    });
    await render(controller());
    expect(container.textContent).toContain("Could not start the terminal");

    const retry = buttonByText("Retry")!;
    await act(async () => {
      retry.click();
    });
    await flush();
    expect(calls.filter(call => call.method === "POST").length).toBeGreaterThanOrEqual(2);

    const dismiss = buttonByText("Dismiss")!;
    await act(async () => {
      dismiss.click();
    });
    await flush();
    expect(container.textContent).not.toContain("Could not start the terminal");
  });

  it("reports an unreachable terminal server instead of an empty panel", async () => {
    installFetch(() => ({ body: { error: { code: "REQUEST_FAILED", message: "listener down" } }, status: 500 }));
    await render(controller());
    expect(container.textContent).toContain("Terminal server unavailable");
    expect(container.textContent).toContain("listener down");
    // The empty state is suppressed while the server is unreachable.
    expect(container.textContent).not.toContain("No terminal open");
  });

  it("collapses from the panel header", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    const panel = controller();
    await render(panel);
    await act(async () => {
      container.querySelector<HTMLButtonElement>("button[aria-label^='Hide terminal']")?.click();
    });
    expect(panel.setOpen).toHaveBeenCalledWith(false);
  });

  it("resizes while the separator is dragged and stops on pointer up", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    const panel = controller({ height: 300 });
    await render(panel);
    const separator = container.querySelector<HTMLDivElement>("[role='separator']")!;
    expect(separator.getAttribute("aria-orientation")).toBe("horizontal");

    await act(async () => {
      separator.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientY: 500 }));
    });
    await act(async () => {
      window.dispatchEvent(new PointerEvent("pointermove", { clientY: 460 }));
    });
    // Dragging up grows the panel by the pointer delta.
    expect(panel.setHeight).toHaveBeenLastCalledWith(340);

    await act(async () => {
      window.dispatchEvent(new PointerEvent("pointerup", {}));
    });
    await act(async () => {
      window.dispatchEvent(new PointerEvent("pointermove", { clientY: 100 }));
    });
    expect(panel.setHeight).toHaveBeenCalledTimes(1);
  });

  it("contains a crash inside the terminal view instead of blanking the panel", async () => {
    installFetch(call => (call.method === "GET" ? { body: [], status: 200 } : { body: running("term_1"), status: 200 }));
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      throwOnRender = true;
      await render(controller());
      expect(container.textContent).toContain("chunk failed to load");
      // The tab strip survives the crash, so the panel is still usable.
      expect(tabButtons()).toHaveLength(1);
      expect(container.querySelector("[data-testid='terminal-view']")).toBeNull();
      expect(consoleError).toHaveBeenCalledWith(
        "terminal view failed",
        expect.any(Error),
        expect.anything(),
      );
    }
    finally {
      throwOnRender = false;
      consoleError.mockRestore();
    }
  });
});
