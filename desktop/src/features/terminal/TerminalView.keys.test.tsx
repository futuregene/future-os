import type { Root } from "react-dom/client";
/**
 * The terminal must hand the panel shortcut back to the window listener.
 *
 * Regression: xterm cancels every key it handles, so with the terminal focused
 * Ctrl+J never reached `useTerminalPanel` — the panel stayed open and the shell
 * received a line feed, which runs the line the user was typing. The view
 * installs a custom key handler that returns false for that one combination and
 * leaves every other key to xterm.
 *
 * The real xterm needs a GPU-backed canvas, which jsdom does not have, so the
 * module is mocked and the captured policy is exercised directly.
 */
// @vitest-environment jsdom
import type { TerminalTab } from "./tabs";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TerminalView } from "./TerminalView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Key policies xterm was handed, in mount order. */
const keyPolicies: Array<(event: KeyboardEvent) => boolean> = [];

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    rows = 24;
    cols = 80;
    buffer = { active: { viewportY: 0 } };
    loadAddon() {}
    open() {}
    write() {}
    focus() {}
    dispose() {}
    scrollToLine() {}
    onData() {
      return { dispose() {} };
    }

    onResize() {
      return { dispose() {} };
    }

    attachCustomKeyEventHandler(policy: (event: KeyboardEvent) => boolean) {
      keyPolicies.push(policy);
    }
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
// The ticket never resolves, so the view stops before opening a socket: this
// test is about the key policy, not the transport.
vi.mock("./client", () => ({
  TerminalApiError: class TerminalApiError extends Error {},
  connectTicket: () => new Promise<string>(() => {}),
  connectUrl: () => "ws://127.0.0.1:1/terminal/none",
  terminalServer: async () => ({ url: "http://127.0.0.1:1", token: "t", port: 1, maxSessions: 32 }),
  updateTerminal: async () => undefined,
}));

const TAB: TerminalTab = { id: "term-1", title: "Terminal 1", titleNumber: 1 };

let host: HTMLDivElement | undefined;
let root: Root | undefined;

beforeEach(() => {
  keyPolicies.length = 0;
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

function mount() {
  act(() => {
    root?.render(createElement(TerminalView, { tab: TAB, onExit: () => {}, onPersist: () => {} }));
  });
  return keyPolicies[keyPolicies.length - 1];
}

describe("terminal key policy", () => {
  it("releases the panel shortcut so the window listener can collapse the panel", () => {
    const policy = mount();
    expect(policy).toBeDefined();
    // jsdom reports a non-macOS platform, so this is the Ctrl convention.
    expect(policy?.(new KeyboardEvent("keydown", { key: "j", ctrlKey: true }))).toBe(false);
  });

  it("leaves ordinary terminal input to xterm", () => {
    const policy = mount();
    expect(policy?.(new KeyboardEvent("keydown", { key: "j" }))).toBe(true);
    expect(policy?.(new KeyboardEvent("keydown", { key: "c", ctrlKey: true }))).toBe(true);
    expect(policy?.(new KeyboardEvent("keydown", { key: "j", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(policy?.(new KeyboardEvent("keydown", { key: "j", ctrlKey: true, altKey: true }))).toBe(true);
  });
});
