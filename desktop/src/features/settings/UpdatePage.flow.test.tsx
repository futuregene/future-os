// @vitest-environment jsdom
import type { UpdateStatus } from "../../components/layout/hooks/useUpdateChecker";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UpdatePage } from "./UpdatePage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  buildVersion: "2.0.0" as string | null,
  invokeCommand: vi.fn(),
  maintenance: vi.fn(),
  listen: vi.fn(),
  openExternalUrl: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: (...args: unknown[]) => mocks.listen(...args) }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: (...args: unknown[]) => args[0] === "app_maintenance_status" ? mocks.maintenance() : mocks.invokeCommand(...args) }));
vi.mock("../../integrations/storage/files", () => ({ openExternalUrl: (...args: unknown[]) => mocks.openExternalUrl(...args) }));
vi.mock("../../integrations/tauri/useBuildInfo", () => ({
  useBuildInfo: () => ({ data: mocks.buildVersion ? { version: mocks.buildVersion, isRelease: true } : null }),
}));

const UP_TO_DATE: UpdateStatus = {
  currentVersion: "2.0.0",
  latestVersion: "2.0.0",
  hasUpdate: false,
  platformSupported: true,
  canInstallInApp: true,
  downloadUrl: null,
};

const UPDATE: UpdateStatus = {
  currentVersion: "2.0.0",
  latestVersion: "2.1.0",
  hasUpdate: true,
  platformSupported: true,
  canInstallInApp: true,
  downloadUrl: null,
};

const MANUAL: UpdateStatus = { ...UPDATE, canInstallInApp: false, downloadUrl: "https://downloads.example.com/app.dmg" };
const NO_ASSET: UpdateStatus = { ...UPDATE, canInstallInApp: false, platformSupported: false, downloadUrl: null };

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];
/** Progress payloads the page subscribed to, in subscription order. */
let progressHandlers: ((event: { payload: { downloaded: number; total: number } }) => void)[] = [];
let unlisteners: ReturnType<typeof vi.fn>[] = [];

beforeEach(() => {
  progressHandlers = [];
  unlisteners = [];
  mocks.buildVersion = "2.0.0";
  mocks.maintenance.mockReset();
  mocks.maintenance.mockResolvedValue({ cli: {}, update: {
    operationId: "update-fixture",
    fromVersion: "2.0.0",
    targetVersion: "2.1.0",
    stage: "finished",
    outcome: "installed",
    diagnostic: null,
    restartSafe: true,
    repairAcknowledged: false,
  } });
  mocks.invokeCommand.mockReset();
  mocks.invokeCommand.mockResolvedValue(undefined);
  mocks.openExternalUrl.mockReset();
  mocks.openExternalUrl.mockResolvedValue(undefined);
  mocks.listen.mockReset();
  mocks.listen.mockImplementation(async (event: string, handler: (payload: never) => void) => {
    expect(event).toBe("app-update-progress");
    progressHandlers.push(handler as never);
    const unlisten = vi.fn();
    unlisteners.push(unlisten);
    return unlisten;
  });
});

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

async function renderPage(cachedStatus: UpdateStatus | null = null) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(<UpdatePage cachedStatus={cachedStatus} />);
  });
  return container;
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent === text) as HTMLButtonElement | undefined;
}

function anyButton(container: HTMLElement) {
  return [...container.querySelectorAll("button")][0]!;
}

function emitProgress(downloaded: number, total: number) {
  act(() => {
    for (const handler of progressHandlers)
      handler({ payload: { downloaded, total } });
  });
}

