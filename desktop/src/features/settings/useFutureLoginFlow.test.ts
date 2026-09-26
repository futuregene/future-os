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

  it("reports the start failure as an error instead of polling", async () => {
    mocks.startFutureLogin.mockRejectedValue(new Error("device endpoint down"));
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });

    expect(hook.current.phase).toBe("error");
    expect(hook.current.message).toBe("device endpoint down");
    expect(hook.current.start).toBeNull();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(mocks.pollFutureLogin).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("stringifies a non-Error start rejection", async () => {
    mocks.startFutureLogin.mockRejectedValue("boom");
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });

    expect(hook.current.phase).toBe("error");
    expect(hook.current.message).toBe("boom");
    hook.unmount();
  });

  it("cancel() returns to idle, drops the device code and stops polling", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    mocks.pollFutureLogin.mockResolvedValue({ status: "pending" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);

    act(() => hook.current.cancel());
    expect(hook.current.phase).toBe("idle");
    expect(hook.current.start).toBeNull();
    expect(hook.current.message).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("re-beginning discards the slower previous attempt's result", async () => {
    const resolvers: ((value: unknown) => void)[] = [];
    mocks.startFutureLogin.mockImplementation(() => new Promise((resolve) => {
      resolvers.push(resolve);
    }));
    mocks.pollFutureLogin.mockResolvedValue({ status: "pending" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    let first!: Promise<void>;
    await act(async () => {
      first = hook.current.begin(); // attempt 1, still in flight
      const second = hook.current.begin(); // attempt 2 wins the attempt id
      resolvers[1]!(loginStart());
      await second;
    });
    // Attempt 1 landing late must not overwrite attempt 2's device code.
    await act(async () => {
      resolvers[0]!({ ...loginStart(), deviceCode: "stale-code" });
      await first;
    });

    expect(mocks.startFutureLogin).toHaveBeenCalledTimes(2);
    expect(hook.current.start?.deviceCode).toBe("device-code");
    expect(hook.current.phase).toBe("waiting");
    hook.unmount();
  });

  it("cancel() discards a late start rejection", async () => {
    let rejectStart!: (reason: unknown) => void;
    mocks.startFutureLogin.mockImplementation(() => new Promise((_resolve, reject) => {
      rejectStart = reject;
    }));
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      const pending = hook.current.begin();
      hook.current.cancel();
      rejectStart(new Error("late failure"));
      await pending;
    });

    expect(hook.current.phase).toBe("idle");
    expect(hook.current.message).toBeNull();
    hook.unmount();
  });

  it("cancel() discards an in-flight poll that later fails", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    let rejectPoll!: (reason: unknown) => void;
    mocks.pollFutureLogin.mockImplementation(() => new Promise((_resolve, reject) => {
      rejectPoll = reject;
    }));
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(hook.current.phase).toBe("waiting");

    await act(async () => {
      hook.current.cancel();
      rejectPoll(new Error("late network failure"));
    });

    expect(hook.current.phase).toBe("idle");
    expect(hook.current.message).toBeNull();
    hook.unmount();
  });

  it("cancel() discards an in-flight poll that later authorizes", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    let resolvePoll!: (value: unknown) => void;
    mocks.pollFutureLogin.mockImplementation(() => new Promise((resolve) => {
      resolvePoll = resolve;
    }));
    const onAuthorized = vi.fn();
    const hook = renderHook(() => useFutureLoginFlow(onAuthorized));

    await act(async () => {
      await hook.current.begin();
    });
    expect(hook.current.phase).toBe("waiting");

    await act(async () => {
      hook.current.cancel();
      resolvePoll({ status: "authorized" });
    });

    expect(hook.current.phase).toBe("idle");
    expect(onAuthorized).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("reports a rejected poll as an error", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    mocks.pollFutureLogin.mockRejectedValue(new Error("agent offline"));
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });

    expect(hook.current.phase).toBe("error");
    expect(hook.current.message).toBe("agent offline");
    hook.unmount();
  });

  it("backs off by 5s per slow_down instead of polling on the next tick", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart(600));
    mocks.pollFutureLogin.mockResolvedValue({ status: "slow_down" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);

    // The reserved slot (2s fast poll + 5s back-off) blocks the 1s ticks.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(6_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(2);
    expect(hook.current.phase).toBe("waiting");
    hook.unmount();
  });

  it("honours the server's retryAfterSeconds over the local back-off", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart(600));
    mocks.pollFutureLogin.mockResolvedValue({ status: "retry", retryAfterSeconds: 12 });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(11_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(2);
    hook.unmount();
  });

  it("gives up after three malformed responses", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart(600));
    mocks.pollFutureLogin.mockResolvedValue({ status: "malformed" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(2);
    expect(hook.current.phase).toBe("waiting");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_000);
    });
    expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(3);
    expect(hook.current.phase).toBe("error");
    expect(hook.current.message).toBe("futureLogin.invalidResponse");
    hook.unmount();
  });

  it("recovers the malformed counter after a good pending poll", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart(600));
    mocks.pollFutureLogin
      .mockResolvedValueOnce({ status: "malformed" })
      .mockResolvedValueOnce({ status: "pending" })
      .mockResolvedValueOnce({ status: "malformed" })
      .mockResolvedValueOnce({ status: "malformed" })
      .mockResolvedValueOnce({ status: "authorized" });
    const onAuthorized = vi.fn();
    const hook = renderHook(() => useFutureLoginFlow(onAuthorized));

    await act(async () => {
      await hook.current.begin();
    });
    // malformed, pending, malformed, malformed, authorized 鈥?never three in a row.
    for (const step of [2_000, 2_000, 2_000, 2_000, 2_000]) {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(step);
      });
    }

    expect(hook.current.phase).toBe("authorized");
    expect(onAuthorized).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("surfaces a denial with the server message, and the default text without one", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    mocks.pollFutureLogin.mockResolvedValueOnce({ status: "denied", message: "user said no" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));
    await act(async () => {
      await hook.current.begin();
    });
    expect(hook.current.phase).toBe("denied");
    expect(hook.current.message).toBe("user said no");
    hook.unmount();

    mocks.pollFutureLogin.mockReset();
    mocks.pollFutureLogin.mockResolvedValueOnce({ status: "denied" });
    const second = renderHook(() => useFutureLoginFlow(() => {}));
    await act(async () => {
      await second.current.begin();
    });
    expect(second.current.phase).toBe("denied");
    expect(second.current.message).toBe("futureLogin.denied");
    second.unmount();
  });

  it("surfaces a server-side expiry with the server message", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    mocks.pollFutureLogin.mockResolvedValueOnce({ status: "expired", message: "code timed out" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });

    expect(hook.current.phase).toBe("expired");
    expect(hook.current.message).toBe("code timed out");
    hook.unmount();
  });

  it("treats an unknown status as a failure with the server message", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart());
    mocks.pollFutureLogin.mockResolvedValueOnce({ status: "teapot", message: "what" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    expect(hook.current.phase).toBe("error");
    expect(hook.current.message).toBe("what");
    hook.unmount();

    mocks.pollFutureLogin.mockReset();
    mocks.pollFutureLogin.mockResolvedValueOnce({ status: "teapot" });
    const second = renderHook(() => useFutureLoginFlow(() => {}));
    await act(async () => {
      await second.current.begin();
    });
    expect(second.current.phase).toBe("error");
    expect(second.current.message).toBe("futureLogin.failed");
    second.unmount();
  });

  it("reports a plain expiry (no transient retry) with the expired text", async () => {
    mocks.startFutureLogin.mockResolvedValue(loginStart(3));
    mocks.pollFutureLogin.mockResolvedValue({ status: "pending" });
    const hook = renderHook(() => useFutureLoginFlow(() => {}));

    await act(async () => {
      await hook.current.begin();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_000);
    });

    expect(hook.current.phase).toBe("expired");
    expect(hook.current.message).toBe("futureLogin.expired");
    hook.unmount();
  });
});
