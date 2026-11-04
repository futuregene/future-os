// @vitest-environment jsdom
import type { TerminalPanelController } from "./useTerminalPanel";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RemoteTerminalPanel } from "./TerminalPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The remote panel is the *same* panel the local one is: same tab controller,
 * same notices, same view. What is pinned here is the part that would be
 * invisible if it broke — that every call goes to the conversation's host, with
 * the session id the wire names it by, and that the panel never presents itself
 * as this machine's.
 */

const hoisted = vi.hoisted(() => ({ request: vi.fn() }));

vi.mock("../remote-peer/remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => hoisted.request(...args),
}));

// The real view needs a GPU-backed xterm; the panel's own behaviour is what is
// under test here.
vi.mock("./TerminalView", () => ({
  TerminalView: (props: { tab: { id: string } }) => createElement("div", { "data-testid": "terminal-view" }, props.tab.id),
}));

const request = hoisted.request;

function controller(): TerminalPanelController {
  return {
    open: true,
    height: 300,
    maxHeight: 600,
    enabled: true,
    toggle: vi.fn(),
    setOpen: vi.fn(),
    setHeight: vi.fn(),
    shortcut: "Ctrl+J",
  };
}

function running(id: string, title: string) {
  return {
    id,
    threadId: "thread-1",
    title,
    cwd: "/srv/app",
    status: "running",
    exitCode: null,
    cols: 80,
    rows: 24,
  };
}

async function flush() {
  await new Promise(resolve => setTimeout(resolve, 0));
  await new Promise(resolve => setTimeout(resolve, 0));
}

describe("remoteTerminalPanel", () => {
  let host: HTMLDivElement;

  beforeEach(() => {
    request.mockReset();
    localStorage.clear();
    host = document.createElement("div");
    document.body.append(host);
  });

  afterEach(() => {
    host.remove();
    document.body.innerHTML = "";
  });

  it("lists the conversation's shells on its host and draws them", async () => {
    request.mockImplementation(async (_desktopId: string, command: { type: string }) => {
      if (command.type === "terminal_list")
        return { terminals: [running("term-b1", "Build")] };
      return {};
    });

    const root = createRoot(host);
    await act(async () => {
      root.render(createElement(RemoteTerminalPanel, {
        desktopId: "desk-b",
        sessionId: "sess-1",
        hostLabel: "Mini",
        panel: controller(),
      }));
    });
    await act(flush);

    const list = request.mock.calls.find(call => call[1].type === "terminal_list");
    expect(list?.[0]).toBe("desk-b");
    // The wire names the conversation by its session, not by that host's thread
    // id: the client never learns which thread a session is.
    expect(list?.[1]).toMatchObject({ sessionId: "sess-1" });
    expect(host.querySelector("[data-testid=\"terminal-view\"]")?.textContent).toBe("term-b1");
    // The panel says whose machine it is, so a shell is never mistaken for this
    // one's.
    expect(host.textContent).toContain("Mini");

    await act(async () => root.unmount());
  });

  it("keeps its tab state apart from this machine's conversation with the same id", async () => {
    request.mockImplementation(async (_desktopId: string, command: { type: string }) => {
      if (command.type === "terminal_list")
        return { terminals: [running("term-b1", "Build")] };
      return {};
    });
    // A local conversation whose id happens to equal the remote session's.
    localStorage.setItem("future.terminal.tabs.v1.sess-1", JSON.stringify({
      active: "term-local",
      all: [{ id: "term-local", title: "Mine", titleNumber: 1 }],
    }));

    const root = createRoot(host);
    await act(async () => {
      root.render(createElement(RemoteTerminalPanel, {
        desktopId: "desk-b",
        sessionId: "sess-1",
        hostLabel: "Mini",
        panel: controller(),
      }));
    });
    await act(flush);

    // The remote tab list is its own: an existing local list is not adopted.
    expect(host.textContent).toContain("Build");
    expect(host.textContent).not.toContain("Mine");

    await act(async () => root.unmount());
  });

  it("creates a shell on the host, never locally", async () => {
    request.mockImplementation(async (_desktopId: string, command: { type: string }) => {
      if (command.type === "terminal_list")
        return { terminals: [] };
      if (command.type === "terminal_create")
        return { terminal: running("term-new", "Terminal 1") };
      return {};
    });

    const root = createRoot(host);
    await act(async () => {
      root.render(createElement(RemoteTerminalPanel, {
        desktopId: "desk-b",
        sessionId: "sess-1",
        hostLabel: "Mini",
        panel: controller(),
      }));
    });
    await act(flush);

    // Opening the panel with no tabs starts one, exactly as it does locally.
    const created = request.mock.calls.find(call => call[1].type === "terminal_create");
    expect(created?.[0]).toBe("desk-b");
    expect(created?.[1]).toMatchObject({ sessionId: "sess-1" });
    expect(host.querySelector("[data-testid=\"terminal-view\"]")?.textContent).toBe("term-new");

    await act(async () => root.unmount());
  });

  it("closes a tab on the host", async () => {
    request.mockImplementation(async (_desktopId: string, command: { type: string }) => {
      if (command.type === "terminal_list")
        return { terminals: [running("term-b1", "Build")] };
      return {};
    });

    const root = createRoot(host);
    await act(async () => {
      root.render(createElement(RemoteTerminalPanel, {
        desktopId: "desk-b",
        sessionId: "sess-1",
        hostLabel: "Mini",
        panel: controller(),
      }));
    });
    await act(flush);

    const close = host.querySelector("button[aria-label=\"Close Build\"]") as HTMLButtonElement | null;
    expect(close).not.toBeNull();
    await act(async () => {
      close!.click();
    });
    await act(flush);

    const removed = request.mock.calls.find(call => call[1].type === "terminal_delete");
    expect(removed?.[1]).toMatchObject({ terminalId: "term-b1" });

    await act(async () => root.unmount());
  });
});