describe("updatePage version and check", () => {
  it("shows the injected build version, falling back to the section title", async () => {
    const container = await renderPage();
    expect(container.textContent).toContain("Current version v2.0.0");

    mocks.buildVersion = null;
    const withoutBuild = await renderPage();
    expect(withoutBuild.textContent).toContain("Check for updates");
    expect(withoutBuild.textContent).not.toContain("Current version");
  });

  it("keeps the check button enabled until a check starts, then reports progress", async () => {
    let resolveCheck!: (value: UpdateStatus) => void;
    mocks.invokeCommand.mockImplementation(() => new Promise<UpdateStatus>((resolve) => {
      resolveCheck = resolve;
    }));
    const container = await renderPage();

    expect(anyButton(container).disabled).toBe(false);
    await act(async () => {
      button(container, "Check for updates")!.click();
    });
    expect(mocks.invokeCommand).toHaveBeenCalledWith("check_app_update");
    expect(anyButton(container).disabled).toBe(true);
    expect(button(container, "Checking…")).toBeTruthy();

    await act(async () => {
      resolveCheck(UP_TO_DATE);
    });
    expect(anyButton(container).disabled).toBe(false);
    expect(container.textContent).toContain("Check for updates");
    expect(container.textContent).toContain("You're on the latest version");
  });

  it("reports a failed check as an Error and clears the stale status", async () => {
    const container = await renderPage(UPDATE);
    expect(container.textContent).toContain("New version v2.1.0 available");

    mocks.invokeCommand.mockRejectedValueOnce(new Error("offline"));
    await act(async () => {
      button(container, "Check for updates")!.click();
    });

    expect(container.textContent).toContain("Couldn’t check for updates. Check your connection and try again.");
    // The stale "available" banner is gone: a failed check must not claim one.
    expect(container.textContent).not.toContain("New version");
  });

  it("stringifies a non-Error check failure", async () => {
    const container = await renderPage();

    mocks.invokeCommand.mockRejectedValueOnce("socket reset");
    await act(async () => {
      button(container, "Check for updates")!.click();
    });

    expect(container.textContent).toContain("Couldn’t check for updates. Check your connection and try again.");
  });

  it("clears a previous error when a new check starts", async () => {
    const container = await renderPage();
    mocks.invokeCommand.mockRejectedValueOnce(new Error("offline"));
    await act(async () => {
      button(container, "Check for updates")!.click();
    });
    expect(container.textContent).toContain("Couldn’t check for updates");
    expect(container.textContent).not.toContain("offline");

    mocks.invokeCommand.mockResolvedValueOnce(UP_TO_DATE);
    await act(async () => {
      button(container, "Check for updates")!.click();
    });
    expect(container.textContent).not.toContain("offline");
  });

  it("renders the cached status without a network round-trip", async () => {
    const container = await renderPage(UPDATE);

    expect(container.textContent).toContain("New version v2.1.0 available");
    expect(mocks.invokeCommand).not.toHaveBeenCalled();
  });
});

