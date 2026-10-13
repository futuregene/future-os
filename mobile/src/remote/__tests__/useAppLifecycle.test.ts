import { createElement, useRef } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import * as Network from "expo-network";
import { AppState } from "react-native";
import {
  beginNativePresentation,
  endNativePresentation,
  NATIVE_PRESENTATION_GRACE_MS,
} from "../nativePresentation";
import { BACKGROUND_GRACE_MS, useAppLifecycle, type AppLifecycle } from "../useAppLifecycle";

jest.mock("expo-network", () => ({
  __esModule: true,
  NetworkStateType: {
    NONE: "NONE",
    UNKNOWN: "UNKNOWN",
    WIFI: "WIFI",
    CELLULAR: "CELLULAR",
    OTHER: "OTHER",
  },
  getNetworkStateAsync: jest.fn(),
  addNetworkStateListener: jest.fn(() => ({ remove: jest.fn() })),
}));

jest.mock("react-native", () => ({
  __esModule: true,
  AppState: {
    currentState: "active",
    addEventListener: jest.fn(() => ({ remove: jest.fn() })),
  },
  Platform: {
    OS: "ios",
    select: (specifics: Record<string, unknown>) =>
      specifics?.ios ?? specifics?.native ?? specifics?.default,
  },
  TurboModuleRegistry: {
    get: () => null,
    getEnforcing: () => {
      throw new Error("native module not found");
    },
  },
  NativeEventEmitter: class {},
}));

const wifiState = { type: "WIFI", isConnected: true, isInternetReachable: true };
const cellularState = { type: "CELLULAR", isConnected: true, isInternetReachable: true };
const noneState = { type: "NONE", isConnected: false, isInternetReachable: false };

function cast<T>(value: unknown): T {
  return value as T;
}

/**
 * The app's lifecycle, driven against fake runtimes.
 *
 * Background/foreground and the network are properties of *this phone*, so what
 * this hook owes the runtimes is exactly two decisions: whether to hold their
 * sockets, and whether to recover them. The runtimes are therefore stand-ins —
 * what is under test is when the phone makes each call, not what a socket does
 * with it.
 */
