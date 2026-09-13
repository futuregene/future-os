import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import * as Network from "expo-network";
import { AppState } from "react-native";
import type { ConnectionState } from "../connectionState";
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
    setNetworkAvailable = jest.fn();
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
  onPresence(p: Presence): void;
  onSessions(s: PresenceSession[]): void;
  onWorkspaces(w: RemoteSession[]): void;
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
  setNetworkAvailable: jest.Mock;
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

  function Harness(): null {
    result.current = useRemoteConnection(options);
    return null;
  }

  function render(): void {
    act(() => {
      renderer = create(createElement(Harness));
    });
  }

  async function flush(times = 20): Promise<void> {
    await act(async () => {
      for (let i = 0; i < times; i += 1) {
        await Promise.resolve();
      }
    });
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

  afterEach(() => {
    if (renderer) {
      act(() => renderer!.unmount());
      renderer = null;
    }
    // The presentation depth is process-global; a failed assertion mid-test
    // must not leak a held connection into the next one.
    endNativePresentation();
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

    test("desktop unpair notices clear only their own pair", async () => {
      render();
      await flush();
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([desktops[1]]);
      act(() => client().callbacks.onPresence({ ...presence, unpaired: true }));
      await flush();
      expect(clearCredentials).toHaveBeenCalledWith(credentials.pairId);
      expect(discardPendingPrompt).toHaveBeenCalledWith(credentials.pairId);
      expect(discardPendingContinuation).toHaveBeenCalledWith(credentials.pairId);
      expect(result.current.desktops).toEqual([desktops[1]]);
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
      expect(result.current.phase).toBe("unpaired");
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

    test("onPresence unpaired tears down and goes unpaired", async () => {
      await mountConnected();
      const c = client();
      act(() => c.callbacks.onPresence({ ...presence, unpaired: true }));
      expect(clearCredentials).toHaveBeenCalled();
      expect(discardPendingContinuation).toHaveBeenCalled();
      expect(discardPendingPrompt).toHaveBeenCalled();
      expect(options.resetCatalog).toHaveBeenCalled();
      expect(options.resetConversation).toHaveBeenCalled();
      expect(options.resetTimeline).toHaveBeenCalled();
      expect(result.current.phase).toBe("unpaired");
      expect(options.clientRef.current).toBeNull();
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
        await flush();
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

    test("foreground recovery refreshes network and recovers the client", async () => {
      await mountConnected();
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await flush();
      });
      expect(client().recoverNow).toHaveBeenCalledWith("foreground");
      expect(options.refreshSettings).toHaveBeenCalledTimes(2);
    });

    test("foreground recovery aborts when the network is unavailable", async () => {
      await mountConnected();
      cast<jest.Mock>(Network.getNetworkStateAsync).mockResolvedValue(noneState);
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await flush();
      });
      expect(client().recoverNow).not.toHaveBeenCalled();
      expect(client().setAppActive).toHaveBeenLastCalledWith(true);
    });

    test("foreground socket replacement uses the same recovery pass as onReconnected", async () => {
      await mountConnected();
      cast<jest.Mock>(options.refreshSettings).mockClear();
      client().recoverNow.mockImplementationOnce(async () => {
        client().callbacks.onReconnected();
        // Let the callback's recovery finish before recoverNow returns: checking
        // only for an in-flight promise would start a redundant second pass.
        await flush();
      });
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await flush();
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
        await flush();
      });
      expect(client().recoverNow).toHaveBeenCalledWith("foreground");
    });

    test("foreground recovery records a recovery failure", async () => {
      await mountConnected();
      client().recoverNow.mockRejectedValueOnce(new Error("invalid_jwt"));
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await flush();
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

    test("reconnect with an invalid JWT clears credentials", async () => {
      await mountConnected();
      cast<jest.Mock>(saveCredentials).mockRejectedValueOnce(new Error("invalid_jwt"));
      await act(async () => {
        await result.current.reconnect();
      });
      expect(clearCredentials).toHaveBeenCalled();
      expect(result.current.phase).toBe("unpaired");
      expect(result.current.error).toBeNull();
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

    test("disables the network when offline at connect time", async () => {
      render();
      await flush();
      networkListeners()[0]!(noneState);
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      await act(async () => {
        await result.current.reconnect();
      });
      expect(client().setNetworkAvailable).toHaveBeenCalledWith(false);
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
});
