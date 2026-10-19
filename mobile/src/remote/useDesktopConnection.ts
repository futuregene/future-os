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
  closeConversation(): void;
  resetConversation(): void;
  resetCatalog(): void;
  resetTimeline(): void;
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
  applySessionActivity,
  applySessionStreaming,
  setWorkspaces,
  refreshModels,
  refreshSessions,
  refreshSettings,
  refreshWorkspaces,
  noteCatalogRevisions,
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
          // Both terminal notices describe *this* pairing ending, and both mean
          // the same thing to the phone: stop, mirror nothing, and keep the
          // local pairing so the user can retry or remove it deliberately.
          if (nextPresence.unpaired) {
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
          latestPresenceRef.current = nextPresence;
          setPresence(nextPresence);
          if (connectionReadyRef.current) {
            // Only past the ready barrier: a pull issued against a
            // half-built connection would race the handshake it depends on.
            noteCatalogRevisions?.(nextPresence.catalogVersion);
            updateDesktopOnline(nextPresence, lastPresenceReceiptRef.current);
          } else {
            setDesktopOnline(false);
          }
        },
        onSessions: (sessionList, version) => {
          if (access !== accessRef.current) return;
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
        onSessionActivity: (activity, version) => {
          if (access === accessRef.current) applySessionActivity(activity, version);
        },
        onWorkspaces: (list, version) => {
          if (access === accessRef.current) setWorkspaces(list, version);
        },
        onFeatures: (features) => {
          if (access === accessRef.current) setCapabilities(new Set(features));
        },
        onConnectionState: (state: ConnectionState) => {
          if (access !== accessRef.current) return;
          connectionReadyRef.current = state === "ready";
          if (state === "ready") {
            setHasConnectedContent(true);
            setPhase("ready");
            setError(null);
            // Handshake presence arrives before ready; evaluate it now rather
            // than leaving the UI disabled until the next heartbeat/timer.
            updateDesktopOnline(latestPresenceRef.current, Date.now());
          } else if (state === "revoked" || state === "unpaired") {
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
      applySessionSnapshot,
      applySessionStreaming,
      closeConversation,
      desktopId,
      handleEvent,
      noteCatalogRevisions,
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
  }, [clientRef, connect, desktopId, stopClient]);

  /**
   * Recover this connection.
   *
   * The connection was healthy and then was not: probe the same socket with an
   * authenticated read before tearing anything down, then rebuild what the
   * catalogue may have missed while the lane was down.
   */
  const recover = useCallback(
    async (reason: RecoveryReason) => {
      const client = clientRef.current;
      if (AppState.currentState !== "active") return;
      if (!client || clientRef.current !== client || !credentialsRef.current) return;
      try {
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
    [clientRef, credentialsRef, recordError, recoverState],
  );

  const setAppActive = useCallback((next: boolean) => {
    appActiveRef.current = next;
    clientRef.current?.setAppActive(next);
  }, [clientRef]);

  const setVisibleSession = useCallback((sessionId: string) => {
    visibleSessionRef.current = sessionId;
    clientRef.current?.setVisibleSession?.(sessionId);
  }, [clientRef]);

  const noteControlActive = useCallback(() => {
    const current = latestPresenceRef.current;
    // Nothing to amend before the handshake has described this pairing: the
    // connection's own presence will be the first word on control.
    if (!current || !connectionReadyRef.current) return;
    if (current.control === "active" && current.holder == null) return;
    const next = { ...current, control: "active" as const, holder: null };
    latestPresenceRef.current = next;
    setPresence(next);
  }, []);

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
