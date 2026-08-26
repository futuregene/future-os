import type { MutableRefObject } from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import * as Network from "expo-network";
import { AppState, type AppStateStatus } from "react-native";
import { RemoteClient } from "./client";
import type { ConnectionState } from "./connectionState";
import { classifyError } from "./connectionState";
import { attemptPendingRevoke, claimPairingCode, serverRevoke } from "./pairing";
import { clearPendingPrompt, loadPendingPrompt } from "./pendingPromptStorage";
import {
  INITIAL_PRESENCE_STATE,
  isDesktopOnline,
  PRESENCE_RECEIPT_STALE_MS,
  type PresenceState,
} from "./presence";
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
  recoverRemoteState(sessionId?: string): Promise<void>;
  applySessionSnapshot(sessions: RemoteSession[]): void;
  applySessionStreaming(sessionId: string, streaming: boolean): void;
  setWorkspaces(workspaces: RemoteWorkspace[]): void;
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
  recoverRemoteState,
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
  const connectionReadyRef = useRef(false);
  const presenceStateRef = useRef<PresenceState>(INITIAL_PRESENCE_STATE);
  const lastPresenceReceiptRef = useRef(0);
  const networkAvailableRef = useRef<boolean | null>(null);
  const refreshNetworkStateRef = useRef<() => Promise<boolean>>(
    async () => networkAvailableRef.current !== false,
  );

  useEffect(() => {
    credentialsRef.current = credentials;
  }, [credentials, credentialsRef]);

  const recordError = useCallback((nextError: unknown) => {
    if (classifyError(nextError) === "transport") return;
    console.warn("[remote] unexpected non-transport error", nextError);
    setError(nextError instanceof Error ? nextError.message : String(nextError));
  }, []);

  const updateDesktopOnline = useCallback((nextPresence: Presence | null, now: number) => {
    const next = isDesktopOnline(nextPresence, now, presenceStateRef.current);
    presenceStateRef.current = next;
    setDesktopOnline(
      next.online && now - lastPresenceReceiptRef.current < PRESENCE_RECEIPT_STALE_MS,
    );
  }, []);

  const connect = useCallback(
    async (nextCredentials: RemoteCredentials) => {
      await clientRef.current?.close();
      credentialsRef.current = nextCredentials;
      setCredentials(nextCredentials);
      setError(null);
      setCapabilities(new Set());
      const client = new RemoteClient(nextCredentials, {
        onCredentials: next => {
          setCredentials(next);
          void saveCredentials(next);
        },
        onEvent: handleEvent,
        onEventDecodeFailure: (sessionId, decodeError) => {
          console.warn("[remote] malformed live event; reconciling session", {
            sessionId,
            error: decodeError,
          });
          reconcileSession(sessionId, "resend");
        },
        onPresence: nextPresence => {
          if (nextPresence.unpaired) {
            credentialsRef.current = null;
            void clientRef.current?.close("Unpair");
            clientRef.current = null;
            void clearCredentials();
            setCredentials(null);
            setPresence(null);
            resetCatalog();
            resetConversation();
            resetTimeline();
            setPhase("unpaired");
            setError(null);
            return;
          }
          lastPresenceReceiptRef.current = Date.now();
          setPresence(nextPresence);
          if (connectionReadyRef.current) {
            updateDesktopOnline(nextPresence, lastPresenceReceiptRef.current);
          } else {
            setDesktopOnline(false);
          }
        },
        onSessions: sessionList => {
          const list: RemoteSession[] = sessionList.map(session => ({ ...session }));
          applySessionSnapshot(list);
          const currentId = selectedRef.current;
          if (currentId && list.length > 0 && !list.some(item => item.sessionId === currentId)) {
            closeConversation();
          } else if (currentId) {
            const streaming =
              list.find(session => session.sessionId === currentId)?.streaming ?? false;
            applySessionStreaming(currentId, streaming);
          }
        },
        onWorkspaces: setWorkspaces,
        onFeatures: features => setCapabilities(new Set(features)),
        onConnectionState: (state: ConnectionState) => {
          connectionReadyRef.current = state === "ready";
          if (state === "ready") {
            setPhase("ready");
            setError(null);
          } else if (state === "revoked") setPhase("revoked");
          else if (state === "unpaired") setPhase("unpaired");
          else if (state === "refreshing") setPhase("refreshing");
          else if (state === "failed") setPhase("failed");
          else if (state === "connecting") setPhase("connecting");
          else setPhase("reconnecting");
          if (state !== "ready") setDesktopOnline(false);
        },
        onReconnected: () => {
          syncEngineRef.current?.restartAll("reconnect");
          void refreshModels();
          void refreshSessions();
          void refreshWorkspaces();
          presenceStateRef.current = INITIAL_PRESENCE_STATE;
        },
        onError: recordError,
      });
      clientRef.current = client;
      if (AppState.currentState === "background") client.pauseForBackground();
      if (networkAvailableRef.current === false) client.setNetworkAvailable(false);
      await client.open();
      await Promise.allSettled([
        refreshModels(),
        refreshSessions(),
        refreshWorkspaces(),
        refreshSettings(),
      ]);
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
      refreshModels,
      refreshSessions,
      refreshSettings,
      refreshWorkspaces,
      resetCatalog,
      resetConversation,
      resetTimeline,
      selectedRef,
      setWorkspaces,
      syncEngineRef,
      updateDesktopOnline,
    ],
  );

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const pending = await loadPendingRevoke();
        if (pending) {
          try {
            await attemptPendingRevoke(pending);
            await clearPendingRevoke();
          } catch {
            // Retry on next launch.
          }
        }
        if (!active) return;
        const stored = await loadCredentials();
        if (!active) return;
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
      void clientRef.current?.close();
    };
  }, [clientRef, connect]);

  const recoverLifecycle = useCallback(
    async (reason: "foreground" | "network-restored" | "network-changed") => {
      if (reason === "foreground") {
        const available = await refreshNetworkStateRef.current();
        if (!available) return;
      }
      const client = clientRef.current;
      if (!client || !credentialsRef.current || networkAvailableRef.current === false) return;
      try {
        await client.recoverNow(reason);
        if (clientRef.current !== client || !credentialsRef.current) return;
        presenceStateRef.current = INITIAL_PRESENCE_STATE;
        await recoverRemoteState();
      } catch (nextError) {
        if (clientRef.current === client) recordError(nextError);
      }
    },
    [clientRef, credentialsRef, recordError, recoverRemoteState],
  );

  useEffect(() => {
    let previous: AppStateStatus = AppState.currentState;
    const subscription = AppState.addEventListener("change", next => {
      const returnedToForeground = next === "active" && previous !== "active";
      const enteredBackground = next === "background" && previous !== "background";
      previous = next;
      if (enteredBackground) clientRef.current?.pauseForBackground();
      if (returnedToForeground) void recoverLifecycle("foreground");
    });
    return () => subscription.remove();
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
        console.warn("[remote] foreground network refresh failed", { error: nextError });
        return networkAvailableRef.current !== false;
      }
    };
    void Network.getNetworkStateAsync()
      .then(state => {
        if (!eventSeen) observe(state);
      })
      .catch(() => undefined);
    const subscription = Network.addNetworkStateListener(state => {
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
      setPhase("claiming");
      setError(null);
      try {
        await connect(await claimPairingCode(code));
      } catch (nextError) {
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
        setCredentials(null);
        setPhase("unpaired");
        setError(null);
      } else setError(message);
    }
  }, [connect, credentials, credentialsRef]);

  const unpair = useCallback(async () => {
    const current = credentials;
    credentialsRef.current = null;
    connectionReadyRef.current = false;
    const remoteUnpair = clientRef.current?.request({ type: "unpair" }).catch(() => undefined);
    if (remoteUnpair) {
      await Promise.race([remoteUnpair, new Promise<void>(resolve => setTimeout(resolve, 750))]);
    }
    await clientRef.current?.close();
    clientRef.current = null;
    resetTimeline();
    if (current) {
      try {
        await serverRevoke(current);
      } catch {
        await savePendingRevoke({
          pairId: current.pairId,
          deviceId: current.deviceId,
          seed: current.seed,
          refreshToken: current.refreshToken,
          tokenUrl: current.tokenUrl,
        });
      }
    }
    await clearCredentials();
    const pendingPrompt = await loadPendingPrompt();
    if (pendingPrompt) await clearPendingPrompt(pendingPrompt.commandId);
    setCredentials(null);
    setPresence(null);
    resetCatalog();
    resetConversation();
    setPhase("unpaired");
    setError(null);
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
