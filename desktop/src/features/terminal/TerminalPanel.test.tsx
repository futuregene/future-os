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

// The real view needs a GPU-backed xterm; the panel's own behaviour is what is
// under test here.
vi.mock("./TerminalView", () => ({
  TerminalView: (props: { tab: { id: string } }) => createElement("div", { "data-testid": "terminal-view" }, props.tab.id),
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
  vi.stubGlobal("fetch", vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: FetchCall = {
      url: String(input),
      method: init?.method ?? "GET",
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    };
    calls.push(call);
    const { status, body } = handler(call);
    return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  }));
}

function controller(overrides: Partial<TerminalPanelController> = {}): TerminalPanelController {
  return {
    canDockRight: true,
    dock: "bottom",
    enabled: true,
    height: 300,
    maxHeight: 600,
    maximized: false,
    maxWidth: 900,
    open: true,
    setDock: vi.fn(),
    setHeight: vi.fn(),
    setOpen: vi.fn(),
    setWidth: vi.fn(),
    shortcut: "Ctrl+J",
    toggle: vi.fn(),
    toggleDock: vi.fn(),
    toggleMaximized: vi.fn(),
    width: 420,
    ...overrides,
  };
}

function running(id: string, title: string) {
  return {
    id,
    threadId: "thread-1",
    title,
    command: "/bin/bash",
    args: [],
    cwd: "/tmp",
    status: "running",
    exitCode: null,
    pid: 1,
    cols: 80,
    rows: 24,
  };
}

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
  await new Promise(resolve => setTimeout(resolve, 0));
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

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

async function render(panel: TerminalPanelController) {
  await act(async () => {
    root.render(createElement(TerminalPanel, { panel, threadId: "thread-1" }));
  });
  await flush();
}

