import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { loadCredentials, loadPairedDesktops } from "../storage";
import type { Presence, RemoteCredentials, RemoteSession } from "../types";
import { useDesktopCatalogs } from "../useDesktopCatalogs";

jest.mock("../storage", () => ({
  __esModule: true,
  loadPairedDesktops: jest.fn(async () => []),
  loadCredentials: jest.fn(async () => null),
  saveCredentials: jest.fn(async () => {}),
}));

jest.mock("../pairing", () => ({
  __esModule: true,
  ensureFreshCredentials: jest.fn(async (credentials: unknown) => credentials),
}));

interface MockCallbacks {
  onCredentials(next: RemoteCredentials): Promise<void>;
  onCatalogEpoch(epoch: string | undefined): void;
  onPresence(presence: Presence): void;
  onSessions(sessions: RemoteSession[], version?: { epoch: string; revision: number }): void;
  onConnectionState(state: string): void;
  onReconnected(): void;
  onError(error: unknown): void;
}

interface MockPeerClient {
  credentials: RemoteCredentials;
  callbacks: MockCallbacks;
  open: jest.Mock;
  close: jest.Mock;
  request: jest.Mock;
}

const mockClients: MockPeerClient[] = [];
jest.mock("../client", () => {
  class MockRemoteClient {
    credentials: unknown;
    callbacks: Record<string, (...args: never[]) => unknown>;
    open = jest.fn(async () => {
      this.callbacks.onConnectionState?.("ready" as never);
    });
    close = jest.fn(async () => {});
    request = jest.fn(async () => ({
      success: true,
      data: { sessions: [], version: { epoch: "epoch-1", revision: 1 } },
    }));
    constructor(
      credentials: unknown,
      callbacks: Record<string, (...args: never[]) => unknown>,
    ) {
      this.credentials = credentials;
      this.callbacks = callbacks;
      mockClients.push(this as unknown as MockPeerClient);
    }
  }
  return { RemoteClient: MockRemoteClient };
});

type State = ReturnType<typeof useDesktopCatalogs>;

const credentials = (desktopId: string): RemoteCredentials =>
  ({
    pairId: `pair-${desktopId}`,
    deviceId: "device",
    seed: "",
    userJwt: "",
    refreshToken: "",
    natsWsUrl: "wss://nats.test",
    tokenUrl: "https://example.test/auth/token",
    expectedDesktopId: desktopId,
    expectedDesktopPublicKey: "",
  }) as RemoteCredentials;

function session(id: string): RemoteSession {
  return { sessionId: id, threadId: `thread-${id}`, title: id, streaming: false };
}

function presence(online: boolean, heartbeatTs: number, agentAvailable = true): Presence {
  return {
    online,
    agentAvailable,
    pairId: "pair-desktop_2",
    bridgeInstanceId: "bridge",
    lastHeartbeatTs: heartbeatTs,
  } as Presence;
}

