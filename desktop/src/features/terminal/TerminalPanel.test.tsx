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
    open: true,
    height: 300,
    maxHeight: 600,
    enabled: true,
    toggle: vi.fn(),
    setOpen: vi.fn(),
    setHeight: vi.fn(),
    shortcut: "Ctrl+J",
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

  it("offers the explicit home fallback for a bad working directory", async () => {
    installFetch((call) => {
      if (call.method === "GET")
        return { status: 200, body: [] };
      return {
        status: 400,
        body: { error: { code: "CWD_INVALID", message: "CWD_INVALID: /nope does not exist", allowsHomeFallback: true } },
      };
    });
    await render(controller());
    const fallback = [...container.querySelectorAll("button")].find(button => button.textContent?.includes("home"));
    expect(fallback).toBeTruthy();
    await act(async () => {
      fallback?.click();
    });
    await flush();
    const attempts = calls.filter(call => call.method === "POST");
    expect(attempts).toHaveLength(2);
    expect(attempts[1]?.body).toMatchObject({ cwdPolicy: "homeConfirmed" });
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
});
