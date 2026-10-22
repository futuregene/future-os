// @vitest-environment jsdom
import { listen } from "@tauri-apps/api/event";
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { UpdatePage } from "./UpdatePage";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: vi.fn() }));
vi.mock("../../integrations/tauri/useBuildInfo", () => ({ useBuildInfo: () => ({ data: { version: "1.0" } }) }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const status = { currentVersion: "1.0", latestVersion: "2.0", hasUpdate: true, platformSupported: true, canInstallInApp: true, downloadUrl: null };

beforeEach(() => vi.clearAllMocks());

it.each([false, true])("handles install under StrictMode including subscription failure=%s", async (failed) => {
  const unlisten = vi.fn();
  if (failed)
    vi.mocked(listen).mockRejectedValueOnce(new Error("subscription unavailable"));
  else
    vi.mocked(listen).mockResolvedValueOnce(unlisten);
  vi.mocked(invokeCommand).mockResolvedValue(undefined);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<StrictMode><UpdatePage cachedStatus={status} /></StrictMode>));
    const button = [...container.querySelectorAll("button")].find(item => item.textContent === "Download and install")!;
    expect(button).toBeTruthy();
    await act(async () => button.click());
    if (failed) {
      expect(container.textContent).toContain("The update couldn’t be completed");
      expect(container.textContent).not.toContain("subscription unavailable");
      expect(invokeCommand).not.toHaveBeenCalledWith("install_app_update", { language: "en" });
      expect([...container.querySelectorAll("button")].find(item => item.textContent === "Download and install")?.disabled).toBe(false);
    }
    else {
      expect(invokeCommand).toHaveBeenCalledWith("install_app_update", { language: "en" });
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});

/**
 * The recovery lane: what the page does when an install does not simply succeed.
 *
 * An in-app install can fail and leave the app in a state that needs a look —
 * files restored, a repair to acknowledge, a restart the host has declared safe.
 * The page's job is to *say* which of those it is and offer the one action that
 * fits, so these tests drive it through the states rather than the happy path.
 */
function maintenance(overrides: Record<string, unknown> = {}) {
  return {
    cli: {},
    update: {
      diagnostic: null,
      fromVersion: "1.0",
      operationId: "op_1",
      outcome: "installed",
      repairAcknowledged: true,
      restartSafe: false,
      stage: "finished",
      targetVersion: "2.0",
      ...overrides,
    },
  };
}

/** Route `invokeCommand` by command name; anything unlisted resolves to undefined. */
function routeCommands(handlers: Record<string, () => unknown>) {
  vi.mocked(invokeCommand).mockImplementation(async (command: string) => {
    const handler = handlers[command];
    if (!handler)
      return undefined;
    return await handler();
  });
}

async function mountPage() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<UpdatePage cachedStatus={status} />));
  const button = (label: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === label);
  const link = (label: string) =>
    [...container.querySelectorAll<HTMLAnchorElement>("a")].find(item => item.textContent === label);
  return {
    button,
    container,
    link,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

it("reports a failed install whose recovery read failed too", async () => {
  // The install failed *and* the host cannot say what state it is in. Both facts
  // have to reach the reader: the first is why the button came back, the second
  // is why the page cannot tell them what to do about it.
  // The mount read answers (so the install control is live); the read the page
  // performs *after* the failed install is the one that cannot be reached.
  let reads = 0;
  routeCommands({
    app_maintenance_status: () => {
      reads += 1;
      return reads === 1 ? Promise.resolve(maintenance()) : Promise.reject(new Error("maintenance offline"));
    },
    install_app_update: () => Promise.reject(new Error("disk full")),
  });
  const page = await mountPage();
  try {
    await act(async () => page.button("Download and install")!.click());

    expect(invokeCommand).toHaveBeenCalledWith("install_app_update", { language: "en" });
    expect(page.container.textContent).toContain("The update couldn’t be completed");
    expect(page.container.textContent).toContain("We can’t confirm the update status");
    expect(page.container.textContent).not.toContain("disk full");
    expect(page.container.textContent).not.toContain("maintenance offline");
  }
  finally {
    page.unmount();
  }
});

it("offers a restart when the host says the failed install is safe to restart from", async () => {
  // The install failed, the host restored its files, and it declares the result
  // restart-safe. The page's own restart action is the way out — the recovery
  // panel `UpdateRecoveryPanel` is for the case where nothing else is mounted.
  routeCommands({
    app_maintenance_status: () => Promise.resolve(maintenance({ outcome: "failed-restored", restartSafe: true })),
    install_app_update: () => Promise.reject(new Error("interrupted")),
    restart_after_app_update: () => Promise.resolve(undefined),
  });
  const page = await mountPage();
  try {
    await act(async () => page.button("Download and install")!.click());
    expect(page.container.textContent).toContain("Your previous version has been restored.");
    expect(page.container.textContent).not.toContain("interrupted");

    const restart = page.button("Restart FutureOS");
    expect(restart).not.toBeUndefined();
    await act(async () => restart!.click());
    expect(invokeCommand).toHaveBeenCalledWith("restart_after_app_update");
  }
  finally {
    page.unmount();
  }
});

it("confirms a repair with the operation it belongs to, and reports a refusal", async () => {
  // The host is holding a repair that needs the user to acknowledge it before it
  // will move on. The acknowledgement is addressed by operation id — the wrong
  // id acknowledges nothing — and a refusal has to be visible rather than
  // leaving a button that looks like it did nothing.
  let acknowledged = false;
  routeCommands({
    acknowledge_update_repair: () => {
      acknowledged = true;
      return Promise.resolve(undefined);
    },
    app_maintenance_status: () => Promise.resolve(
      maintenance({ outcome: "repair-required", repairAcknowledged: acknowledged }),
    ),
    install_app_update: () => Promise.reject(new Error("interrupted")),
  });
  const page = await mountPage();
  try {
    await act(async () => page.button("Download and install")!.click());

    const confirm = page.button("I’ve reinstalled FutureOS");
    expect(confirm).not.toBeUndefined();
    await act(async () => confirm!.click());

    expect(invokeCommand).toHaveBeenCalledWith("acknowledge_update_repair", { operationId: "op_1", language: "en" });
    // The re-read says the repair was acknowledged, so the control is gone.
    expect(page.button("I’ve reinstalled FutureOS")).toBeUndefined();
  }
  finally {
    page.unmount();
  }

  // A refusal is reported, and the control stays so the reader can retry.
  acknowledged = false;
  routeCommands({
    acknowledge_update_repair: () => Promise.reject(new Error("no such operation")),
    app_maintenance_status: () => Promise.resolve(maintenance({ outcome: "repair-required", repairAcknowledged: false })),
    install_app_update: () => Promise.reject(new Error("interrupted")),
  });
  const retry = await mountPage();
  try {
    await act(async () => retry.button("I’ve reinstalled FutureOS")!.click());

    expect(retry.container.textContent).toContain("Couldn’t confirm your reinstallation");
    expect(retry.container.textContent).not.toContain("no such operation");
    expect(retry.button("I’ve reinstalled FutureOS")).not.toBeUndefined();
  }
  finally {
    retry.unmount();
  }
});

it("offers the manual download only when the host names an asset", async () => {
  // A build that cannot install in-app falls back to a link, and the link exists
  // only when there is somewhere to point it.
  routeCommands({ app_maintenance_status: () => Promise.resolve(maintenance()) });
  const page = await mountPage();
  try {
    expect(page.link("Download")).toBeUndefined();
  }
  finally {
    page.unmount();
  }
});
