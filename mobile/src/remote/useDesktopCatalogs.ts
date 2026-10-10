import type { CatalogEntry } from "./mergeSessions";
import type { Presence, RemoteCredentials, RemoteSession, RemoteWorkspace } from "./types";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { RemoteClient } from "./client";
import { ensureFreshCredentials } from "./pairing";
import { loadCredentials, loadPairedDesktops, saveCredentials } from "./storage";

/**
 * How often each non-active desktop's catalogue is re-read.
 *
 * Slower than the active connection's own sync, on purpose: these connections
 * exist to answer "what does that machine have", and a merged list whose rows
 * move a few seconds late is fine. Each extra connection costs a socket, a
 * handshake and a presence heartbeat, so they are deliberately cheap.
 */
const CATALOG_POLL_MS = 20_000;

/**
 * One catalogue connection: enough of a `RemoteClient` to list a desktop's
 * sessions, and nothing else.
 *
 * Deliberately not a full runtime. The phone runs one *active* connection — the
 * one whose conversation, timeline, drafts and outbox the UI is wired to — and
 * these are read-only observers feeding the merged list. Promoting them to full
 * runtimes would mean N timelines, N outboxes and N sets of per-conversation
 * state, which is a much larger change than "show me every desktop's sessions
 * in one list" needs.
 *
 * Callbacks are minimal but not optional: `RemoteClient` reports connection
 * state through them, and a client whose callbacks throw on the first event
 * would fail during the handshake.
 */
async function openCatalogClient(
  credentials: RemoteCredentials,
  onSessions: (sessions: RemoteSession[]) => void,
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
    onPresence: (_presence: Presence) => {},
    onSessions: (sessions) => {
      if (!isCurrent()) return;
      onSessions(sessions.map(toRemoteSession));
    },
    onWorkspaces: (_workspaces: RemoteWorkspace[]) => {},
    onFeatures: () => {},
    onConnectionState: () => {},
    onReconnected: () => {},
    onError: () => {
      // A catalog connection failing is not a user-visible error: the merged
      // list simply loses that desktop's rows until it answers again, and the
      // desktop picker still shows it as unavailable.
    },
  });
  await client.open();
  return client;
}

/** The desktop sends the same shape the active connection consumes. */
function toRemoteSession(session: RemoteSession): RemoteSession {
  return session;
}

export interface DesktopCatalogsState {
  catalogs: CatalogEntry[];
  /** Desktops whose catalogue could not be read this round. */
  unavailable: Set<string>;
  refresh: () => Promise<void>;
}

/**
 * Live session catalogues for every *other* paired desktop.
 *
 * The active desktop is excluded: its catalogue already arrives through the
 * main connection, and opening a second connection to the same machine would
 * double its presence heartbeats and race its catalog revisions.
 */
