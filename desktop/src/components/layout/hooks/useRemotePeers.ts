import type { RemoteCatalog, RemoteSessionRow } from "../../../features/remote-peer/mergeConversations";
import type { RemotePeer, RemotePeerEvent } from "../../../features/remote-peer/remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchRemoteSessions, listRemotePeers } from "../../../features/remote-peer/remotePeerClient";

/** How often the peer list is refreshed. */
const PEER_POLL_MS = 5_000;
/**
 * How often each connected host's catalogue is re-read.
 *
 * Slower than the peer poll on purpose: the live event stream keeps a session's
 * activity* current, and the catalogue only decides membership and titles. A
 * slower catalogue read costs nothing the user notices and keeps N hosts from
 * issuing N request storms on every tick.
 */
const CATALOG_POLL_MS = 15_000;

export interface RemotePeersState {
  peers: RemotePeer[];
  /** One entry per connected host that answered a catalogue read. */
  catalogs: RemoteCatalog[];
  loading: boolean;
  /** A failure of the *local* peer list read (the remote errors ride on `peers`). */
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Owns this machine's view of its paired remote desktops: the peer list, each
 * connected host's session catalogue, and the live event stream that keeps both
 * fresh.
 *
 * Polling rather than a push-only design, for one honest reason: a host moving
 * from reachable to unreachable is not something the *host* can tell us — it is
 * still alive, just not to us. The backend's reconnect supervisor notices and
 * retries, and now reports a link that came back, but nothing announces the
 * drop itself, so a poll is what notices that.
 */
export function useRemotePeers(enabled: boolean): RemotePeersState {
  const [peers, setPeers] = useState<RemotePeer[]>([]);
  const [catalogs, setCatalogs] = useState<RemoteCatalog[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Read inside callbacks that must not be recreated on every poll.
  const peersRef = useRef<RemotePeer[]>([]);

  const refresh = useCallback(async () => {
    try {
      const next = await listRemotePeers();
      peersRef.current = next;
      setPeers(next);
      setError(null);
      // Only connected hosts are asked for a catalogue: a request to a
      // disconnected host would open a connection as a side effect, which is
      // not what a background poll should do.
      const connected = next.filter(peer => peer.connected);
      const results = await Promise.all(connected.map(async (peer) => {
        try {
          return await fetchRemoteSessions(peer.desktopId);
        }
        catch {
          // One unreachable host must not blank the others' rows. Its rows
          // simply drop out until it answers again.
          return null;
        }
      }));
      const answered = results.filter((catalog): catalog is RemoteCatalog => catalog !== null);
      setCatalogs(answered);
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  // The first read also drives `loading`; later polls are silent so the list
  // does not flicker a spinner every five seconds.
  useEffect(() => {
    if (!enabled) {
      setPeers([]);
      setCatalogs([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void refresh().finally(() => {
      if (!cancelled)
        setLoading(false);
    });
    const timer = setInterval(() => void refresh(), PEER_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [enabled, refresh]);

  // A slower, separate catalogue cadence (see CATALOG_POLL_MS).
  useEffect(() => {
    if (!enabled)
      return;
    const timer = setInterval(() => {
      void (async () => {
        const connected = peersRef.current.filter(peer => peer.connected);
        if (connected.length === 0)
          return;
        const results = await Promise.all(connected.map(async (peer) => {
          try {
            return await fetchRemoteSessions(peer.desktopId);
          }
          catch {
            return null;
          }
        }));
        setCatalogs(results.filter((catalog): catalog is RemoteCatalog => catalog !== null));
      })();
    }, CATALOG_POLL_MS);
    return () => clearInterval(timer);
  }, [enabled]);

  /**
   * Live pushes update the catalogue in place, and a link coming back triggers a
   * re-read.
   *
   * A session event settles three things the poll would otherwise lag on: that
   * the session exists, that it is streaming, and that it just became the most
   * recent — which is what moves the row. Reordering on *any* event would make
   * the list jump while merely reading an old conversation, so only a
   * session-scoped event counts.
   *
   * A `resumed` push is the one thing the in-place update cannot fix: every
   * event sent while the link was down is gone, and the row it would have moved
   * is exactly the row that is now stale.
   */
  useEffect(() => {
    if (!enabled)
      return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void listen<RemotePeerEvent>("remote-peer-event", (event) => {
      const { desktopId, kind, payload } = event.payload;
      if (kind === "resumed") {
        void refresh();
        return;
      }
      if (kind !== "event")
        return;
      const sessionId = readString(payload, "sessionId");
      if (!sessionId)
        return;
      const at = Date.now();
      setCatalogs(current => current.map((catalog) => {
        if (catalog.desktopId !== desktopId)
          return catalog;
        const index = catalog.sessions.findIndex(session => session.sessionId === sessionId);
        if (index < 0) {
          // A session the catalogue has not learned about yet (created on the
          // host a moment ago). Placeholding it would need a title we do not
          // have, so it is left for the next catalogue read — which the host's
          // own `state.sessions` push will have made current.
          return catalog;
        }
        const sessions = [...catalog.sessions];
        sessions[index] = {
          ...sessions[index]!,
          streaming: !isTerminal(payload),
          lastMessageAt: at,
        };
        return { ...catalog, sessions };
      }));
    }).then((unlisten) => {
      if (cancelled)
        unlisten();
      else dispose = unlisten;
    }).catch(() => {
      // The app is not running under Tauri (unit tests, a browser harness):
      // polling still works, so this is not an error worth surfacing.
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [enabled]);

  return useMemo(
    () => ({ peers, catalogs, loading, error, refresh }),
    [peers, catalogs, loading, error, refresh],
  );
}

function readString(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" && field ? field : null;
}

/**
 * Whether an event ends a run. Used only to decide whether the row keeps its
 * "running" dot; anything unrecognised is treated as still running, so a dot
 * that lingers is preferred to one that clears before the run is over.
 */
function isTerminal(event: unknown): boolean {
  const type = readString(event, "type");
  return type === "run_finished" || type === "agent_end" || type === "run_failed";
}

export type { RemoteSessionRow };
