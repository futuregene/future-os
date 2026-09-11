// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { Composer } from "./Composer";

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(async (command: string) => command === "list_agent_providers" ? { builtin: [], custom: [] } : []),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("reports an image paste read failure instead of silently dropping the image", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Composer onSend={vi.fn()} modelOptions={[]} />));
    const file = new File(["image"], "clipboard.png", { type: "image/png" });
    file.arrayBuffer = async () => {
      throw new Error("disk unavailable");
    };
    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", { value: {
      getData: () => "",
      items: [{ kind: "file", getAsFile: () => file }],
    } });
    await act(async () => container.querySelector("[role=textbox]")!.dispatchEvent(paste));
    expect(container.textContent).toContain("clipboard.png");
    expect(container.textContent).toContain("read");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
