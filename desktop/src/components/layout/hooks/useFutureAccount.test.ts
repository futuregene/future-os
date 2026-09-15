// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useFutureAccount } from "./useFutureAccount";

const mocks = vi.hoisted(() => ({
  authChanged: null as null | (() => void),
  clearBalance: vi.fn(),
  clearProfile: vi.fn(),
  getAuth: vi.fn(),
  getBalance: vi.fn(),
}));

vi.mock("../../../integrations/agent/providers", () => ({
  clearFutureBalanceCache: mocks.clearBalance,
  clearFutureProfileCache: mocks.clearProfile,
  getFutureAuthState: mocks.getAuth,
  getFutureBalance: mocks.getBalance,
  peekFutureBalance: () => null,
  peekFutureProfile: () => null,
  storeFutureBalance: vi.fn(),
}));

vi.mock("../../../lib/futureEvents", () => ({
  onFutureEvent: (name: string, callback: () => void) => {
    if (name === "future-auth-changed")
      mocks.authChanged = callback;
    return () => {};
  },
}));

vi.mock("../../../lib/useTauriEvent", () => ({
  useTauriEvent: vi.fn(),
}));

describe("useFutureAccount", () => {
  beforeEach(() => {
    mocks.authChanged = null;
    mocks.clearBalance.mockClear();
    mocks.clearProfile.mockClear();
    mocks.getAuth.mockReset();
    mocks.getBalance.mockReset();
  });

  it("does not treat a stored but rejected key as signed in", async () => {
    mocks.getAuth.mockResolvedValue({ status: "invalid", profile: null });
    const hook = renderHook(useFutureAccount);

    await flushAsync();

    expect(hook.current).toMatchObject({
      status: "invalid",
      email: null,
      balance: null,
      balanceStatus: "idle",
    });
    expect(mocks.getBalance).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("ignores an old credential check after authentication changes", async () => {
    let resolveOld!: (value: { status: "invalid"; profile: null }) => void;
    mocks.getAuth
      .mockReturnValueOnce(new Promise(resolve => resolveOld = resolve))
      .mockResolvedValueOnce({
        status: "authenticated",
        profile: { email: "new@example.com", userId: "new", emailVerified: true, createdAt: null },
      });
    mocks.getBalance.mockResolvedValue({ credits: 12 });
    const hook = renderHook(useFutureAccount);

    act(() => mocks.authChanged?.());
    await flushAsync();
    await flushAsync();
    expect(hook.current.status).toBe("authenticated");
    expect(hook.current.email).toBe("new@example.com");

    resolveOld({ status: "invalid", profile: null });
    await flushAsync();
    expect(hook.current.status).toBe("authenticated");
    expect(hook.current.email).toBe("new@example.com");
    hook.unmount();
  });
});
