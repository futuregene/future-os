import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import * as Network from "expo-network";
import { AppState } from "react-native";
import type { ConnectionState } from "../connectionState";
import { attemptPendingRevoke, claimPairingCode, serverRevoke } from "../pairing";
import { clearPendingPrompt, loadPendingPrompt } from "../pendingPromptStorage";
import {
  clearCredentials,
  clearPendingRevoke,
  loadCredentials,
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
import { useRemoteConnection } from "../useRemoteConnection";

jest.mock("../storage", () => ({
  __esModule: true,
  clearCredentials: jest.fn(async () => {}),
  clearPendingRevoke: jest.fn(async () => {}),
  loadCredentials: jest.fn(async () => null),
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
  clearPendingPrompt: jest.fn(async () => {}),
  loadPendingPrompt: jest.fn(async () => null),
}));

jest.mock("../client", () => {
  class MockRemoteClient {
    credentials: unknown;
    callbacks: Record<string, (...args: never[]) => unknown>;
    close = jest.fn(async () => {});
    pauseForBackground = jest.fn();
    setNetworkAvailable = jest.fn();
    open = jest.fn(async () => {});
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
  AppState: { currentState: "active", addEventListener: jest.fn(() => ({ remove: jest.fn() })) },
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
  pauseForBackground: jest.Mock;
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
  return { sessionId: id, threadId: `t-${id}`, title: `Title ${id}`, streaming };
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
    recoverRemoteState: jest.fn(async () => {}),
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

const wifiState = { type: "WIFI", isConnected: true, isInternetReachable: true };
const cellularState = { type: "CELLULAR", isConnected: true, isInternetReachable: true };
const noneState = { type: "NONE", isConnected: false, isInternetReachable: false };

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
      renderer = create(React.createElement(Harness));
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
    return cast<jest.Mock>(AppState.addEventListener).mock.calls.map(c => c[1]);
  }

  function networkListeners(): ((state: unknown) => void)[] {
    return cast<jest.Mock>(Network.addNetworkStateListener).mock.calls.map(c => c[0]);
  }

  beforeEach(() => {
    jest.clearAllMocks();
    options = makeOptions();
    result = { current: undefined as unknown as Result };
    renderer = null;
    cast<jest.Mock>(loadPendingRevoke).mockResolvedValue(null);
    cast<jest.Mock>(loadCredentials).mockResolvedValue(null);
    cast<jest.Mock>(loadPendingPrompt).mockResolvedValue(null);
    cast<jest.Mock>(claimPairingCode).mockResolvedValue(credentials);
    cast<jest.Mock>(Network.getNetworkStateAsync).mockResolvedValue(wifiState);
    cast<{ currentState: string }>(AppState).currentState = "active";
  });

  afterEach(() => {
    if (renderer) {
      act(() => renderer!.unmount());
      renderer = null;
    }
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
      cast<jest.Mock>(loadPendingRevoke).mockRejectedValue(new Error("boom"));
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
      expect(options.applySessionSnapshot).toHaveBeenCalledWith([
        { sessionId: "s2", threadId: "t-s2", title: "Title s2", streaming: false },
      ]);
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
      options.syncEngineRef.current = { restartAll: jest.fn() } as unknown as SyncEngine;
      act(() => client().callbacks.onReconnected());
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

    test("background transition pauses the client", async () => {
      await mountConnected();
      act(() => appStateListeners()[0]!("background"));
      expect(client().pauseForBackground).toHaveBeenCalled();
    });

    test("foreground recovery refreshes network and recovers the client", async () => {
      await mountConnected();
      await act(async () => {
        appStateListeners()[0]!("background");
        appStateListeners()[0]!("active");
        await flush();
      });
      expect(client().recoverNow).toHaveBeenCalledWith("foreground");
      expect(options.recoverRemoteState).toHaveBeenCalled();
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

    test("pair claims a code and connects", async () => {
      render();
      await flush();
      await act(async () => {
        await result.current.pair("code");
      });
      expect(claimPairingCode).toHaveBeenCalledWith("code");
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
      expect(loadPendingPrompt).toHaveBeenCalled();
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
      expect(savePendingRevoke).toHaveBeenCalledWith({
        pairId: "pair",
        deviceId: "device",
        seed: "seed",
        refreshToken: "refresh",
        tokenUrl: "https://example.test/auth/token",
      });
    });

    test("unpair clears a pending prompt", async () => {
      await mountConnected();
      cast<jest.Mock>(loadPendingPrompt).mockResolvedValue({ commandId: "c1" });
      await act(async () => {
        await result.current.unpair();
      });
      expect(clearPendingPrompt).toHaveBeenCalledWith("c1");
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
      expect(client().pauseForBackground).toHaveBeenCalled();
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
