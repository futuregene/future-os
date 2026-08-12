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
  mergeHistoryAttachments,
  normalizeReplayEvents,
  timelineFromEntries,
  timelineFromHistory,
  stripRunItems,
  type ReplayEventWire,
  type TimelineState,
} from "./eventReducer";
import { RemoteClient } from "./client";
import { claimPairingCode, ensureFreshCredentials, revokeCredentials } from "./pairing";
import { isDesktopOnline } from "./presence";
import { advanceCursor, newCursor, nextEvent, type RunCursor } from "./runCursor";
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
import {
  cachedPreviewForAttachment,
  downloadPrepared,
  prepareDownload,
  rememberPreparedPreview,
  uploadAttachments,
} from "./files";
import type { File } from "expo-file-system";
import type {
  ConnectionPhase,
  HistoryEntry,
  HistoryMessage,
  HistoryAttachment,
  DownloadInfo,
  MobileAttachment,
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
  /** Raw replay events — the RPC serializes them with snake_case `run_id`. */
  events?: ReplayEventWire[];
  truncated?: boolean;
  /** Coalesced replica of a run whose event ring overflowed — replaces the
   *  session's timeline wholesale (see `timelineFromProjection`). */
  projection?: { run_id?: string; cursor?: number; events?: ReplayEventWire[] } | null;
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
  fileTransferSupported: boolean;
  pair(code: string): Promise<void>;
  reconnect(): Promise<void>;
  unpair(): Promise<void>;
  refreshSessions(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  selectSession(sessionId: string): Promise<void>;
  newConversation(mode?: "chat" | "workspace", workspaceId?: string): Promise<void>;
  closeConversation(): void;
  sendMessage(
    text: string,
    attachments?: MobileAttachment[],
    onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
  ): Promise<void>;
  prepareAttachment(attachment: HistoryAttachment): Promise<DownloadInfo>;
  cachedAttachment(attachment: HistoryAttachment): { info: DownloadInfo; file: File } | null;
  downloadAttachment(
    info: DownloadInfo,
    onProgress?: (completedBytes: number, totalBytes: number) => void,
  ): Promise<File>;
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
  const [modelId, setModelId] = useState("");
  const [thinkingLevel, setThinkingLevelState] = useState<ThinkingLevel>("off");
  const [busy, setBusy] = useState(false);
  const [fileTransferSupported, setFileTransferSupported] = useState(false);
  const [clock, setClock] = useState(Date.now());
  // Per-session timelines: events for EVERY session are consumed (the desktop
  // observer mirrors all of them), so a background run keeps advancing and
  // switching to it renders its live state without a fresh history load. The
  // draft (a conversation with no session yet) lives under the "" key until a
  // prompt ack binds it to a real session.
  const [timelines, setTimelines] = useState<Record<string, TimelineState>>({});
  const timelinesRef = useRef<Record<string, TimelineState>>({});
  const cursorsRef = useRef<Record<string, RunCursor>>({});
  // Live agent-side renames (`session_name_changed`, data: {name}) are not
  // persisted by the desktop store, so the snapshot title would go stale; the
  // override wins until the next sessions snapshot reflects it. Read via the
  // ref inside long-lived closures (the NATS callbacks) to avoid a stale
  // capture or forcing a client re-create on every rename.
  const [titleOverrides, setTitleOverrides] = useState<Record<string, string>>({});
  const titleOverridesRef = useRef<Record<string, string>>({});
  const clientRef = useRef<RemoteClient | null>(null);
  const credentialsRef = useRef<RemoteCredentials | null>(null);
  const selectedRef = useRef("");
  // Changes whenever the user navigates between conversations. Long uploads
  // capture the epoch so their eventual ack cannot pull the UI back to a
  // conversation the user has already left.
  const conversationEpochRef = useRef(0);
  const recoverRef = useRef<(sessionId?: string) => Promise<void>>(async () => undefined);
  const hydrateAttachmentsRef = useRef<(sessionId: string) => Promise<void>>(async () => undefined);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectAttemptRef = useRef(0);
  const scheduleReconnectRef = useRef<() => void>(() => undefined);
  // Integrity: sync lock + pending buffer + per-session run cursors for gap
  // detection. Events from any session are applied against that session's
  // cache; a gap in one session never blocks another.
  const syncLockRef = useRef(false);
  const pendingRef = useRef<{ event: StreamEvent; sessionId: string }[]>([]);
  const gapInFlightRef = useRef(false);

  useEffect(() => {
    credentialsRef.current = credentials;
  }, [credentials]);
  useEffect(() => {
    selectedRef.current = selectedSessionId;
  }, [selectedSessionId]);
  useEffect(() => {
    timelinesRef.current = timelines;
  }, [timelines]);
  useEffect(() => {
    titleOverridesRef.current = titleOverrides;
  }, [titleOverrides]);

  const refreshSessions = useCallback(async () => {
    const client = clientRef.current;
    if (!client) return;
    try {
      const response = await client.request<SessionsData>({ type: "list_sessions" }, "list");
      const overrides = titleOverridesRef.current;
      const list = (response.data.sessions ?? []).map(session => ({
        ...session,
        title: overrides[session.sessionId] ?? session.title,
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
      const sid = sessionId || "";
      if (!sid) return;
      // While a recovery/resync is in progress, buffer live events.
      if (syncLockRef.current) {
        pendingRef.current.push({ event, sessionId: sid });
        return;
      }
      if (event.type === "run_snapshot") {
        // The host replaced this run's replica with a folded projection (its
        // event ring overflowed). Folded events cannot be applied
        // incrementally — a coalesced chunk's text spans idx values already
        // applied — so heal wholesale: recover resyncs history + live tail,
        // rebuilds the cursor, and folds buffered events.
        void recoverRef.current(sid);
        return;
      }
      if (event.type === "session_name_changed") {
        // Agent-side rename (TUI `/name`, agent-driven). The desktop store is
        // not updated, so the sessions-snapshot title would go stale; remember
        // the new name until the snapshot catches up, then re-read the list so
        // the override reaches the sidebar immediately.
        try {
          const data = JSON.parse(event.data) as Record<string, unknown>;
          const name = typeof data.name === "string" ? data.name.trim() : "";
          if (name) {
            setTitleOverrides(prev => ({ ...prev, [sid]: name }));
            void refreshSessions();
          }
        } catch {
          // Ignore a malformed rename payload.
        }
      }
      if (event.type === "user_message") {
        // Live events intentionally contain only the text. Enrich this bubble
        // from the durable entry without replacing streamed assistant content.
        void hydrateAttachmentsRef.current(sid);
      }
      if (event.type === "approval_decision") {
        // A decision made on another device (desktop/TUI) resolves the pending
        // card here — otherwise it would linger until the session was reopened.
        try {
          const data = JSON.parse(event.data) as Record<string, unknown>;
          const approvalId = data.approval_request_id;
          const status = data.status;
          if (
            approvalId &&
            (status === "approved" || status === "rejected" || status === "cancelled")
          ) {
            const decision = status as "approved" | "rejected" | "cancelled";
            const approvalRequestId = approvalId as string;
            setTimelines(prev => {
              const tl = prev[sid];
              if (!tl) return prev;
              const has = tl.items.some(
                item =>
                  item.kind === "approval" &&
                  item.payload.approval_request_id === approvalRequestId,
              );
              if (!has) return prev;
              return { ...prev, [sid]: markApprovalDecision(tl, approvalRequestId, decision) };
            });
          }
        } catch {
          // Ignore a malformed decision payload.
        }
      }
      // Every session's events are consumed — the desktop observer mirrors all
      // of them, so a background run keeps advancing in its own cache and a
      // later switch to it renders its live state.
      let cursor = cursorsRef.current[sid];
      if (!cursor) {
        cursor = newCursor();
        cursorsRef.current[sid] = cursor;
      }
      const verdict = nextEvent(cursor, event.runId, event.idx);
      if (verdict.kind === "dup") return;
      if (verdict.kind === "gap") {
        // Buffer the gap-triggering event; do NOT apply out of order.
        pendingRef.current.push({ event, sessionId: sid });
        syncLockRef.current = true;
        void fillGapRef.current(sid, event.runId ?? "", verdict.fromIdx);
        return;
      }
      // "apply" or "untracked"
      if (verdict.kind === "apply") {
        advanceCursor(cursor, event.runId!, verdict.idx);
      }
      setTimelines(prev => ({
        ...prev,
        [sid]: applyStreamEvent(prev[sid] ?? emptyTimeline(), event),
      }));
      if (event.type === "agent_end") {
        void refreshSessions();
      }
    },
    [refreshSessions],
  );

  const closeConversation = useCallback(() => {
    conversationEpochRef.current += 1;
    setSelectedSessionId("");
    selectedRef.current = "";
    setDraft(false);
    setDraftMode("chat");
    setDraftWorkspaceId("");
    // Keep the per-session timeline caches — reopening a conversation renders
    // its live state without a fresh history load.
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
      setFileTransferSupported(false);
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
          const overrides = titleOverridesRef.current;
          const list: RemoteSession[] = sessionList.map(s => ({
            sessionId: s.sessionId,
            threadId: s.threadId,
            title: overrides[s.sessionId] ?? s.title,
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
            setTimelines(prev => {
              const existing = prev[currentId];
              return existing && existing.streaming !== streaming
                ? { ...prev, [currentId]: { ...existing, streaming } }
                : prev;
            });
          }
        },
        onWorkspaces: workspaceList => {
          setWorkspaces(workspaceList);
        },
        onFeatures: features => {
          setFileTransferSupported(features.includes("file_transfer_v1"));
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
      setTimelines({});
      cursorsRef.current = {};
      setTitleOverrides({});
      titleOverridesRef.current = {};
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

  useEffect(() => {
    hydrateAttachmentsRef.current = async sessionId => {
      try {
        const durable = await loadHistory(sessionId);
        setTimelines(prev => {
          const live = prev[sessionId];
          if (!live) return prev;
          return { ...prev, [sessionId]: mergeHistoryAttachments(live, durable) };
        });
      } catch {
        // The entry can briefly lag the live event. A reconnect/session open
        // repeats the merge from durable history.
      }
    };
  }, [loadHistory]);

  // ── Gap-fill and full-resync (integrity layer) ──

  /**
   * Top up a session's timeline with the tail of its active run (the events
   * after the run's high-water cursor). The agent's `events_since` reads a
   * settled run's durable journal and an in-flight run's live ring, so this
   * both resyncs a fresh open and heals a mid-run reconnect. When the ring
   * overflowed, the returned projection replaces the run's partial items
   * wholesale.
   */
  const hydrateActiveRun = useCallback(
    async (
      sessionId: string,
      base: TimelineState,
      cursor: RunCursor,
      preserveCache: boolean,
      activeRunId: string,
    ): Promise<TimelineState> => {
      const client = clientRef.current;
      if (!client || !activeRunId) return { ...base, streaming: false };
      // A fresh history base carries the run's partial persisted entries; the
      // replay below supersedes them, so drop them first. A live cache's items
      // are the real events and are kept.
      const baseForReplay = preserveCache ? base : stripRunItems(base, activeRunId);
      const sinceIdx = cursor.get(activeRunId) ?? -1;
      let response: EventsData;
      try {
        response = (
          await client.request<EventsData>(
            { type: "get_events_since", sessionId, runId: activeRunId, sinceIdx },
            sessionId,
          )
        ).data;
      } catch {
        return { ...baseForReplay, streaming: true };
      }
      if (response.projection?.events?.length) {
        // The ring overflowed; the projection is the whole run and replaces
        // the run's partial items — the cache's live events for this run (the
        // replayed ones carry the run_id) and history's partial entries alike.
        // User bubbles survive stripRunItems, so the transcript order holds.
        // Fold the projection through the normal reducer ONTO the stripped base
        // so the run's shared projector (liveRuns) is rebuilt from the full
        // replay — a run still streaming after this backfill keeps projecting
        // correctly instead of restarting from an empty accumulator.
        const baseStripped = stripRunItems(baseForReplay, activeRunId);
        const projectionEvents = normalizeReplayEvents(response.projection.events);
        // The stripped base carries live-accumulated seenEvents keys for this
        // run; folding the replay through applyStreamEvent would dedup every
        // event against them and silently drop the whole reply. Reset the seen
        // set so the projection's events always apply.
        let next = { ...baseStripped, seenEvents: new Set<string>() };
        for (const ev of projectionEvents) next = applyStreamEvent(next, ev);
        const cursorIdx =
          response.projection.cursor ??
          projectionEvents.reduce((max, ev) => Math.max(max, ev.idx ?? -1), -1);
        advanceCursor(cursor, activeRunId, cursorIdx);
        const settled = projectionEvents.some(ev => ev.type === "agent_end");
        return { ...next, streaming: !settled };
      }
      let next = baseForReplay;
      const events = normalizeReplayEvents(response.events);
      for (const ev of events) next = applyStreamEvent(next, ev);
      for (const ev of events) {
        if (ev.runId && ev.idx != null) advanceCursor(cursor, ev.runId, ev.idx);
      }
      const settled = events.some(ev => ev.type === "agent_end");
      // Empty replay means the cursor was already caught up (the active run
      // ended between get_state and this request) — preserve the base's
      // streaming state instead of assuming the run is still live.
      return { ...next, streaming: events.length > 0 ? !settled : base.streaming };
    },
    [],
  );

  const fullResync = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      let activeRunId = "";
      try {
        const state = await client.request<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        activeRunId = state.data.activeRun?.runId ?? "";
      } catch {
        // Fall through with an empty run id — history-only resync.
      }
      const cursor = cursorsRef.current[sessionId] ?? newCursor();
      cursorsRef.current[sessionId] = cursor;
      const history = await loadHistory(sessionId);
      const next = await hydrateActiveRun(sessionId, history, cursor, false, activeRunId);
      setTimelines(prev => ({ ...prev, [sessionId]: next }));
    },
    [hydrateActiveRun, loadHistory],
  );

  /** Fold buffered events (from any session) into each session's cache. */
  const flushPendingLock = useCallback(() => {
    const pending = pendingRef.current;
    pendingRef.current = [];
    if (pending.length === 0) return;
    const toApply: { sessionId: string; event: StreamEvent }[] = [];
    let retrySessionId: string | null = null;
    for (const { event, sessionId } of pending) {
      const cursor = cursorsRef.current[sessionId] ?? (cursorsRef.current[sessionId] = newCursor());
      const verdict = nextEvent(cursor, event.runId, event.idx);
      if (verdict.kind === "dup") continue;
      if (verdict.kind === "gap") {
        retrySessionId = sessionId;
        continue;
      }
      if (verdict.kind === "apply") advanceCursor(cursor, event.runId!, verdict.idx);
      toApply.push({ sessionId, event });
    }
    if (toApply.length > 0) {
      // Functional update so this merges over the latest state (hydration may
      // have landed a new cache since the pending events were buffered).
      setTimelines(prev => {
        const next = { ...prev };
        for (const { sessionId, event } of toApply) {
          next[sessionId] = applyStreamEvent(next[sessionId] ?? emptyTimeline(), event);
        }
        return next;
      });
    }
    if (retrySessionId) void fullResync(retrySessionId);
  }, [fullResync]);

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
        const response = (
          await client.request<EventsData>(
            { type: "get_events_since", sessionId, runId, sinceIdx: fromIdx },
            sessionId,
          )
        ).data;
        const events = normalizeReplayEvents(response.events);
        const firstIdx = events.length > 0 ? events[0]!.idx : undefined;
        // If the agent buffer overflowed (dropped head), firstIdx will be > fromIdx+1.
        if (events.length > 0 && firstIdx != null && firstIdx > fromIdx + 1) {
          await fullResync(sessionId);
          return;
        }
        // Apply fetched events in order (dedup in reducer handles overlaps).
        setTimelines(prev => {
          const next = prev[sessionId] ?? emptyTimeline();
          return {
            ...prev,
            [sessionId]: events.reduce((state, ev) => applyStreamEvent(state, ev), next),
          };
        });
        // Advance cursor for all fetched events.
        const cursor = cursorsRef.current[sessionId] ?? newCursor();
        cursorsRef.current[sessionId] = cursor;
        for (const ev of events) {
          if (ev.runId && ev.idx != null) advanceCursor(cursor, ev.runId, ev.idx);
        }
        // Flush pending buffer (may contain the gap-triggering event + subsequent ones).
        flushPendingLock();
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
  }, [flushPendingLock, fullResync]);

  useEffect(() => {
    recoverRef.current = async (sessionId?: string) => {
      syncLockRef.current = true;
      try {
        await Promise.all([refreshModels(), refreshSessions(), refreshWorkspaces()]);
        const targets = new Set<string>();
        if (sessionId) {
          targets.add(sessionId);
        } else {
          // A reconnect can drop events for ANY cached conversation (NATS is
          // at-most-once) — resync every cached session, not just the open one.
          if (selectedRef.current) targets.add(selectedRef.current);
          for (const id of Object.keys(timelinesRef.current)) {
            if (id) targets.add(id);
          }
        }
        for (const target of targets) await fullResync(target);
        // Fold any events buffered during recovery.
        flushPendingLock();
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        syncLockRef.current = false;
      }
    };
  }, [flushPendingLock, fullResync, refreshModels, refreshSessions, refreshWorkspaces]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      setBusy(true);
      syncLockRef.current = true;
      conversationEpochRef.current += 1;
      setSelectedSessionId(sessionId);
      selectedRef.current = sessionId;
      setDraft(false);
      setUnreadSessions(prev => {
        if (!prev.has(sessionId)) return prev;
        const nextUnread = new Set(prev);
        nextUnread.delete(sessionId);
        return nextUnread;
      });
      try {
        const cached = timelinesRef.current[sessionId];
        const cursor = cursorsRef.current[sessionId] ?? newCursor();
        cursorsRef.current[sessionId] = cursor;
        const state = await client.request<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        const currentModel = state.data.model ?? "";
        const matchingModel = models.find(model => modelReference(model) === currentModel);
        setModelId(matchingModel ? modelReference(matchingModel) : currentModel);
        setThinkingLevelState(state.data.thinkingLevel ?? "off");
        const activeRunId = state.data.activeRun?.runId ?? "";
        if (cached) {
          // A live cache is authoritative — just top up its active run's tail
          // (a run that started or progressed while we were away).
          const hydrated =
            activeRunId && !cached.streaming
              ? await hydrateActiveRun(sessionId, cached, cursor, true, activeRunId)
              : cached;
          setTimelines(prev =>
            prev[sessionId] && prev[sessionId] === cached
              ? { ...prev, [sessionId]: hydrated }
              : prev,
          );
          // A cached timeline may have been assembled from real-time events,
          // whose user_message payload deliberately omits attachments.
          void hydrateAttachmentsRef.current(sessionId);
        } else {
          // No cache yet: load history, then overlay the active run's tail.
          const history = await loadHistory(sessionId);
          const next = await hydrateActiveRun(sessionId, history, cursor, false, activeRunId);
          setTimelines(prev => ({ ...prev, [sessionId]: next }));
        }
        // Fold any events buffered during the switch.
        flushPendingLock();
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        syncLockRef.current = false;
        setBusy(false);
      }
    },
    [flushPendingLock, hydrateActiveRun, loadHistory, models],
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
      conversationEpochRef.current += 1;
      setSelectedSessionId("");
      selectedRef.current = "";
      setDraft(true);
      setDraftMode(mode);
      setDraftWorkspaceId(workspaceId);
      setTimelines(prev => ({ ...prev, "": emptyTimeline() }));
      setModelId(defaultModel);
      setThinkingLevelState((lastThinking as ThinkingLevel | null) ?? "off");
    },
    [models],
  );

  const sendMessage = useCallback(
    async (
      text: string,
      attachments: MobileAttachment[] = [],
      onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
    ) => {
      const client = clientRef.current;
      if (!client || busy || (!text.trim() && attachments.length === 0)) return;
      if (attachments.length > 0 && !fileTransferSupported) {
        throw new Error("attachment_unsupported_desktop");
      }
      // Uploading can take long enough for the user to navigate elsewhere.
      // Freeze every routing value now; never consult selectedRef again for
      // this send operation.
      const targetSessionId = selectedRef.current;
      const targetDraft = draft;
      const targetDraftMode = draftMode;
      const targetDraftWorkspaceId = draftWorkspaceId;
      const conversationEpoch = conversationEpochRef.current;
      const currentTimeline = timelinesRef.current[targetSessionId] ?? emptyTimeline();
      if (currentTimeline.streaming) return;
      setBusy(true);
      try {
        const uploaded = await uploadAttachments(client, attachments, onUploadProgress);
        const optimisticTimeline = appendUserMessage(
          currentTimeline,
          text.trim(),
          attachments.map(attachment => ({
            path: attachment.localUri,
            name: attachment.name,
            kind: attachment.kind,
            mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
          })),
        );
        setTimelines(prev => ({ ...prev, [targetSessionId]: optimisticTimeline }));
        const response = await client.requestRetry<PromptAck>(
          {
            type: "prompt",
            sessionId: targetSessionId,
            message: text.trim(),
            modelId,
            providerId: modelProviderFromReference(modelId),
            level: thinkingLevel,
            ...(uploaded.length
              ? { attachments: uploaded.map(attachment => ({ uploadId: attachment.uploadId! })) }
              : {}),
            ...(targetDraft && targetDraftMode === "workspace"
              ? { mode: "workspace", workspaceId: targetDraftWorkspaceId }
              : {}),
          },
          targetSessionId,
        );
        const nextSessionId = response.data.sessionId;
        if (nextSessionId && nextSessionId !== targetSessionId) {
          // A draft just got bound to a real session. Its live events have been
          // landing in the real session's cache all along (handleEvent consumes
          // every session), so migrate the optimistic user bubble from the ""
          // placeholder cache and finish hydrating the run's tail.
          const draftTimeline = optimisticTimeline;
          const stillViewingSentDraft = conversationEpochRef.current === conversationEpoch;
          if (stillViewingSentDraft) {
            selectedRef.current = nextSessionId;
            setSelectedSessionId(nextSessionId);
            setDraft(false);
            setDraftMode("chat");
            setDraftWorkspaceId("");
          }
          setTimelines(prev => {
            const draftItems = draftTimeline?.items ?? [];
            const current = prev[nextSessionId] ?? emptyTimeline();
            // The user_message mirror may have landed the same prompt in the
            // real session's cache before this migration runs — keep the
            // landed bubble instead of stacking the optimistic one on it.
            const draftUser = draftItems.find(
              item => item.kind === "message" && item.role === "user",
            );
            const alreadyLanded =
              draftUser?.kind === "message" &&
              current.items.some(
                item =>
                  item.kind === "message" &&
                  item.role === "user" &&
                  item.text.trim() === draftUser.text.trim(),
              );
            const currentItems =
              alreadyLanded && draftUser?.kind === "message" && draftUser.attachments?.length
                ? current.items.map(item =>
                    item.kind === "message" &&
                    item.role === "user" &&
                    item.text.trim() === draftUser.text.trim()
                      ? { ...draftUser, runId: item.runId }
                      : item,
                  )
                : current.items;
            return {
              ...prev,
              [nextSessionId]: {
                ...current,
                items: alreadyLanded ? currentItems : [...draftItems, ...current.items],
              },
              ...(stillViewingSentDraft && (draftItems.length === 0 || alreadyLanded)
                ? { "": emptyTimeline() }
                : {}),
            };
          });
          await refreshSessions();
        }
      } finally {
        setBusy(false);
      }
    },
    [
      busy,
      draft,
      draftMode,
      draftWorkspaceId,
      fileTransferSupported,
      modelId,
      refreshSessions,
      thinkingLevel,
    ],
  );

  const prepareAttachment = useCallback(async (attachment: HistoryAttachment) => {
    const client = clientRef.current;
    const sessionId = selectedRef.current;
    if (!client || !sessionId) throw new Error("attachment_no_session");
    const info = await prepareDownload(client, sessionId, attachment);
    rememberPreparedPreview(attachment, info);
    return info;
  }, []);

  const cachedAttachment = useCallback(
    (attachment: HistoryAttachment) => cachedPreviewForAttachment(attachment),
    [],
  );

  const downloadAttachment = useCallback(
    async (
      info: DownloadInfo,
      onProgress?: (completedBytes: number, totalBytes: number) => void,
    ) => {
      const client = clientRef.current;
      if (!client) throw new Error("attachment_not_connected");
      return downloadPrepared(client, info, onProgress);
    },
    [],
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
    setTimelines(prev => {
      const tl = prev[sessionId];
      return tl ? { ...prev, [sessionId]: markApprovalDecision(tl, id, decision) } : prev;
    });
  }, []);

  const desktopOnline = useMemo(
    () => phase === "connected" && isDesktopOnline(presence, clock),
    [clock, phase, presence],
  );
  // The selected conversation's timeline — derived from the per-session cache
  // so ChatScreen reads it exactly as before. An empty draft (no session yet)
  // renders the "" cache.
  const timeline = useMemo(
    () => timelines[selectedSessionId || ""] ?? emptyTimeline(),
    [selectedSessionId, timelines],
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
      fileTransferSupported,
      pair,
      reconnect,
      unpair,
      refreshSessions,
      refreshWorkspaces,
      selectSession,
      newConversation,
      closeConversation,
      sendMessage,
      prepareAttachment,
      cachedAttachment,
      downloadAttachment,
      abort,
      setModel,
      setThinkingLevel,
      rename,
      decideApproval,
    }),
    [
      abort,
      busy,
      fileTransferSupported,
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
      prepareAttachment,
      cachedAttachment,
      downloadAttachment,
      sessions,
      timeline,
      unreadSessions,
      workspaces,
      setModel,
      setThinkingLevel,
      thinkingLevel,
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
