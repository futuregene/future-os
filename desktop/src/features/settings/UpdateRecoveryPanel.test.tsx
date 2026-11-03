// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { UpdateRecoveryPanel } from "./UpdateRecoveryPanel";

const mocks = vi.hoisted(() => ({
  status: {} as Record<string, unknown>,
  invoke: vi.fn(),
  openExternalUrl: vi.fn<(url: string) => Promise<void>>(async () => {}),
}));
vi.mock("../../integrations/tauri/useMaintenanceStatus", () => ({ useMaintenanceStatus: () => mocks.status }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: (...args: unknown[]) => mocks.invoke(...args) }));
vi.mock("../../integrations/storage/files", () => ({
  openExternalUrl: (url: string) => mocks.openExternalUrl(url),
}));
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

describe("the manual download link", () => {
  // The app is stopped and its own updater cannot finish: the last thing this window
  // can offer is the installer. A failed check must not leave the reader without a link.
  function mountWith(outcome: string) {
    mocks.status = { maintenance: { update: {
      operationId: "update-fixture",
      stage: "finished",
      outcome,
      restartSafe: false,
      diagnostic: null,
    } }, maintenanceError: null };
    const container = document.createElement("div");
    const root = createRoot(container);
    return { container, root };
  }

  it("opens the update's own download page", async () => {
    mocks.invoke.mockReset().mockResolvedValue({ downloadUrl: "https://future-os.cn/update/1.2.3" });
    const { container, root } = mountWith("repair-required");
    try {
      await act(async () => root.render(<UpdateRecoveryPanel />));
      const link = [...container.querySelectorAll("button")]
        .find(item => item.textContent === "Download installer");
      expect(link, "the recovery link").not.toBeNull();

      await act(async () => link!.click());

      expect(mocks.invoke).toHaveBeenCalledWith("check_app_update");
      expect(mocks.openExternalUrl).toHaveBeenCalledWith("https://future-os.cn/update/1.2.3");
    }
    finally {
      act(() => root.unmount());
    }
  });

  it("falls back to the download page when the check itself fails", async () => {
    // The updater is the thing that broke; asking it where the installer is may fail
    // too, and the answer is then the published page rather than nothing.
    mocks.invoke.mockReset().mockRejectedValue(new Error("agent offline"));
    const { container, root } = mountWith("repair-required");
    try {
      await act(async () => root.render(<UpdateRecoveryPanel />));
      const link = [...container.querySelectorAll("button")]
        .find(item => item.textContent === "Download installer")!;

      await act(async () => link.click());

      expect(mocks.openExternalUrl).toHaveBeenCalledWith("https://future-os.cn/");
    }
    finally {
      act(() => root.unmount());
    }
  });

  it("says so when the browser refuses to open", async () => {
    mocks.invoke.mockReset().mockResolvedValue(null);
    mocks.openExternalUrl.mockRejectedValueOnce(new Error("no handler"));
    const { container, root } = mountWith("repair-required");
    try {
      await act(async () => root.render(<UpdateRecoveryPanel />));
      const link = [...container.querySelectorAll("button")]
        .find(item => item.textContent === "Download installer")!;

      await act(async () => link.click());

      expect(container.textContent).toContain("The download page couldn’t be opened");
    }
    finally {
      act(() => root.unmount());
    }
  });
});