export function useDesktopCatalogs(
  activeDesktopId: string | null,
  activeSessions: RemoteSession[] | undefined,
): DesktopCatalogsState {
  const [catalogs, setCatalogs] = useState<CatalogEntry[]>([]);
  const [unavailable, setUnavailable] = useState<Set<string>>(() => new Set());
  const clientsRef = useRef(new Map<string, RemoteClient>());
  /** Bumped on every teardown so late callbacks from a closed client are dropped. */
  const epochRef = useRef(0);

  const refresh = useCallback(async () => {
    const epoch = epochRef.current;
    const isCurrent = () => epoch === epochRef.current;
    const paired = await loadPairedDesktops();
    const others = paired.filter(desktop => desktop.desktopId !== activeDesktopId);

    // Close clients for desktops that are no longer paired (or became active).
    for (const [desktopId, client] of [...clientsRef.current]) {
      if (others.some(desktop => desktop.desktopId === desktopId)) continue;
      clientsRef.current.delete(desktopId);
      void client.close("Unpair").catch(() => undefined);
    }

    const failed = new Set<string>();
    const entries = await Promise.all(others.map(async (desktop): Promise<CatalogEntry | null> => {
      const label = desktop.name ?? desktop.desktopId;
      try {
        const stored = await loadCredentials(desktop.desktopId);
        if (!stored) return null;
        const credentials = await ensureFreshCredentials(stored);
        if (!isCurrent()) return null;
        let client = clientsRef.current.get(desktop.desktopId);
        if (!client) {
          let sessions: RemoteSession[] = [];
          const target = desktop.desktopId;
          client = await openCatalogClient(credentials, (next) => {
            if (!isCurrent()) return;
            sessions = next;
            setCatalogs(current => upsert(current, { desktopId: target, label, sessions }));
          }, isCurrent);
          if (!isCurrent()) {
            void client.close().catch(() => undefined);
            return null;
          }
          clientsRef.current.set(desktop.desktopId, client);
        }
        const page = await client.request<{ sessions?: RemoteSession[] }>(
          { type: "list_sessions" },
          "list",
        );
        const sessions = Array.isArray(page?.data?.sessions) ? page.data.sessions : [];
        setCatalogs(current => upsert(current, { desktopId: desktop.desktopId, label, sessions }));
        return { desktopId: desktop.desktopId, label, sessions };
      }
      catch {
        // One desktop failing must not blank the others' rows.
        failed.add(desktop.desktopId);
        return null;
      }
    }));
    if (!isCurrent()) return;
    // A desktop that is no longer paired must not keep a stale catalogue.
    const live = entries.filter((entry): entry is CatalogEntry => entry !== null);
    setCatalogs(current => reconcile(current, live));
    setUnavailable(failed);
  }, [activeDesktopId]);

  // The active desktop contributes through its own connection, so its rows are
  // merged here rather than fetched twice.
  const merged = useMemo(() => {
    const others = catalogs.filter(catalog => catalog.desktopId !== activeDesktopId);
    // `activeSessions` is undefined while the active connection is still
    // loading; an entry with no list would make every consumer guard for it.
    return activeDesktopId
      ? [...others, {
        desktopId: activeDesktopId,
        label: activeDesktopId,
        sessions: activeSessions ?? [],
      }]
      : others;
  }, [catalogs, activeDesktopId, activeSessions]);

  useEffect(() => {
    let cancelled = false;
    // Captured before the effect body opens any client: the cleanup must close
    // exactly the clients this mount created, not whatever the map holds later.
    const clients = clientsRef.current;
    void refresh();
    const timer = setInterval(() => {
      if (!cancelled) void refresh();
    }, CATALOG_POLL_MS);
    return () => {
      cancelled = true;
      // Invalidate every in-flight callback and every open client: a closed
      // client's late session push must not write into the new mount's state.
      epochRef.current += 1;
      clearInterval(timer);
      for (const client of clients.values()) {
        void client.close().catch(() => undefined);
      }
      clients.clear();
    };
  }, [refresh]);

  return useMemo(() => ({ catalogs: merged, unavailable, refresh }), [merged, unavailable, refresh]);
}

function upsert(catalogs: CatalogEntry[], entry: CatalogEntry): CatalogEntry[] {
  const index = catalogs.findIndex(catalog => catalog.desktopId === entry.desktopId);
  if (index < 0) return [...catalogs, entry];
  const next = [...catalogs];
  next[index] = entry;
  return next;
}

/** Keep only the desktops that are still paired, preserving their rows. */
function reconcile(current: CatalogEntry[], live: CatalogEntry[]): CatalogEntry[] {
  return live.map((entry) => {
    const previous = current.find(catalog => catalog.desktopId === entry.desktopId);
    // Prefer the freshly read page, but keep the last one when this round
    // returned nothing (a transient empty page must not blank the list).
    if (entry.sessions.length === 0 && previous && previous.sessions.length > 0) return previous;
    return entry;
  });
}
