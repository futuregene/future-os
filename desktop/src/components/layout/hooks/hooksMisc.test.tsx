// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useAgentStatus } from "./useAgentStatus";
import { useAppSettings } from "./useAppSettings";
import { useAppStartup } from "./useAppStartup";
import { useAutoUpgradeSkills } from "./useAutoUpgradeSkills";

const mocks = vi.hoisted(() => ({
  getAppSettings: vi.fn(),
  getAuth: vi.fn(),
  getStatus: vi.fn(),
  listProviders: vi.fn(),
  sandbox: { available: false, code: "unsupported", definitive: false, resolved: false } as {
    available: boolean;
    code?: string;
    definitive: boolean;
    resolved: boolean;
  },
  syncSkills: vi.fn(),
  updateAppSettings: vi.fn(),
}));

vi.mock("../../../integrations/agent/agentStatus", () => ({
  getAgentStatus: (...args: unknown[]) => mocks.getStatus(...args),
}));
vi.mock("../../../integrations/agent/providers", () => ({
  getFutureAuthState: (...args: unknown[]) => mocks.getAuth(...args),
  listAgentProviders: (...args: unknown[]) => mocks.listProviders(...args),
}));
vi.mock("../../../integrations/skills/skillsClient", () => ({
  syncSkills: (...args: unknown[]) => mocks.syncSkills(...args),
}));
vi.mock("../../../integrations/agent/useSandboxAvailability", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../integrations/agent/useSandboxAvailability")>();
  return { ...actual, useSandboxAvailability: () => mocks.sandbox };
});
vi.mock("../../../integrations/storage/appSettings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../integrations/storage/appSettings")>();
  return {
    ...actual,
    getAppSettings: (...args: unknown[]) => mocks.getAppSettings(...args),
    updateAppSettings: (...args: unknown[]) => mocks.updateAppSettings(...args),
  };
});
vi.mock("../../../lib/useTauriEvent", () => ({ useTauriEvent: () => {} }));

function captureToasts() {
  const toasts: { message: string; tone?: string }[] = [];
  const handler = (event: Event) => toasts.push((event as CustomEvent<{ message: string; tone?: string }>).detail);
  window.addEventListener("futureos:toast", handler);
  return { stop: () => window.removeEventListener("futureos:toast", handler), toasts };
}