describe("updatePage in-app install", () => {
  it("subscribes before installing, streams progress and offers a restart", async () => {
    const container = await renderPage(UPDATE);

    await act(async () => {
      button(container, "Download and install")!.click();
    });

    expect(mocks.listen).toHaveBeenCalledTimes(1);
    expect(mocks.invokeCommand).toHaveBeenCalledWith("install_app_update", { language: "en" });
    // The subscription is torn down once the install command settles.
    expect(unlisteners[0]).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Update installed. Restart to finish.");
    expect(button(container, "Restart FutureOS")).toBeTruthy();
    expect(button(container, "Download and install")).toBeUndefined();
  });

  it("shows a progress bar driven by the streamed payload", async () => {
    let resolveInstall!: () => void;
    mocks.invokeCommand.mockImplementation((command: string) => {
      if (command === "install_app_update") {
        return new Promise<void>((resolve) => {
          resolveInstall = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    const container = await renderPage(UPDATE);

    await act(async () => {
      button(container, "Download and install")!.click();
    });
    expect(container.textContent).toContain("Downloading 0%");

    emitProgress(25, 100);
    expect(container.textContent).toContain("Downloading 25%");
    expect(container.querySelector<HTMLElement>("div[style*='width']")!.style.width).toBe("25%");

    // Rounds to the nearest percent.
    emitProgress(1, 8);
    expect(container.textContent).toContain("Downloading 13%");

    // Clamped at 100 even if the backend overshoots.
    emitProgress(200, 100);
    expect(container.textContent).toContain("Downloading 100%");

    await act(async () => {
      resolveInstall();
    });
  });

  it("ignores a zero-total progress frame (nothing to divide by)", async () => {
    let resolveInstall!: () => void;
    mocks.invokeCommand.mockImplementation((command: string) => {
      if (command === "install_app_update") {
        return new Promise<void>((resolve) => {
          resolveInstall = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    const container = await renderPage(UPDATE);

    await act(async () => {
      button(container, "Download and install")!.click();
    });
    emitProgress(0, 0);
    expect(container.textContent).toContain("Downloading 0%");

    emitProgress(5, 0);
    expect(container.textContent).toContain("Downloading 0%");

    await act(async () => {
      resolveInstall();
    });
  });

  it("reports an install failure and re-enables the install button", async () => {
    mocks.invokeCommand.mockImplementation((command: string) => (command === "install_app_update"
      ? Promise.reject(new Error("signature mismatch"))
      : Promise.resolve(undefined)));
    const container = await renderPage(UPDATE);

    await act(async () => {
      button(container, "Download and install")!.click();
    });

    expect(container.textContent).toContain("The update couldn’t be completed. Please try again later, or download the installer.");
    expect(button(container, "Download and install")!.disabled).toBe(false);
    expect(unlisteners[0]).toHaveBeenCalledTimes(1);
  });

  it("stringifies a non-Error install failure", async () => {
    mocks.invokeCommand.mockImplementation((command: string) => (command === "install_app_update"
      // eslint-disable-next-line prefer-promise-reject-errors -- the rejection is deliberately a non-Error: this test pins the stringified failure.
      ? Promise.reject("permission denied")
      : Promise.resolve(undefined)));
    const container = await renderPage(UPDATE);

    await act(async () => {
      button(container, "Download and install")!.click();
    });

    expect(container.textContent).toContain("The update couldn’t be completed. Please try again later, or download the installer.");
  });

  it("detaches the progress subscription when the page unmounts mid-download", async () => {
    let resolveInstall!: () => void;
    mocks.invokeCommand.mockImplementation((command: string) => {
      if (command === "install_app_update") {
        return new Promise<void>((resolve) => {
          resolveInstall = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    const container = await renderPage(UPDATE);
    const root = roots[0]!.root;

    await act(async () => {
      button(container, "Download and install")!.click();
    });
    expect(unlisteners[0]).not.toHaveBeenCalled();

    act(() => root.unmount());
    // Unmount tears the listener down, and the late settle must not touch state.
    expect(unlisteners[0]).toHaveBeenCalledTimes(1);
    emitProgress(50, 100);
    resolveInstall();
    await act(async () => {
      await Promise.resolve();
    });
  });

  it("offers the install control only while an update exists", async () => {
    const container = await renderPage(UPDATE);
    expect(button(container, "Download and install")).toBeTruthy();

    // Once a fresh check says we're up to date, the control is gone entirely —
    // which is also why `handleInstall`'s `!status?.hasUpdate` guard cannot be
    // reached from the UI (see the waiver ledger).
    await act(async () => {
      mocks.invokeCommand.mockResolvedValue(UP_TO_DATE);
      button(container, "Check for updates")!.click();
    });

    expect(container.textContent).toContain("You're on the latest version");
    expect(button(container, "Download and install")).toBeUndefined();
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("install_app_update", { language: "en" });
  });
});

describe("updatePage restart", () => {
  async function installedPage() {
    const container = await renderPage(UPDATE);
    await act(async () => {
      button(container, "Download and install")!.click();
    });
    return container;
  }

  it("restarts the app and reports a failure with the install error slot", async () => {
    const container = await installedPage();

    mocks.invokeCommand.mockRejectedValueOnce(new Error("restart blocked"));
    await act(async () => {
      button(container, "Restart FutureOS")!.click();
    });
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("restart_after_app_update");
    expect(container.textContent).toContain("Couldn’t restart automatically. Close and reopen FutureOS.");
  });

  it("stringifies a non-Error restart failure and clears it on a retry", async () => {
    const container = await installedPage();

    mocks.invokeCommand.mockRejectedValueOnce("no window");
    await act(async () => {
      button(container, "Restart FutureOS")!.click();
    });
    expect(container.textContent).toContain("Couldn’t restart automatically. Close and reopen FutureOS.");

    mocks.invokeCommand.mockResolvedValueOnce(undefined);
    await act(async () => {
      button(container, "Restart FutureOS")!.click();
    });
    expect(container.textContent).not.toContain("Couldn’t restart automatically");
  });
});

describe("updatePage manual download", () => {
  it("labels a manual-only platform and opens the download link", async () => {
    const container = await renderPage(MANUAL);

    expect(container.textContent).toContain("Download the latest version to update FutureOS.");
    expect(button(container, "Download and install")).toBeUndefined();

    await act(async () => {
      container.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });
    expect(mocks.openExternalUrl).toHaveBeenCalledExactlyOnceWith("https://downloads.example.com/app.dmg");
  });

  it("labels a platform without an asset and offers no link", async () => {
    const container = await renderPage(NO_ASSET);

    expect(container.textContent).toContain("An installer isn’t available for this device yet.");
    expect(container.querySelector("a")).toBeNull();
  });

  it("reports a failed manual download and clears it on a retry", async () => {
    const container = await renderPage(MANUAL);

    mocks.openExternalUrl.mockRejectedValueOnce(new Error("no handler"));
    await act(async () => {
      container.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });
    expect(container.textContent).toContain("The download page couldn’t be opened. Check your connection and try again.");

    await act(async () => {
      container.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });
    expect(container.textContent).not.toContain("no handler");
  });

  it("stringifies a non-Error download failure", async () => {
    const container = await renderPage(MANUAL);

    mocks.openExternalUrl.mockRejectedValueOnce("denied by policy");
    await act(async () => {
      container.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });

    expect(container.textContent).toContain("The download page couldn’t be opened. Check your connection and try again.");
  });

  it("clears a download error when a new check starts", async () => {
    const container = await renderPage(MANUAL);
    mocks.openExternalUrl.mockRejectedValueOnce(new Error("no handler"));
    await act(async () => {
      container.querySelector("a")!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    });
    expect(container.textContent).toContain("The download page couldn’t be opened");
    expect(container.textContent).not.toContain("no handler");

    await act(async () => {
      mocks.invokeCommand.mockResolvedValueOnce(MANUAL);
      button(container, "Check for updates")!.click();
    });
    expect(container.textContent).not.toContain("The download page couldn’t be opened");
  });
});

describe("updatePage subscription failure", () => {
  it("resets the downloading state when the progress subscription cannot be created", async () => {
    mocks.listen.mockRejectedValue(new Error("event bus unavailable"));
    const container = await renderPage(UPDATE);

    await act(async () => {
      button(container, "Download and install")!.click();
    });

    expect(container.textContent).toContain("The update couldn’t be completed. Please try again later, or download the installer.");
    expect(button(container, "Download and install")!.disabled).toBe(false);
    // The install command must not run without a progress subscription.
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("install_app_update", { language: "en" });
  });

  it("detaches an immediately-resolved subscription when the page unmounted first", async () => {
    // The listener resolves after unmount, so the page must release it itself
    // instead of storing it for a cleanup that already ran.
    let resolveListen!: (unlisten: () => void) => void;
    const unlisten = vi.fn();
    mocks.listen.mockImplementation(() => new Promise((resolve) => {
      resolveListen = resolve as never;
    }));
    const container = await renderPage(UPDATE);
    const root = roots[0]!.root;

    await act(async () => {
      button(container, "Download and install")!.click();
    });
    act(() => root.unmount());
    await act(async () => {
      resolveListen(unlisten);
      await Promise.resolve();
    });

    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("install_app_update", { language: "en" });
  });
});

describe("durable update recovery", () => {
  it("does not claim an observer handoff is an installed update", async () => {
    mocks.maintenance.mockResolvedValueOnce({ cli: {}, update: null });
    mocks.maintenance.mockResolvedValue({ cli: {}, update: {
      operationId: "update-pending",
      stage: "awaiting-exit",
      outcome: null,
      diagnostic: null,
      restartSafe: false,
      repairAcknowledged: false,
    } });
    const container = await renderPage(UPDATE);
    await act(async () => button(container, "Download and install")!.click());
    expect(container.textContent).toContain("Installing update…");
    expect(container.textContent).not.toContain("Update installed. Restart to finish.");
    expect(button(container, "Restart FutureOS")).toBeUndefined();
    expect(button(container, "Download and install")).toBeUndefined();
  });

  it("keeps diagnostics private and explains how to repair without offering restart", async () => {
    mocks.maintenance.mockResolvedValue({ cli: { error: "CLI /old/future version mismatch" }, update: {
      operationId: "update-failed",
      stage: "finished",
      outcome: "repair-required",
      diagnostic: "Original files are incomplete",
      restartSafe: false,
      repairAcknowledged: false,
    } });
    const container = await renderPage(UPDATE);
    expect(container.textContent).not.toContain("CLI /old/future version mismatch");
    expect(container.textContent).not.toContain("update-failed");
    expect(container.textContent).toContain("Download the installer, reinstall FutureOS, then open it again.");
    expect(container.textContent).not.toContain("Original files are incomplete");
    expect(button(container, "Download and install")!.disabled).toBe(true);
    expect(button(container, "Restart FutureOS")).toBeUndefined();
    expect(button(container, "I’ve reinstalled FutureOS")).toBeTruthy();
  });
});

it("returns quietly when the user defers installation", async () => {
  mocks.maintenance.mockResolvedValueOnce({ cli: {}, update: null });
  mocks.maintenance.mockResolvedValue({ cli: {}, update: {
    operationId: "cancelled-private-id",
    stage: "finished",
    outcome: "cancelled",
    diagnostic: null,
    restartSafe: false,
    repairAcknowledged: false,
  } });
  const container = await renderPage(UPDATE);
  await act(async () => button(container, "Download and install")!.click());
  expect(container.querySelector("[role='alert']")).toBeNull();
  expect(button(container, "Download and install")!.disabled).toBe(false);
  expect(button(container, "Restart FutureOS")).toBeUndefined();
  expect(container.textContent).not.toContain("cancelled-private-id");
});

it("offers the pending restart even without cached update discovery", async () => {
  mocks.maintenance.mockResolvedValue({ cli: {}, update: {
    operationId: "pending-private-id",
    stage: "awaiting-validation",
    outcome: "pending-validation",
    diagnostic: "worker-native-detail",
    restartSafe: true,
    repairAcknowledged: false,
  } });
  const container = await renderPage();
  expect(button(container, "Restart FutureOS")).toBeTruthy();
  expect(container.textContent).not.toContain("pending-private-id");
  expect(container.textContent).not.toContain("worker-native-detail");
});
