// @vitest-environment jsdom
import type { ProvidersView } from "../../../integrations/agent/providers";
import type { FutureSessionStatus } from "./useFutureAccount";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useHasProviders } from "./useHasProviders";

const mocks = vi.hoisted(() => ({
  list: vi.fn<() => Promise<ProvidersView>>(),
}));

vi.mock("../../../integrations/agent/providers", () => ({
  FUTURE_PROVIDER_ID: "future",
  listAgentProviders: mocks.list,
}));

function providerView(ids: { builtin?: Array<[string, boolean]>; custom?: Array<[string, boolean]> }): ProvidersView {
  return {
    builtin: (ids.builtin ?? []).map(([id, hasApiKey], index) => ({
      id,
      name: id,
      baseUrl: `https://${id}.example.com`,
      hasApiKey,
      modelCount: index,
      requiresBaseUrl: false,
    })),
    custom: (ids.custom ?? []).map(([id, hasApiKey]) => ({
      id,
      name: id,
      api: "openai",
      baseUrl: `https://${id}.example.com`,
      hasApiKey,
      models: [],
    })),
  };
}

const EMPTY = providerView({});
const FUTURE_ONLY = providerView({ builtin: [["future", true]] });
const ANTHROPIC_ONLY = providerView({ builtin: [["anthropic", true]] });

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe("useHasProviders", () => {
  beforeEach(() => {
    mocks.list.mockReset();
  });

  it("shows the gate while the first probe is in flight, then honours an empty catalogue", async () => {
    mocks.list.mockResolvedValue(EMPTY);
    const harness = renderHook(() => useHasProviders("signed_out"));

    // First load: nothing is known yet, so the gate must hold.
    expect(harness.current.initialLoading).toBe(true);
    expect(harness.current.showGate).toBe(true);
    expect(harness.current.hasAnyProvider).toBe(false);

    await flushAsync();

    expect(harness.current.initialLoading).toBe(false);
    expect(harness.current.showGate).toBe(true);
    harness.unmount();
  });

  it("clears the gate for a builtin provider that owns a key", async () => {
    mocks.list.mockResolvedValue(ANTHROPIC_ONLY);
    const harness = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();

    expect(harness.current.hasAnyProvider).toBe(true);
    expect(harness.current.showGate).toBe(false);
    harness.unmount();
  });

  it("only counts the FutureOS builtin key once the account session is usable", async () => {
    mocks.list.mockResolvedValue(FUTURE_ONLY);

    for (const status of ["checking", "signed_out", "invalid", "error"] as unknown as FutureSessionStatus[]) {
      const harness = renderHook(() => useHasProviders(status));
      await flushAsync();
      expect(harness.current.showGate, `status=${status}`).toBe(true);
      expect(harness.current.hasAnyProvider, `status=${status}`).toBe(false);
      harness.unmount();
    }

    for (const status of ["authenticated", "unavailable"] as FutureSessionStatus[]) {
      const harness = renderHook(() => useHasProviders(status));
      await flushAsync();
      expect(harness.current.showGate, `status=${status}`).toBe(false);
      expect(harness.current.hasAnyProvider, `status=${status}`).toBe(true);
      harness.unmount();
    }
  });

  it("treats a keyless provider as unusable, whether builtin or custom", async () => {
    mocks.list.mockResolvedValue(providerView({ custom: [["mine", false]] }));
    const harness = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();
    expect(harness.current.showGate).toBe(true);
    harness.unmount();

    mocks.list.mockResolvedValue(providerView({ custom: [["mine", true]] }));
    const withKey = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();
    expect(withKey.current.showGate).toBe(false);
    expect(withKey.current.hasAnyProvider).toBe(true);
    withKey.unmount();
  });

  it("lets BYOK bypass the gate, and cancelLogin restores it", async () => {
    mocks.list.mockResolvedValue(EMPTY);
    const harness = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();
    expect(harness.current.showGate).toBe(true);

    harness.current.enableBYOK();
    harness.rerender();
    expect(harness.current.byokMode).toBe(true);
    expect(harness.current.showGate).toBe(false);
    // BYOK is a local decision: it does not invent provider data.
    expect(harness.current.hasAnyProvider).toBe(false);

    harness.current.cancelLogin();
    harness.rerender();
    expect(harness.current.byokMode).toBe(false);
    expect(harness.current.showGate).toBe(true);
    harness.unmount();
  });

  it("refetches on future-auth-changed and providers-changed without flashing a loading gate", async () => {
    mocks.list.mockResolvedValue(EMPTY);
    const harness = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();
    expect(mocks.list).toHaveBeenCalledTimes(1);

    mocks.list.mockResolvedValue(ANTHROPIC_ONLY);
    emitFutureEvent("future-auth-changed", undefined);
    // A silent poll tick: the previously loaded data stays on screen.
    expect(harness.current.initialLoading).toBe(false);
    expect(harness.current.showGate).toBe(true);
    await flushAsync();
    expect(mocks.list).toHaveBeenCalledTimes(2);

    emitFutureEvent("providers-changed", {
      revision: 3,
      providerId: "anthropic",
      operation: "set_key",
      authChanged: false,
      modelsChanged: true,
    });
    await flushAsync();
    expect(mocks.list).toHaveBeenCalledTimes(3);
    expect(harness.current.showGate).toBe(false);
    harness.unmount();
  });

  it("ignores a stale response that lands after a newer reload (cancellation)", async () => {
    const first = deferred<ProvidersView>();
    mocks.list.mockReturnValueOnce(first.promise);
    const harness = renderHook(() => useHasProviders("signed_out"));
    expect(harness.current.initialLoading).toBe(true);

    const stale = deferred<ProvidersView>();
    mocks.list.mockReturnValueOnce(stale.promise);
    emitFutureEvent("future-auth-changed", undefined);
    await flushAsync();
    expect(mocks.list).toHaveBeenCalledTimes(2);

    const fresh = deferred<ProvidersView>();
    mocks.list.mockReturnValueOnce(fresh.promise);
    emitFutureEvent("providers-changed", {
      revision: 1,
      providerId: "anthropic",
      operation: "set_key",
      authChanged: false,
      modelsChanged: false,
    });
    await flushAsync();
    expect(mocks.list).toHaveBeenCalledTimes(3);

    fresh.resolve(ANTHROPIC_ONLY);
    await flushAsync();
    expect(harness.current.showGate).toBe(false);

    // The superseded fetch resolving late must not resurrect its payload.
    stale.resolve(EMPTY);
    await flushAsync();
    expect(harness.current.showGate).toBe(false);
    expect(harness.current.hasAnyProvider).toBe(true);
    harness.unmount();
  });

  it("runs the post-login init on a fresh sign-in, not on a session that was already signed in", async () => {
    mocks.list.mockResolvedValue(EMPTY);
    const harness = renderHook(() => useHasProviders("authenticated"));
    await flushAsync();
    expect(harness.current.initPending).toBe(false);

    // A reload that reveals the key for the first time is a fresh sign-in.
    mocks.list.mockResolvedValue(FUTURE_ONLY);
    emitFutureEvent("future-auth-changed", undefined);
    await flushAsync();
    expect(harness.current.initPending).toBe(true);
    expect(harness.current.showGate).toBe(true);

    harness.current.finishInit();
    harness.rerender();
    expect(harness.current.initPending).toBe(false);
    expect(harness.current.showGate).toBe(false);
    harness.unmount();

    // Already signed in on the very first load: no init flow.
    mocks.list.mockResolvedValue(FUTURE_ONLY);
    const already = renderHook(() => useHasProviders("authenticated"));
    await flushAsync();
    expect(already.current.initPending).toBe(false);
    expect(already.current.showGate).toBe(false);
    already.unmount();
  });

  it("forces the gate when Settings asks for onboarding and clears it on finish/cancel", async () => {
    mocks.list.mockResolvedValue(ANTHROPIC_ONLY);
    const harness = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();
    expect(harness.current.showGate).toBe(false);

    emitFutureEvent("show-onboarding", undefined);
    harness.rerender();
    expect(harness.current.forceOnboarding).toBe(true);
    expect(harness.current.showGate).toBe(true);

    harness.current.finishInit();
    harness.rerender();
    expect(harness.current.forceOnboarding).toBe(false);
    expect(harness.current.showGate).toBe(false);

    emitFutureEvent("show-onboarding", undefined);
    harness.rerender();
    harness.current.enableBYOK();
    harness.rerender();
    expect(harness.current.showGate).toBe(false);

    emitFutureEvent("show-onboarding", undefined);
    harness.rerender();
    expect(harness.current.showGate).toBe(true);
    harness.current.cancelLogin();
    harness.rerender();
    expect(harness.current.forceOnboarding).toBe(false);
    expect(harness.current.showGate).toBe(false);
    harness.unmount();
  });

  it("keeps the gate up and stops loading when the provider probe fails", async () => {
    mocks.list.mockRejectedValue(new Error("agent unreachable"));
    const harness = renderHook(() => useHasProviders("signed_out"));
    await flushAsync();

    expect(harness.current.initialLoading).toBe(false);
    expect(harness.current.showGate).toBe(true);
    harness.unmount();
  });

  it("seeds the gate from an initialProviders snapshot before the probe resolves", () => {
    mocks.list.mockReturnValue(new Promise(() => {}));
    const harness = renderHook(() => useHasProviders("signed_out", ANTHROPIC_ONLY));

    // `initialLoading` is only true when nothing is known — the seeded snapshot counts.
    expect(harness.current.initialLoading).toBe(false);
    expect(harness.current.showGate).toBe(false);
    harness.unmount();
  });

  it("holds the documented gate invariant across catalogue and session combinations", async () => {
    const catalogues = [EMPTY, FUTURE_ONLY, ANTHROPIC_ONLY, providerView({ custom: [["mine", true]] })];
    const statuses: FutureSessionStatus[] = ["checking", "signed_out", "invalid", "unavailable", "authenticated"];

    for (const view of catalogues) {
      for (const status of statuses) {
        mocks.list.mockResolvedValue(view);
        const harness = renderHook(() => useHasProviders(status));
        const whileLoading = harness.current;
        expect(whileLoading.showGate).toBe(
          whileLoading.initialLoading || !whileLoading.hasAnyProvider || whileLoading.initPending || whileLoading.forceOnboarding,
        );

        await flushAsync();
        const settled = harness.current;
        expect(settled.initialLoading).toBe(false);
        expect(settled.showGate).toBe(!settled.hasAnyProvider || settled.initPending || settled.forceOnboarding);
        expect(settled.showGate).toBe(Boolean(settled.showGate));
        harness.unmount();
      }
    }
  });
});
