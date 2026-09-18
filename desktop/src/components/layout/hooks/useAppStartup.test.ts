// @vitest-environment jsdom
import type { FutureAuthState, ProvidersView } from "../../../integrations/agent/providers";
import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { getFutureAuthState, listAgentProviders } from "../../../integrations/agent/providers";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useAppStartup } from "./useAppStartup";

vi.mock("../../../integrations/agent/providers", () => ({
  getFutureAuthState: vi.fn(),
  listAgentProviders: vi.fn(),
}));

const AUTHENTICATED: FutureAuthState = {
  status: "authenticated",
  profile: {
    email: "user@example.com",
    userId: "user-1",
    emailVerified: true,
    createdAt: null,
  },
};

const PROVIDERS: ProvidersView = {
  builtin: [{
    id: "future",
    name: "FutureOS",
    baseUrl: "https://example.com",
    hasApiKey: true,
    modelCount: 1,
    requiresBaseUrl: false,
  }],
  custom: [],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

afterEach(() => {
  vi.mocked(getFutureAuthState).mockReset();
  vi.mocked(listAgentProviders).mockReset();
});

describe("useAppStartup", () => {
  it("does not admit the app until both auth and provider snapshots settle", async () => {
    const auth = deferred<FutureAuthState>();
    const providers = deferred<ProvidersView>();
    vi.mocked(getFutureAuthState).mockReturnValue(auth.promise);
    vi.mocked(listAgentProviders).mockReturnValue(providers.promise);
    const hook = renderHook(() => useAppStartup(true));

    await act(async () => auth.resolve(AUTHENTICATED));
    expect(hook.current.phase).toBe("pending");
    await act(async () => providers.resolve(PROVIDERS));
    expect(hook.current).toEqual({ phase: "ready", auth: AUTHENTICATED, providers: PROVIDERS });
    hook.unmount();
  });

  it("keeps authentication unavailable distinct from provider configuration", async () => {
    vi.mocked(getFutureAuthState).mockRejectedValue(new Error("offline"));
    vi.mocked(listAgentProviders).mockResolvedValue(PROVIDERS);
    const hook = renderHook(() => useAppStartup(true));
    await flushAsync();

    expect(hook.current).toEqual({
      phase: "ready",
      auth: { status: "unavailable", profile: null },
      providers: PROVIDERS,
    });
    hook.unmount();
  });

  it("fails startup when provider configuration cannot be read", async () => {
    vi.mocked(getFutureAuthState).mockResolvedValue(AUTHENTICATED);
    vi.mocked(listAgentProviders).mockRejectedValue(new Error("transport error"));
    const hook = renderHook(() => useAppStartup(true));
    await flushAsync();

    expect(hook.current.phase).toBe("failed");
    hook.unmount();
  });
});
