// @vitest-environment jsdom
import type { RemotePeer } from "../../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { mergeConversations, rowKey } from "../../../features/remote-peer/mergeConversations";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useRemotePeers } from "./useRemotePeers";

/**
 * The peer list's live half.
 *
 * The list itself is polled (nothing announces a *drop* — the host is still
 * alive, just not to us), so what is asserted here is the one thing a poll would
 * otherwise lag on by a whole interval: a link that came back.
 */

const listRemotePeers = vi.fn<() => Promise<RemotePeer[]>>();
const fetchRemoteSessions = vi.fn<(desktopId: string) => Promise<unknown>>();
const connectAllRemotePeers = vi.fn<() => Promise<RemotePeer[]>>();
const connectRemotePeer = vi.fn<(desktopId: string) => Promise<RemotePeer>>();
// The host-catalogue reads the poll warms as a connection is established
// (`remoteHostCatalog`); none of the assertions here is about them, but a mock
// that omitted them would take the whole refresh down with it.
const listRemoteModels = vi.fn(async () => []);
const getRemoteDesktopSettings = vi.fn(async () => ({ skillRecommend: false }));
vi.mock("../../../features/remote-peer/remotePeerClient", async (importOriginal) => {
  // The real parser rides along: a pushed snapshot and a polled one reach the
  // list through one field list, and these tests are what prove a field the host
  // sends is not dropped on the push path. Everything that would reach the
  // backend is stubbed.
  const actual = await importOriginal<typeof import("../../../features/remote-peer/remotePeerClient")>();
  return {
    ...actual,
    connectAllRemotePeers: () => connectAllRemotePeers(),
    connectRemotePeer: (...args: Parameters<typeof connectRemotePeer>) => connectRemotePeer(...args),
    fetchRemoteSessions: (...args: Parameters<typeof fetchRemoteSessions>) => fetchRemoteSessions(...args),
    getRemoteDesktopSettings: () => getRemoteDesktopSettings(),
    listRemoteAvailableSkills: () => Promise.resolve([]),
    listRemoteInstalledSkills: () => Promise.resolve([]),
    listRemoteModels: () => listRemoteModels(),
    listRemotePeers: () => listRemotePeers(),
    listRemoteWorkspaces: () => Promise.resolve([]),
  };
});

let listeners: Array<(event: { payload: unknown }) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.push(handler);
    return () => {
      listeners = listeners.filter(item => item !== handler);
    };
  },
}));

function peer(overrides: Partial<RemotePeer> = {}): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "bridge_1",
    connected: true,
    stopped: false,
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    ...overrides,
  };
}

