import type { SnapshotVersion } from "../../../features/remote-peer/catalogVersion";
import type { RemoteCatalog, RemoteSessionRow } from "../../../features/remote-peer/mergeConversations";
import type { RemotePeer, RemotePeerEvent } from "../../../features/remote-peer/remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CatalogVersionGate } from "../../../features/remote-peer/catalogVersion";
import { forgetHostCatalogue, warmHostCatalogue } from "../../../features/remote-peer/remoteHostCatalog";
import { fetchRemoteSessions, listRemotePeers } from "../../../features/remote-peer/remotePeerClient";

/** How often the peer list is refreshed. */
const PEER_POLL_MS = 5_000;
/**
 * Slow safety poll for hosts that do not push catalog snapshots.
 *
 * Push-capable hosts send `state.sessions` / `state.workspaces` on change, so
 * the per-host catalogue read can be rare. This poll is what detects an old
 * host and continues to work as a drop-detection backstop; it is not the
 * primary source of freshness.
 */
const CATALOG_POLL_MS = 60_000;

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
 * Polling is still used for hosts that do not publish `catalogVersion`, and as
 * a slow safety net for drop detection; the primary source of catalogue
 * freshness is now the host-pushed `state.sessions` / `state.workspaces`
 * snapshots, filtered through a per-host version gate.
 */
export function useRemotePeers(enabled: boolean): RemotePeersState {
  const [peers, setPeers] = useState<RemotePeer[]>([]);
  const [catalogs, setCatalogs] = useState<RemoteCatalog[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Read inside callbacks that must not be recreated on every poll.
  const peersRef = useRef<RemotePeer[]>([]);
  // One version gate per connected host; a host's epoch resets on reconnect.
  const gatesRef = useRef(new Map<string, CatalogVersionGate>());

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
      // And their *own* catalogue — its models, its settings, the skills those
      // settings ask for — is read here, as the connection is established, rather
      // than once per conversation opened (`remoteHostCatalog`). Cached per host,
      // so this costs a command each on the first poll after a connection and
      // nothing afterwards.
      for (const peer of connected)
        warmHostCatalogue(peer.desktopId);
      // A host that is not connected has no catalogue worth keeping: what was
      // read describes the connection that answered, and a re-pair or a
      // reconnect re-reads it.
      for (const peer of next) {
        if (!peer.connected)
          forgetHostCatalogue(peer.desktopId);
      }
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
   * A session event settles one thing the poll would otherwise lag on: whether
   * that conversation is running. It deliberately does **not** settle where the
   * row belongs — see the handler, and `RemoteSessionRow.lastMessageAt`.
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
        // Connection healed: drop the stale gate and re-read everything —
        // including the host's own catalogue, which describes the connection
        // that just came back rather than the one that dropped.
        gatesRef.current.delete(desktopId);
        forgetHostCatalogue(desktopId);
        void refresh();
        return;
      }
      if (kind === "presence") {
        handlePresence(desktopId, payload, refresh);
        return;
      }
      if (kind === "catalog") {
        handleCatalog(desktopId, payload, gatesRef.current, setCatalogs);
        return;
      }
      if (kind !== "event")
        return;
      const sessionId = readString(payload, "sessionId");
      if (!sessionId)
        return;
      // What an event settles is whether the conversation is *running* — a fact
      // about the event itself, where the host's own snapshot is the slower path
      // to the same answer.
      //
      // It deliberately does not move the row. A row's ordering timestamp
      // belongs to the host's clock (`remote_host::catalog` reads
      // `threads.last_message_at`, the agent's own event time), so stamping
      // `Date.now()` here put a *second* clock into one row's key: every event
      // lifted it above this machine's own conversations — nothing this machine
      // holds is newer than its own `now` — and the host's next report of that
      // row (a `state.activity` packet, the next snapshot, or a catalogue read)
      // put it back where the host has it, so a row whose run was streaming
      // jumped up and back for the length of the run. The phone draws the host's
      // rows in the host's own order and never had it; this list is the one
      // place a client could disagree with the host about a row's place in it.
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
  }, [enabled, refresh]);

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

function readNumber(value: unknown, key: string): number | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "number" ? field : null;
}

function readVersion(value: unknown): SnapshotVersion | undefined {
  if (typeof value !== "object" || value === null)
    return undefined;
  const obj = value as Record<string, unknown>;
  const epoch = obj.epoch;
  const revision = obj.revision;
  if (typeof epoch !== "string" || typeof revision !== "number")
    return undefined;
  return { epoch, revision };
}

/**
 * React to a host presence tick.
 *
 * The presence payload carries `catalogVersion` for both domains. If the
 * reported revision is newer than the one we have applied, a snapshot was
 * lost on the at-most-once event lane and we re-read to heal.
 */
function handlePresence(
  desktopId: string,
  payload: unknown,
  refresh: () => Promise<void>,
) {
  if (typeof payload !== "object" || payload === null)
    return;
  const obj = payload as Record<string, unknown>;
  const catalogVersion = obj.catalogVersion;
  if (typeof catalogVersion !== "object" || catalogVersion === null)
    return;
  const version = catalogVersion as Record<string, unknown>;
  const epoch = version.epoch;
  if (typeof epoch !== "string")
    return;

  // Authenticate/refresh the gate from the handshake epoch carried in presence.
  const gate = ensureGate(desktopId, epoch);

  const checkDomain = (domain: string, key: string) => {
    const value = readNumber(catalogVersion, key);
    if (value === null)
      return false;
    const gateRevision = gate.revision(domain);
    if (value > gateRevision) {
      // Snapshot lost on the push lane: re-read to catch up. We do not try to
      // guess which domain is behind; a single list read refreshes sessions.
      void refresh();
      return true;
    }
    return false;
  };

  const didHeal = checkDomain("sessions", "sessions") || checkDomain("workspaces", "workspaces");
  if (!didHeal)
    return;

  // Apply the presence-reported revision to the gate so a stale heartbeat does
  // not keep re-triggering heals.
  const sessionsRevision = readNumber(catalogVersion, "sessions");
  if (sessionsRevision !== null)
    gate.accept("sessions", { epoch, revision: sessionsRevision }, true);

  const workspacesRevision = readNumber(catalogVersion, "workspaces");
  if (workspacesRevision !== null)
    gate.accept("workspaces", { epoch, revision: workspacesRevision }, true);
}

