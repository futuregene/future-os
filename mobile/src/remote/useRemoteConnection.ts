import type { MutableRefObject } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import * as Network from "expo-network";
import { AppState, type AppStateStatus } from "react-native";
import { RemoteClient, type RecoveryReason } from "./client";
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
  loadPairedDesktops,
  renameDesktop as renameStoredDesktop,
  loadPendingRevoke,
  saveCredentials,
  savePendingRevoke,
} from "./storage";
import type {
  ConnectionPhase,
  PairedDesktop,
  SnapshotVersion,
  Presence,
  RemoteCredentials,
  RemoteSession,
  RemoteWorkspace,
  StreamEvent,
} from "./types";

// Avoid tearing down for a quick app switch / native activity transition.
// This is not a background execution or push-notification guarantee.
export const BACKGROUND_GRACE_MS = 5_000;
export const NETWORK_PROBE_TIMEOUT_MS = 4_000;

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
  const latestPresenceRef = useRef<Presence | null>(null);
  const [desktops, setDesktops] = useState<PairedDesktop[]>([]);
  const refreshDesktops = useCallback(async () => {
    const storedDesktops = await loadPairedDesktops();
    setDesktops(storedDesktops);
    return storedDesktops;
  }, []);
  const [capabilities, setCapabilities] = useState<Set<string>>(() => new Set());
  const [desktopOnline, setDesktopOnline] = useState(false);
  const [hasConnectedContent, setHasConnectedContent] = useState(false);
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
      if (catalogPairRef.current !== null && catalogPairRef.current !== nextCredentials.pairId) {
        setHasConnectedContent(false);
        resetCatalog();
        resetConversation();
        resetTimeline();
      }
      await previous?.close();
      if (access !== accessRef.current) return;
      // A successful pair must mean its credentials are durable. In
      // particular, do not let the UI report success while SecureStore writes
      // are still in flight and can be lost on immediate process suspension.
      await saveCredentials(nextCredentials);
      if (access !== accessRef.current) return;
      await refreshDesktops();
      if (access !== accessRef.current) return;
      catalogPairRef.current = nextCredentials.pairId;
      credentialsRef.current = nextCredentials;
      setCredentials(nextCredentials);
      // A manual Desktop disconnect is a terminal presence packet. Starting a
      // user-requested connection attempt supersedes that packet immediately,
      // so the UI can enter the normal connecting state rather than remaining
      // pinned to the previous red disconnected presentation.
      setPresence(null);
      setPhase("connecting");
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
            void client.close("UserInitiated");
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
            void client.close("UserInitiated");
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
      refreshDesktops,
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
      let storedDesktops: PairedDesktop[] = [];
      try {
        void drainRevokes().catch(recordError);
        if (!active) return;
        // Resolve the registry before the active credential so final routing
        // can distinguish an empty installation from an unselected desktop.
        storedDesktops = await refreshDesktops();
        if (!active || bootstrapAccess !== accessRef.current) return;
        const stored = await loadCredentials();
        if (!active || bootstrapAccess !== accessRef.current) return;
        if (!stored) {
          // Pairings without an active selection belong in the desktop picker.
          // Keep booting until both registry and selection have been resolved,
          // so a valid active desktop does not flash the picker first.
          setError(null);
          setPhase("unpaired");
          return;
        }
        await connect(stored);
      } catch (nextError) {
        if (!active) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setPhase(storedDesktops.length > 0 ? "failed" : "unpaired");
      }
    })();
    return () => {
      active = false;
      accessRef.current += 1;
      pairingAbortRef.current?.abort();
      void clientRef.current?.close();
      clientRef.current = null;
    };
  }, [clientRef, connect, drainRevokes, recordError, refreshDesktops]);

  const recoverLifecycle = useCallback(
    async (reason: RecoveryReason) => {
      const client = clientRef.current;
      if (reason === "foreground") {
        const available = await refreshNetworkStateRef.current();
        if (!available) return;
      }
      void drainRevokes().catch(recordError);
      if (!client || clientRef.current !== client || !credentialsRef.current || networkAvailableRef.current === false) return;
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
    let backgrounded = previous === "background";
    let backgroundDeadline: number | null = backgrounded ? Date.now() : null;
    let presentationTimer: ReturnType<typeof setTimeout> | null = null;
    const clearPresentationTimer = () => {
      if (presentationTimer) clearTimeout(presentationTimer);
      presentationTimer = null;
    };
    const subscription = AppState.addEventListener("change", (next) => {
      const returnedToForeground = next === "active" && previous !== "active";
      const enteredBackground = next === "background" && previous !== "background";
      previous = next;
      if (enteredBackground) {
        backgrounded = true;
        clearPresentationTimer();
        // Keep a picker/quick-switch connection alive. Do not depend on the
        // picker promise still being pending at expiry: Android can deliver
        // its result before delivering the resumed AppState event.
        const graceMs = nativePresentationInFlight() ? NATIVE_PRESENTATION_GRACE_MS : BACKGROUND_GRACE_MS;
        backgroundDeadline = Date.now() + graceMs;
        presentationTimer = setTimeout(() => {
          presentationTimer = null;
          if (previous !== "active") clientRef.current?.setAppActive(false);
        }, graceMs);
      } else if (returnedToForeground) {
        clearPresentationTimer();
        // iOS alerts/control center emit inactive -> active without suspending
        // the app. They must not restart history/catalogue sync or handshake.
        if (backgrounded) {
          backgrounded = false;
          // The OS can suspend JS before the grace timer fires, then deliver
          // active before overdue timers on resume. Retire the stale socket
          // and in-flight recovery before starting a new foreground attempt.
          if (backgroundDeadline !== null && Date.now() >= backgroundDeadline) {
            clientRef.current?.setAppActive(false);
          }
          backgroundDeadline = null;
          clientRef.current?.setAppActive(true);
          void recoverLifecycle("foreground");
        }
      }
    });
    return () => {
      clearPresentationTimer();
      subscription.remove();
    };
  }, [clientRef, recoverLifecycle]);

  useEffect(() => {
    let active = true;
    let observationRevision = 0;
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
      const revision = ++observationRevision;
      let timer: ReturnType<typeof setTimeout> | undefined;
      try {
        const state = await Promise.race([
          Network.getNetworkStateAsync(),
          new Promise<never>((_, reject) => {
            timer = setTimeout(() => reject(new Error("network_probe_timeout")), NETWORK_PROBE_TIMEOUT_MS);
          }),
        ]);
        // A newer query or native event supersedes this snapshot. Its late
        // offline result must not tear down a newly recovered connection.
        if (!active) return false;
        if (revision !== observationRevision) return networkAvailableRef.current !== false;
        return observe(state, false);
      } catch (nextError) {
        if (!active) return false;
        if (revision === observationRevision) {
          console.warn("[remote] foreground network refresh failed", {
            error: nextError,
          });
        }
        return networkAvailableRef.current !== false;
      } finally {
        if (timer) clearTimeout(timer);
      }
    };
    const initialRevision = observationRevision;
    void Network.getNetworkStateAsync()
      .then((state) => {
        if (initialRevision === observationRevision) observe(state);
      })
      .catch(() => undefined);
    const subscription = Network.addNetworkStateListener((state) => {
      observationRevision += 1;
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
      if (phase === "ready") {
        updateDesktopOnline(presence, Date.now());
        // After a desktop restart the broker connection may stay open, but
        // the old traffic keys are gone. Recover without trusting an unsigned
        // presence beacon (or waiting for the next JWT refresh).
        if (AppState.currentState === "active" && Date.now() - lastPresenceReceiptRef.current >= PRESENCE_RECEIPT_STALE_MS) {
          void recoverLifecycle("presence-stale");
        }
      } else setDesktopOnline(false);
    }, 10_000);
    return () => clearInterval(timer);
  }, [phase, presence, updateDesktopOnline, recoverLifecycle]);

  const pair = useCallback(
    async (code: string) => {
      pairingAbortRef.current?.abort();
      const controller = new AbortController();
      pairingAbortRef.current = controller;
      const previousPhase = phase;
      setPhase("claiming");
      setError(null);
      try {
        const claimed = await claimPairingCode(code, controller.signal);
        if (controller.signal.aborted) return;
        await connect(claimed);
      } catch (nextError) {
        if (controller.signal.aborted) return;
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setPhase(credentialsRef.current ? previousPhase : "unpaired");
        throw nextError;
      }
    },
    [connect, credentialsRef, phase],
  );

  const switchDesktop = useCallback(async (desktopId: string) => {
    pairingAbortRef.current?.abort();
    const controller = new AbortController();
    pairingAbortRef.current = controller;
    const stored = await loadCredentials(desktopId);
    if (controller.signal.aborted) return;
    if (!stored) throw new Error("desktop_not_paired");
    await connect(stored);
  }, [connect]);

  const reconnect = useCallback(async () => {
    try {
      const stored = credentials ?? (await loadCredentials());
      if (!stored) {
        if (desktops.length > 0) {
          setError("incomplete_desktop_credentials");
          setPhase("failed");
        } else {
          setError(null);
          setPhase("unpaired");
        }
        return;
      }
      await connect(stored);
    } catch (nextError) {
      const message = nextError instanceof Error ? nextError.message : String(nextError);
      setError(message);
      setPhase(message === "invalid_jwt" || message === "credentials_revoked" ? "revoked" : "failed");
    }
  }, [connect, credentials, desktops.length]);

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
    setHasConnectedContent(false);
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
      await refreshDesktops();
      await discardPendingPrompt(current?.pairId);
      await discardPendingContinuation(current?.pairId);
      await closing;
    }
    if (current)
      void serverRevoke(current)
        .then(() => clearPendingRevoke(current.pairId))
        .catch(() => undefined);
  }, [clientRef, credentials, credentialsRef, refreshDesktops, resetCatalog, resetConversation, resetTimeline]);

  const renameDesktop = useCallback(async (desktopId: string, name: string) => {
    await renameStoredDesktop(desktopId, name);
    await refreshDesktops();
  }, [refreshDesktops]);

  const removeDesktop = useCallback(async (desktopId: string) => {
    if (credentials?.expectedDesktopId === desktopId) return unpair();
    const stored = await loadCredentials(desktopId);
    if (!stored) return;
    await savePendingRevoke(stored);
    await clearCredentials(stored.pairId);
    await discardPendingPrompt(stored.pairId);
    await discardPendingContinuation(stored.pairId);
    await refreshDesktops();
    void drainRevokes().catch(recordError);
  }, [credentials, drainRevokes, recordError, refreshDesktops, unpair]);

  const clearError = useCallback(() => setError(null), []);

  return {
    phase,
    error,
    credentials,
    desktops,
    switchDesktop,
    renameDesktop,
    removeDesktop,
    presence,
    desktopOnline,
    hasConnectedContent,
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
