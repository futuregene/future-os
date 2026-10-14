import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { prepareTaskNotifications } from "../../notifications/taskNotifications";
import { DesktopScope, RemoteProvider, useRemote, useRemoteControls, type RemoteControls } from "../RemoteContext";
import { claimPairingCode } from "../pairing";
import { loadCredentials, loadPairedDesktops } from "../storage";
import type { Presence, RemoteCredentials, RemoteSession } from "../types";

/**
 * The app's remote surface over several paired desktops.
 *
 * These are the tests that hold the point of the design: every paired desktop
 * keeps its own runtime — its own connection, catalogue and open conversation —
 * so moving between machines is a change of what the screens read rather than a
 * reconnection. The sub-hooks are real here; only the transport and the
 * credential store are stand-ins, because what is being asserted is what the
 * app does with several machines at once, not what any one hook does alone.
 */

jest.mock("../storage", () => ({
  __esModule: true,
  clearCredentials: jest.fn(async () => {}),
  clearPendingRevoke: jest.fn(async () => {}),
  labelDesktop: jest.fn(async () => {}),
  loadActiveDesktopId: jest.fn(async () => "desktop_1"),
  loadCredentials: jest.fn(async () => null),
  loadDeviceId: jest.fn(async () => "device"),
  loadLastModel: jest.fn(async () => null),
  loadLastThinking: jest.fn(async () => null),
  loadPairedDesktops: jest.fn(async () => []),
  loadPendingRevoke: jest.fn(async () => null),
  renameDesktop: jest.fn(async () => {}),
  saveCredentials: jest.fn(async () => {}),
  saveDeviceId: jest.fn(async () => {}),
  saveLastModel: jest.fn(async () => {}),
  saveLastThinking: jest.fn(async () => {}),
  savePendingRevoke: jest.fn(async () => {}),
  setActiveDesktop: jest.fn(async () => {}),
}));

jest.mock("../pairing", () => ({
  __esModule: true,
  attemptPendingRevoke: jest.fn(async () => {}),
  claimPairingCode: jest.fn(async () => null),
  ensureFreshCredentials: jest.fn(async (credentials: unknown) => credentials),
  serverRevoke: jest.fn(async () => {}),
}));

jest.mock("../pendingPromptStorage", () => ({
  __esModule: true,
  clearPendingPrompt: jest.fn(async () => {}),
  discardPendingPrompt: jest.fn(async () => {}),
  loadPendingPrompt: jest.fn(async () => null),
  savePendingPrompt: jest.fn(async () => {}),
}));

jest.mock("../pendingContinuationStorage", () => ({
  __esModule: true,
  clearPendingContinuation: jest.fn(async () => {}),
  discardPendingContinuation: jest.fn(async () => {}),
  loadPendingContinuation: jest.fn(async () => null),
  savePendingContinuation: jest.fn(async () => {}),
}));

jest.mock("../../notifications/taskNotifications", () => ({
  __esModule: true,
  notifyTaskFinished: jest.fn(),
  prepareTaskNotifications: jest.fn(async () => {}),
}));

// File transfer pulls in the native image/crypto chain, which has nothing to do
// with what this suite is about: what the app does with several machines at once.
jest.mock("../files", () => ({
  __esModule: true,
  cachedPreviewForAttachment: jest.fn(() => null),
  downloadPrepared: jest.fn(),
  prepareDownload: jest.fn(),
  rememberPreparedPreview: jest.fn(),
  uploadAttachments: jest.fn(async () => []),
  TransferCancelledError: class TransferCancelledError extends Error {},
}));