/**
 * Apply a pushed catalog snapshot if the version gate accepts it.
 *
 * Workspaces snapshots are tracked by the gate but not stored by this hook yet
 * (desktop currently only renders session catalogues); sessions snapshots are
 * applied to the merged list state.
 */
function handleCatalog(
  desktopId: string,
  payload: unknown,
  gates: Map<string, CatalogVersionGate>,
  setCatalogs: React.Dispatch<React.SetStateAction<RemoteCatalog[]>>,
) {
  if (typeof payload !== "object" || payload === null)
    return;
  const obj = payload as Record<string, unknown>;
  const domain = obj.domain;
  // `sessions_activity` is a packet, not a snapshot: it names only the rows
  // whose activity timestamp moved. Everything below applies it as such — the
  // rows it does not mention are left exactly as they were, and a row this
  // catalogue has never held is left for the next catalogue read (a placeholder
  // would need a title we do not have).
  if (domain !== "sessions" && domain !== "sessions_activity" && domain !== "workspaces")
    return;
  const version = readVersion(obj.version);

  // Without an epoch we cannot authenticate the gate yet. Drop the snapshot
  // conservatively; the first presence will set the epoch and heal if needed.
  if (version === undefined)
    return;

  let gate = gates.get(desktopId);
  if (!gate) {
    gate = new CatalogVersionGate();
    gate.authenticate(version.epoch);
    gates.set(desktopId, gate);
  }

  // Domain must match the authenticated epoch.
  if (!gate.accept(domain === "sessions_activity" ? "sessions" : domain, version))
    return;

  if (domain === "sessions_activity") {
    const rows = obj.activity;
    if (!Array.isArray(rows))
      return;
    const moved = new Map<string, number>();
    for (const entry of rows) {
      if (typeof entry !== "object" || entry === null)
        continue;
      const row = entry as Record<string, unknown>;
      if (typeof row.sessionId === "string" && row.sessionId && typeof row.lastMessageAt === "number" && Number.isFinite(row.lastMessageAt))
        moved.set(row.sessionId, row.lastMessageAt);
    }
    if (moved.size === 0)
      return;
    setCatalogs(current => current.map((catalog) => {
      if (catalog.desktopId !== desktopId)
        return catalog;
      let changed = false;
      const sessions = catalog.sessions.map((session) => {
        const at = moved.get(session.sessionId);
        if (at === undefined || session.lastMessageAt === at)
          return session;
        changed = true;
        return { ...session, lastMessageAt: at };
      });
      return changed ? { ...catalog, sessions } : catalog;
    }));
    return;
  }

  if (domain === "sessions") {
    const raw = obj.sessions;
    if (!Array.isArray(raw))
      return;
    const next: RemoteCatalog = {
      desktopId,
      // Reuse the existing parser by going through the backend-shaped payload.
      // We normalise here to the same shape `fetchRemoteSessions` would return,
      // so the merged list sees no difference between push and poll.
      sessions: raw.flatMap((entry): RemoteSessionRow[] => {
        if (typeof entry !== "object" || entry === null)
          return [];
        const row = entry as Record<string, unknown>;
        const sessionId = row.sessionId;
        if (typeof sessionId !== "string" || !sessionId)
          return [];
        return [{
          sessionId,
          ...(typeof row.threadId === "string" && row.threadId ? { threadId: row.threadId } : {}),
          title: typeof row.title === "string" && row.title ? row.title : sessionId,
          mode: row.mode === "workspace" ? "workspace" : "chat",
          ...(typeof row.workspaceId === "string" ? { workspaceId: row.workspaceId } : {}),
          ...(typeof row.workspaceName === "string" && row.workspaceName
            ? { workspaceName: row.workspaceName }
            : {}),
          ...(typeof row.workspacePinned === "boolean" ? { workspacePinned: row.workspacePinned } : {}),
          ...(typeof row.workspaceOrderedAt === "number" ? { workspaceOrderedAt: row.workspaceOrderedAt } : {}),
          ...(typeof row.parentSessionId === "string" && row.parentSessionId
            ? { parentSessionId: row.parentSessionId }
            : {}),
          pinned: row.pinned === true,
          streaming: row.streaming === true,
          ...(typeof row.status === "string" ? { status: row.status } : {}),
          lastMessageAt: typeof row.lastMessageAt === "number" ? row.lastMessageAt : null,
        }];
      }),
    };
    setCatalogs(current => current.map(catalog => (catalog.desktopId === desktopId ? next : catalog)));
  }
}

function ensureGate(desktopId: string, epoch: string): CatalogVersionGate {
  const gates = ensureGates();
  let gate = gates.get(desktopId);
  if (!gate) {
    gate = new CatalogVersionGate();
    gates.set(desktopId, gate);
  }
  gate.authenticate(epoch);
  return gate;
}

let globalGates: Map<string, CatalogVersionGate> | undefined;

function ensureGates(): Map<string, CatalogVersionGate> {
  if (!globalGates)
    globalGates = new Map();
  return globalGates;
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
