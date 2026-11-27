// @vitest-environment jsdom
import type { FutureTokenPlan } from "../../../integrations/agent/providers";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useFutureSubscription } from "./useFutureSubscription";

const getFutureTokenPlan = vi.fn();
const peekFutureTokenPlan = vi.fn();
vi.mock("../../../integrations/agent/providers", () => ({
  clearFutureTokenPlanCache: vi.fn(),
  getFutureTokenPlan: (...args: unknown[]) => getFutureTokenPlan(...args),
  peekFutureTokenPlan: () => peekFutureTokenPlan(),
}));

function plan(active: boolean): FutureTokenPlan {
  return { active, grantedCredits: 0, usedCredits: 0, remainingCredits: 0, eligibleModels: [] };
}

function emit(name: "agent_end" | "future-auth-changed") {
  window.dispatchEvent(new CustomEvent(`futureos:${name}`, { detail: undefined }));
}

describe("useFutureSubscription", () => {
  beforeEach(() => {
    getFutureTokenPlan.mockReset();
    peekFutureTokenPlan.mockReset();
    peekFutureTokenPlan.mockReturnValue(null);
  });

  it("reports the plan the account has", async () => {
    getFutureTokenPlan.mockResolvedValue(plan(true));
    const hook = renderHook(() => useFutureSubscription(true));
    await flushAsync();
    expect(hook.current).toBe(true);
    hook.unmount();
  });

  it("reports no plan for an expired subscription", async () => {
    getFutureTokenPlan.mockResolvedValue(plan(false));
    const hook = renderHook(() => useFutureSubscription(true));
    await flushAsync();
    expect(hook.current).toBe(false);
    hook.unmount();
  });

  it("stays unknown while signed out instead of asking the platform", async () => {
    const hook = renderHook(() => useFutureSubscription(false));
    await flushAsync();
    expect(hook.current).toBeNull();
    expect(getFutureTokenPlan).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("treats a failed read as unknown, never as an unsubscribed account", async () => {
    // A locked picker is worse than an unlocked one: the API still refuses the
    // request, but a transient failure must not hide callable models.
    getFutureTokenPlan.mockRejectedValue(new Error("platform unreachable"));
    const hook = renderHook(() => useFutureSubscription(true));
    await flushAsync();
    expect(hook.current).toBeNull();
    hook.unmount();
  });

  it("re-reads a plan that lapsed or was bought while the app was open", async () => {
    getFutureTokenPlan.mockResolvedValue(plan(true));
    const hook = renderHook(() => useFutureSubscription(true));
    await flushAsync();
    expect(hook.current).toBe(true);

    getFutureTokenPlan.mockResolvedValue(plan(false));
    await act(async () => {
      emit("agent_end");
      await Promise.resolve();
    });
    expect(hook.current).toBe(false);
    // Forced: the cache belongs to the run that just ended.
    expect(getFutureTokenPlan).toHaveBeenLastCalledWith(true);
    hook.unmount();
  });

  it("forgets the plan when the credential changes", async () => {
    getFutureTokenPlan.mockResolvedValue(plan(true));
    const hook = renderHook(() => useFutureSubscription(true));
    await flushAsync();
    expect(hook.current).toBe(true);

    act(() => emit("future-auth-changed"));
    expect(hook.current).toBeNull();
    hook.unmount();
  });
});
