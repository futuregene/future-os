// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useUpdateChecker } from "./useUpdateChecker";

const mocks = vi.hoisted(() => ({
  handler: null as null | ((payload: unknown) => void),
  invoke: vi.fn(),
  unlisten: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mocks.invoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (event: { payload: unknown }) => void) => {
    if (event === "scheduler-app-update")
      mocks.handler = payload => handler({ payload });
    return Promise.resolve(mocks.unlisten);
  },
}));

function status(overrides: Record<string, unknown> = {}) {
  return {
    currentVersion: "1.0.0",
    latestVersion: "1.1.0",
    hasUpdate: true,
    platformSupported: true,
    canInstallInApp: true,
    downloadUrl: "https://example.com/app",
    ...overrides,
  };
}

describe("useUpdateChecker", () => {
  beforeEach(() => {
    mocks.handler = null;
    mocks.invoke.mockReset().mockResolvedValue(status({ hasUpdate: false }));
    mocks.unlisten.mockReset();
  });

  it("checks once on mount and reports no update", async () => {
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();

    expect(mocks.invoke.mock.calls[0]?.[0]).toBe("check_app_update");
    expect(hook.current.hasUpdate).toBe(false);
    expect(hook.current.cachedStatus?.currentVersion).toBe("1.0.0");
    hook.unmount();
  });

  it("flags an available update once and caches the full status", async () => {
    mocks.invoke.mockResolvedValue(status());
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();

    expect(hook.current.hasUpdate).toBe(true);
    expect(hook.current.cachedStatus?.latestVersion).toBe("1.1.0");
    hook.unmount();
  });

  it("markSeen clears the red dot and keeps it cleared for later statuses", async () => {
    mocks.invoke.mockResolvedValue(status());
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();
    expect(hook.current.hasUpdate).toBe(true);

    act(() => {
      hook.current.markSeen();
    });
    expect(hook.current.hasUpdate).toBe(false);

    // A scheduler push for the same (already seen) update must not re-flag it.
    await act(async () => {
      mocks.handler?.(status());
      await Promise.resolve();
    });
    expect(hook.current.hasUpdate).toBe(false);
    expect(hook.current.cachedStatus?.latestVersion).toBe("1.1.0");
    hook.unmount();
  });

  it("applies a scheduler push and flags an unseen update", async () => {
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();
    expect(hook.current.hasUpdate).toBe(false);

    await act(async () => {
      mocks.handler?.(status({ latestVersion: "2.0.0" }));
      await Promise.resolve();
    });

    expect(hook.current.hasUpdate).toBe(true);
    expect(hook.current.cachedStatus?.latestVersion).toBe("2.0.0");
    hook.unmount();
  });

  it("stays silent when the check fails", async () => {
    mocks.invoke.mockRejectedValue(new Error("offline"));
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();

    expect(hook.current.hasUpdate).toBe(false);
    expect(hook.current.cachedStatus).toBeNull();
    hook.unmount();
  });

  it("ignores a malformed push instead of throwing", async () => {
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();

    await act(async () => {
      mocks.handler?.(status({ hasUpdate: false, downloadUrl: null, canInstallInApp: false }));
      await Promise.resolve();
    });
    expect(hook.current.hasUpdate).toBe(false);
    hook.unmount();
  });

  it("detaches the scheduler listener on unmount", async () => {
    const hook = renderHook(() => useUpdateChecker());
    await flushAsync();

    hook.unmount();
    await act(async () => {
      await Promise.resolve();
    });
    expect(mocks.unlisten).toHaveBeenCalledTimes(1);
  });
});