/** A push of the given kind, as the backend forwards it. */
function pushKind(kind: string, desktopId = "desktop_a", payload: unknown = {}): void {
  act(() => {
    listeners.forEach(listener => listener({ payload: { desktopId, kind, payload } }));
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await flushAsync();
}

beforeEach(() => {
  listeners = [];
  listRemotePeers.mockReset().mockResolvedValue([peer()]);
  fetchRemoteSessions.mockReset().mockResolvedValue({ desktopId: "desktop_a", sessions: [] });
  connectAllRemotePeers.mockReset().mockResolvedValue([]);
  connectRemotePeer.mockReset().mockResolvedValue(peer());
});

/**
 * The liveness pass: what happens to the machines nobody is reading.
 *
 * Nothing on screen retries a host that is not the one being read, so without
 * this a machine that was asleep when the app started — or that dropped while
 * the user was elsewhere — stayed out of the merged list for the rest of the
 * session. It is the desktop half of the phone's "hold a live connection to
 * every paired desktop".
 */
describe("holding every paired host", () => {
  it("brings up a paired host that is not connected, as soon as the list is known", async () => {
    let connected = false;
    listRemotePeers.mockImplementation(async () => [peer({ connected })]);
    connectRemotePeer.mockImplementation(async () => {
      connected = true;
      return peer({ connected: true });
    });

    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    expect(connectRemotePeer).toHaveBeenCalledWith("desktop_a");
    // The list is re-read, so the row the user sees is the state the connect
    // left behind rather than the state before it.
    expect(hook.current.peers[0]!.connected).toBe(true);
    hook.unmount();
  });

  /**
   * The distinction the backend's `stopped` exists for: the pass reaches a host
   * that dropped, and does not undo a host the user turned off.
   */
  it("leaves a host the user stopped alone", async () => {
    listRemotePeers.mockResolvedValue([peer({ connected: false, stopped: true })]);

    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    expect(connectRemotePeer).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("asks again on the backstop, for the hosts that are still down", async () => {
    vi.useFakeTimers();
    try {
      listRemotePeers.mockResolvedValue([peer({ connected: false })]);
      const hook = renderHook(() => useRemotePeers(true));
      await settle();
      expect(connectRemotePeer).toHaveBeenCalledTimes(1);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      await settle();

      expect(connectRemotePeer).toHaveBeenCalledTimes(2);
      hook.unmount();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("reconnect all asks for every machine, the stopped ones included", async () => {
    listRemotePeers.mockResolvedValue([peer({ connected: false, stopped: true })]);
    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    await act(async () => {
      await hook.current.reconnectAll();
    });

    // The per-host pass skips what the user turned off; asking for everything is
    // the one call allowed to overrule it.
    expect(connectAllRemotePeers).toHaveBeenCalledTimes(1);
    expect(connectRemotePeer).not.toHaveBeenCalled();
    hook.unmount();
  });
});

it("re-reads the hosts when one of them comes back", async () => {
  const hook = renderHook(() => useRemotePeers(true));
  await settle();
  listRemotePeers.mockClear();
  fetchRemoteSessions.mockClear();

  pushKind("resumed");
  await settle();

  // The whole point: the rows the host moved while the link was down are the
  // rows that are now stale, and no session event will arrive to correct them.
  expect(listRemotePeers).toHaveBeenCalledTimes(1);
  hook.unmount();
});

/**
 * A presence tick arrives on a healthy link too, so it must not be read as a
 * resume — otherwise the list re-reads on every heartbeat.
 */
it("does not re-read on a presence tick", async () => {
  const hook = renderHook(() => useRemotePeers(true));
  await settle();
  listRemotePeers.mockClear();

  pushKind("presence");
  await settle();

  expect(listRemotePeers).not.toHaveBeenCalled();
  hook.unmount();
});

it("does nothing while disabled", async () => {
  const hook = renderHook(() => useRemotePeers(false));
  await settle();
  listRemotePeers.mockClear();

  pushKind("resumed");
  await settle();

  expect(listRemotePeers).not.toHaveBeenCalled();
  hook.unmount();
});

/** A dead listener must not keep calling back into a hook that has gone. */
it("stops listening once unmounted", async () => {
  const hook = renderHook(() => useRemotePeers(true));
  await settle();
  hook.unmount();
  listRemotePeers.mockClear();

  pushKind("resumed");
  await settle();

  expect(listRemotePeers).not.toHaveBeenCalled();
});

describe("push catalog", () => {
  it("applies a pushed sessions snapshot without fetching", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: { epoch: "epoch_1", revision: 1 },
      sessions: [
        { sessionId: "sess_1", title: "Snapshot Row" },
      ],
    });
    await settle();

    expect(fetchRemoteSessions).not.toHaveBeenCalled();
    expect(hook.current.catalogs).toHaveLength(1);
    const catalog = hook.current.catalogs[0]!;
    expect(catalog.sessions).toHaveLength(1);
    expect(catalog.sessions[0]!.sessionId).toBe("sess_1");
    expect(catalog.sessions[0]!.title).toBe("Snapshot Row");
    hook.unmount();
  });

  it("ignores an out-of-order (old) snapshot", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    // Seed the gate with a newer revision first.
    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: { epoch: "epoch_1", revision: 2 },
      sessions: [{ sessionId: "sess_1", title: "Fresh" }],
    });
    // Then try to apply an older one.
    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: { epoch: "epoch_1", revision: 1 },
      sessions: [{ sessionId: "sess_1", title: "Stale" }],
    });
    await settle();

    expect(fetchRemoteSessions).not.toHaveBeenCalled();
    expect(hook.current.catalogs[0]!.sessions[0]!.title).toBe("Fresh");
    hook.unmount();
  });

  /**
   * The activity packet is the host's replacement for re-sending a whole session
   * list once a second while a run streams. It is a *packet*: it names the rows
   * whose ordering timestamp moved and nothing else, so it must patch what the
   * catalogue already holds rather than be read as a list.
   */
  describe("sessions_activity", () => {
    it("moves a row's ordering timestamp without re-reading the host", async () => {
      const hook = renderHook(() => useRemotePeers(true));
      await settle();
      fetchRemoteSessions.mockClear();

      pushKind("catalog", "desktop_a", {
        domain: "sessions",
        version: { epoch: "epoch_1", revision: 1 },
        sessions: [
          { sessionId: "sess_1", title: "One", lastMessageAt: 100 },
          { sessionId: "sess_2", title: "Two", lastMessageAt: 200 },
        ],
      });
      pushKind("catalog", "desktop_a", {
        domain: "sessions_activity",
        version: { epoch: "epoch_1", revision: 2 },
        activity: [{ sessionId: "sess_1", lastMessageAt: 900 }],
      });
      await settle();

      expect(fetchRemoteSessions).not.toHaveBeenCalled();
      const sessions = hook.current.catalogs[0]!.sessions;
      // The moved row takes the new value; the row the packet does not mention
      // keeps everything it had — its absence is not a deletion.
      expect(sessions).toHaveLength(2);
      expect(sessions[0]!.lastMessageAt).toBe(900);
      expect(sessions[1]!.lastMessageAt).toBe(200);
      expect(sessions[1]!.title).toBe("Two");
      hook.unmount();
    });

    it("cannot introduce a row the catalogue has never held", async () => {
      const hook = renderHook(() => useRemotePeers(true));
      await settle();

      pushKind("catalog", "desktop_a", {
        domain: "sessions",
        version: { epoch: "epoch_1", revision: 1 },
        sessions: [{ sessionId: "sess_1", title: "One", lastMessageAt: 100 }],
      });
      pushKind("catalog", "desktop_a", {
        domain: "sessions_activity",
        version: { epoch: "epoch_1", revision: 2 },
        activity: [
          { sessionId: "sess_1", lastMessageAt: 900 },
          { sessionId: "sess_new", lastMessageAt: 900 },
        ],
      });
      await settle();

      expect(hook.current.catalogs[0]!.sessions.map(row => row.sessionId)).toEqual(["sess_1"]);
      hook.unmount();
    });

    it("refuses a repeated or older packet", async () => {
      const hook = renderHook(() => useRemotePeers(true));
      await settle();

      pushKind("catalog", "desktop_a", {
        domain: "sessions",
        version: { epoch: "epoch_1", revision: 1 },
        sessions: [{ sessionId: "sess_1", title: "One", lastMessageAt: 100 }],
      });
      pushKind("catalog", "desktop_a", {
        domain: "sessions_activity",
        version: { epoch: "epoch_1", revision: 2 },
        activity: [{ sessionId: "sess_1", lastMessageAt: 900 }],
      });
      // The same revision again (at-most-once delivery makes a duplicate
      // ordinary), then one that is behind it.
      pushKind("catalog", "desktop_a", {
        domain: "sessions_activity",
        version: { epoch: "epoch_1", revision: 2 },
        activity: [{ sessionId: "sess_1", lastMessageAt: 900 }],
      });
      pushKind("catalog", "desktop_a", {
        domain: "sessions_activity",
        version: { epoch: "epoch_1", revision: 1 },
        activity: [{ sessionId: "sess_1", lastMessageAt: 500 }],
      });
      await settle();

      expect(hook.current.catalogs[0]!.sessions[0]!.lastMessageAt).toBe(900);
      hook.unmount();
    });

    it("is ignored before the epoch is authenticated", async () => {
      const hook = renderHook(() => useRemotePeers(true));
      await settle();
      fetchRemoteSessions.mockClear();

      // No presence and no snapshot yet: the packet names an epoch this client
      // has not seen, so it describes a host generation it cannot trust.
      pushKind("catalog", "desktop_a", {
        domain: "sessions_activity",
        version: { epoch: "epoch_1", revision: 1 },
        activity: [{ sessionId: "sess_1", lastMessageAt: 900 }],
      });
      await settle();

      expect(hook.current.catalogs[0]!.sessions).toHaveLength(0);
      hook.unmount();
    });
  });
});

