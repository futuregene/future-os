import type { SnapshotVersion } from "../../../features/remote-peer/catalogVersion";
import type { RemoteCatalog, RemoteSessionRow } from "../../../features/remote-peer/mergeConversations";
import type { RemotePeer, RemotePeerEvent } from "../../../features/remote-peer/remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CatalogVersionGate } from "../../../features/remote-peer/catalogVersion";
import { peerRevoked } from "../../../features/remote-peer/peerStatus";
import { forgetHostCatalogue, warmHostCatalogue } from "../../../features/remote-peer/remoteHostCatalog";
import {
  connectAllRemotePeers,
  connectRemotePeer,
  fetchRemoteSessions,
  listRemotePeers,
  normalizeRemoteSessionRows,
  unpairRemotePeer,
} from "../../../features/remote-peer/remotePeerClient";

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
/**
 * How often a paired host that should be live is asked for, when nothing else
 * will.
 *
 * A host that is not the one being read has no retry control on screen, so a
 * machine that was asleep when the app started — or that dropped while the user
 * was elsewhere — would stay out of the merged list for the rest of the
 * session. This is the same one-minute backstop the phone runs for the desktops
 * nobody is reading, and it is deliberately slower than the list poll: a host
 * that is genuinely down is being retried by the runtime's own supervisor, and
 * re-handshaking every five seconds would be a second retry loop fighting the
 * first.
 */
const LIVENESS_BACKSTOP_MS = 60_000;

export interface RemotePeersState {
  peers: RemotePeer[];
  /** One entry per connected host that answered a catalogue read. */
  catalogs: RemoteCatalog[];
  loading: boolean;
  /** A failure of the *local* peer list read (the remote errors ride on `peers`). */
  error: string | null;
  refresh: () => Promise<void>;
  /**
   * Bring every paired host up, including one the user had turned off.
   *
   * The user's own "reconnect all", not the automatic pass: both mean "every
   * pairing should be live", and only this one may overrule a deliberate stop.
   */
  reconnectAll: () => Promise<void>;
}