describe("useDesktopCatalogs", () => {
  let renderer: ReactTestRenderer | null;
  let result: { current: State };

  function Harness({
    activeDesktopId = "desktop_1",
    activeSessions = [] as RemoteSession[],
  }): null {
    result.current = useDesktopCatalogs(activeDesktopId, activeSessions);
    return null;
  }

  async function render(activeDesktopId?: string): Promise<void> {
    await act(async () => {
      renderer = create(createElement(Harness, { activeDesktopId }));
    });
  }

  /** The client the hook opened for a machine. */
  function clientFor(desktopId: string): MockPeerClient {
    const client = latestClientFor(desktopId);
    if (!client) throw new Error(`no client for ${desktopId}`);
    return client;
  }

  /** The most recent client for a machine — after a reconnect, the new one. */
  function latestClientFor(desktopId: string): MockPeerClient | undefined {
    return [...mockClients].reverse().find(
      candidate => (candidate.credentials as RemoteCredentials).expectedDesktopId === desktopId,
    );
  }

  beforeEach(() => {
    jest.useFakeTimers();
    mockClients.length = 0;
    jest.mocked(loadPairedDesktops).mockResolvedValue([
      { desktopId: "desktop_1", pairId: "pair-desktop_1" },
      { desktopId: "desktop_2", pairId: "pair-desktop_2", name: "Office", icon: "office" },
      { desktopId: "desktop_3", pairId: "pair-desktop_3" },
    ]);
    jest.mocked(loadCredentials).mockImplementation(async (desktopId?: string) =>
      desktopId === "desktop_2" ? credentials("desktop_2") : null,
    );
    result = { current: undefined as never };
    renderer = null;
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
    jest.useRealTimers();
  });

  test("holds one live connection per other machine, and none for the active one", async () => {
    await render();

    expect(mockClients).toHaveLength(1);
    expect(clientFor("desktop_2").credentials.expectedDesktopId).toBe("desktop_2");
    // The active machine's rows come from the main connection, never a second
    // socket to the same desktop.
    expect(mockClients.some(
      client => client.credentials.expectedDesktopId === "desktop_1",
    )).toBe(false);
    // A machine with no stored credentials is listed all the same — offline —
    // because a paired machine the phone cannot reach is not "connected" and
    // the dot must not say it is. (It has no rows to show: it never answered.)
    expect(result.current.machines).toEqual([
      { desktopId: "desktop_2", label: "Office", icon: "office", active: false, status: "offline" },
      { desktopId: "desktop_3", label: "desktop_3", active: false, status: "offline" },
    ]);
  });

  test("a heartbeat turns the machine connected, and its absence turns it offline", async () => {
    await render();
    const client = clientFor("desktop_2");
    const now = Date.now();

    act(() => {
      client.callbacks.onPresence(presence(true, now / 1000));
    });
    expect(result.current.machines[0]!.status).toBe("connected");

    // The heartbeat stops. Nothing announces that; the tick that re-judges the
    // last one against the clock is what makes the row honest.
    act(() => {
      jest.advanceTimersByTime(16_000);
    });
    expect(result.current.machines[0]!.status).toBe("offline");
  });

  test("an agent that is down reads as preparing, not connected", async () => {
    await render();
    const client = clientFor("desktop_2");

    act(() => {
      client.callbacks.onPresence(presence(true, Date.now() / 1000, false));
    });

    expect(result.current.machines[0]!.status).toBe("preparing");
  });

  test("a pushed catalogue is the list, and an older one cannot roll it back", async () => {
    await render();
    const client = clientFor("desktop_2");

    act(() => {
      client.callbacks.onCatalogEpoch("epoch-1");
      client.callbacks.onSessions([session("new")], { epoch: "epoch-1", revision: 4 });
    });
    expect(result.current.catalogs[0]!.sessions.map(item => item.sessionId)).toEqual(["new"]);

    // A lost-then-late snapshot (a NATS redelivery, a slow poll answer) must not
    // replace the list with what it knew before.
    act(() => {
      client.callbacks.onSessions([session("old")], { epoch: "epoch-1", revision: 2 });
    });
    expect(result.current.catalogs[0]!.sessions.map(item => item.sessionId)).toEqual(["new"]);
  });

  test("the backstop read happens on its own slow timer", async () => {
    await render();
    const client = clientFor("desktop_2");
    expect(client.request).toHaveBeenCalledTimes(1);

    await act(async () => {
      jest.advanceTimersByTime(60_000);
    });

    expect(client.request).toHaveBeenCalledTimes(2);
  });

  test("reconnect closes every machine's connection and opens it again", async () => {
    await render();
    const first = clientFor("desktop_2");
    expect(first.close).not.toHaveBeenCalled();

    await act(async () => {
      await result.current.reconnect();
    });

    expect(first.close).toHaveBeenCalled();
    const reopened = latestClientFor("desktop_2");
    expect(reopened).not.toBe(first);
    // Still one live connection per machine, not one per attempt.
    expect(mockClients).toHaveLength(2);
  });

  test("a machine whose connection fails is listed offline rather than dropped", async () => {
    jest.mocked(loadCredentials).mockRejectedValue(new Error("keychain locked"));

    await render();

    expect(mockClients).toHaveLength(0);
    expect(result.current.machines.map(machine => [machine.desktopId, machine.status]))
      .toEqual([
        ["desktop_2", "offline"],
        ["desktop_3", "offline"],
      ]);
  });

  test("a machine that stops being paired loses its connection and its rows", async () => {
    await render();
    const first = clientFor("desktop_2");

    jest.mocked(loadPairedDesktops).mockResolvedValue([
      { desktopId: "desktop_1", pairId: "pair-desktop_1" },
    ]);
    await act(async () => {
      await result.current.refresh();
    });

    expect(first.close).toHaveBeenCalledWith("Unpair");
    expect(result.current.machines).toEqual([]);
    expect(result.current.catalogs.map(entry => entry.desktopId)).toEqual(["desktop_1"]);
  });
});
