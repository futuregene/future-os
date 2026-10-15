// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { UpdateRecoveryPanel } from "./UpdateRecoveryPanel";

const mocks = vi.hoisted(() => ({ status: {} as Record<string, unknown>, invoke: vi.fn() }));
vi.mock("../../integrations/tauri/useMaintenanceStatus", () => ({ useMaintenanceStatus: () => mocks.status }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: (...args: unknown[]) => mocks.invoke(...args) }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it.each(["installed", "failed-restored", null, "repair-required"])("offers restart on the stopped screen only when the result permits it: %s", async (outcome) => {
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  mocks.status = { maintenance: { update: {
    operationId: "update-fixture",
    stage: "finished",
    outcome,
    restartSafe: outcome === "installed" || outcome === "failed-restored",
    diagnostic: outcome === "repair-required" ? "Original files changed" : null,
  } }, maintenanceError: null };
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<UpdateRecoveryPanel />));
    const button = container.querySelector("button");
    if (outcome === "installed" || outcome === "failed-restored") {
      expect(button?.textContent).toBe("Restart FutureOS");
      await act(async () => button!.click());
      expect(mocks.invoke).toHaveBeenCalledWith("restart_after_app_update");
    }
    else {
      expect(button).toBeNull();
      expect(mocks.invoke).not.toHaveBeenCalled();
    }
  }
  finally {
    act(() => root.unmount());
  }
});
