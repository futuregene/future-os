import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import * as Network from "expo-network";
import { AppState } from "react-native";
import { RemoteApiError, type ConnectionState } from "../connectionState";
import {
  beginNativePresentation,
  endNativePresentation,
  NATIVE_PRESENTATION_GRACE_MS,
} from "../nativePresentation";
import { attemptPendingRevoke, claimPairingCode, serverRevoke } from "../pairing";
import { discardPendingContinuation } from "../pendingContinuationStorage";
import { discardPendingPrompt } from "../pendingPromptStorage";
import {
  clearCredentials,
  clearPendingRevoke,
  loadCredentials,
  loadPairedDesktops,
  loadPendingRevoke,
  renameDesktop,
  saveCredentials,
  savePendingRevoke,
} from "../storage";
import type { SyncEngine } from "../syncEngine";
import type {
  Presence,
  PresenceSession,
  RemoteCredentials,
  RemoteSession,
  StreamEvent,
} from "../types";
import { BACKGROUND_GRACE_MS, useRemoteConnection } from "../useRemoteConnection";

jest.mock("../storage", () => ({
  __esModule: true,
  clearCredentials: jest.fn(async () => {}),
  clearPendingRevoke: jest.fn(async () => {}),
  loadCredentials: jest.fn(async () => null),
  loadPairedDesktops: jest.fn(async () => []),
  loadPendingRevoke: jest.fn(async () => null),
  renameDesktop: jest.fn(async () => {}),
  saveCredentials: jest.fn(async () => {}),
  savePendingRevoke: jest.fn(async () => {}),
}));

jest.mock("../pairing", () => ({
  __esModule: true,
  attemptPendingRevoke: jest.fn(async () => {}),
  claimPairingCode: jest.fn(async () => null),
  serverRevoke: jest.fn(async () => {}),
}));

jest.mock("../pendingPromptStorage", () => ({
  __esModule: true,
  discardPendingPrompt: jest.fn(async () => {}),
}));

jest.mock("../pendingContinuationStorage", () => ({
  __esModule: true,
  discardPendingContinuation: jest.fn(async () => {}),
}));

jest.mock("../client", () => {
  class MockRemoteClient {
    credentials: unknown;
    callbacks: Record<string, (...args: never[]) => unknown>;
    close = jest.fn(async () => {});
    setAppActive = jest.fn();
    setVisibleSession = jest.fn();
    open = jest.fn(async () => {
      this.callbacks.onConnectionState?.("ready" as never);
      this.callbacks.onReconnected?.();
    });
    recoverNow = jest.fn(async () => {});
    request = jest.fn(async () => ({ success: true, data: {} }));
    constructor(credentials: unknown, callbacks: Record<string, (...args: never[]) => unknown>) {
      this.credentials = credentials;
      this.callbacks = callbacks;
    }
  }
  return { RemoteClient: MockRemoteClient };
});

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

interface MockClientCallbacks {
  onCredentials(c: RemoteCredentials): Promise<void>;
  onEvent(e: StreamEvent, sessionId: string): void;
  onEventDecodeFailure(sessionId: string, error: Error): void;
  onCatalogEpoch(epoch: string | undefined): void;
  onPresence(p: Presence): void;
  onSessions(s: PresenceSession[]): void;
  onWorkspaces(w: RemoteSession[], version: number): void;
  onFeatures(f: string[]): void;
  onConnectionState(s: ConnectionState): void;
  onReconnected(): void;
  onError(e: unknown): void;
}

interface MockClient {
  credentials: RemoteCredentials;
  callbacks: MockClientCallbacks;
  close: jest.Mock;
  setAppActive: jest.Mock;
  setVisibleSession: jest.Mock;
  open: jest.Mock;
  recoverNow: jest.Mock;
  request: jest.Mock;
}

const credentials: RemoteCredentials = {
  pairId: "pair",
  deviceId: "device",
  seed: "seed",
  userJwt: "jwt",
  refreshToken: "refresh",
  natsWsUrl: "wss://nats.test",
  tokenUrl: "https://example.test/auth/token",
  expectedDesktopId: "desktop",
  expectedDesktopPublicKey: "pubkey",
};

const presence: Presence = {
  online: true,
  pairId: "pair",
  bridgeInstanceId: "bridge",
  lastHeartbeatTs: 0,
};

function presenceSession(id: string, streaming = false): PresenceSession {
  return {
    sessionId: id,
    threadId: `t-${id}`,
    title: `Title ${id}`,
    streaming,
  };
}

type Options = Parameters<typeof useRemoteConnection>[0];
type Result = ReturnType<typeof useRemoteConnection>;

function makeOptions(): Options {
  return {
    clientRef: { current: null } as Options["clientRef"],
    credentialsRef: { current: null } as Options["credentialsRef"],
    selectedRef: { current: "" } as Options["selectedRef"],
    syncEngineRef: { current: null } as Options["syncEngineRef"],
    handleEvent: jest.fn(),
    reconcileSession: jest.fn(),
    applySessionSnapshot: jest.fn(),
    applySessionStreaming: jest.fn(),
    setWorkspaces: jest.fn(),
    refreshModels: jest.fn(async () => {}),
    refreshSessions: jest.fn(async () => {}),
    refreshSettings: jest.fn(async () => {}),
    refreshWorkspaces: jest.fn(async () => {}),
    closeConversation: jest.fn(),
    resetConversation: jest.fn(),
    resetCatalog: jest.fn(),
    resetTimeline: jest.fn(),
  };
}

const wifiState = {
  type: "WIFI",
  isConnected: true,
  isInternetReachable: true,
};
const cellularState = {
  type: "CELLULAR",
  isConnected: true,
  isInternetReachable: true,
};
const noneState = {
  type: "NONE",
  isConnected: false,
  isInternetReachable: false,
};

function cast<T>(value: unknown): T {
  return value as T;
}