describe("live events", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  /**
   * What an event settles is the run, not the row's place: the ordering value
   * belongs to the host's clock, and the host states it itself (a snapshot when
   * the structure moves, `state.activity` when only the value did).
   *
   * The stamp this replaced was this machine's `Date.now()`, so every event put
   * the row above every one of *this* machine's conversations — and the host's
   * next report of it, on the host's clock, put it back. That is the jumping
   * this test pins down: the value it starts with is the one it keeps.
   */
  it("settles whether a conversation is running without moving the row", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: { epoch: "epoch_1", revision: 1 },
      sessions: [{ sessionId: "sess_1", title: "One", lastMessageAt: 1_000 }],
    });
    pushKind("event", "desktop_a", {
      sessionId: "sess_1",
      runId: "run_1",
      type: "text_chunk",
      data: "{\"text\":\"hi\"}",
    });
    await settle();

    const [session] = hook.current.catalogs[0]!.sessions;
    expect(session!.streaming).toBe(true);
    expect(session!.lastMessageAt).toBe(1_000);

    // A run that settles clears the dot and still leaves the value alone: the
    // host's own report of the last message is what moves the row from here.
    pushKind("event", "desktop_a", { sessionId: "sess_1", runId: "run_1", type: "agent_end", data: "{}" });
    await settle();
    expect(hook.current.catalogs[0]!.sessions[0]!.streaming).toBe(false);
    expect(hook.current.catalogs[0]!.sessions[0]!.lastMessageAt).toBe(1_000);
    hook.unmount();
  });

  /**
   * The same rule as the user sees it, over the merged list: a host's row cannot
   * overtake this machine's conversation by *speaking*, only by the host saying
   * it moved. The phone's list never could — it draws the host's rows in the
   * host's own order — which is why the desktop was the only place the order
   * jumped.
   */
  it("cannot overtake this machine's rows until the host says the row moved", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(9_000_000);
    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: { epoch: "epoch_1", revision: 1 },
      sessions: [{ sessionId: "sess_remote", title: "On the host", lastMessageAt: 1_000_000 }],
    });
    const order = () => mergeConversations(
      [{
        id: "local_1",
        title: "Mine",
        mode: "chat",
        pinned: false,
        status: "active",
        workspaceId: "ws_1",
        lastMessageAt: 5_000_000,
        updatedAt: 5_000_000,
        createdAt: 5_000_000,
      }],
      hook.current.catalogs,
      { kind: "all" },
    ).map(row => row.key);
    expect(order()).toEqual([rowKey(null, "local_1"), rowKey("desktop_a", "sess_remote")]);

    // The host's conversation is answering, on *this* machine's clock read as
    // "now": the row is running, and its place is still the host's to give.
    pushKind("event", "desktop_a", {
      sessionId: "sess_remote",
      runId: "run_1",
      type: "text_chunk",
      data: "{\"text\":\"hi\"}",
    });
    await settle();
    expect(order()).toEqual([rowKey(null, "local_1"), rowKey("desktop_a", "sess_remote")]);

    // The host's own report — the same subject the desktop now relies on — is
    // what puts the row where the host has it.
    pushKind("catalog", "desktop_a", {
      domain: "sessions_activity",
      version: { epoch: "epoch_1", revision: 2 },
      activity: [{ sessionId: "sess_remote", lastMessageAt: 9_000_000 }],
    });
    await settle();
    expect(order()).toEqual([rowKey("desktop_a", "sess_remote"), rowKey(null, "local_1")]);
    hook.unmount();
  });
});

