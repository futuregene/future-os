// @vitest-environment jsdom
import type { StoredApprovalRequest } from "../../integrations/storage/types";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { saveApprovalRule, saveApprovalRules } from "../../integrations/storage/runs";
import { ApprovalPrompt } from "./ApprovalPrompt";

vi.mock("../../integrations/storage/runs", () => ({
  saveApprovalRule: vi.fn(async () => {}),
  saveApprovalRules: vi.fn(async () => {}),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function approval(saveSuggestion: object): StoredApprovalRequest {
  return { id: "a1", threadId: "t1", kind: "file_write", status: "pending", title: "Write", createdAt: 0, updatedAt: 0, reviewer: "", decisionScope: "", decisionSource: "", saveSuggestion: JSON.stringify(saveSuggestion) };
}

function button(container: HTMLElement, label: string) {
  const result = [...container.querySelectorAll("button")].find(item => item.textContent?.includes(label));
  expect(result, label).toBeTruthy();
  return result!;
}

it.each([false, true])("releases the successful save-rule decision while still mounted (capability=%s)", async (capability) => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onDecision = vi.fn(async () => {});
  const suggestion = capability ? { rules: [{ path: "/tmp/test", access: "write" }] } : { path: "/tmp/test", access: "write" };
  try {
    await act(async () => root.render(<ApprovalPrompt approval={approval(suggestion)} onDecision={onDecision} />));
    await act(async () => button(container, "Allow in this workspace").click());
    if (!capability)
      await act(async () => button(container, "Save & allow").click());
    expect(onDecision).toHaveBeenCalledTimes(1);
    expect(capability ? saveApprovalRules : saveApprovalRule).toHaveBeenCalled();
    expect(button(container, "Deny").disabled).toBe(false);
    expect(container.textContent).not.toContain("Allowing");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});

it("escape closes a focused rule editor before a second Escape rejects", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onDecision = vi.fn(async () => {});
  try {
    await act(async () => root.render(<ApprovalPrompt approval={approval({ path: "/tmp/test", access: "write" })} onDecision={onDecision} />));
    act(() => button(container, "Allow in this workspace").click());
    const input = container.querySelector("input")!;
    expect(document.activeElement).toBe(input);
    act(() => input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(container.querySelector("input")).toBeNull();
    expect(onDecision).not.toHaveBeenCalled();
    await act(async () => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(onDecision).toHaveBeenCalledWith(expect.anything(), "rejected");
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
