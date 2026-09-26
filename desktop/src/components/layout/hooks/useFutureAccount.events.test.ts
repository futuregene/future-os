// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useFutureAccount } from "./useFutureAccount";

const mocks = vi.hoisted(() => ({
  clearBalance: vi.fn(),
  clearProfile: vi.fn(),
  events: {} as Record<string, () => void>,
  getAuth: vi.fn(),
  getBalance: vi.fn(),
  peekBalance: vi.fn(),
  peekProfile: vi.fn(),
  storeBalance: vi.fn(),
  tauri: {} as Record<string, (payload: unknown) => void>,
}));

vi.mock("../../../integrations/agent/providers", () => ({
  clearFutureBalanceCache: (...args: unknown[]) => mocks.clearBalance(...args),
  clearFutureProfileCache: (...args: unknown[]) => mocks.clearProfile(...args),
  getFutureAuthState: (...args: unknown[]) => mocks.getAuth(...args),
  getFutureBalance: (...args: unknown[]) => mocks.getBalance(...args),
  peekFutureBalance: () => mocks.peekBalance(),
  peekFutureProfile: () => mocks.peekProfile(),
  storeFutureBalance: (...args: unknown[]) => mocks.storeBalance(...args),
}));
vi.mock("../../../lib/futureEvents", () => ({
  onFutureEvent: (name: string, callback: () => void) => {
    mocks.events[name] = callback;
    return () => {};
  },
}));
vi.mock("../../../lib/useTauriEvent", () => ({
  useTauriEvent: (name: string, callback: (payload: unknown) => void) => {
    mocks.tauri[name] = callback;
  },
}));

const AUTHENTICATED = {
  profile: { createdAt: null, email: "me@example.com", emailVerified: true, userId: "u1" },
  status: "authenticated",
};

beforeEach(() => {
  mocks.clearBalance.mockReset();
  mocks.clearProfile.mockReset();
  mocks.events = {};
  mocks.getAuth.mockReset().mockResolvedValue(AUTHENTICATED);
  mocks.getBalance.mockReset().mockResolvedValue({ credits: 10 });
  mocks.peekBalance.mockReset().mockReturnValue(null);
  mocks.peekProfile.mockReset().mockReturnValue(null);
  mocks.storeBalance.mockReset();
  mocks.tauri = {};
});

