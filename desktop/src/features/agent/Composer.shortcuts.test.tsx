// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { installShortcuts } from "../../test/installShortcuts";
import { resetAllCommandBindings } from "../keyboard/shortcuts";
import { Composer } from "./Composer";

const dialog = vi.hoisted(() => ({ open: vi.fn(async () => null) }));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: dialog.open }));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let uninstall: () => void;
let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;

beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  resetAllCommandBindings();
  uninstall = installShortcuts();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  uninstall();
});

/** `⌘⌥A` — the paperclip, from the keyboard. */
function pressAttach() {
  const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, altKey: true, code: "KeyA", ctrlKey: true, key: "a" });
  act(() => {
    window.dispatchEvent(event);
  });
  return event;
}

it("attaches files from the keyboard", async () => {
  act(() => root.render(<Composer modelOptions={[]} onSend={() => {}} />));
  await act(async () => {
    await Promise.resolve();
  });
  const event = pressAttach();
  expect(event.defaultPrevented).toBe(true);
  await act(async () => {
    await Promise.resolve();
  });
  expect(dialog.open).toHaveBeenCalledTimes(1);
});

it("leaves the key unclaimed while the composer is disabled", async () => {
  // A picker that cannot add anything to a locked box would be a key that does
  // nothing; the press stays available instead.
  act(() => root.render(<Composer disabled modelOptions={[]} onSend={() => {}} />));
  await act(async () => {
    await Promise.resolve();
  });
  const event = pressAttach();
  expect(event.defaultPrevented).toBe(false);
  expect(dialog.open).not.toHaveBeenCalled();
});
