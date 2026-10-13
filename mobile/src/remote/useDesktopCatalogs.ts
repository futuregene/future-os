import type { ConnectionState } from "./connectionState";
import type { MachineSummary, MachineStatus } from "./desktopStatus";
import { peerMachineStatus } from "./desktopStatus";
import type { CatalogEntry } from "./mergeSessions";
import type {
  Presence,
  RemoteCredentials,
  RemoteSession,
  SnapshotVersion,
} from "./types";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CatalogVersionGate } from "./catalogVersion";
import { RemoteClient } from "./client";
import { ensureFreshCredentials } from "./pairing";
import {
  INITIAL_PRESENCE_STATE,
  PRESENCE_RECEIPT_STALE_MS,
  isDesktopOnline,
  type PresenceState,
} from "./presence";
import { loadCredentials, loadPairedDesktops, saveCredentials } from "./storage";

/**
 * The backstop read, for a machine whose catalog snapshot was lost.
 *
 * The push is the primary source of catalogue freshness — a machine sends
 * `state.sessions` when its list changes, and its heartbeat advertises the
 * revision it is at. This poll only covers the gap NATS' at-most-once delivery
 * leaves, and a machine too old to push at all, so it is deliberately slow: it
 * exists to notice a dropped message, not to deliver a live list.
 */
const CATALOG_BACKSTOP_MS = 60_000;
/**
 * How often a machine's last heartbeat is re-judged.
 *
 * A heartbeat that stops arriving is invisible until something compares it with
 * the clock, and the status row is exactly the place a phone must not lie: a
 * desktop that has gone quiet reads as offline within this window rather than
 * when the next request happens to fail.
 */
const PRESENCE_TICK_MS = 5_000;

/** One machine's connection, as the popover and the merged list read it. */
export interface DesktopConnection {
  desktopId: string;
  /** The user's own name for the machine; the id when they have not named it. */
  label: string;
  icon?: string;
  status: MachineStatus;
  sessions: RemoteSession[];
}

/**
 * Everything a peer connection owns: the transport, the version gate that
 * filters its snapshots, and the last facts it reported about itself.
 */
interface PeerRuntime {
  desktopId: string;
  label: string;
  icon?: string;
  credentials: RemoteCredentials;
  client: RemoteClient;
  gate: CatalogVersionGate;
  presence: Presence | null;
  presenceState: PresenceState;
  /** When this phone last heard from the machine, on its own clock. */
  presenceAt: number;
  /** The heartbeat, judged against this phone's clock. */
  online: boolean;
  connection: ConnectionState;
  failed: boolean;
  sessions: RemoteSession[];
}

export interface DesktopCatalogsState {
  catalogs: CatalogEntry[];
  /** One entry per paired machine other than the active one. */
  machines: MachineSummary[];
  /** Re-read every machine's catalogue after a change the push may not carry. */
  refresh: () => Promise<void>;
  /**
   * Open every machine's connection again — the badge's "reconnect all".
   *
   * A connection that has given up does not try again on its own, so this is
   * the only way back for a machine that went away and came back while the
   * phone was looking at a different one.
   */
  reconnect: () => Promise<void>;
}

/**
 * One catalogue connection: enough of a `RemoteClient` to list a desktop's
 * sessions and to hear what it says about itself, and nothing else.
 *
 * Deliberately not a full runtime. The phone runs one *active* connection — the
 * one whose conversation, timeline, drafts and outbox the UI is wired to — and
 * these are live observers feeding the merged list and the status popover.
 * Promoting them to full runtimes would mean N timelines, N outboxes and N sets
 * of per-conversation state, which is a much larger change than "every machine
 * is connected and tells me what it has" needs.
 *
 * They are *connections*, though, not polls: the handshake is held open, so a
 * machine pushes its own catalogue changes here the moment they happen, and its
 * heartbeat arrives once a second. That is what makes the merged list and the
 * status rows real-time, and why nothing here re-reads on a fast timer.
 */
async function openCatalogClient(
  credentials: RemoteCredentials,
  runtime: PeerRuntime,
  onChanged: () => void,
  isCurrent: () => boolean,
): Promise<RemoteClient> {
  const credentialsRef: { current: RemoteCredentials } = { current: credentials };
  const client = new RemoteClient(credentials, {
    // The rotated token is persisted: the catalog client for a given desktop is
    // the only writer of that desktop's credentials (the active desktop is
    // excluded from these connections), so there is no interleaving to worry
    // about — and leaving it in memory would make the next launch refresh again
    // from a token this process already replaced.
    onCredentials: async (next) => {
      credentialsRef.current = next;
      if (isCurrent()) await saveCredentials(next);
    },
    onEvent: () => {},
    onEventDecodeFailure: () => {},
    onCatalogEpoch: (epoch) => {
      // An epoch is the handshake's answer, never a snapshot's claim: the same
      // fencing the active connection applies, so a machine that restarted
      // cannot have its old revisions read as current.
      if (isCurrent()) runtime.gate.authenticate(epoch);
    },
    onPresence: (presence) => {
      if (!isCurrent()) return;
      runtime.presence = presence;
      runtime.presenceAt = Date.now();
      runtime.online = judgePresence(runtime);
      onChanged();
    },
    onSessions: (sessions, version) => {
      if (!isCurrent()) return;
      if (!runtime.gate.accept("sessions", version)) return;
      runtime.sessions = sessions;
      onChanged();
    },
    onWorkspaces: (_workspaces) => {},
    onFeatures: () => {},
    onConnectionState: (state) => {
      if (!isCurrent()) return;
      runtime.connection = state;
      if (state === "failed" || state === "revoked") runtime.failed = true;
      onChanged();
    },
    onReconnected: () => {
      if (!isCurrent()) return;
      runtime.failed = false;
      onChanged();
    },
    onError: () => {
      // A catalog connection failing is not a user-visible error: the merged
      // list simply loses that machine's rows until it answers again, and the
      // status row says which machine it is.
    },
  });
  await client.open();
  return client;
}