describe("useAgentStatus", () => {
  beforeEach(() => mocks.getStatus.mockReset());

  // NOTE: the `catch` arm of useAgentStatus (the transport-failure branch that
  // synthesizes `phase: "unavailable"`) is covered by "keeps polling after a
  // probe failure so a later recovery is picked up" below: rejecting the probe
  // *once* and then resolving lets the failure arm be asserted without vitest 4
  // reporting a test-scoped unhandled error (a rejected-only mock does).

  it("does not arm a follow-up probe when the gate unmounts before the first one resolves", async () => {
    // Tearing down before the first `await getAgentStatus()` resolves means no
    // follow-up timer was armed. More importantly, the resumed probe must then
    // bail out (its `cancelled` check) instead of scheduling the next poll —
    // otherwise a gate that swapped views in the same tick keeps probing forever.
    const setTimeoutSpy = vi.spyOn(window, "setTimeout");
    mocks.getStatus.mockResolvedValue({ agentVersion: null, desktopVersion: "1.0", phase: "starting" });
    const harness = renderHook(() => useAgentStatus());

    // No `await` between mount and unmount: the probe is still suspended, so this
    // exercises the cleanup with no timer to clear.
    setTimeoutSpy.mockClear();
    harness.unmount();

    // Let the in-flight probe settle after teardown.
    await act(async () => {
      await Promise.resolve();
    });

    // No poll was scheduled: the cancelled probe returned without re-arming.
    expect(setTimeoutSpy).not.toHaveBeenCalled();
    setTimeoutSpy.mockRestore();
  });

  it("shows the wait hint after 5s while the Agent is still starting", async () => {
    vi.useFakeTimers();
    mocks.getStatus.mockResolvedValue({ agentVersion: null, desktopVersion: "1.0", phase: "starting" });
    const harness = renderHook(() => useAgentStatus());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(harness.current.showWait).toBe(false);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(harness.current.showWait).toBe(true);
    harness.unmount();
    vi.useRealTimers();
  });

  it("keeps polling after a probe failure so a later recovery is picked up", async () => {
    // A single failure must not end the probe loop: the Agent can come back.
    mocks.getStatus.mockImplementationOnce(() => {
      const probe = new Promise<never>((_resolve, reject) => {
        queueMicrotask(() => reject(new Error("no agent")));
      });
      void probe.catch(() => {});
      return probe;
    }).mockResolvedValue({ agentVersion: "2.0", desktopVersion: "1.0", phase: "ready" });
    vi.useFakeTimers();
    const harness = renderHook(() => useAgentStatus());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(harness.current.phase).toBe("unavailable");

    // The failure schedules the next probe; once it answers ready, the app is
    // admitted (after the splash floor).
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(mocks.getStatus.mock.calls.length).toBeGreaterThan(1);
    expect(harness.current.phase).toBe("ready");
    expect(harness.current.agentVersion).toBe("2.0");
    harness.unmount();
    vi.useRealTimers();
  });

  it("converts a probe that stays in `checking` past the startup budget into a timeout", async () => {
    vi.useFakeTimers();
    mocks.getStatus.mockResolvedValue({ agentVersion: null, desktopVersion: "1.0", phase: "checking" });
    const harness = renderHook(() => useAgentStatus());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(harness.current.phase).toBe("checking");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(16_000);
    });
    expect(harness.current.phase).toBe("startup_timeout");
    harness.unmount();
    vi.useRealTimers();
  });

  it("admits a ready Agent and keeps polling it", async () => {
    vi.useFakeTimers();
    mocks.getStatus.mockResolvedValue({ agentVersion: "2.0", desktopVersion: "1.0", phase: "ready" });
    const harness = renderHook(() => useAgentStatus());
    // The ready state is held back to avoid a splash flash.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(harness.current.phase).toBe("checking");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(harness.current.phase).toBe("ready");
    expect(harness.current.agentVersion).toBe("2.0");
    harness.unmount();
    vi.useRealTimers();
  });

  it("stops probing once unmounted", async () => {
    vi.useFakeTimers();
    mocks.getStatus.mockResolvedValue({ agentVersion: null, desktopVersion: "1.0", phase: "checking" });
    const harness = renderHook(() => useAgentStatus());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const calls = mocks.getStatus.mock.calls.length;
    harness.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(mocks.getStatus.mock.calls.length).toBe(calls);
    vi.useRealTimers();
  });
});

describe("useAutoUpgradeSkills", () => {
  beforeEach(() => mocks.syncSkills.mockReset());

  it("does nothing while disabled", async () => {
    const harness = renderHook(() => useAutoUpgradeSkills(false));
    await flushAsync();
    expect(mocks.syncSkills).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("announces a sync that installed or upgraded skills and logs failures", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const events: unknown[] = [];
    const listener = () => events.push("skills-changed");
    window.addEventListener("futureos:skills-changed", listener);
    mocks.syncSkills.mockResolvedValue({
      failed: [{ id: "broken", reason: "boom" }],
      installed: ["a"],
      upgraded: [],
    });

    const harness = renderHook(() => useAutoUpgradeSkills(true));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(events).toHaveLength(1);
    expect(warn).toHaveBeenCalledWith("[skills] auto-sync failed", { id: "broken", reason: "boom" });

    window.removeEventListener("futureos:skills-changed", listener);
    warn.mockRestore();
    harness.unmount();
  });

  it("never emits skills-changed when nothing changed", async () => {
    mocks.syncSkills.mockResolvedValue({ failed: [], installed: [], upgraded: [] });
    const events: unknown[] = [];
    const listener = () => events.push("skills-changed");
    window.addEventListener("futureos:skills-changed", listener);
    const harness = renderHook(() => useAutoUpgradeSkills(true));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(events).toHaveLength(0);
    window.removeEventListener("futureos:skills-changed", listener);
    harness.unmount();
  });

  it("logs and swallows a malformed sync result", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    // A result missing `installed` makes the hook throw inside its own try —
    // the same shape as a transport failure, without the test owning a
    // rejected promise the harness would report as unhandled.
    mocks.syncSkills.mockResolvedValue({} as never);
    const harness = renderHook(() => useAutoUpgradeSkills(true));
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(warn).toHaveBeenCalledWith("[skills] auto-upgrade skipped", expect.any(TypeError));
    warn.mockRestore();
    harness.unmount();
  });
});

