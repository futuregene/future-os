// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { UpdateRecoveryPanel } from "./UpdateRecoveryPanel";

const mocks = vi.hoisted(() => ({ status: {} as Record<string, unknown>, invoke: vi.fn() }));
vi.mock("../../integrations/tauri/useMaintenanceStatus", () => ({ useMaintenanceStatus: () => mocks.status }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: (...args: unknown[]) => mocks.invoke(...args) }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it.each(["installed", "pending-validation", "failed-restored", "not-installed", null, "repair-required"])("offers restart on the stopped screen only when the result permits it: %s", async (outcome) => {
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.status = { maintenance: { update: {
    operationId: "update-fixture",
    stage: "finished",
    outcome,
    restartSafe: ["installed", "pending-validation", "failed-restored", "not-installed"].includes(outcome ?? ""),
    diagnostic: outcome === "repair-required" ? "Original files changed" : null,
  } }, maintenanceError: null };
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<UpdateRecoveryPanel />));
    const button = [...container.querySelectorAll("button")].find(item => item.textContent === "Restart FutureOS");
    if (["installed", "pending-validation", "failed-restored", "not-installed"].includes(outcome ?? "")) {
      expect(button?.textContent).toBe("Restart FutureOS");
      await act(async () => button!.click());
      expect(mocks.invoke).toHaveBeenCalledWith("restart_after_app_update");
    }
    else {
      expect(button).toBeUndefined();
      expect(container.textContent).not.toContain("Original files changed");
      expect(container.textContent).not.toContain("update-fixture");
      if (outcome === "repair-required")
        expect(container.textContent).toContain("Download the installer, reinstall FutureOS, then open it again.");
      expect(mocks.invoke).not.toHaveBeenCalled();
    }
  }
  finally {
    act(() => root.unmount());
  }
});

it("surfaces a failed restart instead of leaving the button silently inert", async () => {
  // The restart is the last thing this window can do, and it can fail (the
  // supervisor is gone, the command is refused). Saying so is what keeps the
  // user from pressing a button that appears to do nothing.
  mocks.invoke.mockReset().mockRejectedValue(new Error("supervisor unavailable"));
  mocks.status = { maintenance: { update: {
    operationId: "update-fixture",
    stage: "finished",
    outcome: "installed",
    restartSafe: true,
    diagnostic: null,
  } }, maintenanceError: null };
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<UpdateRecoveryPanel />));
    const button = container.querySelector("button")!;
    expect(button.textContent).toBe("Restart FutureOS");

    await act(async () => button.click());

    expect(container.querySelector("[role='alert']")?.textContent).toContain("Couldn’t restart automatically. Close and reopen FutureOS.");
    expect(container.textContent).not.toContain("supervisor unavailable");
    // The button comes back: a failed restart is retryable.
    expect(container.querySelector("button")!.disabled).toBe(false);
  }
  finally {
    act(() => root.unmount());
  }
});