/**
 * Control: who may drive the machine.
 *
 * This is the one fact a presence tick carries that the catalogue cannot:
 * while another paired device drives the host, this client is refused every
 * read, so the host's rows are **absent** from the merged list — and they stay
 * absent after a move back until something re-reads them. A list that only
 * corrected itself on the next five-second poll (or on a command that happened
 * to fail) is the "the list is not current" the user sees.
 */
describe("control", () => {
  /** A list that answers with the control the host last reported. */
  function controlSource(initial: "active" | "standby") {
    let control = initial;
    listRemotePeers.mockImplementation(async () => [
      peer({
        control,
        ...(control === "standby" ? { holder: { name: "Hui's phone" } } : {}),
      }),
    ]);
    return {
      set: (next: "active" | "standby") => {
        control = next;
      },
    };
  }

  it("re-reads as soon as the host says another device took it", async () => {
    const source = controlSource("active");
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    listRemotePeers.mockClear();
    fetchRemoteSessions.mockClear();

    source.set("standby");
    pushKind("presence", "desktop_a", { control: "standby" });
    await settle();

    // The list is re-read, and the machine now says so — which is what turns
    // the dot amber and puts the reason in the picker.
    expect(listRemotePeers).toHaveBeenCalledTimes(1);
    expect(hook.current.peers[0]!.control).toBe("standby");
    // And nothing is asked of a machine that cannot answer.
    expect(fetchRemoteSessions).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("re-reads the host's conversations when control comes back", async () => {
    const source = controlSource("standby");
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    expect(hook.current.catalogs).toEqual([]);
    fetchRemoteSessions.mockClear();

    source.set("active");
    pushKind("presence", "desktop_a", { control: "active" });
    await settle();

    // The rows the machine could not show while it stood by are readable the
    // moment it takes control back, and this is what puts them back into the
    // merged list without waiting for the next poll.
    expect(fetchRemoteSessions).toHaveBeenCalledTimes(1);
    expect(hook.current.peers[0]!.control).toBe("active");
    hook.unmount();
  });

  it("does not ask a machine in standby for its catalogue, on either cadence", async () => {
    controlSource("standby");
    // The read-ahead mocks are shared with the tests above (their calls are
    // real catalogue fills for an active host); only this test's own mounts
    // count.
    listRemoteModels.mockClear();
    getRemoteDesktopSettings.mockClear();
    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    // Mounting runs the list poll and the host-catalogue read-ahead; neither
    // may address a host that refuses business commands. A refusal per poll
    // would be one wasted round trip every five seconds, for the life of the
    // connection.
    expect(fetchRemoteSessions).not.toHaveBeenCalled();
    expect(listRemoteModels).not.toHaveBeenCalled();
    expect(getRemoteDesktopSettings).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("re-reads on the control packet, which arrives before the tick that names the holder", async () => {
    controlSource("active");
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    listRemotePeers.mockClear();

    pushKind("control", "desktop_a", { control: "standby", pairId: "pair_1" });
    await settle();

    expect(listRemotePeers).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("does not re-read on a tick that merely restates the control it already has", async () => {
    controlSource("active");
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    listRemotePeers.mockClear();

    // No catalogVersion, and the same control: an ordinary heartbeat.
    pushKind("presence", "desktop_a", { control: "active" });
    await settle();

    expect(listRemotePeers).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("does not try to heal a standby machine's snapshot revision", async () => {
    const source = controlSource("standby");
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    // A standby device is not sent snapshots, so a revision ahead of the gate
    // says nothing about a lost one. Pulling it would only confirm the refusal.
    pushKind("presence", "desktop_a", {
      control: "standby",
      catalogVersion: { epoch: "epoch_1", sessions: 9, workspaces: 9 },
    });
    await settle();

    expect(fetchRemoteSessions).not.toHaveBeenCalled();
    expect(source).toBeDefined();
    hook.unmount();
  });
});

describe("heartbeat self-heal", () => {
  it("re-reads once when the presence revision is ahead", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    pushKind("presence", "desktop_a", {
      catalogVersion: { epoch: "epoch_1", sessions: 5, workspaces: 0 },
    });
    await settle();

    expect(fetchRemoteSessions).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("re-reads for a lost snapshot, then again on the next tick (see module-desktop.md §8.6)", async () => {
    // CHARACTERISATION of the current behaviour, not of the design: the presence
    // path calls `ensureGate` → `gate.authenticate(epoch)` on *every* tick, and
    // `authenticate` clears the applied revisions — so the revision comparison
    // that follows always sees the -1 floor and every tick heals, i.e. a peer
    // list + catalogue read per heartbeat (the host publishes presence every 3 s;
    // `remote/publisher.rs::presence_tick`). The `gate.accept(...)` below the
    // comparison cannot suppress the next tick, which is what its comment claims.
    // Asserted as-is so the fix (authenticate only when the epoch *changes*) is a
    // deliberate, visible test change — see module-desktop.md §8.6.
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    const heartbeat = { catalogVersion: { epoch: "epoch_1", sessions: 5, workspaces: 0 } };
    pushKind("presence", "desktop_a", heartbeat);
    await settle();
    expect(fetchRemoteSessions).toHaveBeenCalledTimes(1);

    pushKind("presence", "desktop_a", heartbeat);
    await settle();
    expect(fetchRemoteSessions).toHaveBeenCalledTimes(2);
    hook.unmount();
  });

  it("reads nothing for a heartbeat that reports the gate's own floor", async () => {
    // The comparison is strict, so a presence at the gate's floor (-1, "nothing
    // applied yet", the value `revision()` falls back to) is not evidence of a
    // lost snapshot and costs no read. This is the `return false` arm that keeps
    // a heartbeat from being a read in the one case the gate can observe.
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    pushKind("presence", "desktop_a", { catalogVersion: { epoch: "epoch_1", sessions: -1, workspaces: -1 } });
    await settle();

    expect(fetchRemoteSessions).not.toHaveBeenCalled();
    hook.unmount();
  });
});

describe("old host fallback", () => {
  it("falls back to polling when the host sends no catalogVersion", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    expect(fetchRemoteSessions).toHaveBeenCalled();
    expect(hook.current.catalogs).toHaveLength(1);
    hook.unmount();
  });
});

describe("a failing read", () => {
  it("drops only the unreachable host's rows, keeping the others'", async () => {
    // Two paired hosts, one of which does not answer. The list is built from
    // the catalogues that arrived: blanking everything because one machine is
    // asleep would take the reachable machine's conversations off screen too.
    listRemotePeers.mockResolvedValue([peer({ desktopId: "desktop_a" }), peer({ desktopId: "desktop_b" })]);
    fetchRemoteSessions.mockImplementation(async (desktopId: string) => {
      if (desktopId === "desktop_b")
        throw new Error("peer_not_connected");
      return { desktopId, sessions: [{ sessionId: "sess_a", title: "A", lastMessageAt: 1 }] };
    });

    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    expect(hook.current.catalogs.map(catalog => catalog.desktopId)).toEqual(["desktop_a"]);
    // A per-host failure is not the list's failure: the machine that answered is
    // still drawn, and the panel shows no error.
    expect(hook.current.error).toBeNull();
    expect(hook.current.peers).toHaveLength(2);
    hook.unmount();
  });

  it("reports a failed peer-list read and keeps the last known rows", async () => {
    listRemotePeers.mockRejectedValue(new Error("agent offline"));
    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    expect(hook.current.error).toBe("agent offline");
    expect(hook.current.peers).toEqual([]);
    expect(hook.current.loading).toBe(false);
    hook.unmount();
  });

  it("does not fail the caller and still re-reads the list when reconnect-all fails", async () => {
    // The re-read happens either way: a reconnected host re-subscribed, and its
    // rows are exactly the stale ones — so the list must be refreshed even when
    // the connect call itself failed. The failure is not surfaced as the
    // panel's error (each host's own row carries its reason from the backend),
    // and a successful read of the list then clears it anyway.
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    const readsBefore = listRemotePeers.mock.calls.length;
    connectAllRemotePeers.mockRejectedValue(new Error("bridge unreachable"));

    await act(async () => {
      await hook.current.reconnectAll();
    });

    expect(listRemotePeers.mock.calls.length).toBeGreaterThan(readsBefore);
    expect(hook.current.error).toBeNull();
    hook.unmount();
  });
});

describe("the catalogue backstop", () => {
  it("re-reads the connected hosts on its own slow cadence", async () => {
    // Hosts that do not push snapshots are only as fresh as this poll, and it
    // reads the *hosts'* catalogues rather than the peer list. The list read is
    // made to fail here so its own 5 s poll cannot contribute the fetch being
    // counted: what remains is the catalogue cadence alone.
    vi.useFakeTimers();
    try {
      const hook = renderHook(() => useRemotePeers(true));
      await settle();
      expect(fetchRemoteSessions).toHaveBeenCalledTimes(1);

      listRemotePeers.mockRejectedValue(new Error("list unavailable"));
      fetchRemoteSessions.mockResolvedValue({
        desktopId: "desktop_a",
        sessions: [{ sessionId: "sess_new", title: "New", streaming: false, lastMessageAt: 2 }],
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      await settle();

      // One read from the backstop — not one per list poll (twelve of those fire
      // in the same minute, and each of them fails above without reading).
      expect(fetchRemoteSessions).toHaveBeenCalledTimes(2);
      expect(hook.current.catalogs[0]!.sessions.map(session => session.sessionId)).toEqual(["sess_new"]);
      hook.unmount();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("stays silent when no host is connected", async () => {
    // Nothing to read: a request to a disconnected host would open a connection
    // as a side effect, which is not what a background poll may do.
    vi.useFakeTimers();
    try {
      const hook = renderHook(() => useRemotePeers(true));
      await settle();

      // The host drops; the fast poll is what carries that to `peersRef`.
      listRemotePeers.mockResolvedValue([peer({ connected: false })]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });
      await settle();
      fetchRemoteSessions.mockClear();

      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      await settle();

      expect(fetchRemoteSessions).not.toHaveBeenCalled();
      hook.unmount();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("keeps the other hosts' rows when one of them fails mid-poll", async () => {
    // The backstop reads each connected host; one that drops out during the
    // read must not blank the list it is sharing with a host that answered.
    vi.useFakeTimers();
    try {
      listRemotePeers.mockResolvedValue([peer({ desktopId: "desktop_a" }), peer({ desktopId: "desktop_b" })]);
      fetchRemoteSessions.mockImplementation(async (desktopId: string) => ({
        desktopId,
        sessions: [{ sessionId: `sess_${desktopId}`, title: desktopId, streaming: false, lastMessageAt: 1 }],
      }));
      const hook = renderHook(() => useRemotePeers(true));
      await settle();
      expect(hook.current.catalogs).toHaveLength(2);

      // desktop_b goes away between polls: the list read stops contributing
      // fetches, so what fires here is the catalogue cadence alone.
      listRemotePeers.mockRejectedValue(new Error("list unavailable"));
      fetchRemoteSessions.mockImplementation(async (desktopId: string) => {
        if (desktopId === "desktop_b")
          throw new Error("peer_not_connected");
        return { desktopId, sessions: [{ sessionId: "sess_a", title: "A", streaming: false, lastMessageAt: 2 }] };
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(60_000);
      });
      await settle();

      expect(hook.current.catalogs.map(catalog => catalog.desktopId)).toEqual(["desktop_a"]);
      hook.unmount();
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("abandons the launch pass when the view is gone before the first read answers", async () => {
    // The launch pass runs *behind* the first peer-list read, so an unmount in
    // that window would otherwise bring hosts up for a view nobody is looking
    // at — a connection opened on behalf of a component that no longer exists.
    let releaseRead!: (peers: RemotePeer[]) => void;
    listRemotePeers.mockReturnValue(new Promise<RemotePeer[]>((resolve) => {
      releaseRead = resolve;
    }));

    const hook = renderHook(() => useRemotePeers(true));
    await flushAsync();
    hook.unmount();

    await act(async () => {
      releaseRead([peer({ connected: false })]);
    });
    await settle();

    expect(connectRemotePeer).not.toHaveBeenCalled();
  });

  it("runs the launch pass once per session, not once per enable", async () => {
    // The hook is mounted with a fixed `enabled`, but consumers do toggle it;
    // re-enabling must not re-run the pass (it is the *session's* launch, and
    // every re-run would re-handshake every host).
    let enabled = true;
    listRemotePeers.mockResolvedValue([peer({ connected: false })]);
    const hook = renderHook(() => useRemotePeers(enabled));
    await settle();
    expect(connectRemotePeer).toHaveBeenCalledTimes(1);

    enabled = false;
    hook.rerender();
    await settle();
    enabled = true;
    hook.rerender();
    await settle();

    expect(connectRemotePeer).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("detaches a subscription that resolves only after unmount", async () => {
    // `listen` is async: the unlisten handle arrives a microtask later. If the
    // view is gone by then, the handle has to be released here — nothing else
    // holds it, and a host push would otherwise reach a dead component.
    const hook = renderHook(() => useRemotePeers(true));
    hook.unmount();
    await settle();

    expect(listeners).toEqual([]);
  });
});

describe("a push the hook cannot read", () => {
  const EPOCH = "epoch_shared";
  const version = (revision: number) => ({ epoch: EPOCH, revision });

  /**
   * Host pushes are the untrusted boundary: whatever arrives is parsed by hand,
   * and a field of the wrong shape has to be dropped rather than dereferenced.
   * Each case is type-hostile on purpose — `null` where an object is expected,
   * a string where a list is — so removing the guard throws instead of being
   * ignored, and the assertion is that the hook survived *and* is still able to
   * apply the next well-formed push (a wedged listener would silently stop
   * updating the list, which is the failure the guard exists to prevent).
   */
  const cases: Array<[string, string, unknown]> = [
    ["a presence payload that is not an object", "presence", null],
    ["a presence whose catalogVersion is not an object", "presence", { catalogVersion: null }],
    ["a presence whose epoch is not a string", "presence", { catalogVersion: { epoch: 1, sessions: 4 } }],
    ["a presence that reports no per-domain revision", "presence", { catalogVersion: { epoch: EPOCH } }],
    ["a catalog payload that is not an object", "catalog", null],
    ["a catalog for a domain this hook does not model", "catalog", { domain: "settings", version: version(1) }],
    ["a catalog whose version is not an object", "catalog", { domain: "sessions", version: null, sessions: [] }],
    ["a catalog whose version lacks a revision", "catalog", { domain: "sessions", version: { epoch: EPOCH }, sessions: [] }],
    ["a workspaces snapshot (tracked, not rendered)", "catalog", { domain: "workspaces", version: version(1), workspaces: [] }],
    ["an activity packet whose rows are not a list", "catalog", { domain: "sessions_activity", version: version(1), activity: "nope" }],
    ["an activity packet of rows that are not objects", "catalog", { domain: "sessions_activity", version: version(1), activity: [null, 7] }],
    ["an activity packet that names no session", "catalog", { domain: "sessions_activity", version: version(1), activity: [{ lastMessageAt: 5 }] }],
    ["a sessions snapshot whose sessions are not a list", "catalog", { domain: "sessions", version: version(1), sessions: "nope" }],
  ];

  it.each(cases)("ignores %s", async (_name, kind, payload) => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    const before = hook.current.catalogs;

    pushKind(kind, "desktop_a", payload);
    await settle();

    // Nothing was applied…
    expect(hook.current.catalogs).toBe(before);

    // …and the listener still works: the next readable snapshot lands.
    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: version(99),
      sessions: [{ sessionId: "sess_after", title: "After", lastMessageAt: 7 }],
    });
    await settle();

    expect(hook.current.catalogs[0]!.sessions.map(session => session.sessionId)).toEqual(["sess_after"]);
    hook.unmount();
  });

  it("drops unreadable rows from a sessions snapshot instead of keeping the old list", async () => {
    // A `sessions` packet is the host's *authoritative* list, so a row the
    // parser cannot read is dropped rather than dereferenced — and the snapshot
    // still replaces what we held. The host's next packet restores the row,
    // which is why dropping it is safe and why the old list must not linger.
    fetchRemoteSessions.mockResolvedValue({
      desktopId: "desktop_a",
      sessions: [{ sessionId: "sess_old", title: "Old", streaming: false, lastMessageAt: 1 }],
    });
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    expect(hook.current.catalogs[0]!.sessions.map(session => session.sessionId)).toEqual(["sess_old"]);

    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: version(1),
      sessions: [null, 7, { title: "no session id" }],
    });
    await settle();
    expect(hook.current.catalogs[0]!.sessions).toEqual([]);

    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: version(2),
      sessions: [{ sessionId: "sess_after", title: "After", lastMessageAt: 7 }],
    });
    await settle();
    expect(hook.current.catalogs[0]!.sessions.map(session => session.sessionId)).toEqual(["sess_after"]);
    hook.unmount();
  });

  it("applies an activity packet to one host and leaves the sibling's rows alone", async () => {
    // The packet names rows on *one* host; the map over the catalogues walks the
    // others, and they must come back untouched.
    listRemotePeers.mockResolvedValue([peer({ desktopId: "desktop_a" }), peer({ desktopId: "desktop_b" })]);
    fetchRemoteSessions.mockImplementation(async (desktopId: string) => ({
      desktopId,
      sessions: [{ sessionId: "sess_1", title: desktopId, streaming: false, lastMessageAt: 1_000 }],
    }));
    const hook = renderHook(() => useRemotePeers(true));
    await settle();

    pushKind("catalog", "desktop_a", {
      domain: "sessions_activity",
      version: version(1),
      activity: [{ sessionId: "sess_1", lastMessageAt: 4_000 }],
    });
    await settle();

    const byId = new Map(hook.current.catalogs.map(catalog => [catalog.desktopId, catalog]));
    expect(byId.get("desktop_a")!.sessions[0]!.lastMessageAt).toBe(4_000);
    expect(byId.get("desktop_b")!.sessions[0]!.lastMessageAt).toBe(1_000);
    hook.unmount();
  });
});

describe("a push this hook does not model", () => {
  it("ignores a kind that is not an event", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    const before = hook.current.catalogs;

    pushKind("catalog-ack", "desktop_a", { sessionId: "sess_1", type: "agent_end" });
    await settle();

    expect(hook.current.catalogs).toBe(before);
    hook.unmount();
  });

  it("ignores an event that names no session, or whose payload is not an object", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    pushKind("catalog", "desktop_a", {
      domain: "sessions",
      version: { epoch: "epoch_1", revision: 1 },
      sessions: [{ sessionId: "sess_1", title: "One", lastMessageAt: 1_000 }],
    });
    await settle();

    pushKind("event", "desktop_a", { runId: "run_1", type: "text_chunk" });
    pushKind("event", "desktop_a", "a bare string, not a payload");
    await settle();

    expect(hook.current.catalogs[0]!.sessions[0]!.streaming).toBe(false);
    hook.unmount();
  });

  it("leaves another machine's catalogue and an unknown row alone", async () => {
    // The in-place update is scoped to one host and to rows the catalogue
    // already holds: an event cannot introduce a row (it would need a title we
    // do not have), and it cannot touch a sibling host.
    listRemotePeers.mockResolvedValue([peer({ desktopId: "desktop_a" }), peer({ desktopId: "desktop_b" })]);
    fetchRemoteSessions.mockImplementation(async (desktopId: string) => ({
      desktopId,
      sessions: [{ sessionId: "sess_1", title: desktopId, streaming: false, lastMessageAt: 1_000 }],
    }));
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    expect(hook.current.catalogs).toHaveLength(2);

    pushKind("event", "desktop_b", { sessionId: "sess_unknown", runId: "run_1", type: "text_chunk" });
    pushKind("event", "desktop_c", { sessionId: "sess_1", runId: "run_1", type: "text_chunk" });
    await settle();

    // desktop_b's known row is untouched (the push named no row it holds), the
    // host with no catalogue at all is skipped, and desktop_a — which does hold
    // that session id — is not reached by either push.
    for (const catalog of hook.current.catalogs) {
      expect(catalog.sessions).toHaveLength(1);
      expect(catalog.sessions[0]!.streaming).toBe(false);
      expect(catalog.sessions[0]!.title).toBe(catalog.desktopId);
    }
    hook.unmount();
  });
});
