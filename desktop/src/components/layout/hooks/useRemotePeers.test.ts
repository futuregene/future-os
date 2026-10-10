// @vitest-environment jsdom
import type { RemotePeer } from "../../../features/remote-peer/remotePeerClient";
import { act } from "react";
import { beforeEach, expect, it, vi } from "vitest";
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
function pushKind(kind: string, desktopId = "desktop_a"): void {
  act(() => {
    listeners.forEach(listener => listener({ payload: { desktopId, kind, payload: {} } }));
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
