import type { MutableRefObject } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import * as Network from "expo-network";
import { AppState, type AppStateStatus } from "react-native";
import { RemoteClient } from "./client";
import type { ConnectionState } from "./connectionState";
import { classifyError, RemoteApiError } from "./connectionState";
import { NATIVE_PRESENTATION_GRACE_MS, nativePresentationInFlight } from "./nativePresentation";
import { attemptPendingRevoke, claimPairingCode, serverRevoke } from "./pairing";
import { discardPendingPrompt } from "./pendingPromptStorage";
import { discardPendingContinuation } from "./pendingContinuationStorage";
import {
  INITIAL_PRESENCE_STATE,
  isDesktopOnline,
  PRESENCE_RECEIPT_STALE_MS,
  type PresenceState,
} from "./presence";
import { RecoveryCoordinator } from "./recoveryCoordinator";
import type { ReconcileReason, SyncEngine } from "./syncEngine";
import {
  clearCredentials,
  clearPendingRevoke,
  loadCredentials,
  loadPendingRevoke,
  saveCredentials,
  savePendingRevoke,
} from "./storage";
import type {
  ConnectionPhase,
  SnapshotVersion,
  Presence,
  RemoteCredentials,
  RemoteSession,
  RemoteWorkspace,
  StreamEvent,
} from "./types";

interface RemoteConnectionOptions {
  clientRef: MutableRefObject<RemoteClient | null>;
  credentialsRef: MutableRefObject<RemoteCredentials | null>;
  selectedRef: MutableRefObject<string>;
  syncEngineRef: MutableRefObject<SyncEngine | null>;
  handleEvent(event: StreamEvent, sessionId: string): void;
  reconcileSession(sessionId: string | undefined, reason: ReconcileReason, runId?: string): void;
  setCatalogEpoch?(epoch: string | undefined): void;
  applySessionSnapshot(sessions: RemoteSession[], version?: SnapshotVersion): boolean | void;
  applySessionStreaming(sessionId: string, streaming: boolean): void;
  setWorkspaces(workspaces: RemoteWorkspace[], version?: SnapshotVersion): void;
  refreshModels(): Promise<void>;
  refreshSessions(): Promise<void>;
  refreshSettings(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  closeConversation(): void;
  resetConversation(): void;
  resetCatalog(): void;
  resetTimeline(): void;
}

export function useRemoteConnection({
  clientRef,
  credentialsRef,
  selectedRef,
  syncEngineRef,
  handleEvent,
  reconcileSession,
  setCatalogEpoch,
  applySessionSnapshot,
  applySessionStreaming,
  setWorkspaces,
  refreshModels,
  refreshSessions,
  refreshSettings,
  refreshWorkspaces,
  closeConversation,
  resetConversation,
  resetCatalog,
  resetTimeline,
}: RemoteConnectionOptions) {
  const [phase, setPhase] = useState<ConnectionPhase>("booting");
  const [error, setError] = useState<string | null>(null);
  const [credentials, setCredentials] = useState<RemoteCredentials | null>(null);
  const [presence, setPresence] = useState<Presence | null>(null);
  const [capabilities, setCapabilities] = useState<Set<string>>(() => new Set());
  const [desktopOnline, setDesktopOnline] = useState(false);
  const accessRef = useRef(0);
  const catalogPairRef = useRef<string | null>(null);
  const recoveryRef = useRef(new RecoveryCoordinator<RemoteClient>());
  const pairingAbortRef = useRef<AbortController | null>(null);
  const connectionReadyRef = useRef(false);
  const presenceStateRef = useRef<PresenceState>(INITIAL_PRESENCE_STATE);
  const revokesRunningRef = useRef(false);
  const revokeTerminalRef = useRef(new Set<string>());
  const agentAvailableRef = useRef<boolean | undefined>(undefined);
  const lastPresenceReceiptRef = useRef(0);
  const networkAvailableRef = useRef<boolean | null>(null);
  const refreshNetworkStateRef = useRef(async () => networkAvailableRef.current !== false);

  useEffect(() => {
    credentialsRef.current = credentials;
  }, [credentials, credentialsRef]);

  const recordError = useCallback((nextError: unknown) => {
    if (classifyError(nextError) === "transport") return;
    console.warn("[remote] unexpected non-transport error", nextError);
    setError(nextError instanceof Error ? nextError.message : String(nextError));
  }, []);

  const drainRevokes = useCallback(async () => {
    if (revokesRunningRef.current || networkAvailableRef.current === false) return;
    revokesRunningRef.current = true;
    try {
      const attempted = new Set(revokeTerminalRef.current);
      for (;;) {
        const pending = await loadPendingRevoke(attempted);
        if (!pending || attempted.has(pending.pairId)) return;
        attempted.add(pending.pairId);
        try {
          await attemptPendingRevoke(pending);
          await clearPendingRevoke(pending.pairId);
        } catch (error) {
          if (
            classifyError(error) !== "transport" ||
            (error instanceof RemoteApiError &&
              error.status >= 400 &&
              error.status < 500 &&
              error.status !== 408 &&
              error.status !== 429)
          )
            revokeTerminalRef.current.add(pending.pairId);
        }
      }
    } finally {
      revokesRunningRef.current = false;
    }
  }, []);

  useEffect(() => {
    const timer = setInterval(() => {
      if (AppState.currentState === "active") void drainRevokes().catch(recordError);
    }, 30_000);
    return () => clearInterval(timer);
  }, [drainRevokes, recordError]);

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
    [clientRef, syncEngineRef, refreshModels, refreshSessions, refreshWorkspaces, refreshSettings],
  );

