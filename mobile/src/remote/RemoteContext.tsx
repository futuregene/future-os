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
  timelineFromHistory,
  type TimelineState,
} from "./eventReducer";
import { RemoteClient } from "./client";
import { claimPairingCode, ensureFreshCredentials, revokeCredentials } from "./pairing";
import { isDesktopOnline } from "./presence";
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

export function RemoteProvider({ children }: PropsWithChildren) {
  const [phase, setPhase] = useState<ConnectionPhase>("booting");
  const [error, setError] = useState<string | null>(null);
  const [credentials, setCredentials] = useState<RemoteCredentials | null>(null);
  const [presence, setPresence] = useState<Presence | null>(null);
  const [sessions, setSessions] = useState<RemoteSession[]>([]);
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
      setSessions(response.data.sessions ?? []);
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
      setTimeline(state => applyStreamEvent(state, event));
      if (event.type === "agent_end") {
        void refreshSessions();
      }
    },
    [refreshSessions],
  );

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
          const currentId = selectedRef.current;
          if (!currentId) return;
          const streaming =
            nextPresence.sessions?.find(session => session.id === currentId)?.streaming ?? false;
          setTimeline(state => ({ ...state, streaming }));
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
    [handleEvent, refreshModels, refreshSessions, refreshWorkspaces],
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

  const backfill = useCallback(async (sessionId: string, initial: TimelineState) => {
    const client = clientRef.current;
    if (!client) return initial;
    try {
      const response = await client.request<EventsData>(
        { type: "get_events_since", sessionId, sinceIdx: -1 },
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
  }, []);

  useEffect(() => {
    recoverRef.current = async () => {
      await Promise.all([refreshModels(), refreshSessions(), refreshWorkspaces()]);
      const sessionId = selectedRef.current;
      if (!sessionId) return;
      try {
        let next = await loadHistory(sessionId);
        if (timelineRef.current.streaming) next = await backfill(sessionId, next);
        setTimeline(next);
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      }
    };
  }, [backfill, loadHistory, refreshModels, refreshSessions, refreshWorkspaces]);

  const selectSession = useCallback(
    async (sessionId: string) => {
      const client = clientRef.current;
      if (!client) return;
      setBusy(true);
      setSelectedSessionId(sessionId);
      selectedRef.current = sessionId;
      setDraft(false);
      try {
        let next = await loadHistory(sessionId);
        const streaming =
          presenceRef.current?.sessions.find(session => session.id === sessionId)?.streaming ??
          false;
        if (streaming) next = await backfill(sessionId, next);
        next = { ...next, streaming };
        setTimeline(next);
        const response = await client.request<RemoteSessionState>(
          { type: "get_state", sessionId },
          sessionId,
        );
        const currentModel = response.data.model ?? "";
        const matchingModel = models.find(model => modelReference(model) === currentModel);
        setModelId(matchingModel ? modelReference(matchingModel) : currentModel);
        setThinkingLevelState(response.data.thinkingLevel ?? "off");
      } finally {
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

  const closeConversation = useCallback(() => {
    setSelectedSessionId("");
    selectedRef.current = "";
    setDraft(false);
    setDraftMode("chat");
    setDraftWorkspaceId("");
    setTimeline(emptyTimeline());
  }, []);

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
          const current = await backfill(nextSessionId, optimisticTimeline);
          setTimeline(current);
        }
      } finally {
        setBusy(false);
      }
    },
    [
      backfill,
      busy,
      draft,
      draftMode,
      draftWorkspaceId,
      modelId,
      refreshSessions,
      thinkingLevel,
      timeline,
    ],
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