describe("useAppStartup", () => {
  const ready = { profile: null, status: "authenticated" };
  const providers = { builtin: [], custom: [] };

  beforeEach(() => {
    mocks.getAuth.mockReset();
    mocks.listProviders.mockReset();
  });

  it("stays pending until the agent is ready, then captures both snapshots", async () => {
    const ref = { current: false };
    mocks.getAuth.mockResolvedValue(ready);
    mocks.listProviders.mockResolvedValue(providers);
    const harness = renderHook(() => useAppStartup(ref.current));
    expect(harness.current.phase).toBe("pending");
    expect(mocks.listProviders).not.toHaveBeenCalled();

    ref.current = true;
    harness.rerender();
    await flushAsync();
    expect(harness.current).toEqual({ phase: "ready", auth: ready, providers });
    harness.unmount();
  });

  it("retains an auth verification failure without failing startup", async () => {
    mocks.getAuth.mockRejectedValue(new Error("verify failed"));
    mocks.listProviders.mockResolvedValue(providers);
    const harness = renderHook(() => useAppStartup(true));
    await flushAsync();
    expect(harness.current).toEqual({
      phase: "ready",
      auth: { profile: null, status: "unavailable" },
      providers,
    });
    harness.unmount();
  });

  it("fails startup when the provider configuration cannot be read", async () => {
    mocks.getAuth.mockResolvedValue(ready);
    mocks.listProviders.mockRejectedValue(new Error("config unreadable"));
    const harness = renderHook(() => useAppStartup(true));
    await flushAsync();
    expect(harness.current.phase).toBe("failed");
    harness.unmount();
  });

  // The two arms below are the `cancelled` side of each continuation: the account
  // and provider reads outlive the readiness gate, because the Agent can drop
  // back to `pending` (restart, disconnect) while a request is still open.
  it("discards a snapshot that lands after the agent went away again", async () => {
    let landAuth!: (value: typeof ready) => void;
    mocks.getAuth.mockReturnValue(new Promise((resolve) => {
      landAuth = resolve;
    }));
    mocks.listProviders.mockResolvedValue(providers);

    const ref = { current: true };
    const harness = renderHook(() => useAppStartup(ref.current));
    await act(async () => {
      await Promise.resolve();
    });
    expect(harness.current.phase).toBe("pending");

    // The Agent drops away mid-flight: the effect cleans up and re-arms pending.
    ref.current = false;
    harness.rerender();
    expect(harness.current.phase).toBe("pending");

    await act(async () => {
      landAuth(ready);
      await Promise.resolve();
      await Promise.resolve();
    });
    // A success from the abandoned gate must not resurrect a ready snapshot.
    expect(harness.current.phase).toBe("pending");
    harness.unmount();
  });

  it("discards a provider failure that lands after the agent went away again", async () => {
    let failProviders!: (error: Error) => void;
    mocks.getAuth.mockResolvedValue(ready);
    mocks.listProviders.mockReturnValue(new Promise((_resolve, reject) => {
      failProviders = reject;
    }));

    const ref = { current: true };
    const harness = renderHook(() => useAppStartup(ref.current));
    await act(async () => {
      await Promise.resolve();
    });

    ref.current = false;
    harness.rerender();
    await act(async () => {
      failProviders(new Error("config unreadable"));
      await Promise.resolve();
      await Promise.resolve();
    });
    // Must stay pending, not `failed`: that startup failure belongs to a gate the
    // app is no longer showing.
    expect(harness.current.phase).toBe("pending");
    harness.unmount();
  });
});

