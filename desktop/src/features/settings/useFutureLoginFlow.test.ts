// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useFutureLoginFlow } from "./useFutureLoginFlow";

const mocks = vi.hoisted(() => ({
  pollFutureLogin: vi.fn(),
  startFutureLogin: vi.fn(),
}));

vi.mock("../../integrations/agent/providers", () => ({
  pollFutureLogin: mocks.pollFutureLogin,
  startFutureLogin: mocks.startFutureLogin,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function loginStart(expiresIn = 30) {
  return {
    userCode: "ABCD",
    verificationUri: "https://example.com/device",
    verificationUriComplete: "https://example.com/device?code=ABCD",
    interval: 2,
    expiresIn,
    deviceCode: "device-code",
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(0);
  mocks.pollFutureLogin.mockReset();
  mocks.startFutureLogin.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useFutureLoginFlow", () => {
  it("keeps the same attempt waiting across a transient poll failure", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    mocks.pollFutureLogin
      .mockResolvedValueOnce({ status: "retry" })
      .mockResolvedValueOnce({ status: "pending" })
      .mockResolvedValueOnce({ status: "authorized" });
    const onAuthorized = vi.fn();
    const hook = renderHook(() => useFutureLoginFlow(onAuthorized));

    await act(async () => {
      await hook.current.begin();
    });
    expect(mocks.startFutureLogin).toHaveBeenCalledTimes(1);
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);
    expect(hook.current.phase).toBe("waiting");
    expect(hook.current.message).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(2);
    expect(hook.current.phase).toBe("waiting");
    expect(hook.current.message).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(3);
    expect(hook.current.phase).toBe("authorized");
    expect(onAuthorized).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("does not show a network error before the device code expires", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart(3));
    mocks.pollFutureLogin.mockResolvedValue({ status: "retry" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(hook.current.phase).toBe("waiting");
    expect(hook.current.message).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_000);
    });
    expect(hook.current.phase).toBe("expired");
    expect(hook.current.message).toBe("futureLogin.unavailable");
    hook.unmount();
  });
});