/**
 * Owns this machine's view of its paired remote desktops: the peer list, each
 * connected host's session catalogue, and the live event stream that keeps both
 * fresh.
 *
 * It also owns the client role's *liveness*, because the merged list is only as
 * complete as the set of hosts that are up: a launch pass brings every pairing
 * up, and a one-minute backstop retries the ones that should be live and are not
 * (see `LIVENESS_BACKSTOP_MS`). Both are the desktop half of the phone's "hold a
 * live connection to every paired desktop".
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
  // The launch pass happens once per app session, not once per poll.
  const launchedRef = useRef(false);
  //
  // The hosts whose pairing this machine has already tried to forget. A pairing
  // the other side revoked is refused by the platform on every attempt, so
  // without this a failed removal would be retried on every poll, for the life
  // of the session.
  const forgottenRef = useRef(new Set<string>());

  /**
   * Drop the local half of a pairing the platform no longer honours.
   *
   * The *authoritative* side ended it: the platform refuses this machine's
   * credential for that host, and no retry can bring it back. The row is what
   * the user asked about — "the other computer is gone from its own list, why is
   * it still on mine" — so the pairing is removed here rather than relabelled:
   * a row whose only remaining action is "unpair" asks the user to clean up
   * after a machine that already left.
   *
   * If the removal itself fails (a storage fault), the row stays and the list
   * shows it as "pairing revoked" with Unpair offered — the state the row was in
   * before this existed.
   */
  const forgetDetached = useCallback(async (peer: RemotePeer) => {
    if (forgottenRef.current.has(peer.desktopId))
      return;
    forgottenRef.current.add(peer.desktopId);
    try {
      // Local first (the command removes the registration before it tries the
      // platform), so the row leaves even when the revoke cannot be delivered.
      await unpairRemotePeer(peer.desktopId);
    }
    catch (err) {
      console.error("remote peer: could not forget a revoked pairing:", peer.desktopId, err);
      return;
    }
    gatesRef.current.delete(peer.desktopId);
    forgetHostCatalogue(peer.desktopId);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await listRemotePeers();
      peersRef.current = next;
      setPeers(next);
      setError(null);
      // A host whose pairing the platform refuses is not a paired host any more,
      // whatever this machine still has on file. The failed read is the
      // platform's own answer, so it is acted on rather than drawn: the row and
      // its registration go, and the list read below works from what is left.
      let peers = next;
      const detached = next.filter(peer => peerRevoked(peer));
      if (detached.length > 0) {
        await Promise.all(detached.map(peer => forgetDetached(peer)));
        peers = await listRemotePeers();
        peersRef.current = peers;
        setPeers(peers);
      }
      // A host that left is not a host whose catalogue is worth keeping, and its
      // rows must leave the merged list with it.
      if (detached.length > 0)
        setCatalogs(current => current.filter(catalog => peers.some(peer => peer.desktopId === catalog.desktopId)));
      // Only connected hosts are asked for a catalogue: a request to a
      // disconnected host would open a connection as a side effect, which is
      // not what a background poll should do.
      const connected = peers.filter(peer => peer.connected);
      // And their *own* catalogue — its models, its settings, the skills those
      // settings ask for — is read here, as the connection is established, rather
      // than once per conversation opened (`remoteHostCatalog`). Cached per host,
      // so this costs a command each on the first poll after a connection and
      // nothing afterwards. A host in standby is skipped: it refuses reads, so
      // warming it would buy one refusal per poll for a catalogue it is not
      // allowed to serve.
      for (const peer of connected) {
        if (peer.control !== "standby")
          warmHostCatalogue(peer.desktopId);
      }
      // A host that is not connected has no catalogue worth keeping: what was
      // read describes the connection that answered, and a re-pair or a
      // reconnect re-reads it.
      for (const peer of peers) {
        if (!peer.connected)
          forgetHostCatalogue(peer.desktopId);
      }
      // A standby device reads nothing: the host refuses its business commands
      // with `superseded`, so asking would spend a round trip per poll on an
      // answer that can only be a refusal. What the reader needs instead is the
      // machine's own state, which is why `control` rides on the peer summary.
      const asked = connected.filter(peer => peer.control !== "standby");
      const results = await Promise.all(asked.map(async (peer) => {
        try {
          return await fetchRemoteSessions(peer.desktopId);
        }
        catch {
          // One unreachable host must not blank the others' rows, and it must
          // not blank *its own* either: see `keepUnanswered`.
          return null;
        }
      }));
      const answered = results.filter((catalog): catalog is RemoteCatalog => catalog !== null);
      setCatalogs(current => keepUnanswered(current, answered, asked.map(peer => peer.desktopId)));
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [forgetDetached]);

  /**
   * Ask for the hosts that should already be live.
   *
   * Per host rather than "connect everything", because the two calls mean
   * different things: this one acts on the machines that are *supposed* to be
   * up, and a host the user deliberately disconnected is one of the answers to
   * "should not". The backend records each failure against its own host, which
   * is where the row reads it, so there is nothing to surface here.
   */
  const holdEveryHost = useCallback(async () => {
    const wanted = peersRef.current.filter(peer => !peer.connected && !peer.stopped);
    if (wanted.length === 0)
      return;
    await Promise.all(wanted.map(async (peer) => {
      try {
        await connectRemotePeer(peer.desktopId);
      }
      catch {
        // Its own row says why. One sleeping machine must not throw away the
        // work done for the others.
      }
    }));
    await refresh();
  }, [refresh]);

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
    void (async () => {
      await refresh().finally(() => {
        if (!cancelled)
          setLoading(false);
      });
      // The launch pass, right after the first read answers who is paired.
      // Before this, the merged list was this machine's alone until the user
      // found the Remote page and clicked every host's own Connect.
      if (cancelled || launchedRef.current)
        return;
      launchedRef.current = true;
      await holdEveryHost();
    })();
    const timer = setInterval(() => void refresh(), PEER_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [enabled, holdEveryHost, refresh]);

  // The backstop for the hosts nothing else will reach.
  useEffect(() => {
    if (!enabled)
      return;
    const timer = setInterval(() => void holdEveryHost(), LIVENESS_BACKSTOP_MS);
    return () => clearInterval(timer);
  }, [enabled, holdEveryHost]);

  const reconnectAll = useCallback(async () => {
    try {
      await connectAllRemotePeers();
      setError(null);
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    // Re-read either way: the list is what the merged rows are built from, and
    // a reconnected host has re-subscribed and is exactly the one whose rows
    // were stale.
    await refresh();
  }, [refresh]);

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
        const answered = results.filter((catalog): catalog is RemoteCatalog => catalog !== null);
        setCatalogs(current => keepUnanswered(current, answered, connected.map(peer => peer.desktopId)));
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
        // The host ending the pairing says so on the same lane as its heartbeat
        // (`unpaired`, or `disconnected` with that reason). It is the one event
        // that makes the row itself wrong rather than stale, and it arrives
        // while the link still works — the platform refusing this machine's
        // credential is what follows. Acting on the notice is for the hosts that
        // get the word out; `refresh`'s own revoked check is what catches the
        // rest, whose socket dies with the permission.
        if (presenceEndsPairing(payload)) {
          const peer = peersRef.current.find(entry => entry.desktopId === desktopId);
          if (peer) {
            gatesRef.current.delete(desktopId);
            forgetHostCatalogue(desktopId);
            void forgetDetached(peer).then(() => refresh());
            return;
          }
        }
        handlePresence(desktopId, payload, gatesRef.current, peersRef.current, refresh);
        return;
      }
      if (kind === "control") {
        // The host says the right to drive it moved. What this client holds
        // describes the control it had: a gate may sit at a revision whose
        // snapshot was never applied (a standby device is not sent one), and
        // the catalogue it cached was read by the device that no longer
        // drives the machine. Re-read rather than reconcile.
        gatesRef.current.delete(desktopId);
        forgetHostCatalogue(desktopId);
        void refresh();
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
      //
      // What it also does not settle is whether there is a run at all, when the
      // event belongs to the *conversation* rather than to one of its runs. The
      // Agent clears `runId` for exactly those — a model or title change, a
      // standing compaction — and the host replays that part of its journal
      // whenever a conversation's observer is created
      // (`agent_bridge::observer::replay_session_events`), which is what opening
      // a conversation does. Read as the guess below (an event arrived, so a run
      // is in flight — right for a run's own events, whose types are open-ended),
      // a `model_changed` from days ago filed the row the reader had just clicked
      // under the recent view's "active" stretch, and the next catalogue read
      // moved it back out: the reported session that "jumped to active and back"
      // without ever running. A prompt that has not started needs no guess
      // either: the host's catalogue states its run's `status`, and `isActiveRun`
      // reads it.
      if (!readString(payload, "runId"))
        return;
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
  }, [enabled, forgetDetached, refresh]);

  return useMemo(
    () => ({ peers, catalogs, loading, error, refresh, reconnectAll }),
    [peers, catalogs, loading, error, refresh, reconnectAll],
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

/**
 * Whether a presence packet says the host ended this pairing.
 *
 * Two shapes, because the host sends two: the unpair notice (`unpaired: true`,
 * published while the link is being torn down) and the generic disconnect notice
 * (`disconnected: true` with `reason: "unpaired"`). Every other reason —
 * `user_disconnect`, `system_sleep` — is the host stepping away and leaves the
 * pairing in place, which is exactly the difference the user is told apart.
 */
function presenceEndsPairing(payload: unknown): boolean {
  if (typeof payload !== "object" || payload === null)
    return false;
  const obj = payload as Record<string, unknown>;
  if (obj.unpaired === true)
    return true;
  return obj.disconnected === true && obj.reason === "unpaired";
}

/**
 * This round's answers, with the last snapshot kept for a host that was asked
 * and did not answer.
 *
 * A catalogue read fails for reasons that say nothing about the host: a store
 * momentarily busy while it serves a page of a conversation's history, one round
 * trip that timed out over a socket `is_connected` still reports as live. Reading
 * that as "this machine has no conversations" emptied its rows until the next
 * poll rebuilt them, and the row the user had *just clicked* was one of them —
 * the reported "the session disappears and comes back a while later". The phone
 * has never had it: its catalog keeps the last snapshot on a failed read, and
 * says why ("Not flashing the catalogue empty is the point").
 *
 * The grace lasts one round, not forever. `refresh` reads the peer list before it
 * reads the catalogues, so a host that is genuinely gone is `connected: false`
 * next time, is not asked, and its rows leave then: what survives a failure is a
 * machine that is still paired and live but did not answer this once. A host that
 * is not asked at all — disconnected, stopped, in standby, unpaired — is not in
 * `asked` and keeps today's rule, which is that its rows go.
 */
function keepUnanswered(
  current: RemoteCatalog[],
  answered: RemoteCatalog[],
  asked: readonly string[],
): RemoteCatalog[] {
  const answers = new Map(answered.map(catalog => [catalog.desktopId, catalog] as const));
  const next: RemoteCatalog[] = [];
  for (const catalog of current) {
    const answer = answers.get(catalog.desktopId);
    if (answer) {
      next.push(answer);
      answers.delete(catalog.desktopId);
      continue;
    }
    if (asked.includes(catalog.desktopId))
      next.push(catalog);
  }
  // A host this list has never held: there is nothing to keep, so its answer —
  // if it produced one — is what introduces it.
  next.push(...answers.values());
  return next;
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
 * Two things arrive on it, and they are answered differently.
 *
 * Who may drive the host comes first. Control is the one fact that changes
 * what the list can hold: while another paired device drives the machine this
 * client is refused every read, and its rows are absent — so a move is not a
 * detail to patch, it is a reason to forget what was read under the old state
 * and re-read. A gate that survived the move would be worse than useless: a
 * standby device is never sent a snapshot, so the gate can sit at a revision
 * whose content this client has never applied, and then reject the first
 * snapshot after control returns. That is why the gate is dropped and the
 * catalogue re-read — the same treatment `resumed` gets, for the same reason.
 *
 * What the host has read comes second: `catalogVersion` for both domains. If
 * the reported revision is newer than the one applied, a snapshot was lost on
 * the at-most-once lane and we re-read to heal. A standby device skips this
 * entirely — there is nothing to heal towards, because the snapshot it names
 * was never published to it.
 */
function handlePresence(
  desktopId: string,
  payload: unknown,
  gates: Map<string, CatalogVersionGate>,
  peers: readonly RemotePeer[],
  refresh: () => Promise<void>,
) {
  if (typeof payload !== "object" || payload === null)
    return;
  const obj = payload as Record<string, unknown>;
  const control = obj.control === "standby" || obj.control === "active" ? obj.control : undefined;
  const known = peers.find(peer => peer.desktopId === desktopId)?.control;
  if (control !== undefined && control !== known) {
    // Control moved under this client. Forget what described the old state and
    // re-read: the peer list now carries the new `control`, and a machine this
    // device drives again answers reads the moment it does.
    gates.delete(desktopId);
    forgetHostCatalogue(desktopId);
    void refresh();
    return;
  }
  if (control === "standby")
    return;

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
      // The same parser the poll uses: the merged list must see no difference
      // between a pushed snapshot and a polled one, and one field list is what
      // keeps a host field from reaching one path and not the other.
      sessions: normalizeRemoteSessionRows(obj.sessions),
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