jest.mock("expo-network", () => ({
  __esModule: true,
  NetworkStateType: { NONE: "NONE", UNKNOWN: "UNKNOWN", WIFI: "WIFI", CELLULAR: "CELLULAR", OTHER: "OTHER" },
  getNetworkStateAsync: jest.fn(async () => ({
    type: "WIFI", isConnected: true, isInternetReachable: true,
  })),
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

interface RequestRecord {
  command: { type: string; sessionId?: string };
  sessionId?: string;
  constructor: string;
}

interface MockClient {
  credentials: RemoteCredentials;
  callbacks: Record<string, (...args: never[]) => unknown>;
  requests: RequestRecord[];
  open: jest.Mock;
  close: jest.Mock;
  setAppActive: jest.Mock;
  setVisibleSession: jest.Mock;
  recoverNow: jest.Mock;
  request: jest.Mock;
  requestRetry: jest.Mock;
}

const mockClients: MockClient[] = [];

/** What a desktop answers with; the catalogue the merged list renders. */
const mockCatalogsByDesktop: Record<string, RemoteSession[]> = {
  desktop_1: [{ sessionId: "a-1", threadId: "t-a-1", title: "A one", streaming: false }],
  desktop_2: [{ sessionId: "b-1", threadId: "t-b-1", title: "B one", streaming: false }],
};

function mockReplyFor(desktopId: string, command: { type: string }): Record<string, unknown> {
  switch (command.type) {
    case "list_sessions":
      return { sessions: mockCatalogsByDesktop[desktopId] ?? [] };
    case "list_models":
      return { models: [] };
    case "list_workspaces":
      return { workspaces: [] };
    case "get_state":
      return {};
    default:
      return {};
  }
}

jest.mock("../client", () => {
  class MockRemoteClient {
    credentials: RemoteCredentials;
    callbacks: Record<string, (...args: never[]) => unknown>;
    requests: RequestRecord[] = [];
    open = jest.fn(async () => {
      // The host pushes a catalogue snapshot as part of the handshake, which is
      // what the phone renders before any explicit read.
      this.callbacks.onSessions?.(
        (mockCatalogsByDesktop[this.credentials.expectedDesktopId] ?? []) as never,
      );
      this.callbacks.onConnectionState?.("ready" as never);
      // …and a successful connect is also what has the client read the rest of
      // its own catalogue (see `RemoteClient.connectSocket`), so a machine
      // nobody is reading has its workspaces, models and settings in hand too.
      this.callbacks.onReconnected?.();
    });
    close = jest.fn(async () => {});
    setAppActive = jest.fn();
    setVisibleSession = jest.fn();
    recoverNow = jest.fn(async () => {});
    request = jest.fn(async (command: { type: string }, sessionId?: string) => {
      const desktopId = this.credentials.expectedDesktopId;
      this.requests.push({ command, sessionId, constructor: desktopId });
      return { success: true, data: mockReplyFor(desktopId, command) };
    });
    requestRetry = jest.fn(async (command: { type: string }, sessionId?: string) => {
      const desktopId = this.credentials.expectedDesktopId;
      this.requests.push({ command, sessionId, constructor: desktopId });
      return { success: true, data: mockReplyFor(desktopId, command) };
    });
    get accessIdentity(): string {
      return `identity:${this.credentials.expectedDesktopId}`;
    }
    constructor(credentials: RemoteCredentials, callbacks: Record<string, (...args: never[]) => unknown>) {
      this.credentials = credentials;
      this.callbacks = callbacks;
      mockClients.push(this as unknown as MockClient);
    }
  }
  return { RemoteClient: MockRemoteClient };
});

type Consumer = ReturnType<typeof useRemote> & { streaming: boolean };

function credentialsFor(desktopId: string): RemoteCredentials {
  return {
    pairId: `pair-${desktopId}`,
    deviceId: "device",
    seed: "",
    userJwt: "",
    refreshToken: "",
    natsWsUrl: "wss://nats.test",
    tokenUrl: "https://example.test/auth/token",
    expectedDesktopId: desktopId,
    expectedDesktopPublicKey: "",
  } as RemoteCredentials;
}

function onlinePresence(desktopId: string): Presence {
  return {
    online: true,
    agentAvailable: true,
    pairId: `pair-${desktopId}`,
    bridgeInstanceId: "bridge",
    lastHeartbeatTs: Date.now() / 1000,
  } as Presence;
}

describe("RemoteProvider over several desktops", () => {
  let renderer: ReactTestRenderer | null;
  let remote: Consumer;

  function Consumer(): null {
    remote = useRemote() as unknown as Consumer;
    return null;
  }

  async function mount(): Promise<void> {
    await act(async () => {
      renderer = create(createElement(RemoteProvider, null, createElement(Consumer)));
    });
  }

  async function settle(): Promise<void> {
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
  }

  function clientFor(desktopId: string): MockClient {
    const client = [...mockClients].reverse().find(
      (candidate) => candidate.credentials.expectedDesktopId === desktopId);
    if (!client) throw new Error(`no client for ${desktopId}`);
    return client;
  }

  function clientsFor(desktopId: string): MockClient[] {
    return mockClients.filter((candidate) => candidate.credentials.expectedDesktopId === desktopId);
  }

  /** The commands one machine's client was asked to run. */
  function commandsFor(desktopId: string): string[] {
    return clientFor(desktopId).requests.map((record) => record.command.type);
  }

  beforeEach(() => {
    jest.clearAllMocks();
    mockClients.length = 0;
    jest.mocked(loadPairedDesktops).mockResolvedValue([
      { desktopId: "desktop_1", pairId: "pair-desktop_1", name: "Studio" },
      { desktopId: "desktop_2", pairId: "pair-desktop_2", name: "Office" },
    ]);
    jest.mocked(loadCredentials).mockImplementation(async (desktopId?: string) =>
      desktopId ? credentialsFor(desktopId) : credentialsFor("desktop_1"));
    remote = undefined as unknown as Consumer;
    renderer = null;
  });

  afterEach(async () => {
    if (renderer) {
      await act(async () => { renderer!.unmount(); });
      renderer = null;
    }
  });

  test("every paired desktop holds its own live runtime, the open one included", async () => {
    await mount();
    await settle();
    // Two desktops, two connections. The open machine is not a special case
    // any more: it is the one whose runtime the screens read, not the only one
    // that exists.
    expect(clientsFor("desktop_1")).toHaveLength(1);
    expect(clientsFor("desktop_2")).toHaveLength(1);
    expect(clientFor("desktop_1").open).toHaveBeenCalled();
    expect(clientFor("desktop_2").open).toHaveBeenCalled();
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_1");
    expect(remote.phase).toBe("ready");
  });

  test("selecting another desktop reconnects nothing", async () => {
    await mount();
    await settle();
    const studio = clientFor("desktop_1");
    const office = clientFor("desktop_2");

    await act(async () => { await remote.switchDesktop("desktop_2"); });
    await settle();

    // The whole point: no socket is rebuilt, so the conversation the user asked
    // for appears as fast as one on the machine already being read.
    expect(studio.close).not.toHaveBeenCalled();
    expect(office.close).not.toHaveBeenCalled();
    expect(clientsFor("desktop_1")).toHaveLength(1);
    expect(clientsFor("desktop_2")).toHaveLength(1);
    expect(office.open).toHaveBeenCalledTimes(1);
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_2");
    expect(remote.phase).toBe("ready");

    // And back again, with the same two sockets.
    await act(async () => { await remote.switchDesktop("desktop_1"); });
    await settle();
    expect(studio.close).not.toHaveBeenCalled();
    expect(clientsFor("desktop_1")).toHaveLength(1);
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_1");
  });

  test("a conversation on another desktop is read from that desktop", async () => {    await mount();
    await settle();
    const studio = clientFor("desktop_1");
    const office = clientFor("desktop_2");
    studio.requests.length = 0;
    office.requests.length = 0;

    await act(async () => { await remote.openSessionOnDesktop("desktop_2", "b-1"); });
    await settle();

    expect(remote.credentials?.expectedDesktopId).toBe("desktop_2");
    expect(remote.selectedSessionId).toBe("b-1");
    // The conversation is read down its own machine's connection; the machine
    // that was open a moment ago is not addressed with an id it never had.
    expect(office.requests.some((record) => record.command.sessionId === "b-1")).toBe(true);
    expect(studio.requests.some((record) => record.command.sessionId === "b-1")).toBe(false);
    expect(studio.close).not.toHaveBeenCalled();
  });

  test("the open conversation survives going to another desktop and coming back", async () => {
    await mount();
    await settle();
    await act(async () => { await remote.openSessionOnDesktop("desktop_1", "a-1"); });
    await settle();
    const studio = clientFor("desktop_1");
    studio.requests.length = 0;

    await act(async () => { await remote.switchDesktop("desktop_2"); });
    await settle();
    expect(remote.selectedSessionId).toBe("");

    await act(async () => { await remote.switchDesktop("desktop_1"); });
    await settle();
    // Each runtime keeps its own conversation, so returning to a machine lands
    // on the conversation that was open there rather than on an empty screen.
    expect(remote.selectedSessionId).toBe("a-1");
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_1");
  });

  /**
   * A conversation asked for *by name* is created on the machine that was named.
   *
   * The screens hold the value they were rendered with, so the open machine's own
   * verb acts on the machine the phone was on when that screen drew itself — the
   * test below measures that. A name is what survives a move, which is exactly
   * what a group on another machine, and the creation dialog, are doing.
   */
  test("a new conversation asked for by name is created on that machine", async () => {
    await mount();
    await settle();
    const office = clientFor("desktop_2");
    const readsBefore = office.requests.filter((record) =>
      record.command.type === "list_models" || record.command.type === "list_workspaces").length;

    const stale = remote;
    await act(async () => { await stale.newConversationOn("desktop_2", "workspace", "w9"); });
    await settle();

    // Built on the named machine, and the phone is on it: the screens read the
    // draft that was just created, in the workspace that was asked for.
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_2");
    expect(remote.draft).toBe(true);
    expect(remote.draftMode).toBe("workspace");
    expect(remote.draftWorkspaceId).toBe("w9");
    // The draft's model comes from the machine that will run it, and every paired
    // machine reads its own catalogue — models included — as it connects, so
    // choosing one reads nothing on its behalf.
    expect(office.requests.filter((record) =>
      record.command.type === "list_models" || record.command.type === "list_workspaces").length)
      .toBe(readsBefore);
    expect(readsBefore).toBeGreaterThanOrEqual(2);
  });

  /**
   * The open machine's verb follows the screen that drew it, not the phone.
   *
   * Not a contract worth keeping — it is the measured reason the named verb
   * exists. A screen that moved the phone and then asked the open machine's verb
   * for a conversation built the draft on the machine the phone had just left;
   * this is where that was found (the activity rail's workspace sheet, and the
   * creation dialog's "which machine" step).
   */
  test("the open machine's verb follows the screen, not where the phone moved", async () => {
    await mount();
    await settle();
    const stale = remote;

    await act(async () => { await stale.switchDesktop("desktop_2"); });
    await settle();
    await act(async () => { await stale.newConversation("workspace", "w1"); });
    await settle();
    // The phone is on the machine it moved to, and there is no draft on it…
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_2");
    expect(remote.draft).toBe(false);

    await act(async () => { await stale.switchDesktop("desktop_1"); });
    await settle();
    // …because it was built on the machine the value was drawn with.
    expect(remote.draft).toBe(true);
    expect(remote.draftWorkspaceId).toBe("w1");
  });

  test("a machine that cannot take a conversation refuses it by name", async () => {
    // Paired, but with no credentials on this device: its runtime never gets a
    // client. The wait for one is bounded, so the tap fails instead of hanging.
    jest.mocked(loadCredentials).mockImplementation(async (desktopId?: string) =>
      desktopId === "desktop_2" ? null : credentialsFor(desktopId ?? "desktop_1"));
    jest.useFakeTimers();
    try {
      await mount();
      await settle();
      const pending = remote.newConversationOn("desktop_2", "chat", "");
      const refusal = expect(pending).rejects.toThrow("desktop_unavailable");
      for (let elapsed = 0; elapsed < 20_000; elapsed += 250)
        await act(async () => { jest.advanceTimersByTime(250); });
      await refusal;

      // A machine that is not paired at all has no runtime to wait for.
      await expect(remote.newConversationOn("desktop_9", "chat", "")).rejects
        .toThrow("desktop_unavailable");
      // Nothing moved and nothing was created anywhere.
      expect(remote.credentials?.expectedDesktopId).toBe("desktop_1");
    } finally {
      jest.useRealTimers();
    }
  });

  /**
   * A device page reads the machine it names.
   *
   * Every paired machine keeps its own runtime, so a page opened for a machine
   * the phone is not reading reads *that* machine — its catalogue, its settings —
   * with no connection step and without the phone being pointed at it. This is
   * the half a screen-level test cannot see: mocking the context makes a page
   * that quietly read the open machine pass just the same, and that is how a
   * settings write lands on a machine the user was not looking at.
   */
  test("a device page reads its own machine, and never the one the phone is on", async () => {
    await mount();
    await settle();
    let page: RemoteControls | null = null;
    function DevicePage(): null {
      page = useRemoteControls();
      return null;
    }
    await act(async () => {
      renderer!.update(createElement(RemoteProvider, null,
        createElement(Consumer),
        createElement(DesktopScope, { desktopId: "desktop_2" }, createElement(DevicePage))));
    });
    act(() => {
      clientFor("desktop_1").callbacks.onPresence?.(onlinePresence("desktop_1") as never);
      clientFor("desktop_2").callbacks.onPresence?.(onlinePresence("desktop_2") as never);
    });
    await settle();
    const studio = clientFor("desktop_1");
    const office = clientFor("desktop_2");
    studio.requests.length = 0;
    office.requests.length = 0;

    // The page's own state is the machine it names — including the catalogue it
    // would render — and not the open machine's.
    expect(page!.sessions.map((session) => session.sessionId)).toEqual(["b-1"]);
    expect(remote.sessions.map((session) => session.sessionId)).toEqual(["a-1"]);
    expect(page!.desktopOnline).toBe(true);

    await act(async () => { await page!.getDesktopSettings(); });
    await act(async () => { await remote.getDesktopSettings(); });

    // Each surface addresses its own machine's connection: the named one for the
    // page, the open one for the screens around it.
    expect(commandsFor("desktop_2")).toContain("get_desktop_settings");
    expect(commandsFor("desktop_1")).toContain("get_desktop_settings");
    expect(office.requests.filter((record) => record.command.type === "get_desktop_settings")).toHaveLength(1);
    expect(studio.requests.filter((record) => record.command.type === "get_desktop_settings")).toHaveLength(1);
    // Nothing moved the phone: reading a machine is not choosing it.
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_1");
  });

  test("the merged list holds every machine's conversations, with each one's status", async () => {
    await mount();
    await settle();
    const studio = clientFor("desktop_1");
    const office = clientFor("desktop_2");
    act(() => {
      studio.callbacks.onPresence?.(onlinePresence("desktop_1") as never);
      office.callbacks.onPresence?.(onlinePresence("desktop_2") as never);
    });
    await settle();

    const ids = remote.catalogs.catalogs.map((entry) => entry.desktopId).sort();
    expect(ids).toEqual(["desktop_1", "desktop_2"]);
    // The open machine contributes its catalogue, and the other its own.
    const byDesktop = new Map(remote.catalogs.catalogs.map((entry) => [entry.desktopId, entry]));
    expect(byDesktop.get("desktop_1")!.sessions.map((s) => s.sessionId)).toEqual(["a-1"]);
    expect(byDesktop.get("desktop_2")!.sessions.map((s) => s.sessionId)).toEqual(["b-1"]);
    // Only machines *other* than the open one are peer rows: the open one is
    // already the machine the screens are rendering.
    expect(remote.catalogs.machines.map((machine) => machine.desktopId)).toEqual(["desktop_2"]);
    expect(remote.catalogs.machines[0]!.status).toBe("connected");
    expect(remote.catalogs.machines[0]!.label).toBe("Office");
  });

  test("a machine whose heartbeat stops reads offline while the other stays connected", async () => {
    await mount();
    await settle();
    act(() => {
      clientFor("desktop_1").callbacks.onPresence?.(onlinePresence("desktop_1") as never);
      clientFor("desktop_2").callbacks.onPresence?.({
        ...onlinePresence("desktop_2"),
        online: false,
        lastHeartbeatTs: 0,
      } as never);
    });
    await settle();
    expect(remote.catalogs.machines[0]!.status).toBe("offline");
    expect(remote.desktopOnline).toBe(true);
  });

  test("another machine's conversation is commanded through its own connection", async () => {
    await mount();
    await settle();
    act(() => { clientFor("desktop_2").callbacks.onPresence?.(onlinePresence("desktop_2") as never); });
    await settle();
    const office = clientFor("desktop_2");
    const studio = clientFor("desktop_1");
    office.requests.length = 0;
    studio.requests.length = 0;

    const commands = remote.catalogs.commandsFor("desktop_2");
    expect(commands).not.toBeNull();
    await act(async () => {
      await commands!.renameSession("b-1", "Renamed");
    });
    expect(commandsFor("desktop_2")).toContain("set_session_name");
    expect(commandsFor("desktop_1")).not.toContain("set_session_name");
    // The host pushes the change too, but that push can be lost, so the phone
    // re-reads the machine's own catalogue after an acknowledged command.
    expect(commandsFor("desktop_2")).toContain("list_sessions");
    expect(studio.requests).toHaveLength(0);
  });

  test("a machine that is not ready hands out no commands", async () => {
    await mount();
    await settle();
    act(() => { clientFor("desktop_2").callbacks.onConnectionState?.("reconnecting" as never); });
    await settle();
    expect(remote.catalogs.commandsFor("desktop_2")).toBeNull();
  });

  test("another machine's workspaces are read from that machine", async () => {
    await mount();
    await settle();
    const office = clientFor("desktop_2");
    const studio = clientFor("desktop_1");
    // Both machines have read their own catalogue by now (connecting is what
    // does it), so the read below is the one this test is about.
    office.requests.length = 0;
    studio.requests.length = 0;
    await act(async () => {
      await remote.catalogs.readWorkspaces("desktop_2");
    });
    expect(commandsFor("desktop_2")).toContain("list_workspaces");
    expect(commandsFor("desktop_1")).not.toContain("list_workspaces");
  });

  test("removing another machine takes its runtime and its rows away", async () => {
    await mount();
    await settle();
    const office = clientFor("desktop_2");
    jest.mocked(loadCredentials).mockImplementation(async (desktopId?: string) =>
      desktopId === "desktop_2" ? credentialsFor("desktop_2") : null);
    jest.mocked(loadPairedDesktops).mockResolvedValue([
      { desktopId: "desktop_1", pairId: "pair-desktop_1", name: "Studio" },
    ]);

    await act(async () => { await remote.removeDesktop("desktop_2"); });
    await settle();

    expect(office.close).toHaveBeenCalled();
    expect(remote.desktops.map((desktop) => desktop.desktopId)).toEqual(["desktop_1"]);
    expect(remote.catalogs.catalogs.map((entry) => entry.desktopId)).toEqual(["desktop_1"]);
    expect(remote.catalogs.machines).toEqual([]);
  });

  test("rebuilding one machine's runtime leaves the other machine's alone", async () => {
    await mount();
    await settle();
    const studio = clientFor("desktop_1");
    const office = clientFor("desktop_2");
    act(() => { clientFor("desktop_2").callbacks.onConnectionState?.("failed" as never); });
    await settle();
    await act(async () => { await remote.switchDesktop("desktop_2"); });
    await settle();

    await act(async () => { await remote.reconnect(); });
    await settle();

    // The failed machine is rebuilt from scratch…
    expect(office.close).toHaveBeenCalled();
    expect(clientsFor("desktop_2")).toHaveLength(2);
    expect(clientFor("desktop_2").open).toHaveBeenCalled();
    // …while a healthy machine keeps the connection it has: recovering one
    // machine must not cost the others their sockets.
    expect(studio.close).not.toHaveBeenCalled();
    expect(clientsFor("desktop_1")).toHaveLength(1);
  });

  test("a healthy open machine is probed rather than rebuilt", async () => {
    await mount();
    await settle();
    const studio = clientFor("desktop_1");
    await act(async () => { await remote.reconnect(); });
    await settle();
    expect(studio.recoverNow).toHaveBeenCalledWith("request-failure");
    expect(studio.close).not.toHaveBeenCalled();
    expect(clientsFor("desktop_1")).toHaveLength(1);
  });

  test("the notification channel is prepared once the open machine is ready and paired", async () => {
    await mount();
    await settle();
    expect(prepareTaskNotifications).toHaveBeenCalledTimes(1);
  });

  test("a freshly paired desktop is readable before its runtime finishes booting", async () => {
    // Nothing paired yet, and the new desktop's runtime never finishes reading
    // its credentials: the app must still know which desktop it is on.
    jest.mocked(loadPairedDesktops).mockResolvedValue([]);
    await mount();
    await settle();
    expect(remote.credentials).toBeNull();

    jest.mocked(claimPairingCode).mockResolvedValue(credentialsFor("desktop_9"));
    jest.mocked(loadCredentials).mockReturnValue(new Promise(() => {}));
    jest.mocked(loadPairedDesktops).mockResolvedValue([
      { desktopId: "desktop_9", pairId: "pair-desktop_9" },
    ]);
    await act(async () => { await remote.pair("code"); });
    await settle();

    // The claim is what the screens read until the runtime reports its own:
    // without it the app would show the picker for a machine it has just been
    // told it is connected to.
    expect(remote.credentials?.expectedDesktopId).toBe("desktop_9");
    expect(remote.phase).not.toBe("unpaired");
  });

  test("a machine nobody is reading is reached again after its connection gave up", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      await settle();
      const office = clientFor("desktop_2");
      act(() => { clientFor("desktop_2").callbacks.onConnectionState?.("failed" as never); });
      await settle();

      // Nothing on screen offers a way back for a machine the user is not
      // reading, so the phone keeps trying: a desktop that was asleep when the
      // app started must not read offline until the app restarts.
      await act(async () => { jest.advanceTimersByTime(60_000); });
      await settle();
      expect(office.close).toHaveBeenCalled();
      expect(clientsFor("desktop_2")).toHaveLength(2);
      // The machine being read is the user's to retry, and is left alone.
      expect(clientsFor("desktop_1")).toHaveLength(1);
    } finally {
      jest.useRealTimers();
    }
  });

  test("a machine that is merely connecting is left to its own recovery", async () => {
    jest.useFakeTimers();
    try {
      await mount();
      await settle();
      act(() => { clientFor("desktop_2").callbacks.onConnectionState?.("reconnecting" as never); });
      await settle();
      await act(async () => { jest.advanceTimersByTime(180_000); });
      await settle();
      // Rebuilding a machine that is already recovering would throw away a
      // handshake in flight.
      expect(clientsFor("desktop_2")).toHaveLength(1);
    } finally {
      jest.useRealTimers();
    }
  });

  test("a desktop dropped from storage keeps the app on the picker rather than a phantom", async () => {
    jest.mocked(loadPairedDesktops).mockResolvedValue([]);
    await mount();
    await settle();
    expect(remote.phase).toBe("unpaired");
    expect(remote.credentials).toBeNull();
    expect(remote.catalogs.machines).toEqual([]);
    expect(mockClients).toHaveLength(0);
  });

  test("the controls and the transcript see the open machine's states separately", async () => {
    let controls: RemoteControls | undefined;
    function Controls(): null {
      controls = useRemoteControls();
      return null;
    }
    await act(async () => {
      renderer = create(
        createElement(RemoteProvider, null, createElement(Controls)),
      );
    });
    await settle();
    expect(controls!.selectedSessionId).toBe("");
    expect(controls!.phase).toBe("ready");
    // `timeline` is deliberately absent from the controls: a streamed delta must
    // not re-render every control on the screen.
    expect((controls as unknown as { timeline?: unknown }).timeline).toBeUndefined();
  });
});

