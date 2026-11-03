import { useCallback, useEffect, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import { AppState } from "react-native";
import { RemoteClient, type RecoveryReason } from "./client";
import type { ConnectionState } from "@future-os/remote-client/connectionState";
import { classifyError } from "@future-os/remote-client/connectionState";
import { discardPendingPrompt } from "./pendingPromptStorage";
import { discardPendingContinuation } from "./pendingContinuationStorage";
import {
  INITIAL_PRESENCE_STATE,
  isDesktopOnline,
  PRESENCE_RECEIPT_STALE_MS,
  type PresenceState,
} from "@future-os/remote-client/presence";
import { RecoveryCoordinator } from "@future-os/remote-client/recoveryCoordinator";
import type { ReconcileReason, SyncEngine } from "@future-os/remote-client/syncEngine";
import { loadCredentials, saveCredentials } from "./storage";
import type {
  CatalogDeltaPacket,
  CatalogRevisions,
  ConnectionPhase,
  Presence,
  RemoteCredentials,
  RemoteSession,
  RemoteWorkspace,
  SessionActivity,
  SnapshotVersion,
  StreamEvent,
} from "./types";

/**
 * One desktop's connection: the handshake, its recovery, and what it says about
 * itself.
 *
 * This is the lifecycle half of a runtime, and it is deliberately per desktop.
 * Every paired desktop holds one of these for as long as the app is paired to
 * it, so a phone with three desktops runs three handshakes, three heartbeats
 * and three recovery coordinators — and switching between them is a pointer
 * change rather than a reconnection (see `RemoteContext`). Connection
 * ownership is the reason that is possible: a connection that is always open
 * cannot be "re-established" on the way to another machine.
 *
 * What it owns is exactly the transport's own state — phase, error, presence,
 * capabilities — and nothing about conversations. The catalogue, the timeline
 * and the outbox belong to the runtime that composes this hook, because they
 * are per *desktop* state that outlives any single connection generation.
 *
 * The app-level lives outside it too: background/foreground and network
 * transitions are one process-wide fact, so the provider watches them once and
 * drives every runtime (`recover`, `setAppActive`). A per-desktop
 * implementation of those would be N listeners disagreeing about the network.
 */
export interface DesktopConnectionOptions {
  /** The desktop this connection is paired to. Credentials are read for it. */
  desktopId: string;
  /**
   * The runtime's client slot, shared with the catalogue, timeline and outbox
   * hooks that address the same desktop.
   */
  clientRef: MutableRefObject<RemoteClient | null>;
  credentialsRef: MutableRefObject<RemoteCredentials | null>;
  /** The session this runtime has open, for the catalogue's own bookkeeping. */
  selectedRef: MutableRefObject<string>;
  /**
   * This desktop's timeline lanes. A transport that came back has re-read
   * nothing, so the lanes are restarted before the catalogue is refreshed.
   */
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  handleEvent(event: StreamEvent, sessionId: string): void;
  reconcileSession(sessionId: string | undefined, reason: ReconcileReason, runId?: string): void;
  setCatalogEpoch?(epoch: string | undefined): void;
  applySessionSnapshot(sessions: RemoteSession[], version?: SnapshotVersion): boolean | void;
  /**
   * Merge a row-level catalogue packet into the list this runtime holds.
   *
   * Only a connection that declared `catalog_delta_v1` receives one; the list's
   * merge rule (absence is not deletion) lives in the client package's
   * `applyCatalogDelta`, so the runtime cannot get it wrong on its own.
   */
  applySessionDelta(packet: CatalogDeltaPacket): boolean | void;
  /**
   * A catalog activity packet: the rows whose ordering timestamp moved, under
   * the same version gate as a snapshot.
   */
  applySessionActivity(activity: SessionActivity[], version?: SnapshotVersion): boolean | void;
  applySessionStreaming(sessionId: string, streaming: boolean): void;
  setWorkspaces(workspaces: RemoteWorkspace[], version?: SnapshotVersion): void;
  refreshModels(): Promise<void>;
  refreshSessions(): Promise<void>;
  refreshSettings(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  /**
   * Presence carries the desktop's catalog revisions; a newer one than this
   * client applied means a pushed snapshot was lost.
   */
  noteCatalogRevisions?(version: CatalogRevisions | undefined): void;
  /**
   * Control of this machine moved, as the host itself says.
   *
   * Not a transport fault and not a conversation fact: a machine another device
   * drives refuses this client every read and is sent no snapshot, so the
   * catalogue it mirrored describes a control the phone no longer has. What to
   * do about that is the runtime's judgement (`useDesktopRuntime` forgets the
   * catalogue, and reads it again when control returns); the connection only
   * reports the move — from the heartbeat, or from the host's immediate
   * `state.control` packet.
   */
  controlChanged?(control: "active" | "standby"): void;
  /**
   * This machine ended the pairing — deliberately, or because the platform
   * revoked this device's credentials.
   *
   * Both are definitive and neither is a connection problem: the phone must
   * drop the desktop instead of offering a retry that can never succeed. The
   * runtime decides how (see `RemoteContext`), because removing a pairing is
   * the app's business rather than one connection's.
   */
  onUnpaired?(): void;
  /**
   * Ask the platform whether this pairing still exists.
   *
   * Injected rather than imported: pairing is the app's business, and a
   * connection should not know how a pairing is kept. Resolves while the pairing
   * is still valid (the challenge is the platform's answer), rejects with a
   * credential error once it is not.
   */
  verifyPairing?(credentials: RemoteCredentials): Promise<unknown>;
  closeConversation(): void;
  resetConversation(): void;
  resetCatalog(): void;
  resetTimeline(): void;
}

/**
 * How long a phone waits for the first answer from a desktop it has never
 * reached before saying that no answer is coming.
 *
 * The transport's own numbers set the floor. A client that has never been ready
 * gives one attempt 20 s (`connection_timeout`, `packages/remote-client`), of
 * which a single socket connect is 10 s and a single handshake request 10 s;
 * between attempts it backs off from 1 s to 30 s, and once that window is spent
 * it goes quiet for five minutes before trying again. A network slow enough to
 * fail all of that has already been given up on by the client, so three full
 * windows — 60 s — judges nothing early, while still ending the wait long
 * before the client's own five-minute silence does.
 *
 * Raise it if users on genuinely slow networks are told this while a
 * connection would still have arrived; the only cost of raising it is a longer
 * wait before the phone admits that nothing is answering.
 */
export const FIRST_ANSWER_BUDGET_MS = 60_000;

/**
 * Whether this connection is still waiting for its very first answer.
 *
 * `hasConnectedContent` is the phone's own record that this pairing has been
 * usable at least once; the phases are the ones the transport uses both for a
 * first connection and for a connection coming back (`refreshing` is a
 * credential rotation, which a first attempt can also enter — and leave —
 * without anything ever having answered). A pairing that has answered before is
 * never in this state: whatever took it down, this desktop is a machine that
 * has served this phone, and the ordinary reconnect already explains that.
 */
export function awaitingFirstAnswer(
  phase: ConnectionPhase,
  hasConnectedContent: boolean,
): boolean {
  if (hasConnectedContent) return false;
  return phase === "booting" || phase === "claiming" || phase === "connecting"
    || phase === "reconnecting" || phase === "refreshing";
}

export interface DesktopConnection {
  /**
   * The live client, or null while this desktop has no usable connection. The
   * same slot the catalogue, timeline and outbox address the machine through.
   */
  clientRef: MutableRefObject<RemoteClient | null>;
  credentials: RemoteCredentials | null;
  phase: ConnectionPhase;
  error: string | null;
  presence: Presence | null;
  /** The desktop is reachable and its agent is up, judged from presence. */
  desktopOnline: boolean;
  hasConnectedContent: boolean;
  /**
   * Whether this pairing has stopped answering altogether.
   *
   * Reported once the first-answer budget (`FIRST_ANSWER_BUDGET_MS`) has passed
   * with not one answer from the desktop, and only while the connection is
   * still waiting for one. The transport has no word for it — it says
   * `connecting`, and keeps saying it while it retries behind its own timer —
   * so this is the fact that lets the phone end that wait instead of pretending
   * it is nearly over.
   */
  desktopUnanswered: boolean;
  capabilities: Set<string>;
  fileTransferSupported: boolean;
  promptReceiptSupported: boolean;
  /** Record a non-transport error; transport faults are the transport's own. */
  recordError(error: unknown): void;
  clearError(): void;
  /** Recover this connection (foreground, network restored, stale presence). */
  recover(reason: RecoveryReason): Promise<void>;
  /**
   * Whether the client is inside the app's active-lifecycle window. The
   * provider drives this for every runtime at once; `false` retires the socket
   * so a backgrounded app is not holding N of them.
   */
  setAppActive(active: boolean): void;
  /**
   * The session whose detailed events this desktop should send. Empty for a
   * runtime the user is not reading: the desktop then publishes only the
   * low-rate notices, which is what keeps N connected runtimes affordable.
   */
  setVisibleSession(sessionId: string): void;
  /**
   * Best-effort "this phone is leaving" notice, so the desktop drops the
   * pairing on its side. Bounded: the phone's teardown never waits on a machine
   * that may be gone.
   */
  notifyUnpair(): Promise<void>;
  /**
   * Record that this device now drives the desktop, without waiting for a
   * heartbeat to say so.
   *
   * The machine moves control synchronously and says `control: "active"` in the
   * `takeover` answer itself; presence repeats the fact only on its next tick.
   * A control that is read back from presence alone therefore keeps telling the
   * page the machine is still held for up to a heartbeat after the takeover
   * succeeded — and the button that was pressed comes back for that whole
   * window. Only called with the machine's own affirmative answer.
   */
  noteControlActive(): void;
}

export function useDesktopConnection({
  desktopId,
  clientRef,
  credentialsRef,
  selectedRef,
  syncEngineRef,
  handleEvent,
  reconcileSession,
  setCatalogEpoch,
  applySessionSnapshot,
  applySessionDelta,
  applySessionActivity,
  applySessionStreaming,
  setWorkspaces,
  refreshModels,
  refreshSessions,
  refreshSettings,
  refreshWorkspaces,
  noteCatalogRevisions,
  controlChanged,
  onUnpaired,
  verifyPairing,
  closeConversation,
  resetConversation,
  resetCatalog,
  resetTimeline,
}: DesktopConnectionOptions): DesktopConnection {
  const [phase, setPhase] = useState<ConnectionPhase>("booting");
  const [error, setError] = useState<string | null>(null);
  const [credentials, setCredentials] = useState<RemoteCredentials | null>(null);
  const [presence, setPresence] = useState<Presence | null>(null);
  const latestPresenceRef = useRef<Presence | null>(null);
  const [capabilities, setCapabilities] = useState<Set<string>>(() => new Set());
  const [desktopOnline, setDesktopOnline] = useState(false);
  const [hasConnectedContent, setHasConnectedContent] = useState(false);
  /** The first-answer budget has run out on this attempt (see the clock below). */
  const [firstAnswerSpent, setFirstAnswerSpent] = useState(false);
  // The app's own lifecycle, mirrored for the clock below. The ref version is
  // what the connection reads synchronously; the state is what lets a timer be
  // armed and disarmed as the app comes and goes.
  const [appActive, setAppActiveState] = useState(true);
  // Generations fence late callbacks: a timer or a promise armed against one
  // connection generation must not write into the next one.
  const accessRef = useRef(0);
  const recoveryRef = useRef(new RecoveryCoordinator<RemoteClient>());
  const connectionReadyRef = useRef(false);
  const presenceStateRef = useRef<PresenceState>(INITIAL_PRESENCE_STATE);
  const agentAvailableRef = useRef<boolean | undefined>(undefined);
  const lastPresenceReceiptRef = useRef(0);
  /** The pairing this runtime's state belongs to; a re-pair resets it. */
  const pairRef = useRef<string | null>(null);
  /** The session the desktop is asked to detail; empty while unfocused. */
  const visibleSessionRef = useRef("");
  const appActiveRef = useRef(true);
  // A timed-out stop still has an owner until native/SDK work actually ends.
  const closingClientsRef = useRef(new Set<RemoteClient>());
  const stopClient = useCallback(async (client: RemoteClient | null) => {
    if (!client) return;
    closingClientsRef.current.add(client);
    try {
      const result = await client.shutdown(2_000, "UserInitiated");
      if (result.complete) {
        closingClientsRef.current.delete(client);
        return;
      }
      console.warn("remote_cleanup_timeout", result.pendingTasks);
      // Release the UI after its budget, while retaining the client and an
      // observer for actual completion. Never report the timeout as completion.
      void client.waitTasks().then(() => {
        closingClientsRef.current.delete(client);
      }, error => { console.warn("remote_cleanup_failed", error); });
    } catch (error) {
      console.warn("remote_cleanup_failed", error);
    }
  }, []);

  useEffect(() => {
    credentialsRef.current = credentials;
  }, [credentials, credentialsRef]);

  const recordError = useCallback((nextError: unknown) => {
    if (classifyError(nextError) === "transport") return;
    console.warn("[remote] unexpected non-transport error", {
      desktopId,
      error: nextError instanceof Error ? nextError.message : String(nextError),
    });
    setError(nextError instanceof Error ? nextError.message : String(nextError));
  }, [desktopId]);

  const updateDesktopOnline = useCallback((nextPresence: Presence | null, now: number) => {
    const next = isDesktopOnline(nextPresence, now, presenceStateRef.current);
    presenceStateRef.current = next;
    setDesktopOnline(
      next.online && now - lastPresenceReceiptRef.current < PRESENCE_RECEIPT_STALE_MS,
    );
  }, []);

  /**
   * Record control moving, from whichever packet said so first.
   *
   * The host announces a takeover on `state.control` as it happens and repeats
   * the fact on every heartbeat. Both roads lead here, so a displaced device
   * gives up the machine's conversations in the same tick it lost them, and a
   * device handed the machine back reads them again without waiting for its own
   * next tick — instead of the phone drawing rows it can no longer open.
   */
  const noteControl = useCallback((control: "active" | "standby") => {
    const current = latestPresenceRef.current;
    if (current && current.control === control) return;
    if (current) {
      // The packet names no holder; the next heartbeat does. Until then the
      // page says the machine is taken over without naming who has it, which
      // is the honest reading rather than the previous holder's name.
      const next = {
        ...current,
        control,
        holder: control === "active" ? null : current.holder ?? null,
      };
      latestPresenceRef.current = next;
      setPresence(next);
    }
    controlChanged?.(control);
  }, [controlChanged]);

  const recoverState = useCallback(
    (client: RemoteClient) => {
      const access = accessRef.current;
      return recoveryRef.current.request(
        client,
        () =>
          access === accessRef.current &&
          clientRef.current === client &&
          connectionReadyRef.current,
        async () => {
          // Timeline lanes must abandon work awaiting an obsolete transport.
          // Recover them immediately, independently of slower catalog requests.
          syncEngineRef.current?.restartAll("reconnect");
          await Promise.allSettled([
            refreshModels(),
            refreshSessions(),
            refreshWorkspaces(),
            refreshSettings(),
          ]);
        },
      );
    },
    [clientRef, refreshModels, refreshSessions, refreshWorkspaces, refreshSettings, syncEngineRef],
  );

  const connect = useCallback(
    async (nextCredentials: RemoteCredentials) => {
      const access = ++accessRef.current;
      const previous = clientRef.current;
      clientRef.current = null;
      credentialsRef.current = null;
      connectionReadyRef.current = false;
      presenceStateRef.current = INITIAL_PRESENCE_STATE;
      agentAvailableRef.current = undefined;
      lastPresenceReceiptRef.current = 0;
      latestPresenceRef.current = null;
      setError(null);
      setPresence(null);
      setDesktopOnline(false);
      // A new attempt is a new window: whatever the previous one was told, this
      // one has not been given its budget yet.
      setFirstAnswerSpent(false);
      setPhase("connecting");
      // A pairing change means a different desktop's conversations: everything
      // mirrored from the old one is not this one's.
      if (pairRef.current !== null && pairRef.current !== nextCredentials.pairId) {
        setHasConnectedContent(false);
        resetCatalog();
        resetConversation();
        resetTimeline();
      }
      await stopClient(previous);
      if (access !== accessRef.current) return;
      // A successful pair must mean its credentials are durable. In
      // particular, do not let the UI report success while SecureStore writes
      // are still in flight and can be lost on immediate process suspension.
      await saveCredentials(nextCredentials);
      if (access !== accessRef.current) return;
      pairRef.current = nextCredentials.pairId;
      credentialsRef.current = nextCredentials;
      setCredentials(nextCredentials);
      setCapabilities(new Set());
      const client = new RemoteClient(nextCredentials, {
        onCredentials: async (next) => {
          if (access !== accessRef.current) return;
          await saveCredentials(next);
          if (access !== accessRef.current) return;
          credentialsRef.current = next;
          setCredentials(next);
        },
        onEvent: (event, sessionId) => {
          if (access === accessRef.current) handleEvent(event, sessionId);
        },
        onEventDecodeFailure: (sessionId, decodeError) => {
          if (access !== accessRef.current) return;
          console.warn("[remote] malformed live event; reconciling session", {
            desktopId,
            sessionId,
            error: decodeError,
          });
          reconcileSession(sessionId, "resend");
        },
        onCatalogEpoch: (epoch) => {
          if (access === accessRef.current) setCatalogEpoch?.(epoch);
        },
        onPresence: (nextPresence) => {
          if (access !== accessRef.current) return;
          // `unpaired` is the desktop ending this pairing, and `disconnected` is
          // it merely stepping away. Only the first is final: the second keeps
          // the local pairing so the user can retry, which is why the two take
          // different branches here.
          if (nextPresence.unpaired) {
            // The desktop ended the pairing on its side. Nothing is left to
            // retry, so the app is told to forget this machine outright — the
            // teardown below is only what keeps the screen sane for the moment
            // before that lands.
            onUnpaired?.();
            accessRef.current += 1;
            connectionReadyRef.current = false;
            clientRef.current = null;
            void discardPendingPrompt(nextCredentials.pairId);
            void discardPendingContinuation(nextCredentials.pairId);
            setPresence(nextPresence);
            setDesktopOnline(false);
            setCapabilities(new Set());
            resetCatalog();
            resetConversation();
            resetTimeline();
            setPhase("revoked");
            setError("credentials_revoked");
            void stopClient(client);
            return;
          }
          // An explicit Desktop offline notice (including sleep) is terminal, unlike a
          // transient transport loss. Retain the pairing credentials for the
          // manual retry button, but tear down the mobile client and every
          // mirrored catalogue so it cannot reconnect in the background or
          // show stale conversations.
          if (nextPresence.disconnected) {
            accessRef.current += 1;
            connectionReadyRef.current = false;
            clientRef.current = null;
            setPresence(nextPresence);
            setDesktopOnline(false);
            setCapabilities(new Set());
            resetCatalog();
            resetConversation();
            resetTimeline();
            setPhase("stopped");
            setError(null);
            void stopClient(client);
            return;
          }
          const agentRecovered =
            agentAvailableRef.current === false && nextPresence.agentAvailable === true;
          agentAvailableRef.current = nextPresence.agentAvailable;
          if (agentRecovered && connectionReadyRef.current)
            void recoverState(client).catch(recordError);
          lastPresenceReceiptRef.current = Date.now();
          // A heartbeat repeats control, so only a change is news. Until the
          // first presence of a connection there has been no word on control,
          // and the machine is read as `active` — the same default an older
          // desktop's absent field gets, and the only control a catalogue can
          // have been read under. A first presence that says `standby` is
          // therefore a move: the rows mirrored before the transport dropped
          // belong to a machine somebody else has taken.
          const previousPresence = latestPresenceRef.current;
          latestPresenceRef.current = nextPresence;
          setPresence(nextPresence);
          if (
            nextPresence.control !== undefined
            && nextPresence.control !== (previousPresence ? previousPresence.control : "active")
          )
            controlChanged?.(nextPresence.control);
          if (connectionReadyRef.current) {
            // Only past the ready barrier: a pull issued against a
            // half-built connection would race the handshake it depends on.
            //
            // A standby machine advertises revisions towards a catalogue it
            // never sent this client — there is nothing to heal towards — so a
            // pull would only earn a `superseded` per heartbeat.
            if (nextPresence.control !== "standby")
              noteCatalogRevisions?.(nextPresence.catalogVersion);
            updateDesktopOnline(nextPresence, lastPresenceReceiptRef.current);
          } else {
            setDesktopOnline(false);
          }
        },
        onSessions: (sessionList, version) => {
          if (access !== accessRef.current) return;
          // A snapshot published just before control moved can still be in
          // flight when the other device's takeover lands. The machine has no
          // catalogue for this client any more, so neither that packet nor the
          // next one is applied — the rows would otherwise reappear behind the
          // standby state that just emptied them.
          if (latestPresenceRef.current?.control === "standby") return;
          const list: RemoteSession[] = sessionList.map((session) => ({
            ...session,
          }));
          if (applySessionSnapshot(list, version) === false) return;
          const currentId = selectedRef.current;
          if (currentId && list.length > 0 && !list.some((item) => item.sessionId === currentId)) {
            closeConversation();
          } else if (currentId) {
            const streaming =
              list.find((session) => session.sessionId === currentId)?.streaming ?? false;
            applySessionStreaming(currentId, streaming);
          }
        },
        onSessionDelta: (packet) => {
          if (access !== accessRef.current) return;
          if (latestPresenceRef.current?.control === "standby") return;
          // The same three things a snapshot does: retire a confirmed optimistic
          // title, keep the open conversation consistent with the row that just
          // arrived, and report a run that finished while this packet was in
          // flight. The catalogue owns the merge; this owns the screen.
          if (applySessionDelta(packet) === false) return;
          const currentId = selectedRef.current;
          if (!currentId) return;
          const row = packet.rows?.find((item) => item.sessionId === currentId);
          if (row) applySessionStreaming(currentId, row.streaming === true);
          else if (packet.removed?.includes(currentId)) closeConversation();
        },
        onSessionActivity: (activity, version) => {
          if (access !== accessRef.current) return;
          if (latestPresenceRef.current?.control === "standby") return;
          applySessionActivity(activity, version);
        },
        onWorkspaces: (list, version) => {
          if (access !== accessRef.current) return;
          if (latestPresenceRef.current?.control === "standby") return;
          setWorkspaces(list, version);
        },
        onFeatures: (features) => {
          if (access === accessRef.current) setCapabilities(new Set(features));
        },
        // The host's immediate word on control, ahead of the heartbeat that
        // repeats it. `noteControl` records it and reports it once, from
        // whichever packet said it first.
        onControl: (control) => {
          if (access === accessRef.current) noteControl(control);
        },
        onConnectionState: (state: ConnectionState) => {
          if (access !== accessRef.current) return;
          connectionReadyRef.current = state === "ready";
          if (state === "ready") {
            setHasConnectedContent(true);
            // An answer arrived, so this pairing is not a silent one — whatever
            // the next disconnection turns out to be, it is not this.
            setFirstAnswerSpent(false);
            setPhase("ready");
            setError(null);
            // Handshake presence arrives before ready; evaluate it now rather
            // than leaving the UI disabled until the next heartbeat/timer.
            updateDesktopOnline(latestPresenceRef.current, Date.now());
          } else if (state === "revoked" || state === "unpaired") {
            // `revoked` is the client's word for a credential the platform
            // refused to renew — definitive, and the same ending as the host's
            // own unpaired notice. (`unpaired` here is the FSM's initial state,
            // not a verdict: it arrives on a shutdown, so it decides nothing.)
            if (state === "revoked") onUnpaired?.();
            void discardPendingPrompt(nextCredentials.pairId);
            void discardPendingContinuation(nextCredentials.pairId);
            setCapabilities(new Set());
            resetCatalog();
            resetConversation();
            resetTimeline();
            setPhase("revoked");
            setError("credentials_revoked");
          } else if (state === "refreshing") setPhase("refreshing");
          else if (state === "failed") setPhase("failed");
          else if (state === "connecting") setPhase("connecting");
          else setPhase("reconnecting");
          if (state !== "ready") setDesktopOnline(false);
          if (state === "failed") {
            resetCatalog();
            resetConversation();
            resetTimeline();
            setCapabilities(new Set());
          }
        },
        onReconnected: () => {
          if (access !== accessRef.current) return;
          void recoverState(client).catch((error) => {
            if (access === accessRef.current) recordError(error);
          });
          presenceStateRef.current = INITIAL_PRESENCE_STATE;
        },
        onError: (error) => {
          if (access === accessRef.current) {
            setError(error.message);
          }
        },
      });
      clientRef.current = client;
      client.setVisibleSession?.(visibleSessionRef.current);
      client.setAppActive(appActiveRef.current);
      await client.open();
    },
    [
      applySessionActivity,
      applySessionDelta,
      applySessionSnapshot,
      applySessionStreaming,
      closeConversation,
      controlChanged,
      desktopId,
      handleEvent,
      noteCatalogRevisions,
      noteControl,
      onUnpaired,
      reconcileSession,
      recordError,
      stopClient,
      recoverState,
      resetCatalog,
      resetConversation,
      clientRef,
      credentialsRef,
      resetTimeline,
      selectedRef,
      setCatalogEpoch,
      setWorkspaces,
      updateDesktopOnline,
    ],
  );

  useEffect(() => {
    let active = true;
    const bootstrapAccess = accessRef.current;
    void (async () => {
      try {
        const stored = await loadCredentials(desktopId);
        if (!active || bootstrapAccess !== accessRef.current) return;
        if (!stored) {
          // Paired in the registry but unusable on this device: the picker is
          // where the user can re-pair or remove it.
          setError("incomplete_desktop_credentials");
          setPhase("failed");
          return;
        }
        await connect(stored);
      } catch (nextError) {
        if (!active) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setPhase("failed");
      }
    })();
    return () => {
      active = false;
      accessRef.current += 1;
      void stopClient(clientRef.current);
      clientRef.current = null;
    };
  }, [clientRef, connect, desktopId, onUnpaired, stopClient]);

  /**
   * Recover this connection.
   *
   * The connection was healthy and then was not: probe the same socket with an
   * authenticated read before tearing anything down, then rebuild what the
   * catalogue may have missed while the lane was down.
   */
  const recover = useCallback(
    async (reason: RecoveryReason) => {
      // A recovery is a fresh attempt at the same desktop, so the answer it is
      // waiting for has not been withheld yet: the clock starts again. Without
      // this, pressing the retry on the "not serving" page would leave that
      // page up for the whole of the attempt it asked for.
      setFirstAnswerSpent(false);
      const client = clientRef.current;
      if (AppState.currentState !== "active") {
        // A recovery that never happened is the wedge an app restart used to be
        // the only way out of. Say so, so the next report has evidence instead
        // of a spinner: if this fires while the user is looking at the screen,
        // the lifecycle state is stale, not the connection.
        console.warn("[remote] recovery skipped while the app is not active");
        return;
      }
      if (!client || clientRef.current !== client || !credentialsRef.current) return;
      try {
        if (client.connectionState === "failed") {
          // A fatal client is terminal by design: `transition` keeps it in
          // `failed` and every request answers `communication_frozen`, so
          // probing it recovers nothing — a conversation would not open and a
          // prompt would not send, until the app was killed and started again.
          // The state machine's own answer is "a manual reconnect creates a
          // fresh RemoteClient": do exactly that with the same credentials.
          console.warn("[remote] rebuilding a terminal client", {
            reason,
            state: client.connectionState,
          });
          await connect(credentialsRef.current);
          return;
        }
        const revision = recoveryRef.current.revision(client);
        await client.recoverNow(reason);
        if (
          clientRef.current !== client ||
          !credentialsRef.current ||
          AppState.currentState !== "active"
        )
          return;
        presenceStateRef.current = INITIAL_PRESENCE_STATE;
        // A new transport already requested recovery through onReconnected.
        // A healthy probe still needs one refresh for missed state.
        if (revision === recoveryRef.current.revision(client)) await recoverState(client);
        else await recoveryRef.current.settled(client);
      } catch (nextError) {
        if (clientRef.current === client) recordError(nextError);
      }
    },
    [clientRef, connect, credentialsRef, recordError, recoverState],
  );

  const setAppActive = useCallback((next: boolean) => {
    appActiveRef.current = next;
    setAppActiveState(next);
    clientRef.current?.setAppActive(next);
  }, [clientRef]);

  const setVisibleSession = useCallback((sessionId: string) => {
    visibleSessionRef.current = sessionId;
    clientRef.current?.setVisibleSession?.(sessionId);
  }, [clientRef]);

  const noteControlActive = useCallback(() => {
    // Nothing to amend before the handshake has described this pairing: the
    // connection's own presence will be the first word on control.
    if (!latestPresenceRef.current || !connectionReadyRef.current) return;
    // Through the same road as a packet: the takeover answer is one more way of
    // being told control moved, and the catalogue that was dropped when it left
    // has to be read again whichever way it comes back.
    noteControl("active");
  }, [noteControl]);

  const notifyUnpair = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    // Preserve the existing best-effort desktop notification, with no remote
    // callback allowed to repopulate local state while it settles.
    const remoteUnpair = client.request({ type: "unpair" }).catch(() => undefined);
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        remoteUnpair,
        new Promise<void>((resolve) => {
          timer = setTimeout(resolve, 750);
        }),
      ]);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }, [clientRef]);

  const clearError = useCallback(() => setError(null), []);

  /**
   * A connection that gave up asked the platform whether the pairing still
   * exists.
   *
   * "Failed" is the terminal state of a handshake that got no answer — the
   * desktop is asleep, its bridge is gone, or the pairing was revoked while the
   * phone was in the background and the host's `unpaired` notice was published
   * to nobody. The first two are retryable and belong to the desktop; the third
   * only looks like one, and used to leave the phone on "已断开 / 重试" against a
   * pairing that no longer exists. A credential refresh is the one question the
   * phone can ask itself: the platform answers `invalid_remote_credential` for a
   * pairing it no longer has, and hands out a fresh JWT for one it does. A
   * refresh rotates nothing here (the platform issues no new refresh token), so
   * a successful check costs one request and changes nothing.
   */
  const verificationRef = useRef<string | null>(null);
  useEffect(() => {
    if (phase !== "failed") {
      verificationRef.current = null;
      return;
    }
    const current = credentialsRef.current;
    if (!current) return;
    // One check per failure episode: `phase` stays "failed" while the screen
    // offers its retry, and a rebuild bumps the phase before it can fail again.
    const episode = current.pairId;
    if (verificationRef.current === episode) return;
    verificationRef.current = episode;
    // A caller without the check (an embedding that has no platform) leaves the
    // failure as it is rather than reading it as a revocation.
    if (!verifyPairing) return;
    void verifyPairing(current).catch((failure) => {
      if (classifyError(failure) !== "authTerminal") return;
      // Definitive: this device is not paired to that desktop any more.
      onUnpaired?.();
    });
  }, [credentialsRef, onUnpaired, phase, verifyPairing]);

  // The clock that ends "connecting".
  //
  // Neither phase says whether anything answered: the transport reports
  // `connecting` while an attempt is in flight and `reconnecting` between
  // them, and it keeps retrying a desktop that never answers for as long as
  // the pairing lasts — with a five-minute silence once the first attempt
  // budget is spent. So a phone that has never reached this pairing has no word
  // but "connecting", indefinitely, which is exactly what "nearly there" looks
  // like. Nothing is attempted while the app is in the background either, so
  // the clock only runs in front — and starts over on the way back rather than
  // counting time in which the phone was not trying at all.
  const waitingForFirstAnswer = awaitingFirstAnswer(phase, hasConnectedContent);
  useEffect(() => {
    if (!waitingForFirstAnswer || !appActive || firstAnswerSpent) return;
    const timer = setTimeout(() => setFirstAnswerSpent(true), FIRST_ANSWER_BUDGET_MS);
    return () => clearTimeout(timer);
  }, [appActive, firstAnswerSpent, waitingForFirstAnswer]);
  // Only ever reported while the phone is in fact still waiting: an answer, a
  // disconnect the desktop stated, or a failure the phone can classify all end
  // the wait, and none of them may inherit the previous attempt's verdict.
  const desktopUnanswered = firstAnswerSpent && waitingForFirstAnswer;

  // A heartbeat that stopped is only visible against the clock. Judging it here
  // (rather than on a push) is what makes a silently dead desktop read as
  // offline instead of as "the last packet said it was fine".
  useEffect(() => {
    const timer = setInterval(() => {
      if (phase === "ready") {
        updateDesktopOnline(latestPresenceRef.current, Date.now());
        // After a desktop restart the broker connection may stay open, but
        // the old traffic keys are gone. Recover without trusting an unsigned
        // presence beacon (or waiting for the next JWT refresh).
        if (
          AppState.currentState === "active" &&
          Date.now() - lastPresenceReceiptRef.current >= PRESENCE_RECEIPT_STALE_MS
        ) {
          void recover("presence-stale");
        }
      } else setDesktopOnline(false);
    }, 10_000);
    return () => clearInterval(timer);
  }, [phase, recover, updateDesktopOnline]);

  return {
    clientRef,
    credentials,
    phase,
    error,
    presence,
    desktopOnline,
    hasConnectedContent,
    desktopUnanswered,
    capabilities,
    fileTransferSupported: capabilities.has("file_transfer_v1"),
    promptReceiptSupported: capabilities.has("prompt_receipt_v1"),
    recordError,
    clearError,
    recover,
    setAppActive,
    setVisibleSession,
    notifyUnpair,
    noteControlActive,
  };
}
