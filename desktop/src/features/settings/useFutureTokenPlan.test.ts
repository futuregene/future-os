// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";
import { useFutureTokenPlan } from "./useFutureTokenPlan";

const mocks = vi.hoisted(() => ({
  clear: vi.fn(),
  events: {} as Record<string, () => void>,
  get: vi.fn(),
  peek: vi.fn(),
}));

vi.mock("../../integrations/agent/providers", () => ({
  clearFutureTokenPlanCache: (...args: unknown[]) => mocks.clear(...args),
  getFutureTokenPlan: (...args: unknown[]) => mocks.get(...args),
  peekFutureTokenPlan: () => mocks.peek(),
}));

vi.mock("../../lib/futureEvents", () => ({
  onFutureEvent: (name: string, callback: () => void) => {
    mocks.events[name] = callback;
    return () => {};
  },
}));

const PLAN = {
  active: true,
  planCode: "plus",
  name: "Plus",
  grantedCredits: 5,
  usedCredits: 1,
  remainingCredits: 4,
  eligibleModels: [],
};

beforeEach(() => {
  mocks.clear.mockReset();
  mocks.events = {};
  mocks.get.mockReset().mockResolvedValue(PLAN);
  mocks.peek.mockReset().mockReturnValue(null);
});

describe("useFutureTokenPlan", () => {
  it("fetches the subscription once enabled and exposes the window", async () => {
    const harness = renderHook(() => useFutureTokenPlan(true));
    await flushAsync();

    expect(mocks.get).toHaveBeenCalledWith(true);
    expect(harness.current.tokenPlan).toEqual(PLAN);
    expect(harness.current.status).toBe("available");
    harness.unmount();
  });

  it("starts from a cached snapshot while its refresh is in flight", async () => {
    mocks.peek.mockReturnValue(PLAN);
    const harness = renderHook(() => useFutureTokenPlan(true));

    // The cached snapshot keeps the page from flashing an empty subscription
    // while the forced refetch is in flight.
    expect(harness.current.tokenPlan).toEqual(PLAN);
    expect(harness.current.status).toBe("loading");
    await flushAsync();
    expect(mocks.get).toHaveBeenCalledWith(true);
    harness.unmount();
  });

  it("does not fetch while disabled and drops what is cached", async () => {
    mocks.peek.mockReturnValue(PLAN);
    const harness = renderHook(() => useFutureTokenPlan(false));
    await flushAsync();

    expect(mocks.get).not.toHaveBeenCalled();
    expect(mocks.clear).toHaveBeenCalled();
    expect(harness.current.tokenPlan).toBeNull();
    expect(harness.current.status).toBe("idle");
    harness.unmount();
  });

  it("reports a failed read as unavailable", async () => {
    mocks.get.mockRejectedValue(new Error("platform unreachable"));
    const harness = renderHook(() => useFutureTokenPlan(true));
    await flushAsync();

    expect(harness.current.tokenPlan).toBeNull();
    expect(harness.current.status).toBe("unavailable");
    harness.unmount();
  });

  it("drops a response that lands after the account changed", async () => {
    let resolvePlan!: (value: typeof PLAN) => void;
    mocks.get.mockReturnValueOnce(new Promise((resolve) => {
      resolvePlan = resolve;
    }));
    const harness = renderHook(() => useFutureTokenPlan(true));
    await flushAsync();

    act(() => mocks.events["future-auth-changed"]!());
    await act(async () => {
      resolvePlan(PLAN);
      await Promise.resolve();
    });

    expect(harness.current.tokenPlan).toBeNull();
    expect(harness.current.status).toBe("idle");
    harness.unmount();
  });

  it("refetches the window when a run ends", async () => {
    const harness = renderHook(() => useFutureTokenPlan(true));
    await flushAsync();
    mocks.clear.mockClear();
    mocks.get.mockClear();
    mocks.get.mockResolvedValue({ ...PLAN, usedCredits: 2, remainingCredits: 3 });

    act(() => mocks.events.agent_end!());
    await flushAsync();

    expect(mocks.clear).toHaveBeenCalled();
    expect(mocks.get).toHaveBeenCalledWith(true);
    expect(harness.current.tokenPlan?.usedCredits).toBe(2);
    harness.unmount();
  });
});