describe("useFutureAccount cached-balance edge cases", () => {
  it("retains the last-known account data for a status it does not act on", async () => {
    // `authenticated` with no profile is none of the three handled cases, so the
    // hook falls through and keeps what it already had rather than guessing.
    mocks.peekProfile.mockReturnValue({ email: "cached@example.com" });
    mocks.getAuth.mockResolvedValue({ profile: null, status: "authenticated" });
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("authenticated");
    expect(harness.current.email).toBe("cached@example.com");
    expect(harness.current.balanceStatus).not.toBe("unavailable");
    harness.unmount();
  });

  it("starts from the cached balance and keeps it marked available", () => {
    // A balance cached by an earlier session (`peekFutureBalance()` truthy) is
    // the only thing the hook has until the profile check answers, so the first
    // render must report it as available rather than idle.
    mocks.peekBalance.mockReturnValue({ credits: 42 });
    mocks.getAuth.mockReturnValue(new Promise(() => {}));
    const harness = renderHook(() => useFutureAccount());
    expect(harness.current.balanceStatus).toBe("available");
    expect(harness.current.balance).toBe(42);
    harness.unmount();
  });

  it("does not downgrade a cached balance when the profile check is unavailable", async () => {
    // A tripped verification must not blank a balance that is already known.
    mocks.peekBalance.mockReturnValue({ credits: 42 });
    mocks.getAuth.mockResolvedValue({ profile: null, status: "unavailable" });
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("unavailable");
    expect(harness.current.balanceStatus).toBe("available");
    harness.unmount();
  });

  it("does not downgrade a cached balance when the profile check throws", async () => {
    // The rejection handler's own arm of the same guard.
    mocks.peekBalance.mockReturnValue({ credits: 42 });
    mocks.getAuth.mockImplementation(() => Promise.reject(new Error("network")));
    const harness = renderHook(() => useFutureAccount());
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.status).toBe("unavailable");
    expect(harness.current.balanceStatus).toBe("available");
    harness.unmount();
  });

  it("ignores a profile success that lands after the credentials changed", async () => {
    // The success side of the same generation guard as the failure test above:
    // a check started under the old credential must not report a status for it.
    let landStaleCheck!: (value: { profile: null; status: string }) => void;
    mocks.getAuth
      .mockReturnValueOnce(new Promise((resolve) => {
        landStaleCheck = resolve;
      }))
      .mockResolvedValue({ profile: null, status: "signed_out" });
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("checking");

    act(() => mocks.events["future-auth-changed"]!());
    await flushAsync();
    await flushAsync();
    expect(harness.current.status).toBe("signed_out");

    await act(async () => {
      landStaleCheck({ profile: null, status: "authenticated" });
      await Promise.resolve();
      await Promise.resolve();
    });
    // The stale "authenticated" verdict must not revive a signed-out session.
    expect(harness.current.status).toBe("signed_out");
    expect(harness.current.email).toBeNull();
    harness.unmount();
  });

  it("ignores a profile failure that lands after the credentials changed", async () => {
    let failStaleCheck!: (error: Error) => void;
    mocks.getAuth
      .mockReturnValueOnce(new Promise((_resolve, reject) => {
        failStaleCheck = reject;
      }))
      .mockResolvedValue({ profile: null, status: "signed_out" });
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("checking");

    // Reauthentication bumps the generation; the re-check replaces the status.
    act(() => mocks.events["future-auth-changed"]!());
    await flushAsync();
    await flushAsync();
    expect(harness.current.status).toBe("signed_out");

    // The pre-change check now fails: it belongs to a credential that is gone,
    // so it must not overwrite the newer, authoritative status.
    await act(async () => {
      failStaleCheck(new Error("stale failure"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.status).toBe("signed_out");
    harness.unmount();
  });
});

describe("useFutureAccount auth + balance", () => {
  it("marks the balance unavailable when the platform check is unavailable", async () => {
    mocks.getAuth.mockResolvedValue({ profile: null, status: "unavailable" });
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("unavailable");
    expect(harness.current.balanceStatus).toBe("unavailable");
    expect(mocks.getBalance).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("treats a thrown verification failure as unavailable and keeps the last balance", async () => {
    mocks.getAuth.mockRejectedValueOnce(new Error("network"));
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("unavailable");
    expect(harness.current.balanceStatus).toBe("unavailable");
    harness.unmount();
  });

  it("refuses to refresh the balance while signed out", async () => {
    mocks.getAuth.mockResolvedValue({ profile: null, status: "invalid" });
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    act(() => harness.current.refreshBalance());
    await flushAsync();
    expect(mocks.getBalance).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("loads the balance once authenticated and exposes the credits", async () => {
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    await flushAsync();
    expect(mocks.getBalance).toHaveBeenCalledWith(true);
    expect(harness.current.balance).toBe(10);
    expect(harness.current.balanceStatus).toBe("available");
    expect(harness.current.email).toBe("me@example.com");
    harness.unmount();
  });

  it("drops a balance response that lands after the credentials changed", async () => {
    let resolveBalance!: (value: { credits: number }) => void;
    mocks.getBalance.mockReturnValueOnce(new Promise((resolve) => {
      resolveBalance = resolve;
    }));
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    await flushAsync();

    // A reauthentication invalidates the in-flight response.
    act(() => mocks.events["future-auth-changed"]!());
    await flushAsync();

    await act(async () => {
      resolveBalance({ credits: 999 });
      await Promise.resolve();
    });
    expect(harness.current.balance).not.toBe(999);
    harness.unmount();
  });

  it("drops a balance failure that lands after the credentials changed", async () => {
    let rejectBalance!: (error: Error) => void;
    mocks.getBalance.mockReturnValueOnce(new Promise((_resolve, reject) => {
      rejectBalance = reject;
    }));
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    await flushAsync();
    expect(harness.current.balanceStatus).toBe("loading");

    // A reauthentication invalidates the in-flight fetch...
    act(() => mocks.events["future-auth-changed"]!());
    expect(harness.current.balanceStatus).toBe("idle");
    mocks.getAuth.mockResolvedValue({ profile: null, status: "signed_out" });

    // ...so its failure must not be reported as an unavailable balance.
    await act(async () => {
      rejectBalance(new Error("stale failure"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.balanceStatus).toBe("idle");
    harness.unmount();
  });

  it("clears the balance, marks it unavailable and re-verifies the account when the fetch fails", async () => {
    mocks.getBalance.mockRejectedValueOnce(new Error("revoked"));
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    await flushAsync();
    expect(harness.current.balanceStatus).toBe("unavailable");
    expect(harness.current.balance).toBeNull();
    // A balance failure may be the first observation of a revoked key.
    expect(mocks.getAuth.mock.calls.length).toBeGreaterThan(1);
    harness.unmount();
  });
});

describe("useFutureAccount pushed updates", () => {
  it("adopts a balance pushed by the scheduler", async () => {
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    act(() => mocks.tauri["scheduler-future-balance"]!({ credits: 42 }));
    expect(mocks.storeBalance).toHaveBeenCalledWith({ credits: 42 });
    expect(harness.current.balance).toBe(42);
    expect(harness.current.balanceStatus).toBe("available");
    harness.unmount();
  });

  it("clears and refetches the balance when an agent run ends", async () => {
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    await flushAsync();
    expect(harness.current.balance).toBe(10);
    mocks.clearBalance.mockClear();
    mocks.getBalance.mockClear();

    act(() => mocks.events.agent_end!());
    await flushAsync();
    expect(mocks.clearBalance).toHaveBeenCalled();
    expect(mocks.getBalance).toHaveBeenCalledWith(true);
    harness.unmount();
  });

  it("resets to checking and re-verifies after a credential change", async () => {
    const harness = renderHook(() => useFutureAccount());
    await flushAsync();
    expect(harness.current.status).toBe("authenticated");

    mocks.getAuth.mockResolvedValue({ profile: null, status: "signed_out" });
    act(() => mocks.events["future-auth-changed"]!());
    expect(harness.current.status).toBe("checking");
    expect(harness.current.balance).toBeNull();
    expect(harness.current.email).toBeNull();
    expect(harness.current.balanceStatus).toBe("idle");

    await flushAsync();
    await flushAsync();
    expect(mocks.clearProfile).toHaveBeenCalled();
    expect(harness.current.status).toBe("signed_out");
    harness.unmount();
  });
});
