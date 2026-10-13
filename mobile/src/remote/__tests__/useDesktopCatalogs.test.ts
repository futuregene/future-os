import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState } from "react-native";
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

function presence(
  online: boolean,
  heartbeatTs: number,
  agentAvailable = true,
  catalogVersion?: { epoch?: string; sessions?: number; workspaces?: number },
): Presence {
  return {
    online,
    agentAvailable,
    pairId: "pair-desktop_2",
    bridgeInstanceId: "bridge",
    lastHeartbeatTs: heartbeatTs,
    ...(catalogVersion ? { catalogVersion } : {}),
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

  /** How many times one client has read its catalogue. */
  function catalogReads(client: MockPeerClient): number {
    return client.request.mock.calls.filter(
      call => (call[0] as { type: string }).type === "list_sessions",
    ).length;
  }

  test("a connection that came back re-reads the catalogue it may have missed", async () => {
    await render();
    const client = clientFor("desktop_2");
    expect(catalogReads(client)).toBe(1);

    // The host publishes when its list *changes*, so nothing arrives to repair a
    // lane that was down: the read is what makes "connected again" also mean
    // "up to date again".
    await act(async () => {
      client.callbacks.onReconnected();
    });

    expect(catalogReads(client)).toBe(2);
  });

  test("a heartbeat advertising a revision we never applied pulls the catalogue", async () => {
    await render();
    const client = clientFor("desktop_2");
    act(() => {
      client.callbacks.onCatalogEpoch("epoch-1");
      client.callbacks.onSessions([session("a")], { epoch: "epoch-1", revision: 3 });
    });
    expect(catalogReads(client)).toBe(1);

    // The revision we hold is the one the machine advertises: nothing was lost.
    act(() => {
      client.callbacks.onPresence(
        presence(true, Date.now() / 1000, true, { epoch: "epoch-1", sessions: 3, workspaces: 1 }),
      );
    });
    expect(catalogReads(client)).toBe(1);

    // A newer one proves the push never arrived (the lane is at-most-once).
    await act(async () => {
      client.callbacks.onPresence(
        presence(true, Date.now() / 1000, true, { epoch: "epoch-1", sessions: 4, workspaces: 1 }),
      );
    });
    expect(catalogReads(client)).toBe(2);

    // A revision from another epoch describes a desktop generation this
    // connection has not authenticated; the handshake owns that transition.
    act(() => {
      client.callbacks.onPresence(
        presence(true, Date.now() / 1000, true, { epoch: "epoch-2", sessions: 9, workspaces: 9 }),
      );
    });
    expect(catalogReads(client)).toBe(2);
  });

  test("a machine whose connection gave up is reached again, not left offline", async () => {
    await render();
    const first = clientFor("desktop_2");

    // A client that reached `failed` is terminal: it will not retry itself, so a
    // machine that was asleep when the phone started would read offline until the
    // app restarts — the state a reader reads as "the phone lost that desktop".
    // The slow pass is what opens a fresh connection to it.
    act(() => {
      first.callbacks.onConnectionState("failed");
    });
    expect(result.current.commandsFor("desktop_2")).toBeNull();

    await act(async () => {
      await result.current.refresh();
    });

    expect(first.close).toHaveBeenCalled();
    const reopened = latestClientFor("desktop_2");
    expect(reopened).not.toBe(first);
  });

  test("a machine that was revoked is not re-handshaked on every pass", async () => {
    await render();
    const first = clientFor("desktop_2");

    // The desktop rejected this phone's credential for that pairing: retrying it
    // every minute is noise, not recovery, and re-pairing is the user's move.
    act(() => {
      first.callbacks.onConnectionState("revoked");
    });
    await act(async () => {
      await result.current.refresh();
    });

    expect(first.close).not.toHaveBeenCalled();
    expect(latestClientFor("desktop_2")).toBe(first);
  });

  test("coming back to the foreground reads every machine again", async () => {
    await render();
    const client = clientFor("desktop_2");
    expect(catalogReads(client)).toBe(1);
    const onChange = (AppState.addEventListener as jest.Mock).mock.calls.at(-1)![1] as
      (state: string) => void;

    await act(async () => {
      onChange("background");
    });
    expect(catalogReads(client)).toBe(1);
    await act(async () => {
      onChange("active");
    });

    // The phone was away; what happened to these sockets, and to a machine that
    // was asleep then, is worth one pass now.
    expect(catalogReads(client)).toBe(2);
  });

  test("one machine's conversation is commanded through that machine's connection", async () => {    await render();
    const client = clientFor("desktop_2");
    act(() => {
      client.callbacks.onCatalogEpoch("epoch-1");
      client.callbacks.onConnectionState("ready");
    });

    const commands = result.current.commandsFor("desktop_2");
    expect(commands).not.toBeNull();
    await act(async () => {
      await commands!.rename("s9", "  Renamed  ");
      await commands!.setPinned("s9", "thread-s9", true);
      await commands!.delete("s9", "thread-s9");
    });

    // Addressed to the machine that owns the row, and on that row's own lane.
    expect(client.request).toHaveBeenCalledWith(
      { type: "set_session_name", sessionId: "s9", name: "Renamed" },
      "s9",
    );
    expect(client.request).toHaveBeenCalledWith(
      { type: "set_session_pinned", sessionId: "s9", threadId: "thread-s9", pinned: true },
      "s9",
    );
    expect(client.request).toHaveBeenCalledWith(
      { type: "delete_session", sessionId: "s9", threadId: "thread-s9" },
      "s9",
    );
    // Each accepted command re-reads the catalogue, so the phone's list follows
    // the change it just made rather than waiting for a push that may be lost.
    expect(catalogReads(client)).toBe(4);

    // Nothing is offered for a machine whose connection has given up, or for one
    // this phone has never paired with.
    act(() => {
      client.callbacks.onConnectionState("failed");
    });
    expect(result.current.commandsFor("desktop_2")).toBeNull();
    expect(result.current.commandsFor("desktop_3")).toBeNull();
  });
});
