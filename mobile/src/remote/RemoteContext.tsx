import type { PropsWithChildren } from "react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  appendUserMessage,
  applyStreamEvent,
  emptyTimeline,
  markApprovalDecision,
  timelineFromEntries,
  timelineFromHistory,
  type TimelineState,
} from "./eventReducer";
import { RemoteClient } from "./client";
import { claimPairingCode, ensureFreshCredentials, revokeCredentials } from "./pairing";
import { isDesktopOnline } from "./presence";
import {
  advanceCursor,
  newCursor,
  nextEvent,
  rebuildCursorFromEvents,
  type RunCursor,
} from "./runCursor";
import {
  clearCredentials,
  loadCredentials,
  loadLastModel,
  loadLastThinking,
  saveCredentials,
  saveLastModel,
  saveLastThinking,
} from "./storage";
import { modelProviderFromReference, modelReference } from "./types";
import type {
  ConnectionPhase,
  HistoryEntry,
  HistoryMessage,
  Presence,
  RemoteCredentials,
  RemoteModel,
  RemoteSession,
  RemoteSessionState,
  RemoteWorkspace,
  StreamEvent,
  ThinkingLevel,
} from "./types";

interface SessionsData {
  sessions: RemoteSession[];
}

interface ModelsData {
  models: RemoteModel[];
}

interface WorkspacesData {
  workspaces: RemoteWorkspace[];
}

interface HistoryData {
  messages: HistoryMessage[];
  total?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

interface EntriesData {
  entries: HistoryEntry[];
  total?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

interface EventsData {
  events: StreamEvent[];
  truncated?: boolean;
}

interface PromptAck {
  sessionId: string;
  threadId: string;
  runId: string;
}

interface RemoteContextValue {
  phase: ConnectionPhase;
  error: string | null;
  credentials: RemoteCredentials | null;
  presence: Presence | null;
  desktopOnline: boolean;
  sessions: RemoteSession[];
  workspaces: RemoteWorkspace[];
  unreadSessions: Set<string>;
  models: RemoteModel[];
  selectedSessionId: string;
  selectedTitle: string;
  draft: boolean;
  timeline: TimelineState;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  busy: boolean;
  pair(code: string): Promise<void>;
  reconnect(): Promise<void>;
  unpair(): Promise<void>;
  refreshSessions(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  selectSession(sessionId: string): Promise<void>;
  newConversation(mode?: "chat" | "workspace", workspaceId?: string): Promise<void>;
  closeConversation(): void;
  sendMessage(text: string): Promise<void>;
  abort(): Promise<void>;
  setModel(modelId: string): Promise<void>;
  setThinkingLevel(level: ThinkingLevel): Promise<void>;
  rename(name: string): Promise<void>;
  decideApproval(id: string, decision: "approved" | "rejected"): Promise<void>;
}

const RemoteContext = createContext<RemoteContextValue | null>(null);

const RUNNING_STATUSES = new Set(["running", "queued", "waiting_approval"]);
const FINISHED_STATUSES = new Set(["completed", "failed"]);

/**
 * Compares each session's status against the previously seen status map and
 * returns the ids whose run just finished (running/queued/waiting_approval →
 * completed/failed), plus the new status map. Pure — the caller owns state.
 */
function detectFinished(
  prevStatus: Record<string, string | undefined>,
  sessions: RemoteSession[],
): { finished: string[]; next: Record<string, string | undefined> } {
  const finished: string[] = [];
  const next: Record<string, string | undefined> = {};
  for (const s of sessions) {
    const before = prevStatus[s.sessionId];
    if (
      before !== undefined &&
      RUNNING_STATUSES.has(before) &&
      s.status &&
      FINISHED_STATUSES.has(s.status)
    ) {
      finished.push(s.sessionId);
    }
    next[s.sessionId] = s.status;
  }
  return { finished, next };
}

export function RemoteProvider({ children }: PropsWithChildren) {
  const [phase, setPhase] = useState<ConnectionPhase>("booting");
  const [error, setError] = useState<string | null>(null);
  const [credentials, setCredentials] = useState<RemoteCredentials | null>(null);
  const [presence, setPresence] = useState<Presence | null>(null);
  const [sessions, setSessions] = useState<RemoteSession[]>([]);
  const [unreadSessions, setUnreadSessions] = useState<Set<string>>(() => new Set());
  const lastStatusRef = useRef<Record<string, string | undefined>>({});
  const [workspaces, setWorkspaces] = useState<RemoteWorkspace[]>([]);
  const [models, setModels] = useState<RemoteModel[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [draft, setDraft] = useState(false);
  const [draftMode, setDraftMode] = useState<"chat" | "workspace">("chat");
  const [draftWorkspaceId, setDraftWorkspaceId] = useState("");
  const [timeline, setTimeline] = useState<TimelineState>(emptyTimeline);
  const [modelId, setModelId] = useState("");
  const [thinkingLevel, setThinkingLevelState] = useState<ThinkingLevel>("off");
  const [busy, setBusy] = useState(false);
  const [clock, setClock] = useState(Date.now());
  const clientRef = useRef<RemoteClient | null>(null);
  const credentialsRef = useRef<RemoteCredentials | null>(null);
  const selectedRef = useRef("");
  const presenceRef = useRef<Presence | null>(null);
  const timelineRef = useRef<TimelineState>(emptyTimeline());
  const recoverRef = useRef<() => Promise<void>>(async () => undefined);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectAttemptRef = useRef(0);
  const scheduleReconnectRef = useRef<() => void>(() => undefined);
  // Integrity: sync lock + pending buffer + per-run cursor for gap detection.
  const syncLockRef = useRef(false);
  const pendingRef = useRef<{ event: StreamEvent; sessionId: string }[]>([]);
  const cursorRef = useRef<RunCursor>(newCursor());
  const gapInFlightRef = useRef(false);

  useEffect(() => {
    credentialsRef.current = credentials;
  }, [credentials]);
  useEffect(() => {
    selectedRef.current = selectedSessionId;
  }, [selectedSessionId]);
  useEffect(() => {
    presenceRef.current = presence;
  }, [presence]);
  useEffect(() => {
    timelineRef.current = timeline;
  }, [timeline]);

  const refreshSessions = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const response = await client.request<SessionsData>({ type: "list_sessions" }, "list");
      const list = response.data.sessions ?? [];
      const { finished, next } = detectFinished(lastStatusRef.current, list);
      lastStatusRef.current = next;
      setSessions(list);
      if (finished.length > 0) {
        setUnreadSessions(prev => {
          const nextUnread = new Set(prev);
          for (const id of finished) nextUnread.add(id);
          return nextUnread;
        });
      }
    } catch {
      // If the connection has gone (refresh/reconnect cycle), swallow
      // the error — the reconnect handler will re-fetch.
    }
  }, []);

