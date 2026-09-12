import { CatalogVersionGate } from "./catalogVersion";
import type {
  SnapshotVersion,
  ModelsData,
  RemoteModel,
  RemoteSession,
  RemoteWorkspace,
  SessionsData,
  WorkspacesData,
} from "./types";
import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import type { RemoteClient } from "./client";
import { detectFinished, sortPinnedFirst } from "./sessionStatus";

export type CatalogSyncState = Record<
  "sessions" | "workspaces" | "models" | "settings",
  "idle" | "syncing" | "ready" | "failed"
>;
const INITIAL_SYNC: CatalogSyncState = {
  sessions: "idle",
  workspaces: "idle",
  models: "idle",
  settings: "idle",
};
const MODEL_RECOVERY_DELAYS_MS = [1_000, 5_000, 15_000, 30_000] as const;

/**
 * The desktop's control-plane catalogue — sessions, workspaces, the model
 * list, approval settings — and the unread/rename bookkeeping that rides on
 * top of it. Isolated from connection lifecycle and the per-session timeline
 * so this is the only place that owns `sessions`-shaped state.
 *
 * `applySessionSnapshot` is the single commit point for a sessions list (from
 * either an explicit `list_sessions` request or the pushed NATS snapshot), so
 * title overrides, unread detection and pinned ordering stay consistent.
 */