  const connect = useCallback(
    async (nextCredentials: RemoteCredentials) => {
      const access = ++accessRef.current;
      const previous = clientRef.current;
      clientRef.current = null;
      await previous?.close();
      if (access !== accessRef.current) return;
      // A successful pair must mean its credentials are durable. In
      // particular, do not let the UI report success while SecureStore writes
      // are still in flight and can be lost on immediate process suspension.
      await saveCredentials(nextCredentials);
      if (access !== accessRef.current) return;
      if (catalogPairRef.current !== null && catalogPairRef.current !== nextCredentials.pairId) {
        resetCatalog();
        resetConversation();
        resetTimeline();
      }
      catalogPairRef.current = nextCredentials.pairId;
      credentialsRef.current = nextCredentials;
      setCredentials(nextCredentials);
      setError(null);
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
          if (nextPresence.unpaired) {
            accessRef.current += 1;
            credentialsRef.current = null;
            void clientRef.current?.close("Unpair");
            clientRef.current = null;
            void clearCredentials();
            void discardPendingPrompt();
            void discardPendingContinuation();
            setCredentials(null);
            setPresence(null);
            resetCatalog();
            resetConversation();
            resetTimeline();
            setPhase("unpaired");
            setError(null);
            return;
          }
          const agentRecovered =
            agentAvailableRef.current === false && nextPresence.agentAvailable === true;
          agentAvailableRef.current = nextPresence.agentAvailable;
          if (agentRecovered && connectionReadyRef.current)
            void recoverState(client).catch(recordError);
          lastPresenceReceiptRef.current = Date.now();
          setPresence(nextPresence);
          if (connectionReadyRef.current) {
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
            setPhase("ready");
            setError(null);
          } else if (state === "revoked" || state === "unpaired") {
            credentialsRef.current = null;
            void discardPendingPrompt();
            void discardPendingContinuation();
            setPhase(state);
          } else if (state === "refreshing") setPhase("refreshing");
          else if (state === "failed") setPhase("failed");
          else if (state === "connecting") setPhase("connecting");
          else setPhase("reconnecting");
          if (state !== "ready") setDesktopOnline(false);
        },
        onReconnected: () => {
          if (access !== accessRef.current) return;
          void recoverState(client).catch((error) => {
            if (access === accessRef.current) recordError(error);
          });
          presenceStateRef.current = INITIAL_PRESENCE_STATE;
        },
        onError: (error) => {
          if (access === accessRef.current) recordError(error);
        },
      });
      clientRef.current = client;
      client.setAppActive(AppState.currentState !== "background");
      if (networkAvailableRef.current === false) client.setNetworkAvailable(false);
      await client.open();
    },
    [
      applySessionSnapshot,
      applySessionStreaming,
      clientRef,
      closeConversation,
      credentialsRef,
      handleEvent,
      reconcileSession,
      recordError,
      recoverState,
      resetCatalog,
      resetConversation,
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
        void drainRevokes().catch(recordError);
        if (!active) return;
        const stored = await loadCredentials();
        if (!active || bootstrapAccess !== accessRef.current) return;
        if (!stored) {
          setPhase("unpaired");
          return;
        }
        await connect(stored);
      } catch (nextError) {
        if (!active) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setPhase("unpaired");
      }
    })();
    return () => {
      active = false;
      accessRef.current += 1;
      pairingAbortRef.current?.abort();
      void clientRef.current?.close();
      clientRef.current = null;
    };
  }, [clientRef, connect, drainRevokes, recordError]);

  const recoverLifecycle = useCallback(
    async (reason: "foreground" | "network-restored" | "network-changed") => {
      if (reason === "foreground") {
        const available = await refreshNetworkStateRef.current();
        if (!available) return;
      }
      void drainRevokes().catch(recordError);
      const client = clientRef.current;
      if (!client || !credentialsRef.current || networkAvailableRef.current === false) return;
      try {
        const revision = recoveryRef.current.revision(client);
        await client.recoverNow(reason);
        if (clientRef.current !== client || !credentialsRef.current) return;
        presenceStateRef.current = INITIAL_PRESENCE_STATE;
        // A new transport already requested recovery through onReconnected.
        // A healthy foreground probe still needs one refresh for missed state.
        if (revision === recoveryRef.current.revision(client)) await recoverState(client);
        else await recoveryRef.current.settled(client);
      } catch (nextError) {
        if (clientRef.current === client) recordError(nextError);
      }
    },
    [clientRef, credentialsRef, drainRevokes, recordError, recoverState],
  );

  useEffect(() => {
    let previous: AppStateStatus = AppState.currentState;
    let presentationTimer: ReturnType<typeof setTimeout> | null = null;
    const clearPresentationTimer = () => {
      if (presentationTimer) clearTimeout(presentationTimer);
      presentationTimer = null;
    };
    const subscription = AppState.addEventListener("change", (next) => {
      const returnedToForeground = next === "active" && previous !== "active";
      const enteredBackground = next === "background" && previous !== "background";
      previous = next;
      if (enteredBackground && nativePresentationInFlight()) {
        // A native picker paused the activity — not a real background. Keep the
        // socket, but bound the grace: a flow the user never comes back from
        // must not hold a live connection open indefinitely.
        clearPresentationTimer();
        presentationTimer = setTimeout(() => {
          presentationTimer = null;
          if (AppState.currentState !== "active" && nativePresentationInFlight()) {
            clientRef.current?.setAppActive(false);
          }
        }, NATIVE_PRESENTATION_GRACE_MS);
      } else if (enteredBackground || returnedToForeground) {
        clearPresentationTimer();
        clientRef.current?.setAppActive(next === "active");
      }
      if (returnedToForeground) void recoverLifecycle("foreground");
    });
    return () => {
      clearPresentationTimer();
      subscription.remove();
    };
  }, [clientRef, recoverLifecycle]);

  useEffect(() => {
    let active = true;
    let eventSeen = false;
    let previousType: Network.NetworkStateType | undefined;
    const observe = (state: Network.NetworkState, triggerRecovery = true): boolean => {
      if (!active) return false;
      const available =
        state.type !== Network.NetworkStateType.NONE &&
        state.isConnected !== false &&
        state.isInternetReachable !== false;
      const wasAvailable = networkAvailableRef.current;
      const pathChanged =
        wasAvailable === true &&
        available &&
        previousType !== undefined &&
        previousType !== Network.NetworkStateType.UNKNOWN &&
        state.type !== undefined &&
        state.type !== Network.NetworkStateType.UNKNOWN &&
        state.type !== previousType;
      networkAvailableRef.current = available;
      previousType = state.type;
      clientRef.current?.setNetworkAvailable(available);
      if (!available) return false;
      if (triggerRecovery && wasAvailable === false) void recoverLifecycle("network-restored");
      else if (triggerRecovery && pathChanged) void recoverLifecycle("network-changed");
      return true;
    };
    refreshNetworkStateRef.current = async () => {
      try {
        return observe(await Network.getNetworkStateAsync(), false);
      } catch (nextError) {
        console.warn("[remote] foreground network refresh failed", {
          error: nextError,
        });
        return networkAvailableRef.current !== false;
      }
    };
    void Network.getNetworkStateAsync()
      .then((state) => {
        if (!eventSeen) observe(state);
      })
      .catch(() => undefined);
    const subscription = Network.addNetworkStateListener((state) => {
      eventSeen = true;
      observe(state);
    });
    return () => {
      active = false;
      subscription.remove();
      refreshNetworkStateRef.current = async () => networkAvailableRef.current !== false;
    };
  }, [clientRef, recoverLifecycle]);

  useEffect(() => {
    const timer = setInterval(() => {
      if (phase === "ready") updateDesktopOnline(presence, Date.now());
      else setDesktopOnline(false);
    }, 10_000);
    return () => clearInterval(timer);
  }, [phase, presence, updateDesktopOnline]);

  const pair = useCallback(
    async (code: string) => {
      accessRef.current += 1;
      pairingAbortRef.current?.abort();
      const controller = new AbortController();
      pairingAbortRef.current = controller;
      setPhase("claiming");
      setError(null);
      try {
        const claimed = await claimPairingCode(code, controller.signal);
        if (controller.signal.aborted) return;
        await connect(claimed);
      } catch (nextError) {
        if (controller.signal.aborted) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setPhase("unpaired");
        throw nextError;
      }
    },
    [connect],
  );

  const reconnect = useCallback(async () => {
    const stored = credentials ?? (await loadCredentials());
    if (!stored) {
      setPhase("unpaired");
      return;
    }
    try {
      await connect(stored);
    } catch (nextError) {
      const message = nextError instanceof Error ? nextError.message : String(nextError);
      if (message === "invalid_jwt") {
        credentialsRef.current = null;
        await clearCredentials();
        await discardPendingPrompt();
        await discardPendingContinuation();
        setCredentials(null);
        setPhase("unpaired");
        setError(null);
      } else setError(message);
    }
  }, [connect, credentials, credentialsRef]);

  const unpair = useCallback(async () => {
    const unpairAccess = ++accessRef.current;
    pairingAbortRef.current?.abort();
    const current = credentialsRef.current ?? credentials;
    credentialsRef.current = null;
    connectionReadyRef.current = false;
    const client = clientRef.current;
    clientRef.current = null;
    // Preserve the existing best-effort desktop notification, with no remote
    // callbacks allowed to repopulate local state while it settles.
    const remoteUnpair = client?.request({ type: "unpair" }).catch(() => undefined);
    setCredentials(null);
    setPresence(null);
    setDesktopOnline(false);
    setCapabilities(new Set());
    resetCatalog();
    resetConversation();
    resetTimeline();
    setPhase("unpaired");
    setError(null);
    const closing = (async () => {
      if (remoteUnpair) {
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
      }
      await client?.close("Unpair");
    })();
    // Queue before clearing the credential bundle so offline cleanup survives
    // process exit. Network work never blocks finishing the local unpair.
    try {
      if (current) await savePendingRevoke(current);
    } finally {
      if (current) await clearCredentials(current.pairId);
      else if (accessRef.current === unpairAccess) await clearCredentials();
      await discardPendingPrompt();
      await discardPendingContinuation();
      await closing;
    }
    if (current)
      void serverRevoke(current)
        .then(() => clearPendingRevoke(current.pairId))
        .catch(() => undefined);
  }, [clientRef, credentials, credentialsRef, resetCatalog, resetConversation, resetTimeline]);

  const clearError = useCallback(() => setError(null), []);

  return {
    phase,
    error,
    credentials,
    presence,
    desktopOnline,
    capabilities,
    fileTransferSupported: capabilities.has("file_transfer_v1"),
    promptReceiptSupported: capabilities.has("prompt_receipt_v1"),
    recordError,
    pair,
    reconnect,
    unpair,
    clearError,
  };
}
