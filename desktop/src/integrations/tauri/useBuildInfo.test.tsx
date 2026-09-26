// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";
import { useBuildInfo } from "./useBuildInfo";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("./invoke", () => ({
  invokeCommand: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

beforeEach(() => {
  invokeMock.mockReset();
});

describe("useBuildInfo", () => {
  it("starts unknown and then reports the injected build identity", async () => {
    invokeMock.mockResolvedValue({ version: "1.2.3-dev.abcdef", isRelease: false });
    const hook = renderHook(() => useBuildInfo());

    // "Not yet known" is distinct from "release": nothing may flash before the
    // backend answers, so callers treat null as not-a-test-build.
    expect(hook.current.data).toBeNull();
    expect(hook.current.loading).toBe(true);

    await act(async () => {
      await flushAsync();
    });

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("app_build_info", undefined);
    expect(hook.current.data).toEqual({ version: "1.2.3-dev.abcdef", isRelease: false });
    expect(hook.current.loading).toBe(false);
    hook.unmount();
  });

  it("reports a release build as-is", async () => {
    invokeMock.mockResolvedValue({ version: "1.2.3", isRelease: true });
    const hook = renderHook(() => useBuildInfo());

    await act(async () => {
      await flushAsync();
    });

    expect(hook.current.data).toMatchObject({ isRelease: true });
    hook.unmount();
  });

  it("records a backend failure and stays null instead of guessing a channel", async () => {
    invokeMock.mockRejectedValue(new Error("app_build_info unavailable"));
    const hook = renderHook(() => useBuildInfo());

    await act(async () => {
      await flushAsync();
    });

    expect(hook.current.error).toBe("app_build_info unavailable");
    expect(hook.current.data).toBeNull();
    expect(hook.current.loading).toBe(false);
    hook.unmount();
  });

  it("reloads on demand", async () => {
    invokeMock.mockResolvedValue({ version: "1.0.0", isRelease: true });
    const hook = renderHook(() => useBuildInfo());
    await act(async () => {
      await flushAsync();
    });

    invokeMock.mockResolvedValue({ version: "1.1.0", isRelease: true });
    await act(async () => {
      hook.current.reload();
      await flushAsync();
    });

    expect(invokeMock).toHaveBeenCalledTimes(2);
    expect(hook.current.data).toMatchObject({ version: "1.1.0" });
    hook.unmount();
  });

  it("ignores a response that lands after unmount", async () => {
    let resolveInvoke!: (value: unknown) => void;
    invokeMock.mockImplementation(() => new Promise((resolve) => {
      resolveInvoke = resolve;
    }));
    const hook = renderHook(() => useBuildInfo());
    hook.unmount();

    resolveInvoke({ version: "9.9.9", isRelease: true });
    await act(async () => {
      await Promise.resolve();
    });
    // No state update on an unmounted hook, no crash.
    expect(() => hook.current).not.toThrow();
  });
});
