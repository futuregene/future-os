// @vitest-environment jsdom
import type { RemoteStatus } from "../../../features/remote/remoteClient";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useRemoteStatus } from "./useRemoteStatus";

const mocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  presentation: vi.fn(),
}));

vi.mock("../../../features/remote/remoteClient", () => ({
  getRemoteStatus: (...args: unknown[]) => mocks.getStatus(...args),
  remoteConnectionPresentation: (...args: unknown[]) => mocks.presentation(...args),
}));

const CONNECTED: RemoteStatus = {
  phase: "ready",
  reason: null,
  recovery: null,
  natsUrl: "nats://x",
  pairId: "pair-1",
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "d1",
  desktopPublicKey: "k",
  webUrl: null,
  webLanUrl: null,
  warningCode: null,
};

describe("useRemoteStatus", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.getStatus.mockReset().mockResolvedValue(CONNECTED);
    mocks.presentation.mockReset().mockReturnValue({ level: "connected" });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts unknown, polls immediately and keeps the derived indicator in sync", async () => {
    const hook = renderHook(() => useRemoteStatus(true));

    await flushAsync();
    // `renderHook`'s act flushes the immediate poll, so the first committed
    // value is already the backend's answer.
    expect(mocks.getStatus).toHaveBeenCalledTimes(1);
    expect(hook.current.status).toBe(CONNECTED);
    expect(hook.current.indicator).toBe("connected");
    expect(mocks.presentation).toHaveBeenCalledWith(CONNECTED);

    // A changed status is what re-runs the presentation mapping — an equal
    // payload is deliberately skipped (see the no-re-render test below).
    const connecting: RemoteStatus = { ...CONNECTED, phase: "connecting", pairId: "" };
    mocks.getStatus.mockResolvedValue(connecting);
    mocks.presentation.mockReturnValue({ level: "connecting" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(hook.current.indicator).toBe("connecting");

    // An unpaired remote presents nothing (no indicator dot).
    mocks.getStatus.mockResolvedValue({ ...connecting, pairId: "pair-2" });
    mocks.presentation.mockReturnValue(null);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(hook.current.indicator).toBeNull();
    hook.unmount();
  });

  it("polls every 3s while enabled and stops when disabled", async () => {
    const hook = renderHook(() => useRemoteStatus(true));
    await flushAsync();
    expect(mocks.getStatus).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(mocks.getStatus).toHaveBeenCalledTimes(2);

    const disabled = renderHook(() => useRemoteStatus(false));
    await flushAsync();
    const calls = mocks.getStatus.mock.calls.length;
    expect(disabled.current.status).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    // Only the still-enabled hook kept ticking.
    expect(mocks.getStatus.mock.calls.length).toBeGreaterThan(calls);
    hook.unmount();
    disabled.unmount();
  });

  it("stops polling on unmount", async () => {
    const hook = renderHook(() => useRemoteStatus(true));
    await flushAsync();
    const calls = mocks.getStatus.mock.calls.length;

    hook.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(mocks.getStatus).toHaveBeenCalledTimes(calls);
  });

  it("keeps the last known status when a poll fails", async () => {
    const hook = renderHook(() => useRemoteStatus(true));
    await flushAsync();
    expect(hook.current.status).toBe(CONNECTED);

    mocks.getStatus.mockRejectedValue(new Error("bridge gone"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });

    expect(hook.current.status).toBe(CONNECTED);
    expect(hook.current.indicator).toBe("connected");
    hook.unmount();
  });

  it("does not re-render when the poll returns the same status", async () => {
    const hook = renderHook(() => useRemoteStatus(true));
    await flushAsync();
    const first = hook.current.status;

    // A structurally identical payload must be swapped for the previous object,
    // so the 3s app-level poll cannot re-render the whole shell.
    mocks.getStatus.mockResolvedValue({ ...CONNECTED });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });

    expect(hook.current.status).toBe(first);
    hook.unmount();
  });

  it("exposes a manual refresh that updates the status", async () => {
    const hook = renderHook(() => useRemoteStatus(false));
    await flushAsync();
    expect(hook.current.status).toBeNull();

    const next: RemoteStatus = { ...CONNECTED, pairId: "pair-2" };
    mocks.getStatus.mockResolvedValue(next);
    await act(async () => {
      await hook.current.refresh();
    });
    expect(hook.current.status).toBe(next);
    hook.unmount();
  });
});
