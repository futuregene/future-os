import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// @vitest-environment jsdom
import { renderHook } from "../../test/renderHook";
import { invokeCommand } from "./invoke";
import { useMaintenanceStatus } from "./useMaintenanceStatus";

/**
 * The native maintenance read: what the update and recovery surfaces are drawn
 * from, and the only thing that can tell the reader the host cannot be asked.
 *
 * It polls (a repair can be acknowledged from another surface), so the failure
 * path and the re-entrancy guard are as much a part of the contract as the happy
 * read: an error that is swallowed leaves a page that looks like it has nothing
 * to report, and a read that runs on top of itself can answer out of order.
 */

vi.mock("./invoke", () => ({ invokeCommand: vi.fn() }));
vi.mock("../../lib/errors", async importOriginal => ({
  ...await importOriginal<typeof import("../../lib/errors")>(),
  errorMessage: (error: unknown) => (error instanceof Error ? error.message : String(error)),
}));

beforeEach(() => {
  vi.mocked(invokeCommand).mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useMaintenanceStatus", () => {
  it("reports what the host said, and clears an earlier failure", async () => {
    vi.mocked(invokeCommand).mockResolvedValue({
      cli: {},
      update: { operationId: "op_1", outcome: "installed", repairAcknowledged: true, restartSafe: true, stage: "finished" },
    });
    const hook = renderHook(useMaintenanceStatus);

    await act(async () => {
      await Promise.resolve();
    });
    expect(hook.current.maintenance?.update?.operationId).toBe("op_1");
    expect(hook.current.maintenanceError).toBeNull();
    hook.unmount();
  });

  it("says so when the host cannot be asked", async () => {
    // The page has to distinguish "nothing is in progress" from "I could not
    // find out": the first is quiet, the second disables the controls that would
    // act on an unknown state.
    vi.mocked(invokeCommand).mockRejectedValue(new Error("bridge unavailable"));
    const hook = renderHook(useMaintenanceStatus);

    await act(async () => {
      await Promise.resolve();
    });
    expect(hook.current.maintenanceError).toBe("bridge unavailable");
    expect(hook.current.maintenance).toBeNull();
    hook.unmount();
  });

  it("does not start a poll on top of a read that is still in flight", async () => {
    // The 3 s poll can fire while a slow read is outstanding; a second read would
    // race it, and the older answer could land last. The guard is what keeps one
    // read at a time.
    let settle!: (value: unknown) => void;
    vi.mocked(invokeCommand).mockReturnValue(new Promise((resolve) => {
      settle = resolve;
    }));
    vi.useFakeTimers();
    const hook = renderHook(useMaintenanceStatus);
    expect(invokeCommand).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(9_000);
    });
    expect(invokeCommand).toHaveBeenCalledTimes(1);

    // Once it settles, the next tick reads again.
    await act(async () => {
      settle({ cli: {}, update: null });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_000);
    });
    expect(invokeCommand).toHaveBeenCalledTimes(2);
    hook.unmount();
  });
});