describe("useAppLifecycle", () => {
  let lifecycle: { current: AppLifecycle };
  let renderer: ReactTestRenderer | null;
  const setRuntimesActive = jest.fn();
  const recoverAll = jest.fn();

  function Harness(): null {
    // The ref must survive re-renders, as it does in the provider.
    const networkAvailableRef = useRef<boolean | null>(null);
    lifecycle.current = useAppLifecycle({ networkAvailableRef, setRuntimesActive, recoverAll });
    return null;
  }

  function render(): void {
    act(() => { renderer = create(createElement(Harness)); });
  }

  async function drain(times = 40): Promise<void> {
    for (let i = 0; i < times; i += 1) await Promise.resolve();
  }

  async function flush(): Promise<void> {
    await act(async () => { await drain(); });
  }

  function appStateListeners(): ((state: string) => void)[] {
    return cast<jest.Mock>(AppState.addEventListener).mock.calls.map((c) => c[1]);
  }

  function networkListeners(): ((state: unknown) => void)[] {
    return cast<jest.Mock>(Network.addNetworkStateListener).mock.calls.map((c) => c[0]);
  }

  beforeEach(() => {
    jest.clearAllMocks();
    lifecycle = { current: undefined as unknown as AppLifecycle };
    renderer = null;
    cast<jest.Mock>(Network.getNetworkStateAsync).mockResolvedValue(wifiState);
    cast<{ currentState: string }>(AppState).currentState = "active";
  });

  afterEach(async () => {
    if (renderer) {
      await act(async () => { renderer!.unmount(); await drain(); });
      renderer = null;
    }
    endNativePresentation();
  });

  async function mount(): Promise<void> {
    render();
    await flush();
  }

  test("an inactive-only native overlay does not restart anything", async () => {
    await mount();
    await act(async () => {
      appStateListeners()[0]!("inactive");
      appStateListeners()[0]!("active");
      await drain();
    });
    expect(recoverAll).not.toHaveBeenCalled();
    expect(setRuntimesActive).not.toHaveBeenCalled();
  });

  test("the app is reported as in front until it goes to the background", async () => {
    await mount();
    expect(lifecycle.current.foreground).toBe(true);
    act(() => appStateListeners()[0]!("background"));
    expect(lifecycle.current.foreground).toBe(false);
    act(() => appStateListeners()[0]!("active"));
    expect(lifecycle.current.foreground).toBe(true);
  });

  test("real background releases every socket after the bounded quick-switch grace", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      setRuntimesActive.mockClear();
      act(() => appStateListeners()[0]!("background"));
      expect(setRuntimesActive).not.toHaveBeenCalled();
      await act(async () => { await jest.advanceTimersByTimeAsync(BACKGROUND_GRACE_MS); });
      expect(setRuntimesActive).toHaveBeenCalledWith(false);
    } finally { jest.useRealTimers(); }
  });

  test.each([false, true])(
    "resume enforces elapsed grace even when JS timers were suspended (picker=%s)",
    async (picker) => {
      jest.useFakeTimers();
      try {
        await mount();
        setRuntimesActive.mockClear();
        if (picker) beginNativePresentation();
        act(() => appStateListeners()[0]!("background"));
        if (picker) endNativePresentation();
        // Move wall time without running timers, as with a suspended JS VM.
        jest.setSystemTime(Date.now() + 60 * 60_000);
        expect(setRuntimesActive).not.toHaveBeenCalled();
        await act(async () => {
          appStateListeners()[0]!("active");
          await drain();
        });
        expect(setRuntimesActive.mock.calls).toEqual([[false], [true]]);
        expect(recoverAll).toHaveBeenCalledWith("foreground");
        setRuntimesActive.mockClear();
        await act(async () => { await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS); });
        expect(setRuntimesActive).not.toHaveBeenCalled();
      } finally { jest.useRealTimers(); }
    });

  test("a native picker's background transition keeps the connections", async () => {
    await mount();
    setRuntimesActive.mockClear();
    beginNativePresentation();
    act(() => appStateListeners()[0]!("background"));
    // The OS reported background because the picker paused the activity; the
    // links must survive the photo round trip.
    expect(setRuntimesActive).not.toHaveBeenCalled();
    act(() => appStateListeners()[0]!("active"));
    expect(setRuntimesActive).toHaveBeenCalledWith(true);
    endNativePresentation();
  });

  test("the picker grace releases the sockets if the user never comes back", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      setRuntimesActive.mockClear();
      beginNativePresentation();
      cast<{ currentState: string }>(AppState).currentState = "background";
      act(() => appStateListeners()[0]!("background"));
      expect(setRuntimesActive).not.toHaveBeenCalled();
      await act(async () => {
        await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS + 1);
      });
      // Still presented, still backgrounded: the bound wins.
      expect(setRuntimesActive).toHaveBeenCalledWith(false);
      endNativePresentation();
    } finally {
      jest.useRealTimers();
    }
  });

  test("a quick app switch cancels the delayed teardown", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      setRuntimesActive.mockClear();
      act(() => appStateListeners()[0]!("background"));
      expect(setRuntimesActive).not.toHaveBeenCalled();
      // Returning to the app mid-flow clears the pending bound.
      act(() => appStateListeners()[0]!("active"));
      setRuntimesActive.mockClear();
      await act(async () => {
        await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS + 1);
      });
      expect(setRuntimesActive).not.toHaveBeenCalled();
    } finally {
      jest.useRealTimers();
    }
  });

  test("a picker result delivered before resume cannot leave the sockets alive forever", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      setRuntimesActive.mockClear();
      beginNativePresentation();
      act(() => appStateListeners()[0]!("background"));
      endNativePresentation();
      await act(async () => { await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS); });
      expect(setRuntimesActive).toHaveBeenCalledWith(false);
    } finally { jest.useRealTimers(); }
  });

  test("a late foreground snapshot cannot overwrite a newer network event", async () => {
    await mount();
    let resolve!: (value: typeof noneState) => void;
    const stale = new Promise<typeof noneState>(done => { resolve = done; });
    cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValueOnce(stale);
    await act(async () => {
      appStateListeners()[0]!("background");
      appStateListeners()[0]!("active");
      networkListeners()[0]!(noneState);
      networkListeners()[0]!(wifiState);
      await drain();
    });
    await act(async () => { resolve(noneState); await drain(); });
    recoverAll.mockClear();
    await act(async () => { networkListeners()[0]!(wifiState); await drain(); });
    // If the stale offline snapshot won, this identical online event would
    // incorrectly trigger another network-restored recovery.
    expect(recoverAll).not.toHaveBeenCalled();
  });

  test("an older foreground query cannot overwrite a newer foreground query", async () => {
    await mount();
    let resolve!: (value: typeof noneState) => void;
    cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValueOnce(
      new Promise<typeof noneState>(done => { resolve = done; }),
    );
    await act(async () => {
      appStateListeners()[0]!("background");
      appStateListeners()[0]!("active");
      appStateListeners()[0]!("background");
      appStateListeners()[0]!("active");
      await drain();
      resolve(noneState);
      await drain();
    });
    recoverAll.mockClear();
    await act(async () => { networkListeners()[0]!(wifiState); await drain(); });
    expect(recoverAll).not.toHaveBeenCalled();
  });

  test("a hung foreground network query is bounded and ignores its eventual result", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      let resolve!: (value: typeof noneState) => void;
      cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValueOnce(
        new Promise<typeof noneState>(done => { resolve = done; }),
      );
      act(() => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
      });
      expect(recoverAll).toHaveBeenCalledWith("foreground");
      await act(async () => { await jest.advanceTimersByTimeAsync(4_000); });
      await act(async () => { resolve(noneState); await drain(); });
      recoverAll.mockClear();
      await act(async () => { networkListeners()[0]!(wifiState); await drain(); });
      expect(recoverAll).not.toHaveBeenCalled();
    } finally { jest.useRealTimers(); }
  });

  test("foreground recovery refreshes the network and recovers every runtime", async () => {
    await mount();
    await act(async () => {
      appStateListeners()[0]!("background");
      appStateListeners()[0]!("active");
      await drain();
    });
    expect(recoverAll).toHaveBeenCalledWith("foreground");
    expect(Network.getNetworkStateAsync).toHaveBeenCalled();
  });

  test("foreground recovery happens even when the network hint stays offline", async () => {
    await mount();
    networkListeners()[0]!(noneState);
    cast<jest.Mock>(Network.getNetworkStateAsync).mockResolvedValue(noneState);
    await act(async () => {
      appStateListeners()[0]!("background");
      appStateListeners()[0]!("active");
      expect(recoverAll).toHaveBeenCalledWith("foreground");
      await drain();
    });
    expect(recoverAll).toHaveBeenCalledTimes(1);
    expect(setRuntimesActive).toHaveBeenLastCalledWith(true);
  });

  test("offline hints do not tear anything down or add a polling loop", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      act(() => networkListeners()[0]!(noneState));
      expect(setRuntimesActive).not.toHaveBeenCalled();
      expect(recoverAll).not.toHaveBeenCalled();
      cast<jest.Mock>(Network.getNetworkStateAsync).mockClear();
      await act(async () => { await jest.advanceTimersByTimeAsync(30_000); });
      // The clients own transport retries; the lifecycle must not introduce a
      // competing 5s loop or reset their exponential backoff.
      expect(Network.getNetworkStateAsync).not.toHaveBeenCalled();
      expect(recoverAll).not.toHaveBeenCalled();
    } finally { jest.useRealTimers(); }
  });

  test("network-restored cannot start recovery while the app is in its background grace", async () => {
    await mount();
    cast<{ currentState: string }>(AppState).currentState = "background";
    await act(async () => {
      appStateListeners()[0]!("background");
      networkListeners()[0]!(noneState);
      networkListeners()[0]!(wifiState);
      await drain();
    });
    expect(recoverAll).not.toHaveBeenCalled();
  });

  test.each(["background", "unmount"])(
    "a late foreground network snapshot does not start another recovery after %s",
    async (stop) => {
      await mount();
      let resolve!: (value: typeof wifiState) => void;
      cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValueOnce(
        new Promise<typeof wifiState>(done => { resolve = done; }),
      );
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(recoverAll).toHaveBeenCalledTimes(1);
      await act(async () => {
        if (stop === "background") {
          cast<{ currentState: string }>(AppState).currentState = "background";
          appStateListeners()[0]!("background");
        } else {
          renderer!.unmount();
          renderer = null;
        }
        await drain();
      });
      await act(async () => { resolve(wifiState); await drain(); });
      expect(recoverAll).toHaveBeenCalledTimes(1);
    },
  );

  test("network restore triggers a recovery", async () => {
    await mount();
    networkListeners()[0]!(noneState);
    networkListeners()[0]!(wifiState);
    await flush();
    expect(recoverAll).toHaveBeenCalledWith("network-restored");
  });

  test("network path change triggers a network-changed recovery", async () => {
    await mount();
    networkListeners()[0]!(wifiState);
    networkListeners()[0]!(cellularState);
    await flush();
    expect(recoverAll).toHaveBeenCalledWith("network-changed");
  });

  test("network refresh failure logs a warning and falls back", async () => {
    const warn = jest.spyOn(console, "warn").mockImplementation(() => {});
    try {
      await mount();
      cast<jest.Mock>(Network.getNetworkStateAsync).mockRejectedValue(new Error("offline"));
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(recoverAll).toHaveBeenCalledWith("foreground");
      expect(warn).toHaveBeenCalledWith(
        "[remote] foreground network refresh failed",
        expect.anything(),
      );
    } finally { warn.mockRestore(); }
  });

  test("initial network probe failure is swallowed", async () => {
    cast<jest.Mock>(Network.getNetworkStateAsync).mockRejectedValue(new Error("offline"));
    render();
    await flush();
    expect(lifecycle.current.foreground).toBe(true);
  });
});