describe("useRemoteConnection", () => {
  let options: Options;
  let result: { current: Result };
  let renderer: ReactTestRenderer | null;
  let consoleError: jest.SpyInstance;

  function Harness(): null {
    result.current = useRemoteConnection(options);
    return null;
  }

  function render(): void {
    act(() => {
      renderer = create(createElement(Harness));
    });
  }

  async function drain(times = 40): Promise<void> {
    for (let i = 0; i < times; i += 1) await Promise.resolve();
  }

  async function flush(): Promise<void> {
    await act(async () => { await drain(); });
  }

  function client(): MockClient {
    return cast<MockClient>(options.clientRef.current);
  }

  function appStateListeners(): ((state: string) => void)[] {
    return cast<jest.Mock>(AppState.addEventListener).mock.calls.map((c) => c[1]);
  }

  function networkListeners(): ((state: unknown) => void)[] {
    return cast<jest.Mock>(Network.addNetworkStateListener).mock.calls.map((c) => c[0]);
  }

  beforeEach(() => {
    consoleError = jest.spyOn(console, "error");
    jest.clearAllMocks();
    options = makeOptions();
    result = { current: undefined as unknown as Result };
    renderer = null;
    cast<jest.Mock>(loadPendingRevoke).mockResolvedValue(null);
    cast<jest.Mock>(loadCredentials).mockResolvedValue(null);
    cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([]);
    cast<jest.Mock>(claimPairingCode).mockResolvedValue(credentials);
    cast<jest.Mock>(Network.getNetworkStateAsync).mockResolvedValue(wifiState);
    cast<{ currentState: string }>(AppState).currentState = "active";
  });

  afterEach(async () => {
    if (renderer) {
      await act(async () => { renderer!.unmount(); await drain(); });
      renderer = null;
    }
    // The presentation depth is process-global; a failed assertion mid-test
    // must not leak a held connection into the next one.
    endNativePresentation();
    try { expect(consoleError).not.toHaveBeenCalled(); }
    finally { consoleError.mockRestore(); }
  });

  describe("multiple desktops", () => {
    const second = { ...credentials, pairId: "pair-2", expectedDesktopId: "desktop-2" };
    const desktops = [credentials, second].map((item) => ({
      desktopId: item.expectedDesktopId, pairId: item.pairId,
    }));

    beforeEach(() => {
      cast<jest.Mock>(loadCredentials).mockImplementation(async (id?: string) =>
        id === second.expectedDesktopId ? second : credentials);
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(desktops);
    });

    test("switches saved desktops without claiming or revoking a pair", async () => {
      render();
      await flush();
      const first = client();
      act(() => {
        first.callbacks.onPresence({ ...presence, lastHeartbeatTs: Date.now() });
        first.callbacks.onFeatures(["file_transfer_v1"]);
      });
      await act(async () => result.current.switchDesktop(second.expectedDesktopId));
      expect(loadCredentials).toHaveBeenCalledWith(second.expectedDesktopId);
      expect(result.current.credentials).toEqual(second);
      expect(result.current.desktops).toEqual(desktops);
      expect(first.close).toHaveBeenCalled();
      expect(first.request).not.toHaveBeenCalledWith({ type: "unpair" });
      expect(claimPairingCode).not.toHaveBeenCalled();
      expect(serverRevoke).not.toHaveBeenCalled();
      expect(result.current.presence).toBeNull();
      expect(result.current.desktopOnline).toBe(false);
      expect(result.current.capabilities.size).toBe(0);
      expect(options.resetCatalog).toHaveBeenCalled();
      expect(options.resetConversation).toHaveBeenCalled();
      expect(options.resetTimeline).toHaveBeenCalled();
      act(() => {
        first.callbacks.onPresence({ ...presence, unpaired: true });
        first.callbacks.onSessions([presenceSession("old")]);
      });
      await first.callbacks.onCredentials({ ...credentials, userJwt: "late" });
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(options.applySessionSnapshot).not.toHaveBeenCalled();
      expect(result.current.credentials).toEqual(second);
      expect(saveCredentials).not.toHaveBeenCalledWith(expect.objectContaining({ userJwt: "late" }));
    });

    test("adding a desktop does not unpair the previous desktop", async () => {
      render();
      await flush();
      cast<jest.Mock>(claimPairingCode).mockResolvedValue(second);
      await act(async () => result.current.pair("second-qr"));
      expect(result.current.credentials).toEqual(second);
      expect(result.current.desktops).toEqual(desktops);
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(serverRevoke).not.toHaveBeenCalled();
    });

    test("a failed QR claim leaves the previous connection usable", async () => {
      render();
      await flush();
      const first = client();
      cast<jest.Mock>(claimPairingCode).mockRejectedValueOnce(new Error("invalid_pairing_code"));
      await act(async () => {
        await expect(result.current.pair("bad-qr")).rejects.toThrow("invalid_pairing_code");
      });
      expect(result.current.phase).toBe("ready");
      expect(result.current.credentials).toEqual(credentials);
      expect(first.close).not.toHaveBeenCalled();
      act(() => first.callbacks.onSessions([presenceSession("still-connected")]));
      expect(options.applySessionSnapshot).toHaveBeenCalled();
    });

    test("desktop unpair notices preserve the local pairing until explicit removal", async () => {
      render();
      await flush();
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([desktops[1]]);
      act(() => client().callbacks.onPresence({ ...presence, unpaired: true }));
      await flush();
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(discardPendingPrompt).toHaveBeenCalledWith(credentials.pairId);
      expect(discardPendingContinuation).toHaveBeenCalledWith(credentials.pairId);
      expect(result.current.desktops).toEqual(desktops);
      expect(result.current.phase).toBe("revoked");
    });

    test("removing an inactive desktop leaves the active connection alone", async () => {
      render();
      await flush();
      const first = client();
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([desktops[0]]);
      await act(async () => result.current.removeDesktop(second.expectedDesktopId));
      expect(savePendingRevoke).toHaveBeenCalledWith(second);
      expect(clearCredentials).toHaveBeenCalledWith(second.pairId);
      expect(discardPendingPrompt).toHaveBeenCalledWith(second.pairId);
      expect(first.close).not.toHaveBeenCalled();
      expect(result.current.credentials).toEqual(credentials);
      expect(result.current.desktops).toEqual([desktops[0]]);
    });

    test("a failed saved-desktop lookup leaves the active connection usable", async () => {
      render();
      await flush();
      const first = client();
      cast<jest.Mock>(loadCredentials).mockRejectedValueOnce(new Error("disk error"));
      await act(async () => {
        await expect(result.current.switchDesktop(second.expectedDesktopId)).rejects.toThrow("disk error");
      });
      act(() => first.callbacks.onSessions([presenceSession("still-connected")]));
      expect(options.applySessionSnapshot).toHaveBeenCalled();
      expect(first.close).not.toHaveBeenCalled();
    });

    test("local unpair preserves the other saved desktop", async () => {
      render();
      await flush();
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([desktops[1]]);
      await act(async () => result.current.unpair());
      expect(clearCredentials).toHaveBeenCalledWith(credentials.pairId);
      expect(serverRevoke).toHaveBeenCalledWith(credentials);
      expect(result.current.desktops).toEqual([desktops[1]]);
      await act(async () => result.current.switchDesktop(second.expectedDesktopId));
      expect(result.current.credentials).toEqual(second);
    });
  });

  describe("mount lifecycle", () => {
    test("mounts unpaired when no credentials are stored", async () => {
      render();
      await flush();
      expect(result.current.phase).toBe("unpaired");
      expect(loadCredentials).toHaveBeenCalled();
    });

    test("keeps saved desktops available when there is no active selection", async () => {
      const saved = [{ desktopId: "desktop-1", pairId: "pair-1" }];
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(saved);
      render();
      await flush();
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.error).toBeNull();
      expect(result.current.credentials).toBeNull();
      expect(result.current.desktops).toEqual(saved);
      expect(options.clientRef.current).toBeNull();
    });

    test("keeps saved desktops available when the active credential cannot load", async () => {
      const saved = [{ desktopId: "desktop-1", pairId: "pair-1" }];
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(saved);
      cast<jest.Mock>(loadCredentials).mockRejectedValue(new Error("incomplete_desktop_credentials"));
      render();
      await flush();
      expect(result.current.phase).toBe("failed");
      expect(result.current.error).toBe("incomplete_desktop_credentials");
      expect(result.current.credentials).toBeNull();
      expect(result.current.desktops).toEqual(saved);
    });

    test("attempts and clears a pending revoke on mount", async () => {
      cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({
        pairId: "pair",
        deviceId: "device",
        seed: "seed",
        refreshToken: "refresh",
        tokenUrl: "https://example.test/auth/token",
      });
      render();
      await flush();
      expect(attemptPendingRevoke).toHaveBeenCalled();
      expect(clearPendingRevoke).toHaveBeenCalled();
      expect(result.current.phase).toBe("unpaired");
    });

    test("swallows a pending-revoke failure and continues", async () => {
      cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "pair" });
      cast<jest.Mock>(attemptPendingRevoke).mockRejectedValue(new Error("offline"));
      render();
      await flush();
      expect(result.current.phase).toBe("unpaired");
    });

    test("pending cloud revoke does not block loading an existing pairing", async () => {
      cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "old-pair" });
      cast<jest.Mock>(attemptPendingRevoke).mockReturnValue(new Promise(() => {}));
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
      expect(client().open).toHaveBeenCalled();
      expect(result.current.credentials).toEqual(credentials);
    });

    test("connects when stored credentials exist", async () => {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
      expect(client()).toBeTruthy();
      expect(client().open).toHaveBeenCalled();
      expect(options.refreshModels).toHaveBeenCalled();
      expect(options.refreshSessions).toHaveBeenCalled();
      expect(options.refreshWorkspaces).toHaveBeenCalled();
      expect(options.refreshSettings).toHaveBeenCalled();
    });

    test("surfaces a load failure as unpaired with an error", async () => {
      cast<jest.Mock>(loadCredentials).mockRejectedValue(new Error("boom"));
      render();
      await flush();
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.error).toBe("boom");
    });
  });

  describe("client callbacks", () => {
    async function mountConnected(): Promise<void> {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
    }

    test("onConnectionState maps every lifecycle state to a phase", async () => {
      await mountConnected();
      const c = client();
      act(() => c.callbacks.onConnectionState("ready"));
      expect(result.current.phase).toBe("ready");
      act(() => c.callbacks.onConnectionState("revoked"));
      expect(result.current.phase).toBe("revoked");
      act(() => c.callbacks.onConnectionState("unpaired"));
      expect(result.current.phase).toBe("revoked");
      act(() => c.callbacks.onConnectionState("refreshing"));
      expect(result.current.phase).toBe("refreshing");
      act(() => c.callbacks.onConnectionState("failed"));
      expect(result.current.phase).toBe("failed");
      act(() => c.callbacks.onConnectionState("connecting"));
      expect(result.current.phase).toBe("connecting");
      act(() => c.callbacks.onConnectionState("reconnecting"));
      expect(result.current.phase).toBe("reconnecting");
      act(() => c.callbacks.onConnectionState("stopped"));
      expect(result.current.phase).toBe("reconnecting");
    });

    test("onPresence updates presence and desktop online once ready", async () => {
      await mountConnected();
      const c = client();
      act(() => c.callbacks.onConnectionState("ready"));
      act(() => c.callbacks.onPresence(presence));
      expect(result.current.presence).toBe(presence);
    });

    test("handshake presence becomes usable as soon as ready arrives", async () => {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
      act(() => client().callbacks.onConnectionState("reconnecting"));
      act(() => client().callbacks.onPresence({ ...presence, lastHeartbeatTs: Date.now() / 1000 }));
      expect(result.current.desktopOnline).toBe(false);
      act(() => client().callbacks.onConnectionState("ready"));
      expect(result.current.desktopOnline).toBe(true);
    });

    test("onPresence before ready clears desktop online", async () => {
      await mountConnected();
      const c = client();
      act(() => c.callbacks.onPresence(presence));
      expect(result.current.presence).toBe(presence);
      expect(result.current.desktopOnline).toBe(false);
    });

    test("onPresence unpaired tears down but preserves the local pairing", async () => {
      await mountConnected();
      const c = client();
      await act(async () => { c.callbacks.onPresence({ ...presence, unpaired: true }); await drain(); });
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(discardPendingContinuation).toHaveBeenCalled();
      expect(discardPendingPrompt).toHaveBeenCalled();
      expect(options.resetCatalog).toHaveBeenCalled();
      expect(options.resetConversation).toHaveBeenCalled();
      expect(options.resetTimeline).toHaveBeenCalled();
      expect(result.current.phase).toBe("revoked");
      expect(result.current.credentials).toEqual(credentials);
      expect(options.clientRef.current).toBeNull();
    });

    test.each(["user_disconnect", "system_sleep", "app_exit"])("desktop %s stops retries and clears the mirrored catalogue", async (reason) => {
      await mountConnected();
      const c = client();
      act(() =>
        c.callbacks.onPresence({
          ...presence,
          online: false,
          disconnected: true,
          reason,
        }),
      );
      expect(c.close).toHaveBeenCalledWith("UserInitiated");
      expect(options.clientRef.current).toBeNull();
      expect(options.resetCatalog).toHaveBeenCalled();
      expect(options.resetConversation).toHaveBeenCalled();
      expect(options.resetTimeline).toHaveBeenCalled();
      expect(result.current.phase).toBe("stopped");
      expect(result.current.credentials).toEqual(credentials);
    });

    test("onSessions closes the conversation when the selected session is gone", async () => {
      await mountConnected();
      options.selectedRef.current = "s1";
      act(() => client().callbacks.onSessions([presenceSession("s2")]));
      expect(options.applySessionSnapshot).toHaveBeenCalledWith(
        [
          {
            sessionId: "s2",
            threadId: "t-s2",
            title: "Title s2",
            streaming: false,
          },
        ],
        undefined,
      );
      expect(options.closeConversation).toHaveBeenCalled();
    });

    test("onSessions applies streaming for the selected session", async () => {
      await mountConnected();
      options.selectedRef.current = "s1";
      act(() => client().callbacks.onSessions([presenceSession("s1", true)]));
      expect(options.applySessionStreaming).toHaveBeenCalledWith("s1", true);
    });

    test("onSessions with an empty list applies a non-streaming selected session", async () => {
      await mountConnected();
      options.selectedRef.current = "s1";
      act(() => client().callbacks.onSessions([]));
      expect(options.applySessionStreaming).toHaveBeenCalledWith("s1", false);
    });

    test("onFeatures updates capabilities", async () => {
      await mountConnected();
      act(() => client().callbacks.onFeatures(["file_transfer_v1", "prompt_receipt_v1"]));
      expect(result.current.fileTransferSupported).toBe(true);
      expect(result.current.promptReceiptSupported).toBe(true);
    });

    test("onReconnected restarts sync and refreshes catalogues", async () => {
      await mountConnected();
      options.syncEngineRef.current = {
        restartAll: jest.fn(),
      } as unknown as SyncEngine;
      await act(async () => client().callbacks.onReconnected());
      expect(options.syncEngineRef.current!.restartAll).toHaveBeenCalledWith("reconnect");
      expect(options.refreshModels).toHaveBeenCalled();
      expect(options.refreshSessions).toHaveBeenCalled();
      expect(options.refreshWorkspaces).toHaveBeenCalled();
    });

    test("onCredentials persists a refreshed credential set", async () => {
      await mountConnected();
      const refreshed = { ...credentials, userJwt: "new-jwt" };
      await act(async () => {
        await client().callbacks.onCredentials(refreshed);
      });
      expect(saveCredentials).toHaveBeenCalledWith(refreshed);
      expect(result.current.credentials).toEqual(refreshed);
    });

    test("onEventDecodeFailure reconciles the session", async () => {
      await mountConnected();
      act(() => client().callbacks.onEventDecodeFailure("s1", new Error("decode")));
      expect(options.reconcileSession).toHaveBeenCalledWith("s1", "resend");
    });

    test("recordError ignores transport errors and records non-transport errors", async () => {
      await mountConnected();
      act(() => result.current.recordError(new Error("plain transport")));
      expect(result.current.error).toBeNull();
      act(() => result.current.recordError(new Error("invalid_jwt")));
      expect(result.current.error).toBe("invalid_jwt");
    });

    test("onError routes through recordError", async () => {
      await mountConnected();
      act(() => client().callbacks.onError(new Error("invalid_jwt")));
      expect(result.current.error).toBe("invalid_jwt");
    });
  });

  describe("app state and network effects", () => {
    async function mountConnected(): Promise<void> {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
    }

    test("an inactive-only native overlay does not resync a healthy connection", async () => {
      await mountConnected();
      const connected = client();
      cast<jest.Mock>(options.refreshSessions).mockClear();
      await act(async () => {
        appStateListeners()[0]!("inactive");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(connected.recoverNow).not.toHaveBeenCalled();
      expect(options.refreshSessions).not.toHaveBeenCalled();
    });

    test("missing encrypted heartbeats requests secure recovery, not just a broker ping", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        act(() => client().callbacks.onPresence({ ...presence, lastHeartbeatTs: Date.now() / 1000 }));
        await act(async () => { await jest.advanceTimersByTimeAsync(20_000); });
        expect(client().recoverNow).toHaveBeenCalledWith("presence-stale");
      } finally { jest.useRealTimers(); }
    });

    test("real background releases the socket after the bounded quick-switch grace", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        client().setAppActive.mockClear();
        act(() => appStateListeners()[0]!("background"));
        expect(client().setAppActive).not.toHaveBeenCalled();
        await act(async () => { await jest.advanceTimersByTimeAsync(BACKGROUND_GRACE_MS); });
        expect(client().setAppActive).toHaveBeenCalledWith(false);
      } finally { jest.useRealTimers(); }
    });

    test.each([false, true])("resume enforces elapsed grace even when JS timers were suspended (picker=%s)", async (picker) => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        client().setAppActive.mockClear();
        if (picker) beginNativePresentation();
        act(() => appStateListeners()[0]!("background"));
        if (picker) endNativePresentation();
        // Move wall time without running timers, as with a suspended JS VM.
        jest.setSystemTime(Date.now() + 60 * 60_000);
        expect(client().setAppActive).not.toHaveBeenCalled();
        await act(async () => {
          appStateListeners()[0]!("active");
          await drain();
        });
        expect(client().setAppActive.mock.calls).toEqual([[false], [true]]);
        expect(client().recoverNow).toHaveBeenCalledWith("foreground");
        client().setAppActive.mockClear();
        await act(async () => { await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS); });
        expect(client().setAppActive).not.toHaveBeenCalled();
      } finally { jest.useRealTimers(); }
    });

    test("a native picker's background transition keeps the connection", async () => {
      await mountConnected();
      client().setAppActive.mockClear();
      beginNativePresentation();
      act(() => appStateListeners()[0]!("background"));
      // The OS reported background because the picker paused the activity; the
      // link must survive the photo round trip.
      expect(client().setAppActive).not.toHaveBeenCalled();
      act(() => appStateListeners()[0]!("active"));
      expect(client().setAppActive).toHaveBeenCalledWith(true);
      endNativePresentation();
    });

    test("the picker grace releases the client if the user never comes back", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        client().setAppActive.mockClear();
        beginNativePresentation();
        cast<{ currentState: string }>(AppState).currentState = "background";
        act(() => appStateListeners()[0]!("background"));
        expect(client().setAppActive).not.toHaveBeenCalled();
        await act(async () => {
          await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS + 1);
        });
        // Still presented, still backgrounded: the bound wins.
        expect(client().setAppActive).toHaveBeenCalledWith(false);
        endNativePresentation();
      } finally {
        jest.useRealTimers();
      }
    });

    test("a quick app switch cancels the delayed teardown", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        client().setAppActive.mockClear();
        act(() => appStateListeners()[0]!("background"));
        expect(client().setAppActive).not.toHaveBeenCalled();
        // Returning to the app mid-flow clears the pending bound.
        act(() => appStateListeners()[0]!("active"));
        client().setAppActive.mockClear();
        await act(async () => {
          await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS + 1);
        });
        expect(client().setAppActive).not.toHaveBeenCalled();
      } finally {
        jest.useRealTimers();
      }
    });

    test("a picker result delivered before resume cannot leave the socket alive forever", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        client().setAppActive.mockClear();
        beginNativePresentation();
        act(() => appStateListeners()[0]!("background"));
        endNativePresentation();
        await act(async () => { await jest.advanceTimersByTimeAsync(NATIVE_PRESENTATION_GRACE_MS); });
        expect(client().setAppActive).toHaveBeenCalledWith(false);
      } finally { jest.useRealTimers(); }
    });

    test("a late foreground snapshot cannot overwrite a newer network event", async () => {
      await mountConnected();
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
      client().recoverNow.mockClear();
      await act(async () => { networkListeners()[0]!(wifiState); await drain(); });
      // If the stale offline snapshot won, this identical online event would
      // incorrectly trigger another network-restored recovery.
      expect(client().recoverNow).not.toHaveBeenCalled();
    });

    test("an older foreground query cannot overwrite a newer foreground query", async () => {
      await mountConnected();
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
      client().recoverNow.mockClear();
      await act(async () => { networkListeners()[0]!(wifiState); await drain(); });
      expect(client().recoverNow).not.toHaveBeenCalled();
    });

    test("a hung foreground network query is bounded and ignores its eventual result", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        let resolve!: (value: typeof noneState) => void;
        cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValueOnce(
          new Promise<typeof noneState>(done => { resolve = done; }),
        );
        act(() => {
          appStateListeners()[0]!("background");
          appStateListeners()[0]!("active");
        });
        expect(client().recoverNow).toHaveBeenCalledWith("foreground");
        await act(async () => { await jest.advanceTimersByTimeAsync(4_000); });
        await act(async () => { resolve(noneState); await drain(); });
        client().recoverNow.mockClear();
        await act(async () => { networkListeners()[0]!(wifiState); await drain(); });
        expect(client().recoverNow).not.toHaveBeenCalled();
      } finally { jest.useRealTimers(); }
    });

    test("foreground recovery refreshes network and recovers the client", async () => {
      options.selectedRef.current = "s1";
      await mountConnected();
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(client().recoverNow).toHaveBeenCalledWith("foreground");
      expect(options.refreshSettings).toHaveBeenCalledTimes(2);
      expect(client().setVisibleSession.mock.calls.slice(-2)).toEqual([[""], ["s1"]]);
    });

    test("foreground immediately recovers even when the network hint stays offline", async () => {
      await mountConnected();
      networkListeners()[0]!(noneState);
      cast<jest.Mock>(Network.getNetworkStateAsync).mockResolvedValue(noneState);
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        expect(client().recoverNow).toHaveBeenCalledWith("foreground");
        await drain();
      });
      expect(client().recoverNow).toHaveBeenCalledTimes(1);
      expect(client().setAppActive).toHaveBeenLastCalledWith(true);
    });

    test("offline hints do not tear down a healthy connection or add a polling loop", async () => {
      jest.useFakeTimers();
      try {
        await mountConnected();
        const connected = client();
        act(() => networkListeners()[0]!(noneState));
        expect(connected.close).not.toHaveBeenCalled();
        expect(connected.recoverNow).not.toHaveBeenCalled();
        expect(result.current.phase).toBe("ready");
        act(() => connected.callbacks.onConnectionState("reconnecting"));
        cast<jest.Mock>(Network.getNetworkStateAsync).mockClear();
        await act(async () => { await jest.advanceTimersByTimeAsync(30_000); });
        // The client owns transport retries; the hook must not introduce a
        // competing 5s loop or reset its exponential backoff.
        expect(Network.getNetworkStateAsync).not.toHaveBeenCalled();
        expect(connected.recoverNow).not.toHaveBeenCalled();
      } finally { jest.useRealTimers(); }
    });

    test("network-restored cannot start recovery while the app is in its background grace", async () => {
      await mountConnected();
      cast<{ currentState: string }>(AppState).currentState = "background";
      await act(async () => {
        appStateListeners()[0]!("background");
        networkListeners()[0]!(noneState);
        networkListeners()[0]!(wifiState);
        await drain();
      });
      expect(client().recoverNow).not.toHaveBeenCalled();
    });

    test.each(["background", "stopped", "unmount"])(
      "a late foreground network snapshot does not start another recovery after %s",
      async stop => {
        await mountConnected();
        const connected = client();
        let resolve!: (value: typeof wifiState) => void;
        cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValueOnce(
          new Promise<typeof wifiState>(done => { resolve = done; }),
        );
        await act(async () => {
          appStateListeners()[0]!("background");
          appStateListeners()[0]!("active");
          await drain();
        });
        expect(connected.recoverNow).toHaveBeenCalledTimes(1);
        await act(async () => {
          if (stop === "background") {
            cast<{ currentState: string }>(AppState).currentState = "background";
            appStateListeners()[0]!("background");
          } else if (stop === "unmount") {
            renderer!.unmount();
            renderer = null;
          } else {
            connected.callbacks.onPresence({ ...presence, disconnected: true, reason: "user_disconnect" });
          }
          await drain();
        });
        await act(async () => { resolve(wifiState); await drain(); });
        expect(connected.recoverNow).toHaveBeenCalledTimes(1);
      },
    );

    test("foreground socket replacement uses the same recovery pass as onReconnected", async () => {
      await mountConnected();
      cast<jest.Mock>(options.refreshSettings).mockClear();
      client().recoverNow.mockImplementationOnce(async () => {
        client().callbacks.onReconnected();
        // Let the callback's recovery finish before recoverNow returns: checking
        // only for an in-flight promise would start a redundant second pass.
        await drain();
      });
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(options.refreshSettings).toHaveBeenCalledTimes(1);
    });

    test("network restore triggers a recovery", async () => {
      await mountConnected();
      networkListeners()[0]!(noneState);
      networkListeners()[0]!(wifiState);
      await flush();
      expect(client().recoverNow).toHaveBeenCalledWith("network-restored");
    });

    test("network path change triggers a network-changed recovery", async () => {
      await mountConnected();
      networkListeners()[0]!(wifiState);
      networkListeners()[0]!(cellularState);
      await flush();
      expect(client().recoverNow).toHaveBeenCalledWith("network-changed");
    });

    test("network refresh failure logs a warning and falls back", async () => {
      await mountConnected();
      cast<jest.Mock>(Network.getNetworkStateAsync).mockRejectedValue(new Error("offline"));
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(client().recoverNow).toHaveBeenCalledWith("foreground");
    });

    test("foreground recovery records a recovery failure", async () => {
      await mountConnected();
      client().recoverNow.mockRejectedValueOnce(new Error("invalid_jwt"));
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(result.current.error).toBe("invalid_jwt");
    });

    test("initial network probe failure is swallowed", async () => {
      cast<jest.Mock>(Network.getNetworkStateAsync).mockRejectedValue(new Error("offline"));
      render();
      await flush();
      expect(result.current.phase).toBe("unpaired");
    });
  });

  describe("pair / reconnect / unpair", () => {
    async function mountConnected(): Promise<void> {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
    }

    test("late claim cannot reconnect after local unpair", async () => {
      render();
      await flush();
      let resolve!: (value: RemoteCredentials) => void;
      cast<jest.Mock>(claimPairingCode).mockReturnValue(
        new Promise((done) => {
          resolve = done;
        }),
      );
      let pairing!: Promise<void>;
      act(() => {
        pairing = result.current.pair("code");
      });
      await act(async () => {
        await result.current.unpair();
      });
      await act(async () => {
        resolve(credentials);
        await pairing;
      });
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.credentials).toBeNull();
      expect(options.clientRef.current).toBeNull();
    });

    test("pair claims a code and connects", async () => {
      render();
      await flush();
      await act(async () => {
        await result.current.pair("code");
      });
      expect(claimPairingCode).toHaveBeenCalledWith("code", expect.any(AbortSignal));
      expect(client()).toBeTruthy();
      expect(client().open).toHaveBeenCalled();
    });

    test("pair failure surfaces the error and throws", async () => {
      render();
      await flush();
      cast<jest.Mock>(claimPairingCode).mockRejectedValue(new Error("bad code"));
      await act(async () => {
        await expect(result.current.pair("code")).rejects.toThrow("bad code");
      });
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.error).toBe("bad code");
    });

    test("reconnect with no credentials goes unpaired", async () => {
      render();
      await flush();
      await act(async () => {
        await result.current.reconnect();
      });
      expect(result.current.phase).toBe("unpaired");
      expect(client()).toBeFalsy();
    });

    test("reconnect loads stored credentials and connects", async () => {
      render();
      await flush();
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      await act(async () => {
        await result.current.reconnect();
      });
      expect(client()).toBeTruthy();
      expect(client().open).toHaveBeenCalled();
    });

    test("reconnect with an invalid JWT preserves pairing metadata", async () => {
      await mountConnected();
      cast<jest.Mock>(saveCredentials).mockRejectedValueOnce(new Error("invalid_jwt"));
      await act(async () => {
        await result.current.reconnect();
      });
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(result.current.credentials).toEqual(credentials);
      expect(result.current.phase).toBe("revoked");
      expect(result.current.error).toBe("invalid_jwt");
    });

    test("reconnect surfaces a non-JWT failure", async () => {
      await mountConnected();
      cast<jest.Mock>(saveCredentials).mockRejectedValueOnce(new Error("network down"));
      await act(async () => {
        await result.current.reconnect();
      });
      expect(result.current.error).toBe("network down");
    });

    test("unpair revokes server-side and clears local state", async () => {
      await mountConnected();
      const c = client();
      await act(async () => {
        await result.current.unpair();
      });
      expect(c.request).toHaveBeenCalledWith({ type: "unpair" });
      expect(options.resetTimeline).toHaveBeenCalled();
      expect(serverRevoke).toHaveBeenCalledWith(credentials);
      expect(clearCredentials).toHaveBeenCalled();
      expect(discardPendingPrompt).toHaveBeenCalled();
      expect(options.resetCatalog).toHaveBeenCalled();
      expect(options.resetConversation).toHaveBeenCalled();
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.error).toBeNull();
    });

    test("unpair queues a pending revoke when server revoke fails", async () => {
      await mountConnected();
      cast<jest.Mock>(serverRevoke).mockRejectedValue(new Error("offline"));
      await act(async () => {
        await result.current.unpair();
      });
      expect(savePendingRevoke).toHaveBeenCalledWith(
        expect.objectContaining({
          pairId: "pair",
          deviceId: "device",
          seed: "seed",
          refreshToken: "refresh",
          tokenUrl: "https://example.test/auth/token",
        }),
      );
    });

    test("unpair clears a pending prompt", async () => {
      await mountConnected();
      await act(async () => {
        await result.current.unpair();
      });
      expect(discardPendingPrompt).toHaveBeenCalled();
    });

    test("clearError resets the error", async () => {
      await mountConnected();
      act(() => result.current.recordError(new Error("invalid_jwt")));
      expect(result.current.error).toBe("invalid_jwt");
      act(() => result.current.clearError());
      expect(result.current.error).toBeNull();
    });
  });

  describe("connect-time guards and interval", () => {
    test("pauses the client when the app is backgrounded at connect time", async () => {
      cast<{ currentState: string }>(AppState).currentState = "background";
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
      expect(client().setAppActive).toHaveBeenCalled();
    });

    test("manual reconnect attempts a real connection despite an offline hint", async () => {
      render();
      await flush();
      networkListeners()[0]!(noneState);
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      await act(async () => {
        await result.current.reconnect();
      });
      expect(client().open).toHaveBeenCalledTimes(1);
      expect(result.current.phase).toBe("ready");
    });

    test("interval refreshes presence while ready", async () => {
      jest.useFakeTimers();
      try {
        cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
        render();
        await flush();
        act(() => client().callbacks.onConnectionState("ready"));
        await act(async () => {
          await jest.advanceTimersByTimeAsync(10_000);
        });
        expect(result.current.desktopOnline).toBe(false);
      } finally {
        jest.useRealTimers();
      }
    });

    test("interval clears desktop online while not ready", async () => {
      jest.useFakeTimers();
      try {
        render();
        await flush();
        await act(async () => {
          await jest.advanceTimersByTimeAsync(10_000);
        });
        expect(result.current.desktopOnline).toBe(false);
      } finally {
        jest.useRealTimers();
      }
    });
  });

  describe("transport callbacks, revoke retries and desktop bookkeeping", () => {
    async function mountConnected(): Promise<void> {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
    }

    test("live events, catalogue epochs and workspace lists reach their sinks on the current generation only", async () => {
      const setCatalogEpoch = jest.fn();
      options.setCatalogEpoch = setCatalogEpoch;
      await mountConnected();
      const current = client();
      const event = { type: "agent_end", data: "{}" } as StreamEvent;
      act(() => current.callbacks.onEvent(event, "s1"));
      expect(options.handleEvent).toHaveBeenCalledWith(event, "s1");
      act(() => current.callbacks.onCatalogEpoch("epoch-1"));
      expect(setCatalogEpoch).toHaveBeenCalledWith("epoch-1");
      const workspaces = [{ sessionId: "s1" } as RemoteSession];
      act(() => current.callbacks.onWorkspaces(workspaces, 3));
      expect(options.setWorkspaces).toHaveBeenCalledWith(workspaces, 3);

      // A fresh connect bumps the generation. The replaced client can still be
      // draining its socket, and every frame it delivers belongs to the
      // connection that is gone.
      await act(async () => {
        await result.current.reconnect();
        await drain();
      });
      expect(client()).not.toBe(current);
      cast<jest.Mock>(options.handleEvent).mockClear();
      cast<jest.Mock>(options.setWorkspaces).mockClear();
      setCatalogEpoch.mockClear();
      act(() => current.callbacks.onEvent(event, "s1"));
      act(() => current.callbacks.onCatalogEpoch("stale"));
      act(() => current.callbacks.onWorkspaces(workspaces, 4));
      expect(options.handleEvent).not.toHaveBeenCalled();
      expect(setCatalogEpoch).not.toHaveBeenCalled();
      expect(options.setWorkspaces).not.toHaveBeenCalled();
    });

    test("an agent that comes back while connected restarts the lanes and re-reads state", async () => {
      const restartAll = jest.fn();
      options.syncEngineRef.current = cast({ restartAll });
      await mountConnected();
      const c = client();
      act(() => c.callbacks.onPresence({ ...presence, agentAvailable: false }));
      await flush();
      restartAll.mockClear();
      cast<jest.Mock>(options.refreshSessions).mockClear();
      act(() => c.callbacks.onPresence({
        ...presence, agentAvailable: true, lastHeartbeatTs: Date.now(),
      }));
      await flush();
      // The agent going away and coming back invalidates every lane the phone
      // was holding: recovering only the catalogue would leave the timeline
      // pinned to the process that died.
      expect(restartAll).toHaveBeenCalledWith("reconnect");
      expect(options.refreshSessions).toHaveBeenCalled();
    });

    test("a 4xx revoke failure is terminal and is not re-sent", async () => {
      jest.useFakeTimers();
      try {
        cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "pair" });
        cast<jest.Mock>(attemptPendingRevoke).mockRejectedValue(
          new RemoteApiError("forbidden", "PA002", 403),
        );
        render();
        await flush();
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(1);
        // The drainer runs again on its interval. A 4xx the server will keep
        // refusing must not be re-sent forever; a transport error must be,
        // which the next test pins.
        await act(async () => { jest.advanceTimersByTime(30_000); await drain(); });
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(1);
      } finally {
        jest.useRealTimers();
      }
    });

    test("a transport revoke failure is retried on the next drain", async () => {
      jest.useFakeTimers();
      try {
        cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "pair" });
        cast<jest.Mock>(attemptPendingRevoke).mockRejectedValue(new Error("offline"));
        render();
        await flush();
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(1);
        await act(async () => { jest.advanceTimersByTime(30_000); await drain(); });
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(2);
      } finally {
        jest.useRealTimers();
      }
    });

    test("a recovery that fails after a reconnect is reported rather than swallowed", async () => {
      const warn = jest.spyOn(console, "warn").mockImplementation(() => {});
      try {
        options.syncEngineRef.current = cast({
          restartAll: () => { throw new RemoteApiError("recovery exploded", "invalid_remote_credential", 401); },
        });
        await mountConnected();
        act(() => client().callbacks.onReconnected());
        await flush();
        // The reconnect is only complete once the lanes it restarts are
        // actually rebuilt; a failure there must reach the user.
        expect(result.current.error).toBe("recovery exploded");
        expect(warn).toHaveBeenCalledWith("[remote] unexpected non-transport error", expect.anything());
      } finally {
        warn.mockRestore();
      }
    });

    test("reconnect with known desktops but no stored credentials reports the incomplete pairing", async () => {
      const saved = [{ desktopId: "desktop-1", pairId: "pair-1" }];
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(saved);
      render();
      await flush();
      expect(result.current.desktops).toEqual(saved);
      await act(async () => {
        await result.current.reconnect();
        await drain();
      });
      // A desktop is listed but nothing can open it: saying so beats silently
      // returning to the unpaired screen, which reads as a lost pairing.
      expect(result.current.error).toBe("incomplete_desktop_credentials");
      expect(result.current.phase).toBe("failed");
    });

    test("renaming a desktop persists it and re-reads the list", async () => {
      const saved = [{ desktopId: "desktop-1", pairId: "pair-1" }];
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(saved);
      render();
      await flush();
      const renamed = [{ desktopId: "desktop-1", pairId: "pair-1", name: "Lab" }];
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(renamed);
      await act(async () => {
        await result.current.renameDesktop("desktop-1", "Lab");
        await drain();
      });
      expect(renameDesktop).toHaveBeenCalledWith("desktop-1", "Lab");
      expect(loadPairedDesktops).toHaveBeenCalledTimes(2);
      expect(result.current.desktops).toEqual(renamed);
    });
  });

  /**
   * A connection attempt is not a transaction: the user can unpair, switch
   * desktop, or leave the screen at any await. Every one of those has to stop
   * the attempt where it is, and the callbacks of the replaced client must not
   * be able to drive the new state.
   */
  describe("interrupted and superseded attempts", () => {
    function deferred<T>() {
      let resolve!: (value: T) => void;
      let reject!: (reason?: unknown) => void;
      const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
      return { promise, resolve, reject };
    }

    async function mountConnected(): Promise<void> {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
    }

    test("an unpair that lands while the previous client is closing stops the reconnect", async () => {
      await mountConnected();
      const previous = client();
      const closing = deferred<void>();
      previous.close.mockReturnValue(closing.promise);
      cast<jest.Mock>(saveCredentials).mockClear();
      let reconnecting: Promise<void> | undefined;
      await act(async () => {
        reconnecting = result.current.reconnect();
        await drain();
      });
      // The user unpairs while the old socket is still being closed.
      await act(async () => { await result.current.unpair(); await drain(); });
      closing.resolve();
      await act(async () => { await reconnecting; await drain(); });
      // The superseded attempt must not persist or adopt credentials for a
      // pairing the user just removed.
      expect(cast<jest.Mock>(saveCredentials)).not.toHaveBeenCalled();
      expect(options.credentialsRef.current).toBeNull();
      expect(result.current.phase).toBe("unpaired");
    });

    test("an unpair that lands while credentials are being saved stops the reconnect", async () => {
      await mountConnected();
      const saving = deferred<void>();
      cast<jest.Mock>(saveCredentials).mockReturnValue(saving.promise);
      let reconnecting: Promise<void> | undefined;
      await act(async () => {
        reconnecting = result.current.reconnect();
        await drain();
      });
      await act(async () => { await result.current.unpair(); await drain(); });
      saving.resolve();
      await act(async () => { await reconnecting; await drain(); });
      // Nothing may re-enter the connecting phase after the unpair.
      expect(result.current.phase).toBe("unpaired");
      expect(options.credentialsRef.current).toBeNull();
      expect(options.clientRef.current).toBeNull();
    });

    test("an unpair that lands while the desktop list reloads stops the reconnect", async () => {
      await mountConnected();
      const listed = deferred<unknown>();
      cast<jest.Mock>(loadPairedDesktops).mockReturnValueOnce(listed.promise);
      let reconnecting: Promise<void> | undefined;
      await act(async () => {
        reconnecting = result.current.reconnect();
        await drain();
      });
      await act(async () => { await result.current.unpair(); await drain(); });
      listed.resolve([]);
      await act(async () => { await reconnecting; await drain(); });
      expect(result.current.phase).toBe("unpaired");
      expect(options.credentialsRef.current).toBeNull();
    });

    test("a replaced client cannot write credentials or reconcile sessions", async () => {
      await mountConnected();
      const stale = client();
      await act(async () => { await result.current.reconnect(); await drain(); });
      expect(client()).not.toBe(stale);
      cast<jest.Mock>(saveCredentials).mockClear();
      await act(async () => {
        await stale.callbacks.onCredentials({ ...credentials, userJwt: "rotated" });
        await drain();
      });
      // The replaced client is still draining its socket; its token rotation
      // belongs to a pairing this screen no longer shows.
      expect(cast<jest.Mock>(saveCredentials)).not.toHaveBeenCalled();
      act(() => stale.callbacks.onEventDecodeFailure("s1", new Error("bad frame")));
      expect(options.reconcileSession).not.toHaveBeenCalled();
    });

    test("a replaced client cannot publish connection state or recovery", async () => {
      await mountConnected();
      const stale = client();
      act(() => stale.callbacks.onConnectionState("ready"));
      await act(async () => { await result.current.reconnect(); await drain(); });
      const phase = result.current.phase;
      cast<jest.Mock>(options.refreshSessions).mockClear();
      cast<jest.Mock>(options.refreshModels).mockClear();
      act(() => stale.callbacks.onConnectionState("failed"));
      act(() => stale.callbacks.onReconnected());
      await flush();
      // A late phase would flip the screen to a failure the user's desktop did
      // not report, and a late recovery would pull against the new socket.
      expect(result.current.phase).toBe(phase);
      expect(options.refreshSessions).not.toHaveBeenCalled();
      expect(options.refreshModels).not.toHaveBeenCalled();
    });

    test("a snapshot the catalogue rejects does not touch the conversation", async () => {
      await mountConnected();
      options.selectedRef.current = "s1";
      cast<jest.Mock>(options.applySessionSnapshot).mockReturnValue(false);
      act(() => client().callbacks.onSessions([presenceSession("s2")]));
      // `false` means the snapshot was a duplicate or out of order: acting on it
      // would close a conversation that is still current.
      expect(options.closeConversation).not.toHaveBeenCalled();
      expect(options.applySessionStreaming).not.toHaveBeenCalled();
    });

    test("a bootstrap superseded while reading the desktop registry never loads credentials", async () => {
      const listed = deferred<unknown>();
      cast<jest.Mock>(loadPairedDesktops).mockReturnValue(listed.promise);
      render();
      await flush();
      await act(async () => { renderer!.unmount(); renderer = null; await drain(); });
      listed.resolve([]);
      await flush();
      expect(loadCredentials).not.toHaveBeenCalled();
    });

    test("a bootstrap failure that lands after it was superseded is not reported", async () => {
      const listed = deferred<unknown>();
      cast<jest.Mock>(loadPairedDesktops).mockReturnValueOnce(listed.promise);
      render();
      await flush();
      // The bootstrap is parked on the registry read. A new callbacks identity
      // supersedes it (connect/recoverState are rebuilt), so the effect tears
      // down and starts a fresh attempt that finishes unpaired.
      options.refreshSessions = jest.fn(async () => {});
      act(() => { renderer!.update(createElement(Harness)); });
      await flush();
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.error).toBeNull();
      // The abandoned attempt's read now fails. Its failure belongs to a
      // bootstrap that no longer owns the screen: reporting it would paint an
      // error over the healthy (unpaired) state the current attempt produced.
      listed.reject(new Error("registry unreadable"));
      await flush();
      expect(result.current.error).toBeNull();
      expect(result.current.phase).toBe("unpaired");
    });

    test("a bootstrap superseded while loading credentials never connects", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([]);
      const stored = deferred<RemoteCredentials | null>();
      cast<jest.Mock>(loadCredentials).mockReturnValue(stored.promise);
      render();
      await flush();
      await act(async () => { renderer!.unmount(); renderer = null; await drain(); });
      stored.resolve(credentials);
      await flush();
      expect(options.clientRef.current).toBeNull();
    });

    test("a network event that lands on a superseded listener cannot drive recovery", async () => {
      await mountConnected();
      const stale = networkListeners()[0]!;
      const connected = client();
      // A new callbacks identity rebuilds recoverState/recoverLifecycle, so the
      // network effect tears down and installs a fresh listener.
      options.refreshSessions = jest.fn(async () => {});
      act(() => { renderer!.update(createElement(Harness)); });
      await flush();
      connected.recoverNow.mockClear();
      // The retired listener still fires — offline, then back online. It must
      // not restart recovery: the effect that owned it is gone, and letting a
      // dead listener act would double every native reachability event.
      await act(async () => {
        stale(noneState);
        stale(wifiState);
        await drain();
      });
      expect(connected.recoverNow).not.toHaveBeenCalledWith("network-restored");
    });

    test("a foreground event after the network listener was torn down does not start a query", async () => {
      await mountConnected();
      const listener = appStateListeners()[0]!;
      listener("background");
      await act(async () => { renderer!.unmount(); renderer = null; await drain(); });
      cast<jest.Mock>(Network.getNetworkStateAsync).mockClear();
      // A queued AppState event can still arrive after the screen is gone (the
      // native subscription removes asynchronously). The platform query belongs
      // to the effect that was torn down; the fallback reports the last known
      // availability instead of opening a new probe nobody will read.
      act(() => listener("active"));
      await flush();
      expect(Network.getNetworkStateAsync).not.toHaveBeenCalled();
    });

    test("a foreground recovery with no client never becomes an error", async () => {
      // No stored credentials: the screen is unpaired and owns no client.
      render();
      await flush();
      expect(client()).toBeNull();
      cast<jest.Mock>(Network.getNetworkStateAsync).mockClear();
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      // The advisory radio refresh still runs, but the recovery itself must not
      // treat a missing client as a failure to report.
      expect(Network.getNetworkStateAsync).toHaveBeenCalled();
      expect(result.current.error).toBeNull();
      expect(result.current.phase).toBe("unpaired");
    });

    test("a foreground recovery that lands after the app left again is dropped", async () => {
      await mountConnected();
      const pending = deferred<void>();
      client().recoverNow.mockReturnValue(pending.promise);
      cast<jest.Mock>(options.refreshSessions).mockClear();
      cast<jest.Mock>(options.refreshModels).mockClear();
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      expect(client().recoverNow).toHaveBeenCalledWith("foreground");
      // The app is backgrounded again before the probe answers.
      cast<{ currentState: string }>(AppState).currentState = "background";
      pending.resolve();
      await flush();
      // Recalling state now would race the suspension that is about to close
      // the socket.
      expect(options.refreshSessions).not.toHaveBeenCalled();
      expect(options.refreshModels).not.toHaveBeenCalled();
    });

    test("a credential rotation that lands while the pairing is being removed is not adopted", async () => {
      await mountConnected();
      const saving = deferred<void>();
      cast<jest.Mock>(saveCredentials).mockReturnValue(saving.promise);
      const rotating = client().callbacks.onCredentials({ ...credentials, userJwt: "rotated" });
      await drain();
      await act(async () => { await result.current.unpair(); await drain(); });
      saving.resolve();
      await act(async () => { await rotating; await drain(); });
      // The rotated token was written before the unpair, but the client it
      // belongs to is gone: re-adopting it would restore a deleted pairing.
      expect(options.credentialsRef.current).toBeNull();
      expect(result.current.credentials).toBeNull();
      expect(result.current.phase).toBe("unpaired");
    });

    test("a superseded pairing claim whose answer never comes is not reported", async () => {
      render();
      await flush();
      const claim = deferred<RemoteCredentials>();
      cast<jest.Mock>(claimPairingCode).mockReturnValueOnce(claim.promise);
      let first: Promise<void> | undefined;
      await act(async () => {
        first = result.current.pair("111111").catch(() => undefined);
        await drain();
      });
      cast<jest.Mock>(claimPairingCode).mockResolvedValueOnce(credentials);
      await act(async () => { await result.current.pair("222222"); await drain(); });
      expect(result.current.phase).toBe("ready");
      // The abandoned code finally fails. It must not surface as the error of
      // the pairing the user is now connected to.
      claim.reject(new Error("invalid_pairing_code"));
      await act(async () => { await first; await drain(); });
      expect(result.current.error).toBeNull();
      expect(result.current.phase).toBe("ready");
    });

    test("a failed network snapshot that lands after the screen closed is not logged", async () => {
      const warn = jest.spyOn(console, "warn").mockImplementation(() => {});
      try {
        await mountConnected();
        warn.mockClear();
        const probe = deferred<unknown>();
        cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValue(probe.promise);
        await act(async () => {
          appStateListeners()[0]!("background");
          appStateListeners()[0]!("active");
          await drain();
        });
        await act(async () => { renderer!.unmount(); renderer = null; await drain(); });
        // The radio query fails after the screen is gone; the failure of a
        // refresh nobody is waiting for is not worth a support log line.
        probe.reject(new Error("network_probe_timeout"));
        await flush();
        expect(warn).not.toHaveBeenCalled();
      } finally { warn.mockRestore(); }
    });

    test("a successful network snapshot that lands after the screen closed recovers nothing", async () => {
      await mountConnected();
      const probe = deferred<unknown>();
      cast<jest.Mock>(Network.getNetworkStateAsync).mockReturnValue(probe.promise);
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await drain();
      });
      cast<jest.Mock>(options.refreshSessions).mockClear();
      cast<jest.Mock>(options.refreshModels).mockClear();
      await act(async () => { renderer!.unmount(); renderer = null; await drain(); });
      // The radio answers after the screen is gone: a snapshot that arrived for
      // a dead effect must not restart history or catalogue work.
      probe.resolve(wifiState);
      await flush();
      expect(options.refreshSessions).not.toHaveBeenCalled();
      expect(options.refreshModels).not.toHaveBeenCalled();
      expect(consoleError).not.toHaveBeenCalled();
    });

    test("a desktop switch superseded by a second switch never connects the first", async () => {
      render();
      await flush();
      const stored = deferred<RemoteCredentials | null>();
      cast<jest.Mock>(loadCredentials).mockReturnValueOnce(stored.promise);
      let first: Promise<void> | undefined;
      await act(async () => {
        first = result.current.switchDesktop("desktop-1").catch(() => undefined);
        await drain();
      });
      cast<jest.Mock>(loadCredentials).mockResolvedValueOnce(credentials);
      await act(async () => { await result.current.switchDesktop("desktop-2"); await drain(); });
      expect(client()).not.toBeNull();
      const current = client();
      stored.resolve(credentials);
      await act(async () => { await first; await drain(); });
      // The first switch must not replace the connection the second one built.
      expect(client()).toBe(current);
    });

    test("switching to a desktop with no stored credentials fails loudly", async () => {
      render();
      await flush();
      cast<jest.Mock>(loadCredentials).mockResolvedValueOnce(null);
      await expect(result.current.switchDesktop("gone")).rejects.toThrow("desktop_not_paired");
    });

    test("a pairing claim superseded by a second attempt never connects the first", async () => {
      render();
      await flush();
      const claim = deferred<RemoteCredentials>();
      cast<jest.Mock>(claimPairingCode).mockReturnValueOnce(claim.promise);
      let first: Promise<void> | undefined;
      await act(async () => {
        first = result.current.pair("111111").catch(() => undefined);
        await drain();
      });
      cast<jest.Mock>(claimPairingCode).mockResolvedValueOnce(credentials);
      await act(async () => { await result.current.pair("222222"); await drain(); });
      const current = client();
      expect(current).not.toBeNull();
      claim.resolve(credentials);
      await act(async () => { await first; await drain(); });
      // Only the code the user actually submitted last may open a connection.
      expect(client()).toBe(current);
    });

    test("unpair survives a desktop that never answers the unpair command", async () => {
      await mountConnected();
      const answered = client();
      answered.request.mockRejectedValue(new Error("offline"));
      await act(async () => { await result.current.unpair(); await drain(); });
      // A best-effort notification must not leave the local pairing stuck: the
      // rejection is swallowed and the local teardown still completes.
      expect(result.current.phase).toBe("unpaired");
      expect(answered.close).toHaveBeenCalledWith("Unpair");
      expect(clearCredentials).toHaveBeenCalledWith("pair");
    });

    test("removing the active desktop unpairs it", async () => {
      await mountConnected();
      await act(async () => { await result.current.removeDesktop("desktop"); await drain(); });
      expect(result.current.phase).toBe("unpaired");
      expect(clearCredentials).toHaveBeenCalledWith("pair");
    });

    test("removing a desktop that is not paired locally changes nothing", async () => {
      await mountConnected();
      cast<jest.Mock>(loadCredentials).mockResolvedValueOnce(null);
      const listed = result.current.desktops;
      await act(async () => { await result.current.removeDesktop("desktop-9"); await drain(); });
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(savePendingRevoke).not.toHaveBeenCalled();
      expect(result.current.desktops).toBe(listed);
    });
  });
});