describe("terminalPanel", () => {
  it("creates the first tab when it opens with none", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    await render(controller());
    const created = calls.filter(call => call.method === "POST");
    expect(created).toHaveLength(1);
    expect(created[0]?.body).toMatchObject({ threadId: "thread-1" });
    expect(container.querySelectorAll("[role=\"tab\"]")).toHaveLength(1);
    expect(container.querySelector("[data-testid=\"terminal-view\"]")).not.toBeNull();
  });

  it("shows the exit bar and restarts an exited shell", async () => {
    installFetch((call) => {
      if (call.method === "GET") {
        return {
          status: 200,
          body: [{ ...running("term_1", "Terminal 1"), status: "exited", exitCode: 130, pid: null }],
        };
      }
      if (call.method === "DELETE")
        return { status: 200, body: { removed: true } };
      return { status: 200, body: running("term_2", "Terminal 1") };
    });
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1 }],
    }));
    await render(controller());
    expect(container.textContent).toContain("130");
    const restart = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Restart"));
    expect(restart).toBeTruthy();
    await act(async () => {
      restart?.click();
    });
    await flush();
    expect(calls.some(call => call.method === "DELETE")).toBe(true);
    expect(calls.some(call => call.method === "POST")).toBe(true);
  });

  it("shows a create failure with a retry and no home-fallback shortcut", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 400,
        body: { error: { code: "SPAWN_FAILED", message: "SPAWN_FAILED: nope" } },
      };
    });
    await render(controller());
    expect(container.textContent).toContain("SPAWN_FAILED");
    const buttons = [...container.querySelectorAll("button")].map(button => button.textContent ?? "");
    expect(buttons.some(text => text.includes("Retry"))).toBe(true);
    expect(buttons.some(text => text.toLowerCase().includes("home"))).toBe(false);
  });

  it("marks a session the server forgot and offers a restart", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 200,
        body: running("term_2", "Terminal 1"),
      };
    });
    localStorage.setItem("future.terminal.tabs.v1.thread-1", JSON.stringify({
      active: "term_1",
      all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1, buffer: "final screen" }],
    }));
    await render(controller());
    expect(container.textContent).toContain("no longer running");
    const restart = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("Restart"));
    expect(restart).toBeTruthy();
  });

  it("closing a tab asks the server to stop the shell", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      if (call.method === "DELETE")
        return { status: 200, body: { removed: true } };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    await render(controller());
    const close = container.querySelector<HTMLButtonElement>("button[aria-label^=\"Close\"]");
    expect(close).not.toBeNull();
    await act(async () => {
      close?.click();
    });
    await flush();
    expect(calls.some(call => call.method === "DELETE" && call.url.endsWith("/terminal/term_1"))).toBe(true);
    expect(container.querySelectorAll("[role=\"tab\"]")).toHaveLength(0);
  });

  it("collapses the panel from the toolbar", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    const panel = controller();
    await render(panel);
    const collapse = container.querySelector<HTMLButtonElement>("button[aria-label^=\"Hide terminal\"]");
    expect(collapse).not.toBeNull();
    await act(async () => {
      collapse?.click();
    });
    expect(panel.setOpen).toHaveBeenCalledWith(false);
  });

  it("docks to the right as a column of the panel's own width", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    await render(controller({ dock: "right", width: 512 }));
    const section = container.querySelector<HTMLElement>("[data-component=\"terminal-panel\"]")!;
    expect(section.getAttribute("data-dock")).toBe("right");
    // The size follows the edge it is attached to: a column is as wide as the
    // reader dragged it and as tall as the row, not the other way round.
    expect(section.style.width).toBe("512px");
    expect(section.style.height).toBe("");
    // And so does the grip: a vertical edge is dragged sideways.
    const separator = container.querySelector("[role=\"separator\"]")!;
    expect(separator.getAttribute("aria-orientation")).toBe("vertical");
  });

  it("resizes a right docked panel by dragging its left edge", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    const panel = controller({ dock: "right", width: 400 });
    await render(panel);
    const separator = container.querySelector("[role=\"separator\"]")!;
    // Dragging the left edge to the left makes the column wider.
    await act(async () => {
      separator.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, cancelable: true, clientX: 900 }));
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 820 }));
      window.dispatchEvent(new MouseEvent("pointerup"));
    });
    expect(panel.setWidth).toHaveBeenCalledWith(480);
    expect(panel.setHeight).not.toHaveBeenCalled();
  });

  it("moves between the bottom edge and the right edge from the header", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    const panel = controller();
    await render(panel);
    // Docked at the bottom, the header offers the right edge.
    const toRight = container.querySelector<HTMLButtonElement>("button[aria-label=\"Dock terminal to the right\"]");
    expect(toRight).not.toBeNull();
    await act(async () => {
      toRight?.click();
    });
    expect(panel.toggleDock).toHaveBeenCalledTimes(1);

    // Docked right, it offers the bottom again — one button, two directions.
    await render(controller({ dock: "right", toggleDock: panel.toggleDock }));
    const toBottom = container.querySelector<HTMLButtonElement>("button[aria-label=\"Dock terminal to the bottom\"]");
    expect(toBottom).not.toBeNull();
    await act(async () => {
      toBottom?.click();
    });
    expect(panel.toggleDock).toHaveBeenCalledTimes(2);
    expect(container.querySelector("button[aria-label=\"Dock terminal to the right\"]")).toBeNull();
  });

  it("offers no dock choice while the window cannot host a column", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    await render(controller({ canDockRight: false }));
    // The action is gone rather than inert: "dock right" here would ask for a
    // layout the window cannot host.
    expect(container.querySelector("button[aria-label=\"Dock terminal to the right\"]")).toBeNull();
    expect(container.querySelector("button[aria-label=\"Maximize terminal\"]")).not.toBeNull();
  });

  it("maximizes into the whole content area, with no docked size and no grip", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return { status: 200, body: running("term_1", "Terminal 1") };
    });
    const panel = controller({ maximized: true });
    await render(panel);
    const section = container.querySelector<HTMLElement>("[data-component=\"terminal-panel\"]")!;
    expect(section.getAttribute("data-dock")).toBe("maximized");
    // Neither axis is set: the shell's flex layout gives it the content area.
    expect(section.style.height).toBe("");
    expect(section.style.width).toBe("");
    expect(container.querySelector("[role=\"separator\"]")).toBeNull();
    // The dock choice belongs to the docked layout; there is nothing to dock
    // into while the panel owns the content area.
    expect(container.querySelector("button[aria-label^=\"Dock terminal\"]")).toBeNull();

    const restore = container.querySelector<HTMLButtonElement>("button[aria-label=\"Restore terminal size\"]");
    expect(restore).not.toBeNull();
    await act(async () => {
      restore?.click();
    });
    expect(panel.toggleMaximized).toHaveBeenCalledTimes(1);
  });

  it("hides rather than closes: the shell survives the collapse and the next open re-attaches", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [running("term_1", "Terminal 1")] };
      if (call.method === "DELETE")
        return { status: 200, body: { removed: true } };
      return { status: 200, body: running("term_2", "Terminal 1") };
    });
    const panel = controller();
    await render(panel);
    expect(container.querySelectorAll("[role=\"tab\"]")).toHaveLength(1);

    // Collapsing takes the panel out of the layout, so `AppShell` unmounts it.
    await act(async () => {
      root.render(createElement("div"));
    });
    await flush();

    // Not one stopping request — the shells, their tabs and their stored
    // screens all outlive the panel.
    expect(calls.some(call => call.method === "DELETE")).toBe(false);
    expect(calls.some(call => call.method === "POST")).toBe(false);
    expect(JSON.parse(localStorage.getItem("future.terminal.tabs.v1.thread-1") ?? "{}").all).toHaveLength(1);

    // Reopening re-attaches to the same session instead of starting a new one.
    await render(panel);
    expect(container.querySelectorAll("[role=\"tab\"]")).toHaveLength(1);
    expect(calls.some(call => call.method === "POST")).toBe(false);
    expect(calls.some(call => call.method === "DELETE")).toBe(false);
  });
});
