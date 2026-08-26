// @vitest-environment jsdom
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";
import {
  probeWindowsSandboxWithRetry,
  shouldPersistSandboxFallback,
  windowsSandboxAvailable,
} from "./useSandboxAvailability";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
}));

// Load a fresh module instance so the module-level `windowsProbe` cache and the
// platform flags are isolated per test.
async function loadWindowsModule(): Promise<typeof import("./useSandboxAvailability")> {
  vi.resetModules();
  vi.doMock("../../lib/platform", () => ({ isMacOS: false, isWindows: true }));
  return await import("./useSandboxAvailability");
}

async function loadMacModule(): Promise<typeof import("./useSandboxAvailability")> {
  vi.resetModules();
  vi.doMock("../../lib/platform", () => ({ isMacOS: true, isWindows: false }));
  return await import("./useSandboxAvailability");
}

async function loadNeutralModule(): Promise<typeof import("./useSandboxAvailability")> {
  vi.resetModules();
  vi.doUnmock("../../lib/platform");
  return await import("./useSandboxAvailability");
}

afterEach(() => {
  vi.doUnmock("../../lib/platform");
  invokeMock.mockReset();
});

async function settle() {
  for (let index = 0; index < 10; index += 1)
    await flushAsync();
}

describe("windowsSandboxAvailable", () => {
  it("reflects the native host probe without a separate product switch", () => {
    expect(
      windowsSandboxAvailable({ available: true, code: "available" }),
    ).toBe(true);
    expect(
      windowsSandboxAvailable({
        available: false,
        code: "write_boundary_failed",
      }),
    ).toBe(false);
  });
});

describe("shouldPersistSandboxFallback", () => {
  it("falls back only for an authoritative unavailable result", () => {
    expect(
      shouldPersistSandboxFallback(
        { available: false, definitive: true, resolved: true },
        "sandbox",
      ),
    ).toBe(true);
  });

  it("preserves the saved tier after a transient probe error", () => {
    expect(
      shouldPersistSandboxFallback(
        { available: false, definitive: false, resolved: true },
        "sandbox",
      ),
    ).toBe(false);
  });
});

describe("probeWindowsSandboxWithRetry", () => {
  it("retries transient Agent connection failures", async () => {
    let attempts = 0;
    const waits: number[] = [];

    const available = await probeWindowsSandboxWithRetry(
      async () => {
        attempts += 1;
        if (attempts < 3)
          throw new Error("connection refused");
        return { available: true, code: "available" };
      },
      [0, 100, 250],
      async milliseconds => void waits.push(milliseconds),
    );

    expect(available).toBe(true);
    expect(attempts).toBe(3);
    expect(waits).toEqual([100, 250]);
  });

  it("does not retry an explicit unavailable result", async () => {
    let attempts = 0;

    const available = await probeWindowsSandboxWithRetry(
      async () => {
        attempts += 1;
        return { available: false, code: "write_boundary_failed" };
      },
      [0, 100],
      async () => {},
    );

    expect(available).toBe(false);
    expect(attempts).toBe(1);
  });

  it("rejects after exhausting transient failures", async () => {
    let attempts = 0;

    await expect(
      probeWindowsSandboxWithRetry(
        async () => {
          attempts += 1;
          throw new Error("connection refused");
        },
        [0, 100, 250],
        async () => {},
      ),
    ).rejects.toThrow("connection refused");

    expect(attempts).toBe(3);
  });
});

describe("useSandboxAvailability", () => {
  it("is definitive available on macOS without probing", async () => {
    const { useSandboxAvailability } = await loadMacModule();
    const h = renderHook(() => useSandboxAvailability());
    expect(h.current).toEqual({ available: true, definitive: true, resolved: true });
    expect(invokeMock).not.toHaveBeenCalled();
    h.unmount();
  });

  it("is definitive unavailable on a non-mac, non-Windows platform", async () => {
    const { useSandboxAvailability } = await loadNeutralModule();
    const h = renderHook(() => useSandboxAvailability());
    expect(h.current).toEqual({ available: false, definitive: true, resolved: true });
    expect(invokeMock).not.toHaveBeenCalled();
    h.unmount();
  });

  it("resolves the native probe on Windows and caches the shared promise", async () => {
    invokeMock.mockResolvedValue({ available: true, code: "available" });
    const { useSandboxAvailability } = await loadWindowsModule();

    const h = renderHook(() => useSandboxAvailability());
    expect(h.current).toEqual({ available: false, definitive: false, resolved: false });
    await settle();
    expect(h.current).toEqual({ available: true, definitive: true, resolved: true });
    h.unmount();

    // The shared probe is cached: a second mount reuses it instead of probing again.
    const h2 = renderHook(() => useSandboxAvailability());
    expect(h2.current).toEqual({ available: false, definitive: false, resolved: false });
    await settle();
    expect(h2.current).toEqual({ available: true, definitive: true, resolved: true });
    h2.unmount();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("fails closed after exhausting retries and lets a later mount retry", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout"] });
    try {
      invokeMock.mockRejectedValue(new Error("connection refused"));
      const { useSandboxAvailability } = await loadWindowsModule();

      const h = renderHook(() => useSandboxAvailability());
      expect(h.current).toEqual({ available: false, definitive: false, resolved: false });
      await act(async () => {
        await vi.runAllTimersAsync();
      });
      expect(h.current).toEqual({ available: false, definitive: false, resolved: true });
      h.unmount();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("ignores a probe result that lands after unmount", async () => {
    let resolveProbe!: (value: { available: boolean; code: string }) => void;
    invokeMock.mockImplementation(
      () =>
        new Promise<{ available: boolean; code: string }>((resolve) => {
          resolveProbe = resolve;
        }),
    );
    const { useSandboxAvailability } = await loadWindowsModule();

    const h = renderHook(() => useSandboxAvailability());
    expect(h.current).toEqual({ available: false, definitive: false, resolved: false });
    h.unmount();
    // Resolve after unmount; the stale result must not update state.
    resolveProbe({ available: true, code: "available" });
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });
});

describe("useSandboxAvailability (mutable platform)", () => {
  it("reports the platform verdict through the public hook shape", async () => {
    const { useSandboxAvailability } = await loadWindowsModule();
    invokeMock.mockResolvedValue({ available: false, code: "write_boundary_failed" });
    const h = renderHook(() => useSandboxAvailability());
    await settle();
    expect(h.current).toEqual({ available: false, definitive: true, resolved: true });
    h.unmount();
  });
});