export function useSessionCatalog(
  clientRef: MutableRefObject<RemoteClient | null>,
  selectedRef: MutableRefObject<string>,
) {
  const [catalogSync, setCatalogSync] = useState<CatalogSyncState>(INITIAL_SYNC);
  const markSync = useCallback(
    (domain: keyof CatalogSyncState, state: CatalogSyncState[keyof CatalogSyncState]) =>
      setCatalogSync((current) =>
        current[domain] === state ? current : { ...current, [domain]: state },
      ),
    [],
  );
  const [sessions, setSessions] = useState<RemoteSession[]>([]);
  const [unreadSessions, setUnreadSessions] = useState<Set<string>>(() => new Set());
  const [workspaces, setWorkspaces] = useState<RemoteWorkspace[]>([]);
  const [models, setModels] = useState<RemoteModel[]>([]);
  const [approvalTier, setApprovalTier] = useState("off");
  const [sandboxAvailable, setSandboxAvailable] = useState(false);
  const [titleOverrides, setTitleOverrides] = useState<Record<string, string>>({});
  const versionGate = useRef(new CatalogVersionGate());
  const authenticatedEpoch = useRef<string | undefined>(undefined);
  const catalogEpoch = useRef(0);
  const revisions = useRef({ sessions: 0, workspaces: 0, settings: 0 });
  const lastStatusRef = useRef<Record<string, string | undefined>>({});
  const titleOverridesRef = useRef<Record<string, string>>({});
  const modelsRef = useRef<RemoteModel[]>([]);
  const sessionsRef = useRef<RemoteSession[]>([]);
  const sessionRefreshRef = useRef<{
    client: RemoteClient; epoch: number; dirty: boolean; promise: Promise<void>;
  } | null>(null);
  const modelRecoveryRef = useRef<{
    generation: number;
    timer: ReturnType<typeof setTimeout> | null;
  }>({ generation: 0, timer: null });

  const setCatalogEpoch = useCallback((epoch: string | undefined) => {
    if (authenticatedEpoch.current !== epoch) {
      authenticatedEpoch.current = epoch;
      catalogEpoch.current += 1;
      modelRecoveryRef.current.generation += 1;
      setCatalogSync(INITIAL_SYNC);
    }
    versionGate.current.authenticate(epoch);
  }, []);

  useEffect(() => {
    titleOverridesRef.current = titleOverrides;
  }, [titleOverrides]);

  useEffect(() => {
    modelsRef.current = models;
  }, [models]);

  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);

  useEffect(
    () => () => {
      catalogEpoch.current += 1;
      modelRecoveryRef.current.generation += 1;
      if (modelRecoveryRef.current.timer) clearTimeout(modelRecoveryRef.current.timer);
      modelRecoveryRef.current.timer = null;
    },
    [],
  );

  const applySessionSnapshot = useCallback(
    (list: RemoteSession[], version?: SnapshotVersion) => {
      if (!versionGate.current.accept("sessions", version)) return false;
      markSync("sessions", "ready");
      revisions.current.sessions += 1;
      const overrides = titleOverridesRef.current;
      const decorated = list.map((session) => ({
        ...session,
        title: overrides[session.sessionId] ?? session.title,
      }));
      const { finished, next } = detectFinished(
        lastStatusRef.current,
        decorated,
        selectedRef.current,
      );
      lastStatusRef.current = next;
      setSessions(decorated);
      if (finished.length > 0) {
        setUnreadSessions((prev) => {
          const nextUnread = new Set(prev);
          for (const id of finished) nextUnread.add(id);
          return nextUnread;
        });
      }
      return true;
    },
    [selectedRef, markSync],
  );

  const readSessions = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    const epoch = catalogEpoch.current;
    const revision = ++revisions.current.sessions;
    markSync("sessions", "syncing");
    try {
      const response = await client.requestRetry<SessionsData>({ type: "list_sessions" }, "list");
      if (
        clientRef.current !== client ||
        catalogEpoch.current !== epoch ||
        (!response.data.version && revisions.current.sessions !== revision)
      )
        return;
      applySessionSnapshot(response.data.sessions ?? [], response.data.version);
      markSync("sessions", "ready");
    } catch {
      if (
        clientRef.current === client &&
        catalogEpoch.current === epoch &&
        revisions.current.sessions === revision
      )
        markSync("sessions", "failed");
      // If the connection has gone (refresh/reconnect cycle), swallow
      // the error — the reconnect handler will re-fetch.
    }
  }, [applySessionSnapshot, clientRef, markSync]);

  // Coalesce terminal/approval bursts without losing a change that arrived
  // after the in-flight server snapshot: one trailing read refreshes it.
  const refreshSessions = useCallback((): Promise<void> => {
    const client = clientRef.current;
    if (!client) return Promise.resolve();
    const epoch = catalogEpoch.current;
    const pending = sessionRefreshRef.current;
    if (pending?.client === client && pending.epoch === epoch) {
      pending.dirty = true;
      return pending.promise;
    }
    const flight = { client, epoch, dirty: false, promise: Promise.resolve() };
    sessionRefreshRef.current = flight;
    flight.promise = (async () => {
      do {
        flight.dirty = false;
        await readSessions();
      } while (flight.dirty && clientRef.current === client && catalogEpoch.current === epoch);
    })().finally(() => {
      if (sessionRefreshRef.current === flight) sessionRefreshRef.current = null;
    });
    return flight.promise;
  }, [clientRef, readSessions]);

  const refreshModels = useCallback(async () => {
    markSync("models", "syncing");
    const recovery = modelRecoveryRef.current;
    recovery.generation += 1;
    const generation = recovery.generation;
    if (recovery.timer) clearTimeout(recovery.timer);
    recovery.timer = null;

    // Resolve the immediate probe before returning, then continue a bounded
    // background recovery while the desktop Agent warms. A later refresh or
    // reconnect invalidates this generation, so stale timers cannot overwrite a
    // newer catalogue.
    const run = async (attempt: number): Promise<void> => {
      if (modelRecoveryRef.current.generation !== generation) return;
      const client = clientRef.current;
      if (!client) return;
      let list: RemoteModel[] | null = null;
      try {
        list =
          (await client.requestRetry<ModelsData>({ type: "list_models" }, "list")).data.models ??
          [];
      } catch {
        // A connection or Agent warm-up failure shares the same bounded retry.
      }
      if (modelRecoveryRef.current.generation !== generation || clientRef.current !== client)
        return;
      if (list && list.length > 0) {
        setModels(list);
        markSync("models", "ready");
        return;
      }
      const delay = MODEL_RECOVERY_DELAYS_MS[attempt];
      if (delay === undefined) {
        // Preserve a previously usable catalogue through a transient outage.
        if (modelsRef.current.length === 0) setModels([]);
        markSync("models", "failed");
        return;
      }
      modelRecoveryRef.current.timer = setTimeout(() => {
        modelRecoveryRef.current.timer = null;
        void run(attempt + 1);
      }, delay);
    };
    await run(0);
  }, [clientRef, markSync]);

  const refreshSettings = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    const epoch = catalogEpoch.current;
    const revision = ++revisions.current.settings;
    markSync("settings", "syncing");
    try {
      const data = await client.requestRetry<{
        approvalTier: string;
        sandboxAvailable: boolean;
      }>({ type: "get_settings" }, "list");
      if (
        clientRef.current !== client ||
        catalogEpoch.current !== epoch ||
        revisions.current.settings !== revision
      )
        return;
      markSync("settings", "ready");
      setApprovalTier(data.data.approvalTier);
      setSandboxAvailable(data.data.sandboxAvailable);
    } catch {
      if (
        clientRef.current === client &&
        catalogEpoch.current === epoch &&
        revisions.current.settings === revision
      )
        markSync("settings", "failed");
      // Keep the previous tier on a failed read.
    }
  }, [clientRef, markSync]);

  const refreshWorkspaces = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    const epoch = catalogEpoch.current;
    const revision = ++revisions.current.workspaces;
    markSync("workspaces", "syncing");
    try {
      const response = await client.requestRetry<WorkspacesData>(
        { type: "list_workspaces" },
        "list",
      );
      if (
        clientRef.current !== client ||
        catalogEpoch.current !== epoch ||
        (!response.data.version && revisions.current.workspaces !== revision)
      )
        return;
      if (versionGate.current.accept("workspaces", response.data.version))
        setWorkspaces(response.data.workspaces ?? []);
      markSync("workspaces", "ready");
    } catch {
      if (
        clientRef.current === client &&
        catalogEpoch.current === epoch &&
        revisions.current.workspaces === revision
      )
        markSync("workspaces", "failed");
      // Keep the last snapshot. The desktop also pushes a 20-second baseline,
      // so a transient read failure must not flash the catalogue empty.
    }
  }, [clientRef, markSync]);

  /** Drop catalogue state (unpair / credentials cleared). */
  const applyWorkspaces = useCallback(
    (list: RemoteWorkspace[], version?: SnapshotVersion) => {
      if (!versionGate.current.accept("workspaces", version)) return;
      markSync("workspaces", "ready");
      revisions.current.workspaces += 1;
      setWorkspaces(list);
    },
    [markSync],
  );

  const reset = useCallback(() => {
    setCatalogSync(INITIAL_SYNC);
    authenticatedEpoch.current = undefined;
    versionGate.current = new CatalogVersionGate();
    catalogEpoch.current += 1;
    setUnreadSessions(new Set());
    setApprovalTier("off");
    setSandboxAvailable(false);
    modelRecoveryRef.current.generation += 1;
    if (modelRecoveryRef.current.timer) clearTimeout(modelRecoveryRef.current.timer);
    modelRecoveryRef.current.timer = null;
    setSessions([]);
    setWorkspaces([]);
    setModels([]);
    setTitleOverrides({});
    titleOverridesRef.current = {};
    // Clear the status baseline too — a stale running→completed comparison
    // after an unpair/re-pair would otherwise mark old sessions unread.
    lastStatusRef.current = {};
  }, []);

  const rename = useCallback(
    async (sessionId: string, name: string) => {
      const client = clientRef.current;
      if (!client || !sessionId || !name.trim()) return;
      const epoch = catalogEpoch.current;
      const trimmed = name.trim();
      await client.request({ type: "set_session_name", sessionId, name: trimmed }, sessionId);
      if (clientRef.current !== client || catalogEpoch.current !== epoch) return;
      revisions.current.sessions += 1;
      setTitleOverrides((prev) => ({ ...prev, [sessionId]: trimmed }));
      setSessions((current) =>
        current.map((session) =>
          session.sessionId === sessionId ? { ...session, title: trimmed } : session,
        ),
      );
    },
    [clientRef, setSessions, setTitleOverrides],
  );

  /**
   * Delete a session on the desktop and drop it locally. Returns true when the
   * deleted session was the one currently selected, so the caller can close the
   * conversation (a navigation concern the catalogue doesn't own).
   */
  const deleteSession = useCallback(
    async (sessionId: string, threadId: string): Promise<boolean> => {
      const client = clientRef.current;
      if (!client || !sessionId || !threadId) throw new Error("Session unavailable");
      const epoch = catalogEpoch.current;
      await client.request({ type: "delete_session", sessionId, threadId }, sessionId);
      if (clientRef.current !== client || catalogEpoch.current !== epoch) return false;
      revisions.current.sessions += 1;
      setSessions((current) => current.filter((session) => session.sessionId !== sessionId));
      return selectedRef.current === sessionId;
    },
    [clientRef, selectedRef, setSessions],
  );

  /**
   * Delete a whole workspace on the desktop. The desktop cascade removes the
   * workspace's threads too, so the local catalogue drops both. Returns true
   * when the selected session lived in that workspace, so the caller can close
   * the conversation it was showing.
   */
  const deleteWorkspace = useCallback(
    async (workspaceId: string): Promise<boolean> => {
      const client = clientRef.current;
      if (!client || !workspaceId) throw new Error("Workspace unavailable");
      const epoch = catalogEpoch.current;
      const removed = sessionsRef.current.filter(
        (session) => (session.workspaceId ?? "") === workspaceId,
      );
      await client.request({ type: "delete_workspace", workspaceId }, "list");
      if (clientRef.current !== client || catalogEpoch.current !== epoch) return false;
      revisions.current.sessions += 1;
      revisions.current.workspaces += 1;
      setWorkspaces((current) => current.filter((workspace) => workspace.id !== workspaceId));
      setSessions((current) =>
        current.filter((session) => (session.workspaceId ?? "") !== workspaceId),
      );
      if (removed.length > 0) {
        const removedIds = new Set(removed.map((session) => session.sessionId));
        // Unread markers and title overrides for gone sessions would otherwise
        // leak into a re-used id or linger in memory for the session's life.
        setUnreadSessions((current) => {
          if (![...removedIds].some((id) => current.has(id))) return current;
          const next = new Set(current);
          for (const id of removedIds) next.delete(id);
          return next;
        });
        setTitleOverrides((current) => {
          if (![...removedIds].some((id) => id in current)) return current;
          const next = { ...current };
          for (const id of removedIds) delete next[id];
          return next;
        });
      }
      return removed.some((session) => session.sessionId === selectedRef.current);
    },
    [clientRef, selectedRef, setSessions, setTitleOverrides, setUnreadSessions, setWorkspaces],
  );

  const setSessionPinned = useCallback(
    async (sessionId: string, threadId: string, pinned: boolean) => {
      const client = clientRef.current;
      if (!client || !sessionId || !threadId) return;
      const epoch = catalogEpoch.current;
      await client.request({ type: "set_session_pinned", sessionId, threadId, pinned }, sessionId);
      if (clientRef.current !== client || catalogEpoch.current !== epoch) return;
      revisions.current.sessions += 1;
      // Optimistic local reorder: pinned sessions stay on top, everything else
      // keeps the desktop's recency order. The pushed snapshot converges on the
      // same layout (the desktop sorts by `pinned DESC, last_message_at DESC`).
      setSessions((current) =>
        sortPinnedFirst(
          current.map((session) =>
            session.sessionId === sessionId ? { ...session, pinned } : session,
          ),
        ),
      );
    },
    [clientRef, setSessions],
  );

  return {
    catalogSync,
    setCatalogEpoch,
    sessions,
    unreadSessions,
    setUnreadSessions,
    workspaces,
    setWorkspaces: applyWorkspaces,
    models,
    approvalTier,
    setApprovalTier,
    sandboxAvailable,
    titleOverrides,
    setTitleOverrides,
    applySessionSnapshot,
    refreshSessions,
    refreshModels,
    refreshSettings,
    refreshWorkspaces,
    rename,
    deleteSession,
    deleteWorkspace,
    setSessionPinned,
    reset,
  };
}