  const refreshModels = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const response = await client.request<ModelsData>({ type: "list_models" }, "list");
      setModels(response.data.models ?? []);
    } catch {
      setModels([]);
    }
  }, []);

  const refreshWorkspaces = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const response = await client.request<WorkspacesData>({ type: "list_workspaces" }, "list");
      setWorkspaces(response.data.workspaces ?? []);
    } catch {
      setWorkspaces([]);
    }
  }, []);

  const handleEvent = useCallback(
    (event: StreamEvent, sessionId: string) => {
      if (!sessionId || sessionId !== selectedRef.current) return;
      // While a recovery/resync is in progress, buffer live events.
      if (syncLockRef.current) {
        pendingRef.current.push({ event, sessionId });
        return;
      }
      const verdict = nextEvent(cursorRef.current, event.runId, event.idx);
      if (verdict.kind === "dup") return;
      if (verdict.kind === "gap") {
        // Buffer the gap-triggering event; do NOT apply out of order.
        pendingRef.current.push({ event, sessionId });
        syncLockRef.current = true;
        void fillGapRef.current(sessionId, event.runId ?? "", verdict.fromIdx);
        return;
      }
      // "apply" or "untracked"
      if (verdict.kind === "apply") {
        advanceCursor(cursorRef.current, event.runId!, verdict.idx);
      }
      setTimeline(state => applyStreamEvent(state, event));
      if (event.type === "agent_end") {
        void refreshSessions();
      }
    },
    [refreshSessions],
  );

  const closeConversation = useCallback(() => {
    setSelectedSessionId("");
    selectedRef.current = "";
    setDraft(false);
    setDraftMode("chat");
    setDraftWorkspaceId("");
    setTimeline(emptyTimeline());
    void refreshSessions();
    void refreshWorkspaces();
  }, [refreshSessions, refreshWorkspaces]);

  const connect = useCallback(
    async (nextCredentials: RemoteCredentials) => {
      await clientRef.current?.close();
      const fresh = await ensureFreshCredentials(nextCredentials);
      credentialsRef.current = fresh;
      setCredentials(fresh);
      setError(null);
      setPhase("connecting");
      const client = new RemoteClient(fresh, {
        onCredentials: next => {
          setCredentials(next);
          void saveCredentials(next);
        },
        onEvent: handleEvent,
        onPresence: nextPresence => {
          setPresence(nextPresence);
        },
        onSessions: sessionList => {
          const list: RemoteSession[] = sessionList.map(s => ({
            sessionId: s.sessionId,
            threadId: s.threadId,
            title: s.title,
            mode: s.mode,
            workspaceId: s.workspaceId,
            streaming: s.streaming,
            status: s.status,
          }));
          const { finished, next } = detectFinished(lastStatusRef.current, list);
          lastStatusRef.current = next;
          setSessions(list);
          if (finished.length > 0) {
            setUnreadSessions(prev => {
              const nextUnread = new Set(prev);
              for (const id of finished) nextUnread.add(id);
              return nextUnread;
            });
          }
          const currentId = selectedRef.current;
          if (currentId && !list.some(item => item.sessionId === currentId)) {
            closeConversation();
          } else if (currentId) {
            const streaming =
              sessionList.find(session => session.sessionId === currentId)?.streaming ?? false;
            setTimeline(state => ({ ...state, streaming }));
          }
        },
        onWorkspaces: workspaceList => {
          setWorkspaces(workspaceList);
        },
        onConnectionState: state => {
          if (state === "connected") {
            if (reconnectTimerRef.current) clearTimeout(reconnectTimerRef.current);
            reconnectTimerRef.current = null;
            reconnectAttemptRef.current = 0;
            setPhase("connected");
          }
          if (state === "reconnecting" || state === "disconnected") {
            setPhase("reconnecting");
            scheduleReconnectRef.current();
          }
        },
        onReconnected: () => {
          void recoverRef.current();
        },
        onError: nextError => {
          setError(nextError.message);
          setPhase("reconnecting");
          scheduleReconnectRef.current();
        },
      });
      clientRef.current = client;
      await client.open();
      await Promise.all([refreshModels(), refreshSessions(), refreshWorkspaces()]);
    },
    [closeConversation, handleEvent, refreshModels, refreshSessions, refreshWorkspaces],
  );

  useEffect(() => {
    scheduleReconnectRef.current = () => {
      if (reconnectTimerRef.current || !credentialsRef.current) return;
      const delay = Math.min(30_000, 1_000 * 2 ** reconnectAttemptRef.current);
      reconnectTimerRef.current = setTimeout(() => {
        reconnectTimerRef.current = null;
        const stored = credentialsRef.current;
        if (!stored) return;
        void (async () => {
          try {
            await connect(stored);
          } catch (nextError) {
            setError(nextError instanceof Error ? nextError.message : String(nextError));
            reconnectAttemptRef.current += 1;
            scheduleReconnectRef.current();
          }
        })();
      }, delay);
    };
    return () => {
      scheduleReconnectRef.current = () => undefined;
      if (reconnectTimerRef.current) clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    };
  }, [connect]);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
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
        setPhase("reconnecting");
        scheduleReconnectRef.current();
      }
    })();
    return () => {
      active = false;
      void clientRef.current?.close();
    };
  }, [connect]);

  useEffect(() => {
    const timer = setInterval(() => setClock(Date.now()), 10_000);
    return () => clearInterval(timer);
  }, []);

  const pair = useCallback(
    async (code: string) => {
      setBusy(true);
      setPhase("claiming");
      setError(null);
      try {
        const next = await claimPairingCode(code);
        await connect(next);
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
        setPhase("unpaired");
        throw nextError;
      } finally {
        setBusy(false);
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
    setBusy(true);
    try {
      await connect(stored);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
      setPhase("error");
    } finally {
      setBusy(false);
    }
  }, [connect, credentials]);

  const unpair = useCallback(async () => {
    const current = credentials;
    setBusy(true);
    try {
      if (reconnectTimerRef.current) clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
      reconnectAttemptRef.current = 0;
      credentialsRef.current = null;
      await clientRef.current?.close();
      clientRef.current = null;
      if (current) await revokeCredentials(current);
      else await clearCredentials();
      setCredentials(null);
      setPresence(null);
      setSessions([]);
      setWorkspaces([]);
      setSelectedSessionId("");
      setDraft(false);
      setTimeline(emptyTimeline());
      setPhase("unpaired");
      setError(null);
    } finally {
      setBusy(false);
    }
  }, [credentials]);

  const loadHistory = useCallback(async (sessionId: string): Promise<TimelineState> => {
    const client = clientRef.current;
    if (!client) return emptyTimeline();
    // Prefer display entries — they carry user attachments (message-shaped
    // history doesn't). Older desktops that don't know get_session_entries
    // reply "Unsupported command" and fall through to the fallback below.
    try {
      const entries: HistoryEntry[] = [];
      let offset = 0;
      for (;;) {
        const response = await client.request<EntriesData>(
          { type: "get_session_entries", sessionId, offset },
          sessionId,
        );
        entries.push(...(response.data.entries ?? []));
        if (!response.data.hasMore) break;
        const next = response.data.nextOffset;
        if (typeof next !== "number" || next <= offset) break;
        offset = next;
      }
      return timelineFromEntries(entries);
    } catch {
      // Fall through to message-shaped history.
    }
    const history: HistoryMessage[] = [];
    let offset = 0;
    for (;;) {
      const response = await client.request<HistoryData>(
        { type: "get_messages", sessionId, offset },
        sessionId,
      );
      history.push(...(response.data.messages ?? []));
      if (!response.data.hasMore) break;
      const next = response.data.nextOffset;
      if (typeof next !== "number" || next <= offset) break;
      offset = next;
    }
    return timelineFromHistory(history);
  }, []);

  const backfill = useCallback(
    async (sessionId: string, initial: TimelineState, runId?: string, sinceIdx?: number) => {
      const client = clientRef.current;
      if (!client) return initial;
      try {
        const response = await client.request<EventsData>(
          { type: "get_events_since", sessionId, runId, sinceIdx: sinceIdx ?? -1 },
          sessionId,
        );
        let next = initial;
        for (const event of response.data.events ?? []) next = applyStreamEvent(next, event);
        if (response.data.truncated) {
          next = {
            ...next,
            items: [
              ...next.items,
              {
                id: `truncated:${Date.now()}`,
                kind: "notice",
                tone: "warning",
                text: "truncated",
              },
            ],
          };
        }
        return next;
      } catch {
        return initial;
      }
    },
    [],
  );

  // ── Gap-fill and full-resync (integrity layer) ──

  const fullResync = useCallback(
    async (sessionId: string) => {
      let next = await loadHistory(sessionId);
      const streaming =
        presenceRef.current?.sessions?.find(s => s.sessionId === sessionId)?.streaming ?? false;
      if (streaming) next = await backfill(sessionId, next);
      next = { ...next, streaming };
      // Rebuild cursor from the resynced timeline events.
      const cursor = cursorRef.current;
      for (const item of next.items) {
        if (
          item.runId &&
          "idx" in item &&
          typeof (item as Record<string, unknown>).idx === "number"
        ) {
          advanceCursor(cursor, item.runId, (item as unknown as { idx: number }).idx);
        }
      }
      setTimeline(next);
    },
    [backfill, loadHistory],
  );

  const fillGapRef = useRef<(sessionId: string, runId: string, fromIdx: number) => Promise<void>>(
    async () => undefined,
  );

  useEffect(() => {
    fillGapRef.current = async (sessionId: string, runId: string, fromIdx: number) => {
      if (gapInFlightRef.current) return;
      gapInFlightRef.current = true;
      try {
        const client = clientRef.current;
        if (!client) throw new Error("no client");
        const response = await client.request<EventsData>(
          { type: "get_events_since", sessionId, runId, sinceIdx: fromIdx },
          sessionId,
        );
        const events = response.data.events ?? [];
        const firstIdx = events.length > 0 ? events[0]!.idx : undefined;
        // If the agent buffer overflowed (dropped head), firstIdx will be > fromIdx+1.
        if (events.length > 0 && firstIdx != null && firstIdx > fromIdx + 1) {
          await fullResync(sessionId);
          return;
        }
        // Apply fetched events in order (dedup in reducer handles overlaps).
        setTimeline(prev => events.reduce((state, ev) => applyStreamEvent(state, ev), prev));
        // Advance cursor for all fetched events.
        for (const ev of events) {
          if (ev.runId && ev.idx != null) advanceCursor(cursorRef.current, ev.runId, ev.idx);
        }
        // Flush pending buffer (may contain the gap-triggering event + subsequent ones).
        const pending = pendingRef.current;
        pendingRef.current = [];
        if (pending.length > 0) {
          setTimeline(prev => {
            let state = prev;
            for (const { event } of pending) {
              const v = nextEvent(cursorRef.current, event.runId, event.idx);
              if (v.kind === "dup") continue;
              if (v.kind === "gap") {
                // Another gap during flush — retry once, then full resync.
                void (async () => {
                  try {
                    await fullResync(sessionId);
                  } finally {
                    syncLockRef.current = false;
                    gapInFlightRef.current = false;
                  }
                })();
                return state;
              }
              if (v.kind === "apply") advanceCursor(cursorRef.current, event.runId!, v.idx);
              state = applyStreamEvent(state, event);
            }
            return state;
          });
        }
      } catch {
        // Request failed (agent restart, etc.) — degrade to full resync.
        try {
          await fullResync(sessionId);
        } catch {
          // Last resort: nothing more we can do.
        }
      } finally {
        pendingRef.current = [];
        syncLockRef.current = false;
        gapInFlightRef.current = false;
      }
    };
  }, [fullResync]);

  useEffect(() => {
    recoverRef.current = async () => {
      syncLockRef.current = true;
      try {
        await Promise.all([refreshModels(), refreshSessions(), refreshWorkspaces()]);
        const sessionId = selectedRef.current;
        if (!sessionId) return;
        let next = await loadHistory(sessionId);
        // Use fresh presence snapshot for streaming (not stale timelineRef).
        const streaming =
          presenceRef.current?.sessions?.find(s => s.sessionId === sessionId)?.streaming ?? false;
        if (streaming) next = await backfill(sessionId, next);
        next = { ...next, streaming };
        // Rebuild cursor from the recovered state.
        cursorRef.current = newCursor();
        rebuildCursorFromEvents(
          cursorRef.current,
          (next as TimelineState & { items: { runId?: string; idx?: number }[] }).items,
        );
        setTimeline(next);
        // Fold any events buffered during recovery.
        const pending = pendingRef.current;
        pendingRef.current = [];
        if (pending.length > 0) {
          setTimeline(prev =>
            pending.reduce((state, { event }) => applyStreamEvent(state, event), prev),
          );
        }
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        syncLockRef.current = false;
      }
    };
  }, [backfill, loadHistory, refreshModels, refreshSessions, refreshWorkspaces]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      setBusy(true);
      syncLockRef.current = true;
      setSelectedSessionId(sessionId);
      selectedRef.current = sessionId;
      setDraft(false);
      setUnreadSessions(prev => {
        if (!prev.has(sessionId)) return prev;
        const nextUnread = new Set(prev);
        nextUnread.delete(sessionId);
        return nextUnread;
      });
      // Clear the previous conversation up front — it must not stay on screen
      // while the new session's history is in flight.
      setTimeline(emptyTimeline());
      try {
        let next = await loadHistory(sessionId);
        const streaming =
          presenceRef.current?.sessions?.find(session => session.sessionId === sessionId)
            ?.streaming ?? false;
        if (streaming) next = await backfill(sessionId, next);
        next = { ...next, streaming };
        // Reset cursor for the new session.
        cursorRef.current = newCursor();
        rebuildCursorFromEvents(
          cursorRef.current,
          (next as TimelineState & { items: { runId?: string; idx?: number }[] }).items,
        );
        setTimeline(next);
        // Fold any events buffered during the switch.
        const pending = pendingRef.current;
        pendingRef.current = [];
        if (pending.length > 0) {
          setTimeline(prev =>
            pending.reduce((state, { event }) => applyStreamEvent(state, event), prev),
          );
        }
        const response = await client.request<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        const currentModel = response.data.model ?? "";
        const matchingModel = models.find(model => modelReference(model) === currentModel);
        setModelId(matchingModel ? modelReference(matchingModel) : currentModel);
        setThinkingLevelState(response.data.thinkingLevel ?? "off");
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        syncLockRef.current = false;
        setBusy(false);
      }
    },
    [backfill, loadHistory, models],
  );

  const newConversation = useCallback(
    async (mode: "chat" | "workspace" = "chat", workspaceId = "") => {
      const [lastModel, lastThinking] = await Promise.all([loadLastModel(), loadLastThinking()]);
      const defaultOption = models.find(model => model.isDefault);
      const defaultModel =
        (lastModel && models.some(model => modelReference(model) === lastModel)
          ? lastModel
          : null) ??
        (defaultOption ? modelReference(defaultOption) : null) ??
        (models[0] ? modelReference(models[0]) : "");
      setSelectedSessionId("");
      selectedRef.current = "";
      setDraft(true);
      setDraftMode(mode);
      setDraftWorkspaceId(workspaceId);
      setTimeline(emptyTimeline());
      setModelId(defaultModel);
      setThinkingLevelState((lastThinking as ThinkingLevel | null) ?? "off");
    },
    [models],
  );

  const sendMessage = useCallback(
    async (text: string) => {
      const client = clientRef.current;
      if (!client || timeline.streaming || busy || !text.trim()) return;
      const wasDraft = draft;
      const optimisticTimeline = appendUserMessage(timeline, text.trim());
      setBusy(true);
      setTimeline(optimisticTimeline);
      try {
        const response = await client.request<PromptAck>(
          {
            type: "prompt",
            sessionId: selectedRef.current,
            message: text.trim(),
            modelId,
            providerId: modelProviderFromReference(modelId),
            level: thinkingLevel,
            ...(draft && draftMode === "workspace"
              ? { mode: "workspace", workspaceId: draftWorkspaceId }
              : {}),
          },
          selectedRef.current,
        );
        const nextSessionId = response.data.sessionId;
        if (nextSessionId && nextSessionId !== selectedRef.current) {
          selectedRef.current = nextSessionId;
          setSelectedSessionId(nextSessionId);
          setDraft(false);
          setDraftMode("chat");
          setDraftWorkspaceId("");
          await refreshSessions();
        }
        if (wasDraft && nextSessionId) {
          // Catch up on the run's first events: while the draft had no session
          // id, handleEvent dropped them at the session filter. Merge the
          // fetched prefix into the LATEST timeline under the sync lock —
          // rebasing onto the stale pre-prompt snapshot would wipe live events
          // that already landed (the streaming placeholder vanished, then was
          // recreated by the next event with a receipt-time anchor, visibly
          // restarting the footer timer mid-run).
          syncLockRef.current = true;
          try {
            const catchup = await client.request<EventsData>(
              { type: "get_events_since", sessionId: nextSessionId, sinceIdx: -1 },
              nextSessionId,
            );
            const events = catchup.data.events ?? [];
            // Apply-all (seenEvents dedups) rather than cursor verdicts: events
            // predating the cursor's first sighting of this run are still
            // missing and must land, not be skipped as "dup".
            setTimeline(prev => {
              let next = events.reduce((state, ev) => applyStreamEvent(state, ev), prev);
              if (catchup.data.truncated) {
                next = {
                  ...next,
                  items: [
                    ...next.items,
                    {
                      id: `truncated:${Date.now()}`,
                      kind: "notice",
                      tone: "warning",
                      text: "truncated",
                    },
                  ],
                };
              }
              return next;
            });
            for (const ev of events) {
              if (ev.runId && ev.idx != null) advanceCursor(cursorRef.current, ev.runId, ev.idx);
            }
            // Fold live events buffered while the catch-up was in flight.
            const pending = pendingRef.current;
            pendingRef.current = [];
            if (pending.length > 0) {
              setTimeline(prev => {
                let state = prev;
                for (const { event } of pending) {
                  const verdict = nextEvent(cursorRef.current, event.runId, event.idx);
                  if (verdict.kind === "dup") continue;
                  if (verdict.kind === "gap") {
                    // A live event leapfrogged the catch-up — heal wholesale
                    // (recover re-syncs history, backfill, cursor, pending).
                    void recoverRef.current();
                    return state;
                  }
                  state = applyStreamEvent(state, event);
                }
                return state;
              });
            }
          } catch {
            // Desktop without get_events_since (or a dropped connection): the
            // live stream and the next gap check carry on regardless.
          } finally {
            syncLockRef.current = false;
          }
        }
      } finally {
        setBusy(false);
      }
    },
    [busy, draft, draftMode, draftWorkspaceId, modelId, refreshSessions, thinkingLevel, timeline],
  );

  const abort = useCallback(async () => {
    const client = clientRef.current;
    if (!client || !selectedRef.current) return;
    await client.request({ type: "abort", sessionId: selectedRef.current }, selectedRef.current);
  }, []);

  const setModel = useCallback(async (nextModelId: string) => {
    setModelId(nextModelId);
    await saveLastModel(nextModelId);
    const client = clientRef.current;
    if (client && selectedRef.current) {
      await client.request(
        {
          type: "set_model",
          sessionId: selectedRef.current,
          modelId: nextModelId,
          providerId: modelProviderFromReference(nextModelId),
        },
        selectedRef.current,
      );
    }
  }, []);

  const setThinkingLevel = useCallback(async (level: ThinkingLevel) => {
    setThinkingLevelState(level);
    await saveLastThinking(level);
    const client = clientRef.current;
    if (client && selectedRef.current) {
      await client.request(
        { type: "set_thinking_level", sessionId: selectedRef.current, level },
        selectedRef.current,
      );
    }
  }, []);

  const rename = useCallback(async (name: string) => {
    const client = clientRef.current;
    const sessionId = selectedRef.current;
    if (!client || !sessionId || !name.trim()) return;
    await client.request({ type: "set_session_name", sessionId, name: name.trim() }, sessionId);
    setSessions(current =>
      current.map(session =>
        session.sessionId === sessionId ? { ...session, title: name.trim() } : session,
      ),
    );
  }, []);

  const decideApproval = useCallback(async (id: string, decision: "approved" | "rejected") => {
    const client = clientRef.current;
    const sessionId = selectedRef.current;
    if (!client || !sessionId) return;
    await client.request(
      {
        type: "approval_decision",
        sessionId,
        entryId: id,
        mode: decision,
      },
      sessionId,
    );
    setTimeline(state => markApprovalDecision(state, id, decision));
  }, []);

  const desktopOnline = useMemo(
    () => phase === "connected" && isDesktopOnline(presence, clock),
    [clock, phase, presence],
  );
  const selectedTitle =
    sessions.find(session => session.sessionId === selectedSessionId)?.title ?? "";

  const value = useMemo<RemoteContextValue>(
    () => ({
      phase,
      error,
      credentials,
      presence,
      desktopOnline,
      sessions,
      workspaces,
      unreadSessions,
      models,
      selectedSessionId,
      selectedTitle,
      draft,
      timeline,
      modelId,
      thinkingLevel,
      busy,
      pair,
      reconnect,
      unpair,
      refreshSessions,
      refreshWorkspaces,
      selectSession,
      newConversation,
      closeConversation,
      sendMessage,
      abort,
      setModel,
      setThinkingLevel,
      rename,
      decideApproval,
    }),
    [
      abort,
      busy,
      credentials,
      closeConversation,
      decideApproval,
      desktopOnline,
      draft,
      error,
      modelId,
      models,
      newConversation,
      pair,
      phase,
      presence,
      reconnect,
      refreshSessions,
      refreshWorkspaces,
      rename,
      selectSession,
      selectedSessionId,
      selectedTitle,
      sendMessage,
      sessions,
      unreadSessions,
      workspaces,
      setModel,
      setThinkingLevel,
      thinkingLevel,
      timeline,
      unpair,
    ],
  );

  return <RemoteContext.Provider value={value}>{children}</RemoteContext.Provider>;
}

export function useRemote(): RemoteContextValue {
  const value = useContext(RemoteContext);
  if (!value) throw new Error("useRemote must be used inside RemoteProvider");
  return value;
}
