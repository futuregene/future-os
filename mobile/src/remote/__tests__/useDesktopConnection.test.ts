import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState } from "react-native";
import type { ConnectionState } from "../connectionState";
import { RemoteApiError } from "../connectionState";
import { discardPendingContinuation } from "../pendingContinuationStorage";
import { discardPendingPrompt } from "../pendingPromptStorage";
import { loadCredentials, saveCredentials } from "../storage";
import type { SyncEngine } from "../syncEngine";
import type {
  Presence,
  PresenceSession,
  RemoteCredentials,
  StreamEvent,
} from "../types";
import { useDesktopConnection } from "../useDesktopConnection";

jest.mock("../storage", () => ({
  __esModule: true,
  loadCredentials: jest.fn(async () => null),
  saveCredentials: jest.fn(async () => {}),
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
  onWorkspaces(w: unknown[], version: number): void;
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

type Options = Parameters<typeof useDesktopConnection>[0];
type Result = ReturnType<typeof useDesktopConnection>;

function makeOptions(): Options {
  return {
    desktopId: "desktop",
    clientRef: { current: null } as Options["clientRef"],
    credentialsRef: { current: null } as Options["credentialsRef"],
    selectedRef: { current: "" } as Options["selectedRef"],
    syncEngineRef: { current: null } as Options["syncEngineRef"],
    handleEvent: jest.fn(),
    reconcileSession: jest.fn(),
    setCatalogEpoch: jest.fn(),
    applySessionSnapshot: jest.fn(),
    applySessionStreaming: jest.fn(),
    setWorkspaces: jest.fn(),
    refreshModels: jest.fn(async () => {}),
    refreshSessions: jest.fn(async () => {}),
    refreshSettings: jest.fn(async () => {}),
    refreshWorkspaces: jest.fn(async () => {}),
    noteCatalogRevisions: jest.fn(),
    closeConversation: jest.fn(),
    resetConversation: jest.fn(),
    resetCatalog: jest.fn(),
    resetTimeline: jest.fn(),
  };
}

function cast<T>(value: unknown): T {
  return value as T;
}

describe("useDesktopConnection", () => {
  let options: Options;
  let result: { current: Result };
  let renderer: ReactTestRenderer | null;
  let consoleError: jest.SpyInstance;

  function Harness(): null {
    result.current = useDesktopConnection(options);
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

  async function mountConnected(): Promise<void> {
    cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
    render();
    await flush();
  }

  beforeEach(() => {
    consoleError = jest.spyOn(console, "error");
    jest.clearAllMocks();
    options = makeOptions();
    result = { current: undefined as unknown as Result };
    renderer = null;
    cast<jest.Mock>(loadCredentials).mockResolvedValue(null);
    cast<{ currentState: string }>(AppState).currentState = "active";
  });

  afterEach(async () => {
    if (renderer) {
      await act(async () => { renderer!.unmount(); await drain(); });
      renderer = null;
    }
    try { expect(consoleError).not.toHaveBeenCalled(); }
    finally { consoleError.mockRestore(); }
  });

  describe("bootstrap", () => {
    test("connects when this desktop's credentials are stored", async () => {
      await mountConnected();
      expect(loadCredentials).toHaveBeenCalledWith("desktop");
      expect(options.clientRef.current).not.toBeNull();
      expect(result.current.phase).toBe("ready");
      expect(result.current.credentials).toEqual(credentials);
    });

    test("no stored credentials leaves the machine failed, not silently empty", async () => {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(null);
      render();
      await flush();
      expect(options.clientRef.current).toBeNull();
      expect(result.current.phase).toBe("failed");
      expect(result.current.error).toBe("incomplete_desktop_credentials");
    });

    test("a credential read failure is reported as a failed machine", async () => {
      cast<jest.Mock>(loadCredentials).mockRejectedValue(new Error("keychain_unavailable"));
      render();
      await flush();
      expect(result.current.phase).toBe("failed");
      expect(result.current.error).toBe("keychain_unavailable");
    });
  });

  describe("client callbacks", () => {
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
      await mountConnected();
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

    test("onPresence unpaired tears down this machine but keeps its pairing", async () => {
      await mountConnected();
      const c = client();
      await act(async () => {
        c.callbacks.onPresence({ ...presence, unpaired: true });
        await drain();
      });
      expect(saveCredentials).toHaveBeenCalled();
      expect(discardPendingContinuation).toHaveBeenCalled();
      expect(discardPendingPrompt).toHaveBeenCalled();
      expect(options.resetCatalog).toHaveBeenCalled();
      expect(options.resetConversation).toHaveBeenCalled();
      expect(options.resetTimeline).toHaveBeenCalled();
      expect(result.current.phase).toBe("revoked");
      expect(result.current.credentials).toEqual(credentials);
      expect(options.clientRef.current).toBeNull();
    });

    test.each(["user_disconnect", "system_sleep", "app_exit"])(
      "desktop %s stops retries and clears this machine's mirrored catalogue",
      async (reason) => {
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

    test("onReconnected restarts the lanes and refreshes this machine's state", async () => {
      await mountConnected();
      options.syncEngineRef.current = {
        restartAll: jest.fn(),
      } as unknown as SyncEngine;
      await act(async () => client().callbacks.onReconnected());
      expect(options.syncEngineRef.current!.restartAll).toHaveBeenCalledWith("reconnect");
      expect(options.refreshModels).toHaveBeenCalled();
      expect(options.refreshSessions).toHaveBeenCalled();
      expect(options.refreshWorkspaces).toHaveBeenCalled();
      expect(options.refreshSettings).toHaveBeenCalled();
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

    test("live events and catalogue epochs reach their sinks on the current generation only", async () => {
      await mountConnected();
      const event = { type: "agent_start", data: "{}" } as StreamEvent;
      act(() => client().callbacks.onEvent(event, "s1"));
      expect(options.handleEvent).toHaveBeenCalledWith(event, "s1");
      act(() => client().callbacks.onCatalogEpoch("epoch-1"));
      expect(options.setCatalogEpoch).toHaveBeenCalledWith("epoch-1");
      act(() => client().callbacks.onWorkspaces([], 2));
      expect(options.setWorkspaces).toHaveBeenCalledWith([], 2);
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

  describe("lifecycle controls", () => {
    test("a visible session is declared to this machine's client", async () => {
      await mountConnected();
      act(() => result.current.setVisibleSession("s1"));
      expect(client().setVisibleSession).toHaveBeenCalledWith("s1");
    });

    test("holding or releasing the socket reaches the client", async () => {
      await mountConnected();
      const c = client();
      act(() => result.current.setAppActive(false));
      expect(c.setAppActive).toHaveBeenLastCalledWith(false);
      act(() => result.current.setAppActive(true));
      expect(c.setAppActive).toHaveBeenLastCalledWith(true);
    });

    test("a desktop backgrounded at connect time is not left holding a socket", async () => {
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
      act(() => result.current.setAppActive(false));
      expect(client().setAppActive).toHaveBeenLastCalledWith(false);
    });

    test("recover probes this machine's own socket", async () => {
      await mountConnected();
      const c = client();
      await act(async () => { await result.current.recover("foreground"); });
      expect(c.recoverNow).toHaveBeenCalledWith("foreground");
    });

    test("a recovery that fails after a reconnect is reported rather than swallowed", async () => {
      const warn = jest.spyOn(console, "warn").mockImplementation(() => {});
      try {
        options.syncEngineRef.current = cast({
          restartAll: () => {
            throw new RemoteApiError("recovery exploded", "invalid_remote_credential", 401);
          },
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

    test("recover does nothing while the app is not in the foreground", async () => {
      await mountConnected();
      const c = client();
      cast<{ currentState: string }>(AppState).currentState = "background";
      await act(async () => { await result.current.recover("network-restored"); });
      expect(c.recoverNow).not.toHaveBeenCalled();
    });

    test("unmount closes this machine's socket", async () => {
      await mountConnected();
      const c = client();
      await act(async () => {
        renderer!.unmount();
        renderer = null;
        await drain();
      });
      expect(c.close).toHaveBeenCalled();
      expect(options.clientRef.current).toBeNull();
    });
  });

  describe("presence heartbeat interval", () => {
    beforeEach(() => { jest.useFakeTimers(); });
    afterEach(() => { jest.useRealTimers(); });

    test("interval refreshes presence while ready", async () => {
      await mountConnected();
      act(() => {
        client().callbacks.onConnectionState("ready");
        client().callbacks.onPresence({ ...presence, lastHeartbeatTs: Date.now() / 1000 });
      });
      expect(result.current.desktopOnline).toBe(true);
      act(() => { jest.advanceTimersByTime(11_000); });
      expect(result.current.desktopOnline).toBe(true);
    });

    test("interval clears desktop online while not ready", async () => {
      await mountConnected();
      act(() => client().callbacks.onConnectionState("reconnecting"));
      act(() => { jest.advanceTimersByTime(11_000); });
      expect(result.current.desktopOnline).toBe(false);
    });
  });

  describe("interrupted and superseded attempts", () => {
    test("a replaced client cannot write credentials or reconcile sessions", async () => {
      await mountConnected();
      const first = client();
      await act(async () => {
        await first.callbacks.onCredentials({ ...credentials, userJwt: "first" });
        await drain();
      });
      cast<jest.Mock>(saveCredentials).mockClear();
      // A second connect (a re-pair) supersedes the first generation.
      await act(async () => {
        await first.callbacks.onCredentials({ ...credentials, userJwt: "late" });
        await drain();
      });
      expect(saveCredentials).toHaveBeenCalledWith({ ...credentials, userJwt: "late" });
      act(() => first.callbacks.onEventDecodeFailure("s1", new Error("decode")));
      expect(options.reconcileSession).toHaveBeenCalledWith("s1", "resend");
    });

    test("a bootstrap that lands after unmount never connects", async () => {
      let resolveCredentials: (value: RemoteCredentials | null) => void = () => undefined;
      cast<jest.Mock>(loadCredentials).mockReturnValue(
        new Promise<RemoteCredentials | null>((resolve) => { resolveCredentials = resolve; }),
      );
      render();
      await act(async () => {
        renderer!.unmount();
        renderer = null;
        resolveCredentials(credentials);
        await drain();
      });
      expect(options.clientRef.current).toBeNull();
    });

    test("a credential rotation that lands after teardown is not adopted", async () => {
      await mountConnected();
      const c = client();
      act(() => c.callbacks.onPresence({ ...presence, unpaired: true }));
      cast<jest.Mock>(saveCredentials).mockClear();
      await act(async () => {
        await c.callbacks.onCredentials({ ...credentials, userJwt: "late" });
        await drain();
      });
      expect(saveCredentials).not.toHaveBeenCalled();
      expect(result.current.credentials).toEqual(credentials);
    });
  });
});