/**
 * Whether a machine is there, from the two signals the active connection also
 * uses: the heartbeat's offset against this phone's clock (a skewed clock must
 * not read as a dead desktop) and the gap since the last packet at all (a
 * desktop that dies silently sends nothing, and no amount of re-judging what it
 * last said will notice).
 */
function judgePresence(runtime: PeerRuntime): boolean {
  if (!runtime.presence) return false;
  const judged = isDesktopOnline(runtime.presence, Date.now(), runtime.presenceState);
  runtime.presenceState = judged;
  return judged.online && Date.now() - runtime.presenceAt < PRESENCE_RECEIPT_STALE_MS;
}

/**
 * Live connections to every *other* paired desktop.
 *
 * The active desktop is excluded: its catalogue, presence and revisions already
 * arrive through the main connection, and opening a second connection to the
 * same machine would double its presence heartbeats and race its catalog
 * revisions. The caller adds the active machine's own status to the list.
 */
export function useDesktopCatalogs(
  activeDesktopId: string | null,
  activeSessions: RemoteSession[] | undefined,
): DesktopCatalogsState {
  /**
   * What the popover and the merged list read, published from the runtimes.
   *
   * The runtimes themselves live in a ref — they hold sockets and callbacks, and
   * nothing about them belongs in a render — so every change they report is
   * flattened into this plain list once, in the callback that saw it.
   */
  const [peers, setPeers] = useState<DesktopConnection[]>([]);
  const runtimesRef = useRef(new Map<string, PeerRuntime>());
  /** Paired machines whose connection could not be built (see `publish`). */
  const unreachableRef = useRef(
    new Map<string, { label: string; icon?: string }>(),
  );
  /** Bumped on every teardown so late callbacks from a closed client are dropped. */
  const epochRef = useRef(0);

  const publish = useCallback(() => {
    const live = [...runtimesRef.current.values()].map(runtime => ({
      desktopId: runtime.desktopId,
      label: runtime.label,
      ...(runtime.icon ? { icon: runtime.icon } : {}),
      status: peerMachineStatus({
        failed: runtime.failed,
        ready: runtime.connection === "ready",
        online: runtime.online,
        agentAvailable: runtime.presence?.agentAvailable !== false,
      }),
      sessions: runtime.sessions,
    }));
    // A paired machine whose connection could not be built at all is still a
    // machine this phone holds: it is listed, offline, so the dot cannot report
    // "all connected" while one of them is missing.
    const unreachable: DesktopConnection[] = [...unreachableRef.current]
      .filter(([desktopId]) => !runtimesRef.current.has(desktopId))
      .map(([desktopId, machine]) => ({
        desktopId,
        label: machine.label,
        ...(machine.icon ? { icon: machine.icon } : {}),
        status: "offline" as const,
        sessions: [],
      }));
    setPeers([...live, ...unreachable]);
  }, []);

  const refresh = useCallback(async () => {
    const isCurrent = () => epoch === epochRef.current;
    const epoch = epochRef.current;
    const paired = await loadPairedDesktops();
    const others = paired.filter(desktop => desktop.desktopId !== activeDesktopId);

    // Close clients for desktops that are no longer paired (or became active).
    let dropped = false;
    for (const [desktopId, runtime] of [...runtimesRef.current]) {
      if (others.some(desktop => desktop.desktopId === desktopId)) continue;
      runtimesRef.current.delete(desktopId);
      dropped = true;
      void runtime.client.close("Unpair").catch(() => undefined);
    }
    for (const desktopId of [...unreachableRef.current.keys()]) {
      if (others.some(desktop => desktop.desktopId === desktopId)) continue;
      unreachableRef.current.delete(desktopId);
      dropped = true;
    }
    // An unpaired machine must leave the list the moment its connection goes,
    // not keep a stale row and a stale status until the next publish.
    if (dropped) publish();

    await Promise.all(others.map(async (desktop): Promise<void> => {
      const label = desktop.name ?? desktop.desktopId;
      const icon = desktop.icon;
      const record = () => {
        unreachableRef.current.set(
          desktop.desktopId,
          icon ? { label, icon } : { label },
        );
        publish();
      };
      try {
        let runtime = runtimesRef.current.get(desktop.desktopId);
        if (!runtime) {
          const stored = await loadCredentials(desktop.desktopId);
          if (!stored) return record();
          const credentials = await ensureFreshCredentials(stored);
          if (!isCurrent()) return;
          const created: PeerRuntime = {
            desktopId: desktop.desktopId,
            label,
            ...(icon ? { icon } : {}),
            credentials,
            client: null as unknown as RemoteClient,
            gate: new CatalogVersionGate(),
            presence: null,
            presenceState: INITIAL_PRESENCE_STATE,
            presenceAt: 0,
            online: false,
            connection: "connecting",
            failed: false,
            sessions: [],
          };
          created.client = await openCatalogClient(credentials, created, publish, isCurrent);
          if (!isCurrent()) {
            void created.client.close().catch(() => undefined);
            return;
          }
          runtimesRef.current.set(desktop.desktopId, created);
          unreachableRef.current.delete(desktop.desktopId);
          runtime = created;
          publish();
        }
        const page = await runtime.client.request<{
          sessions?: RemoteSession[];
          version?: SnapshotVersion;
        }>(
          { type: "list_sessions" },
          "list",
        );
        // The read is fenced by the same gate the pushes go through: a slow
        // answer that arrives after a newer snapshot must not roll the list
        // back, and a machine too old to version its snapshots is read as it
        // always was (the gate lets an unversioned answer through).
        const sessions = Array.isArray(page?.data?.sessions) ? page.data.sessions : [];
        if (!isCurrent()) return;
        if (runtime.gate.accept("sessions", page?.data?.version)) {
          runtime.sessions = sessions;
          runtime.failed = false;
        }
        publish();
      }
      catch {
        // One machine failing must not blank the others' rows. The status row
        // is where this shows; the rows simply stay as they last were.
        const runtime = runtimesRef.current.get(desktop.desktopId);
        if (!isCurrent()) return;
        if (runtime) {
          runtime.failed = true;
          publish();
        } else record();
      }
    }));
  }, [activeDesktopId, publish]);

  const reconnect = useCallback(async () => {
    // A connection that has given up is not re-opened from the inside, so the
    // way back is a fresh one: close every peer client and let `refresh` build
    // them again, handshake and all.
    for (const runtime of runtimesRef.current.values()) {
      runtime.failed = false;
      runtime.connection = "connecting";
      runtime.online = false;
      runtime.presence = null;
      runtime.presenceAt = 0;
      runtime.presenceState = INITIAL_PRESENCE_STATE;
      void runtime.client.close().catch(() => undefined);
    }
    runtimesRef.current.clear();
    unreachableRef.current.clear();
    publish();
    await refresh();
  }, [publish, refresh]);

  useEffect(() => {
    let cancelled = false;
    // Captured before the effect body opens any client: the cleanup must close
    // exactly the clients this mount created, not whatever the map holds later.
    const clients = runtimesRef.current;
    const missing = unreachableRef.current;
    void refresh();
    const backstop = setInterval(() => {
      if (!cancelled) void refresh();
    }, CATALOG_BACKSTOP_MS);
    // A heartbeat that stopped is only visible against the clock, and the
    // status row is where a phone must not claim a machine is there.
    const tick = setInterval(() => {
      if (cancelled) return;
      let changed = false;
      for (const runtime of clients.values()) {
        if (!runtime.presence) continue;
        const online = judgePresence(runtime);
        if (online !== runtime.online) {
          runtime.online = online;
          changed = true;
        }
      }
      if (changed) publish();
    }, PRESENCE_TICK_MS);
    return () => {
      cancelled = true;
      // Invalidate every in-flight callback and every open client: a closed
      // client's late session push must not write into the new mount's state.
      epochRef.current += 1;
      clearInterval(backstop);
      clearInterval(tick);
      for (const client of clients.values()) {
        void client.client.close().catch(() => undefined);
      }
      clients.clear();
      missing.clear();
    };
  }, [publish, refresh]);

  return useMemo(() => {
    const machines: MachineSummary[] = peers.map(peer => ({
      desktopId: peer.desktopId,
      label: peer.label,
      ...(peer.icon ? { icon: peer.icon } : {}),
      active: false,
      status: peer.status,
    }));
    const entries: CatalogEntry[] = peers.map(peer => ({
      desktopId: peer.desktopId,
      label: peer.label,
      sessions: peer.sessions,
    }));
    // The active desktop contributes through its own connection, so its rows are
    // merged here rather than fetched twice.
    const catalogs = activeDesktopId
      ? [...entries, {
        desktopId: activeDesktopId,
        label: activeDesktopId,
        sessions: activeSessions ?? [],
      }]
      : entries;
    return { catalogs, machines, refresh, reconnect };
  }, [activeDesktopId, activeSessions, peers, refresh, reconnect]);
}
