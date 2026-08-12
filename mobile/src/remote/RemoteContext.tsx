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
  emptyTimeline,
  markApprovalDecision,
  mergeHistoryAttachments,
  timelineFromEntries,
  timelineFromHistory,
  type ReplayEventWire,
  type TimelineState,
} from "./eventReducer";
import { RemoteClient } from "./client";
import { MAX_PROMPT_MESSAGE_BYTES, utf8Bytes } from "./codec";
import type { ConnectionState } from "./connectionState";
import {
  claimPairingCode,
  ensureFreshCredentials,
  serverRevoke,
} from "./pairing";
import { isDesktopOnline } from "./presence";
import { detectFinished } from "./sessionStatus";
import { type RunCursor } from "./runCursor";
import { SyncEngine, type ReconcileReason } from "./syncEngine";
import {
  clearCredentials,
  clearPendingRevoke,
  loadCredentials,
  loadLastModel,
  loadLastThinking,
  loadPendingRevoke,
  saveCredentials,
  saveLastModel,
  saveLastThinking,
  savePendingRevoke,
  type PendingRevoke,
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

interface EventsPage extends EventsData {
  hasMore?: boolean;
  nextOffset?: number;
}

/**
 * Fetch a run's replay tail via `get_events_since`, looping paginated replies
 * until `hasMore=false`. The desktop pages replay events under the NATS payload
 * cap (a multi-MB journal tail must never ship as one oversized reply), so a
 * single-shot request would silently truncate. The merged envelope carries the
 * first page's `projection` (the whole-run replacement) through unchanged.
 */
async function fetchEventsSince(
  client: RemoteClient,
  sessionId: string,
  runId: string,
  sinceIdx: number,
): Promise<EventsData> {
  const events: ReplayEventWire[] = [];
  let projection: EventsData["projection"] = null;
  let truncated = false;
  let offset = 0;
  for (;;) {
    const page = (
      await client.request<EventsPage>(
        { type: "get_events_since", sessionId, runId, sinceIdx, offset },
        sessionId,
      )
    ).data;
    events.push(...(page.events ?? []));
    if (page.projection?.events?.length) projection = page.projection;
    if (page.truncated) truncated = true;
    if (!page.hasMore) break;
    const next = page.nextOffset;
    if (typeof next !== "number" || next <= offset) break;
    offset = next;
  }
  const merged: EventsData = { events };
  if (projection) merged.projection = projection;
  if (truncated) merged.truncated = true;
  return merged;
}

interface PromptAck {
  sessionId: string;
  threadId: string;
  runId: string;
}

/**
 * Fire the queued server-side revoke (M7). Best-effort: a failure is dropped —
 * the queue entry stays in storage and retries on the next launch. A success
 * (or a terminal 401/404, which serverRevoke treats as success) drains the
 * queue via the caller's clearPendingRevoke.
 */
async function attemptPendingRevoke(pending: PendingRevoke): Promise<void> {
  await serverRevoke({
    pairId: pending.pairId,
    deviceId: pending.deviceId,
    seed: pending.seed,
    userJwt: "",
    refreshToken: pending.refreshToken,
    natsWsUrl: "",
    tokenUrl: pending.tokenUrl,
    expectedDesktopId: "",
    expectedDesktopPublicKey: "",
  });
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

/**
 * Compares each session's status against the previously seen status map and
 * returns the ids whose run just finished (running/queued/waiting_approval →
 * completed/failed), plus the new status map. Pure — the caller owns state.
 */
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
  // Relative-heartbeat state (L7): the desktop-presence check judges staleness
  // by clock-offset drift, so the running baseline survives recomputes.
  const presenceStateRef = useRef({ deltaMs: 0, count: 0, online: false });
  // Per-session timelines: events for EVERY session are consumed (the desktop
  // observer mirrors all of them), so a background run keeps advancing and
  // switching to it renders its live state without a fresh history load. The
  // draft (a conversation with no session yet) lives under the "" key until a
  // prompt ack binds it to a real session.
  const [timelines, setTimelines] = useState<Record<string, TimelineState>>({});
  const timelinesRef = useRef<Record<string, TimelineState>>({});
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
  // The per-session serial sync engine owns the timelines and cursors — every
  // write goes through one lane per session (atomic cursor+timeline commits),
  // and reconcile is the only backfill path. The cursors map mirrors the
  // engine's for the send guard's live-streaming check.
  const syncEngineRef = useRef<SyncEngine | null>(null);
  const cursorsRef = useRef<Record<string, RunCursor>>({});
  // Session streaming is mirrored for the send guard: reads must reflect the
  // latest snapshot even before the subscriber's setState re-renders.
  const streamingRef = useRef<Record<string, boolean>>({});
  // Unsubscribes the current engine's commit feed when the client is replaced
  // or torn down (unpair).
  const engineRefCleanupRef = useRef<(() => void) | null>(null);

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

  /**
   * Enqueue a reconcile instruction for a session (or all established sessions
   * when `sessionId` is omitted) on the sync engine's serial lane.
   */
  const reconcileSession = useCallback(
    (sessionId: string | undefined, reason: ReconcileReason) => {
      const engine = syncEngineRef.current;
      if (!engine) return;
      if (sessionId) {
        engine.reconcile(sessionId, reason);
      } else {
        engine.reconcileAll(reason);
      }
    },
    [],
  );

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
      const { finished, next } = detectFinished(lastStatusRef.current, list, selectedRef.current);
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
      // Side effects that read event payloads directly (not timeline writes):
      // these stay outside the lane because they don't mutate the session
      // timeline — the lane only receives pure timeline events.
      if (event.type === "run_snapshot") {
        // The host replaced this run's replica with a folded projection. Folded
        // events cannot be applied incrementally — a coalesced chunk's text
        // spans idx values already applied — so reconcile the run from -1.
        reconcileSession(sid, "resend");
        return;
      }
      if (event.type === "session_name_changed") {
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
        return;
      }
      if (event.type === "user_message") {
        // Live events intentionally contain only the text. Enrich this bubble
        // from the durable entry without replacing streamed assistant content.
        void hydrateAttachmentsRef.current(sid);
      }
      if (event.type === "approval_decision") {
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
            syncEngineRef.current?.mutate(sid, timeline => {
              const has = timeline.items.some(
                item =>
                  item.kind === "approval" &&
                  item.payload.approval_request_id === approvalRequestId,
              );
              return has ? markApprovalDecision(timeline, approvalRequestId, decision) : timeline;
            });
          }
        } catch {
          // Ignore a malformed decision payload.
        }
        return;
      }
      // Timeline events — live application, gap detection, prefix healing and
      // snapshot flips all run on the session's serial lane.
      syncEngineRef.current?.event(sid, event);
      if (event.type === "agent_end") void refreshSessions();
    },
    [reconcileSession, refreshSessions],
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

  // The sync engine is created ONCE and survives client generations — a
  // reconnect replaces the client, but reconcileAll (reconnect recovery) and
  // the per-session lanes must keep running. Its deps read clientRef.current
  // on every call, so a client swap needs no engine rebuild.
  useEffect(() => {
    const engine = new SyncEngine({
      requestGetState: async sessionId => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        const response = await client.request<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        return response.data;
      },
      requestHistory: loadHistory,
      fetchReplay: async (sessionId, runId, sinceIdx) => {
        const client = clientRef.current;
        if (!client) throw new Error("not_connected");
        const merged = await fetchEventsSince(client, sessionId, runId, sinceIdx);
        return { ...merged, events: merged.events ?? [] };
      },
    });
    const unsubscribe = engine.subscribe(commit => {
      // Atomic cursor + timeline commit from the lane.
      setTimelines(prev => {
        const existing = prev[commit.sessionId];
        return existing === commit.timeline
          ? prev
          : { ...prev, [commit.sessionId]: commit.timeline };
      });
      cursorsRef.current[commit.sessionId] = commit.cursor;
      streamingRef.current[commit.sessionId] = commit.timeline.streaming;
    });
    syncEngineRef.current = engine;
    engineRefCleanupRef.current = () => {
      unsubscribe();
    };
    return () => {
      engineRefCleanupRef.current?.();
      engineRefCleanupRef.current = null;
      syncEngineRef.current = null;
    };
  }, [loadHistory]);

  const connect = useCallback(
    async (nextCredentials: RemoteCredentials) => {
      // Dispose any previous client first — it owns its own reconnect timer,
      // which must not keep running under a new pairing.
      await clientRef.current?.close();
      const fresh = await ensureFreshCredentials(nextCredentials);
      credentialsRef.current = fresh;
      setCredentials(fresh);
      setError(null);
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
          const { finished, next } = detectFinished(
            lastStatusRef.current,
            list,
            selectedRef.current,
          );
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
            // A snapshot flip (streaming true↔false) is a reconcile trigger:
            // a run that settled may have lost its tail to an at-most-once drop
            // (M11), and one that started again needs its head re-fetched (H3).
            const engine = syncEngineRef.current;
            if (engine) {
              const before = streamingRef.current[currentId] ?? false;
              if (before !== streaming) {
                streamingRef.current[currentId] = streaming;
                if (!streaming) {
                  const timeline = engine.timelineFor(currentId);
                  const run = timeline?.currentRunId;
                  engine.reconcile(currentId, "snapshot-flip", run ?? undefined);
                }
              }
            }
          }
        },
        onWorkspaces: workspaceList => {
          setWorkspaces(workspaceList);
        },
        onFeatures: features => {
          setFileTransferSupported(features.includes("file_transfer_v1"));
        },
        onConnectionState: (state: ConnectionState) => {
          // The FSM states map onto the UI phases. A transport disconnect
          // while ready is handled internally by the client's own status loop
          // and backoff timer — the context only mirrors the state.
          if (state === "ready") {
            setPhase("connected");
          } else if (state === "revoked") {
            setPhase("revoked");
          } else if (state === "unpaired") {
            setPhase("unpaired");
          } else if (state === "refreshing") {
            setPhase("refreshing");
          } else if (state === "connecting") {
            setPhase("connecting");
          } else {
            setPhase("reconnecting");
          }
        },
        onReconnected: () => {
          // A reconnect can drop events for any cached conversation (NATS is
          // at-most-once) — reconcile every established session. Also re-fetch
          // the control-plane lists: a first connect that recovered from a
          // failure never populated them.
          reconcileSession(undefined, "reconnect");
          void refreshSessions();
          void refreshWorkspaces();
        },
        onError: nextError => {
          setError(nextError.message);
        },
      });
      clientRef.current = client;
      // The sync engine is created ONCE and survives client generations —
      // reconcileAll (reconnect recovery) must keep working across a client
      // replacement. Its deps read the live clientRef on every call.
      await client.open();
      await Promise.all([refreshModels(), refreshSessions(), refreshWorkspaces()]);
    },
    [closeConversation, handleEvent, reconcileSession, refreshModels, refreshSessions, refreshWorkspaces],
  );

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        // A pending server-side revoke (M7) fires before anything else: an
        // offline unpair queued it on the last run. A failure keeps the entry
        // in storage (retries next launch) and must not block the normal boot.
        const pending = await loadPendingRevoke();
        if (pending) {
          try {
            await attemptPendingRevoke(pending);
            await clearPendingRevoke();
          } catch {
            // Retry on the next launch.
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
        const message = nextError instanceof Error ? nextError.message : String(nextError);
        setError(message);
        setPhase("unpaired");
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
      const message = nextError instanceof Error ? nextError.message : String(nextError);
      if (message === "invalid_jwt") {
        // A corrupt/revoked JWT can't be refreshed — retrying would loop. Drop
        // the stored credentials so the user can re-pair instead.
        credentialsRef.current = null;
        await clearCredentials();
        setCredentials(null);
        setPhase("unpaired");
        setError(null);
      } else {
        // The client handles transport failures internally (backoff retry);
        // surface the error but let the connection phase keep driving.
        setError(message);
      }
    } finally {
      setBusy(false);
    }
  }, [connect, credentials]);

  const unpair = useCallback(async () => {
    const current = credentials;
    setBusy(true);
    try {
      // Local deregistration — ALWAYS succeeds (M7). Even an offline unpair
      // must free the device; only the server-side revoke is best-effort.
      credentialsRef.current = null;
      await clientRef.current?.close();
      clientRef.current = null;
      engineRefCleanupRef.current?.();
      engineRefCleanupRef.current = null;
      syncEngineRef.current?.clear();
      if (current) {
        try {
          await serverRevoke(current);
        } catch {
          // Offline / token endpoint unreachable — queue the revoke for the
          // next launch rather than blocking the local unpair.
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
      setCredentials(null);
      setPresence(null);
      setSessions([]);
      setWorkspaces([]);
      setSelectedSessionId("");
      setDraft(false);
      setTimelines({});
      cursorsRef.current = {};
      streamingRef.current = {};
      setTitleOverrides({});
      titleOverridesRef.current = {};
      setPhase("unpaired");
      setError(null);
    } finally {
      setBusy(false);
    }
  }, [credentials]);


  useEffect(() => {
    hydrateAttachmentsRef.current = async sessionId => {
      const engine = syncEngineRef.current;
      if (!engine || !engine.timelineFor(sessionId)) return;
      try {
        const durable = await loadHistory(sessionId);
        engine.mutate(sessionId, live => mergeHistoryAttachments(live, durable));
      } catch {
        // The entry can briefly lag the live event. A reconnect/session open
        // repeats the merge from durable history.
      }
    };
  }, [loadHistory]);

  // ── Recovery (reconcile is the only backfill path) ──
  //
  // A reconnect can drop events for ANY cached conversation (NATS is
  // at-most-once), and a snapshot flip can signal a run whose tail was lost.
  // Both converge on the same instruction: reconcile the affected sessions on
  // their serial lanes. The engine rebuilds from durable history + replay, so
  // no full-resync/gap-fill special casing remains.

  useEffect(() => {
    recoverRef.current = async (sessionId?: string) => {
      await Promise.all([refreshModels(), refreshSessions(), refreshWorkspaces()]);
      reconcileSession(sessionId, "reconnect");
    };
  }, [reconcileSession, refreshModels, refreshSessions, refreshWorkspaces]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      setBusy(true);
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
        // Model + thinking come from the durable session state; the timeline
        // itself reconciles on the lane (replay from the cursor / from -1 for
        // a run whose prefix isn't proven).
        const state = await client.request<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        const currentModel = state.data.model ?? "";
        const matchingModel = models.find(model => modelReference(model) === currentModel);
        setModelId(matchingModel ? modelReference(matchingModel) : currentModel);
        setThinkingLevelState(state.data.thinkingLevel ?? "off");
        syncEngineRef.current?.reconcile(sessionId, "open");
        // A cached timeline may have been assembled from real-time events,
        // whose user_message payload deliberately omits attachments.
        void hydrateAttachmentsRef.current(sessionId);
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        setBusy(false);
      }
    },
    [hydrateAttachmentsRef, models],
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
      setTimelines(prev => (prev[""] ? prev : { ...prev, "": emptyTimeline() }));
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
      // Nothing to send is a no-op, not an error — the composer guards this too.
      if (!text.trim() && attachments.length === 0) return;
      if (!client) throw new Error("not_connected");
      if (busy) throw new Error("send_busy");
      if (attachments.length > 0 && !fileTransferSupported) {
        throw new Error("attachment_unsupported_desktop");
      }
      if (utf8Bytes(text) > MAX_PROMPT_MESSAGE_BYTES) {
        throw new Error("prompt_too_large");
      }
      // Uploading can take long enough for the user to navigate elsewhere.
      // Freeze every routing value now; never consult selectedRef again for
      // this send operation.
      const targetSessionId = selectedRef.current;
      const targetDraft = draft;
      const targetDraftMode = draftMode;
      const targetDraftWorkspaceId = draftWorkspaceId;
      const conversationEpoch = conversationEpochRef.current;
      const engine = syncEngineRef.current;
      // A run may have started (on this device or another) after the composer
      // cleared the input — a silent return here would swallow the user's
      // message. Throw so the UI restores the draft instead. Reads the live
      // mirror so the check reflects the latest lane snapshot even before the
      // subscriber re-renders.
      if (streamingRef.current[targetSessionId] ?? false) throw new Error("send_streaming");
      setBusy(true);
      try {
        const uploaded = await uploadAttachments(client, attachments, onUploadProgress);
        // The optimistic bubble is a lane instruction (append to the current
        // snapshot), not a whole-cache overwrite — events that landed during
        // the upload stay (M8).
        engine?.mutate(targetSessionId, timeline => {
          const base = timeline ?? emptyTimeline();
          return {
            ...base,
            items: [
              ...base.items,
              {
                id: `local:${Date.now()}:${base.items.length}`,
                kind: "message",
                role: "user",
                text: text.trim(),
                ...(attachments.length
                  ? {
                      attachments: attachments.map(attachment => ({
                        path: attachment.localUri,
                        name: attachment.name,
                        kind: attachment.kind,
                        mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
                      })),
                    }
                  : {}),
              },
            ],
          };
        });
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
          // A draft just got bound to a real session. Migrate the optimistic
          // bubble from the "" placeholder lane into the real session's lane —
          // the real session's events have been landing there all along.
          const stillViewingSentDraft = conversationEpochRef.current === conversationEpoch;
          if (stillViewingSentDraft) {
            selectedRef.current = nextSessionId;
            setSelectedSessionId(nextSessionId);
            setDraft(false);
            setDraftMode("chat");
            setDraftWorkspaceId("");
          }
          const draftItems = timelinesRef.current[targetSessionId]?.items ?? [];
          const draftUser = draftItems.find(
            item => item.kind === "message" && item.role === "user",
          );
          if (draftUser?.kind === "message") {
            const textToMove = draftUser.text;
            engine?.mutate(nextSessionId, timeline => {
              const current = timeline ?? emptyTimeline();
              const alreadyLanded = current.items.some(
                item =>
                  item.kind === "message" &&
                  item.role === "user" &&
                  item.text.trim() === textToMove.trim(),
              );
              if (alreadyLanded) return current;
              return {
                ...current,
                items: [...current.items, draftUser],
              };
            });
          }
          engine?.mutate(targetSessionId, timeline => ({
            ...(timeline ?? emptyTimeline()),
            items: [],
          }));
          if (stillViewingSentDraft) {
            void refreshSessions();
          }
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
    syncEngineRef.current?.mutate(sessionId, timeline => markApprovalDecision(timeline, id, decision));
  }, []);

  const desktopOnline = useMemo(() => {
    if (phase !== "connected") return false;
    const next = isDesktopOnline(presence, clock, presenceStateRef.current);
    presenceStateRef.current = next;
    return next.online;
  }, [clock, phase, presence]);
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