describe("useAppSettings", () => {
  interface ServerSettings {
    approvalTier: "off" | "sandbox" | "manual";
    autoConnectRemote: boolean;
    autoTitleFirstTurn: boolean;
    autoUpgradeSkills: boolean;
    bellOnComplete: boolean;
    communityEdition: boolean;
    hiddenModels: string[];
    skillGuideDismissed: boolean;
    skillIntroDismissed: boolean;
    skillRecommend: boolean;
    titleLanguage: string;
  }

  const baseSettings = (): ServerSettings => ({
    approvalTier: "off",
    autoConnectRemote: false,
    autoTitleFirstTurn: true,
    autoUpgradeSkills: false,
    bellOnComplete: true,
    communityEdition: false,
    hiddenModels: [],
    skillGuideDismissed: false,
    skillIntroDismissed: false,
    skillRecommend: true,
    titleLanguage: "en",
  });

  /** The backend's stored copy; writes mutate it like the real store does. */
  let server: ServerSettings;

  beforeEach(() => {
    server = baseSettings();
    mocks.getAppSettings.mockReset().mockImplementation(async () => ({ ...server }));
    mocks.updateAppSettings.mockReset().mockImplementation(async (patch: Partial<ServerSettings>) => {
      server = { ...server, ...patch };
      return { ...server };
    });
    mocks.sandbox = { available: false, code: "unsupported", definitive: false, resolved: false };
  });

  it("adopts the persisted settings (including values this client never touched)", async () => {
    server = { ...server, communityEdition: true };
    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    await flushAsync();
    expect(harness.current.appSettings.communityEdition).toBe(true);
    harness.unmount();
  });

  it("applies an edit optimistically and adopts the server snapshot", async () => {
    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    await act(async () => {
      await harness.current.changeSettings({ bellOnComplete: false });
    });
    expect(mocks.updateAppSettings).toHaveBeenCalledWith({ bellOnComplete: false });
    expect(harness.current.appSettings.bellOnComplete).toBe(false);
    harness.unmount();
  });

  it("reports a failed write and reloads the authoritative settings", async () => {
    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    mocks.updateAppSettings.mockRejectedValueOnce(new Error("write failed"));
    server = { ...server, bellOnComplete: true };
    const toasts = captureToasts();

    await act(async () => {
      await harness.current.changeSettings({ bellOnComplete: false });
    });

    expect(toasts.toasts[0]!.message).toContain("write failed");
    expect(toasts.toasts[0]!.tone).toBe("error");
    expect(mocks.getAppSettings.mock.calls.length).toBeGreaterThan(1);
    expect(harness.current.appSettings.bellOnComplete).toBe(true);
    toasts.stop();
    harness.unmount();
  });

  it("forces the manual tier and warns when the sandbox probe says it is unavailable", async () => {
    server = { ...server, approvalTier: "sandbox" };
    mocks.sandbox = { available: false, code: "no_sandbox", definitive: true, resolved: true };
    const toasts = captureToasts();
    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    await flushAsync();
    await flushAsync();

    expect(toasts.toasts.some(toast => toast.message.includes("no_sandbox") && toast.tone === "info")).toBe(true);
    expect(harness.current.appSettings.approvalTier).toBe("manual");
    expect(mocks.updateAppSettings).toHaveBeenCalledWith({ approvalTier: "manual" });
    toasts.stop();
    harness.unmount();
  });

  it("names the generic probe failure when the probe reports no code", async () => {
    // `sandboxAvailability.code ?? "probe_failed"`: the fallback notice still has
    // to name a diagnostic when the probe gave none.
    server = { ...server, approvalTier: "sandbox" };
    mocks.sandbox = { available: false, definitive: true, resolved: true };
    const toasts = captureToasts();
    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    await flushAsync();
    await flushAsync();

    expect(toasts.toasts.some(toast => toast.message.includes("probe_failed") && toast.tone === "info")).toBe(true);
    expect(harness.current.appSettings.approvalTier).toBe("manual");
    toasts.stop();
    harness.unmount();
  });

  // ── the serialized write queue ──────────────────────────────────────────────
  // Writes are chained so two rapid edits cannot complete out of order. The
  // guards under test are the other half of that contract: when an *older*
  // request finally answers, its snapshot (or its failure) must not overwrite the
  // newer edit the user is already looking at. Mount first and let the
  // mount-time `titleLanguage` write settle, so the mock call history below
  // contains only the writes each test starts itself.

  /** Mount, settle the startup write, then forget it so call indices are clean. */
  async function mountSettled() {
    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    await flushAsync();
    mocks.getAppSettings.mockClear();
    mocks.updateAppSettings.mockClear();
    return harness;
  }

  /** A promise whose settlement this test controls. */
  function pending<T>() {
    let resolve!: (value: T) => void;
    let reject!: (error: Error) => void;
    const promise = new Promise<T>((res, rej) => {
      resolve = res;
      reject = rej;
    });
    // Marked handled up front: the hook does catch it, but the runtime must not
    // report it as unhandled in the window before that continuation runs.
    void promise.catch(() => {});
    return { promise, reject, resolve };
  }

  it("does not apply a superseded write's snapshot over the newer edit", async () => {
    const harness = await mountSettled();
    const first = pending<ServerSettings>();
    const second = pending<ServerSettings>();
    mocks.updateAppSettings.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    // Two edits in quick succession: the second bumps the generation, so the
    // first is superseded while its request is still open. Both calls are made
    // inside `act` because each one applies an optimistic update immediately.
    let writeA!: Promise<void>;
    let writeB!: Promise<void>;
    await act(async () => {
      writeA = harness.current.changeSettings({ bellOnComplete: false });
      writeB = harness.current.changeSettings({ communityEdition: true });
      await Promise.resolve();
    });

    // The older request answers with the values it saw. Applying that snapshot
    // would roll the newer edit back to `communityEdition: false`.
    await act(async () => {
      first.resolve({ ...server, bellOnComplete: false, communityEdition: false, skillRecommend: false });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.appSettings.communityEdition).toBe(true);
    expect(harness.current.appSettings.skillRecommend).toBe(true);

    await act(async () => {
      second.resolve({ ...server, bellOnComplete: false, communityEdition: true });
      await writeB;
    });
    await act(async () => {
      await writeA;
    });
    expect(harness.current.appSettings.communityEdition).toBe(true);
    harness.unmount();
  });

  it("does not reconcile from a superseded write's failure", async () => {
    const harness = await mountSettled();
    const first = pending<ServerSettings>();
    const second = pending<ServerSettings>();
    mocks.updateAppSettings.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const toasts = captureToasts();

    let writeA!: Promise<void>;
    let writeB!: Promise<void>;
    await act(async () => {
      writeA = harness.current.changeSettings({ bellOnComplete: false });
      writeB = harness.current.changeSettings({ communityEdition: true });
      await Promise.resolve();
    });

    mocks.getAppSettings.mockClear();
    await act(async () => {
      first.reject(new Error("write failed"));
      await Promise.resolve();
      await Promise.resolve();
    });

    // The user is still told about the failure...
    expect(toasts.toasts.some(toast => toast.message.includes("write failed"))).toBe(true);
    // ...but only the newest write owns reconciliation: a superseded failure must
    // not refetch and clobber the newer, still-optimistic state.
    expect(mocks.getAppSettings).not.toHaveBeenCalled();
    expect(harness.current.appSettings.communityEdition).toBe(true);

    await act(async () => {
      second.resolve({ ...server, communityEdition: true });
      await writeB;
    });
    await act(async () => {
      await writeA;
    });
    expect(harness.current.appSettings.communityEdition).toBe(true);
    toasts.stop();
    harness.unmount();
  });

  it("does not apply a reload that a newer edit superseded", async () => {
    const harness = await mountSettled();
    const read = pending<ServerSettings>();
    mocks.getAppSettings.mockReturnValueOnce(read.promise);

    // A window-focus reload starts and hangs.
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await act(async () => {
      await Promise.resolve();
    });

    // The user edits while that read is in flight, superseding it. Wrapped in
    // `act` because the optimistic update lands immediately.
    let write!: Promise<void>;
    await act(async () => {
      write = harness.current.changeSettings({ communityEdition: true });
      await Promise.resolve();
    });

    await act(async () => {
      read.resolve({ ...server, communityEdition: false, skillRecommend: false });
      await Promise.resolve();
      await Promise.resolve();
    });
    // The stale read must not roll the edit back.
    expect(harness.current.appSettings.communityEdition).toBe(true);
    expect(harness.current.appSettings.skillRecommend).toBe(true);

    await act(async () => {
      await write;
    });
    harness.unmount();
  });

  it("applies a reload that nothing superseded", async () => {
    // The other side of the reload guard: with no edit in between, the read is
    // still authoritative and must be applied.
    const harness = await mountSettled();
    server = { ...server, communityEdition: true };
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await flushAsync();
    expect(harness.current.appSettings.communityEdition).toBe(true);
    harness.unmount();
  });

  it("does not queue a duplicate sandbox fallback write on a re-render", async () => {
    // End-to-end property behind the fallback's re-entrancy ref: however many
    // times the effect re-runs while the probe says the sandbox is unavailable,
    // the user gets one notice and the tier is persisted once. The fallback write
    // is held open here so every re-run happens while the ref is still set — the
    // most adversarial ordering available (see §4's note on the ref's true arm).
    server = { ...server, approvalTier: "sandbox" };
    mocks.sandbox = { available: false, code: "no_sandbox", definitive: true, resolved: true };
    const toasts = captureToasts();
    const heldWrite = pending<ServerSettings>();
    mocks.updateAppSettings
      .mockResolvedValueOnce({ ...server, titleLanguage: "en" }) // the startup write
      .mockReturnValueOnce(heldWrite.promise); // the fallback

    const harness = renderHook(() => useAppSettings());
    await flushAsync();
    await flushAsync();
    expect(mocks.updateAppSettings).toHaveBeenLastCalledWith({ approvalTier: "manual" });

    // Re-run the effect (new probe diagnostic) while the fallback is still open.
    mocks.sandbox = { available: false, code: "still_no_sandbox", definitive: true, resolved: true };
    harness.rerender();
    await flushAsync();

    // And a reload attempt, which cannot land before the held write resolves
    // because the write queue serializes it behind.
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await flushAsync();

    const fallbackWrites = mocks.updateAppSettings.mock.calls
      .filter(([patch]) => (patch as { approvalTier?: string }).approvalTier === "manual");
    expect(fallbackWrites).toHaveLength(1);
    expect(toasts.toasts.filter(toast => toast.message.includes("no_sandbox"))).toHaveLength(1);

    await act(async () => {
      heldWrite.resolve({ ...server, approvalTier: "manual" });
      await Promise.resolve();
    });
    expect(harness.current.appSettings.approvalTier).toBe("manual");
    toasts.stop();
    harness.unmount();
  });
});
