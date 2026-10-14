// @vitest-environment jsdom
import type { RemotePeer } from "../../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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
vi.mock("../../../features/remote-peer/remotePeerClient", () => ({
  fetchRemoteSessions: (...args: Parameters<typeof fetchRemoteSessions>) => fetchRemoteSessions(...args),
  listRemotePeers: () => listRemotePeers(),
  listRemoteWorkspaces: () => Promise.resolve([]),
}));

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

  it("does not re-read when the presence revision matches applied revision", async () => {
    const hook = renderHook(() => useRemotePeers(true));
    await settle();
    fetchRemoteSessions.mockClear();

    // No snapshot has been applied, so revision is -1 and any positive
    // presence revision should trigger a heal.
    pushKind("presence", "desktop_a", {
      catalogVersion: { epoch: "epoch_1", sessions: 1, workspaces: 0 },
    });
    await settle();

    expect(fetchRemoteSessions).toHaveBeenCalledTimes(1);
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
