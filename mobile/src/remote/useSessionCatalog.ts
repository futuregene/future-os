import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import type { RemoteClient } from "./client";
import { detectFinished, sortPinnedFirst } from "./sessionStatus";
import type {
  ModelsData,
  RemoteModel,
  RemoteSession,
  RemoteWorkspace,
  SessionsData,
  WorkspacesData,
} from "./types";

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
  const [sessions, setSessions] = useState<RemoteSession[]>([]);
  const [unreadSessions, setUnreadSessions] = useState<Set<string>>(() => new Set());
  const [workspaces, setWorkspaces] = useState<RemoteWorkspace[]>([]);
  const [models, setModels] = useState<RemoteModel[]>([]);
  const [approvalTier, setApprovalTier] = useState("off");
  const [sandboxAvailable, setSandboxAvailable] = useState(false);
  const [titleOverrides, setTitleOverrides] = useState<Record<string, string>>({});
  const lastStatusRef = useRef<Record<string, string | undefined>>({});
  const titleOverridesRef = useRef<Record<string, string>>({});
  const modelsRef = useRef<RemoteModel[]>([]);
  const modelRecoveryRef = useRef<{
    generation: number;
    timer: ReturnType<typeof setTimeout> | null;
  }>({ generation: 0, timer: null });

  useEffect(() => {
    titleOverridesRef.current = titleOverrides;
  }, [titleOverrides]);

  useEffect(() => {
    modelsRef.current = models;
  }, [models]);

  useEffect(
    () => () => {
      modelRecoveryRef.current.generation += 1;
      if (modelRecoveryRef.current.timer) clearTimeout(modelRecoveryRef.current.timer);
      modelRecoveryRef.current.timer = null;
    },
    [],
  );

  const applySessionSnapshot = useCallback(
    (list: RemoteSession[]) => {
      const overrides = titleOverridesRef.current;
      const decorated = list.map(session => ({
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
        setUnreadSessions(prev => {
          const nextUnread = new Set(prev);
          for (const id of finished) nextUnread.add(id);
          return nextUnread;
        });
      }
    },
    [selectedRef],
  );

  const refreshSessions = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const response = await client.requestRetry<SessionsData>({ type: "list_sessions" }, "list");
      applySessionSnapshot(response.data.sessions ?? []);
    } catch {
      // If the connection has gone (refresh/reconnect cycle), swallow
      // the error — the reconnect handler will re-fetch.
    }
  }, [applySessionSnapshot, clientRef]);

  const refreshModels = useCallback(async () => {
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
      if (modelRecoveryRef.current.generation !== generation) return;
      if (list && list.length > 0) {
        setModels(list);
        return;
      }
      const delay = MODEL_RECOVERY_DELAYS_MS[attempt];
      if (delay === undefined) {
        // Preserve a previously usable catalogue through a transient outage.
        if (modelsRef.current.length === 0) setModels([]);
        return;
      }
      modelRecoveryRef.current.timer = setTimeout(() => {
        modelRecoveryRef.current.timer = null;
        void run(attempt + 1);
      }, delay);
    };
    await run(0);
  }, [clientRef]);

  const refreshSettings = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const data = await client.requestRetry<{ approvalTier: string; sandboxAvailable: boolean }>(
        { type: "get_settings" },
        "list",
      );
      setApprovalTier(data.data.approvalTier);
      setSandboxAvailable(data.data.sandboxAvailable);
    } catch {
      // Keep the previous tier on a failed read.
    }
  }, [clientRef]);

  const refreshWorkspaces = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const response = await client.requestRetry<WorkspacesData>(
        { type: "list_workspaces" },
        "list",
      );
      setWorkspaces(response.data.workspaces ?? []);
    } catch {
      // Keep the last snapshot. The desktop also pushes a 20-second baseline,
      // so a transient read failure must not flash the catalogue empty.
    }
  }, [clientRef]);

  /** Drop catalogue state (unpair / credentials cleared). */
  const reset = useCallback(() => {
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
      const trimmed = name.trim();
      await client.request({ type: "set_session_name", sessionId, name: trimmed }, sessionId);
      setTitleOverrides(prev => ({ ...prev, [sessionId]: trimmed }));
      setSessions(current =>
        current.map(session =>
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
      if (!client || !sessionId || !threadId) return false;
      await client.request({ type: "delete_session", sessionId, threadId }, sessionId);
      setSessions(current => current.filter(session => session.sessionId !== sessionId));
      return selectedRef.current === sessionId;
    },
    [clientRef, selectedRef, setSessions],
  );

  const setSessionPinned = useCallback(
    async (sessionId: string, threadId: string, pinned: boolean) => {
      const client = clientRef.current;
      if (!client || !sessionId || !threadId) return;
      await client.request({ type: "set_session_pinned", sessionId, threadId, pinned }, sessionId);
      // Optimistic local reorder: pinned sessions stay on top, everything else
      // keeps the desktop's recency order. The pushed snapshot converges on the
      // same layout (the desktop sorts by `pinned DESC, last_message_at DESC`).
      setSessions(current =>
        sortPinnedFirst(
          current.map(session =>
            session.sessionId === sessionId ? { ...session, pinned } : session,
          ),
        ),
      );
    },
    [clientRef, setSessions],
  );

  return {
    sessions,
    unreadSessions,
    setUnreadSessions,
    workspaces,
    setWorkspaces,
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
    setSessionPinned,
    reset,
  };
}
