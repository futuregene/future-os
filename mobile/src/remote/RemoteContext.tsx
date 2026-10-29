import type { PropsWithChildren, ReactNode } from "react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { RemoteClient, RecoveryReason } from "./client";
import { peerMachineStatus } from "./desktopStatus";
import type { MachineSummary, MachineStatus } from "./desktopStatus";
import type { DesktopCommands } from "./desktopCommands";
import { prepareTaskNotifications } from "../notifications/taskNotifications";
import i18n from "../i18n";
import { AppAlert } from "../components/appAlerts";
import { connectionPresentation as buildConnectionPresentation } from "./connectionPresentation";
import type { ConnectionPresentation } from "./connectionPresentation";
import type { CatalogSyncState } from "./useSessionCatalog";
import { useAppLifecycle } from "./useAppLifecycle";
import { useDesktopManagement } from "./useDesktopManagement";
import { useDesktopRuntime, type DesktopRuntime } from "./useDesktopRuntime";
import { usePairedDesktops } from "./usePairedDesktops";
import type { TimelineState } from "@future-os/remote-client/timeline";
import type { CatalogEntry } from "./mergeSessions";
import { DEFAULT_THINKING_LEVEL } from "./types";
import type {
  ConnectionPhase,
  DownloadInfo,
  HistoryAttachment,
  MobileAttachment,
  PairedDesktop,
  Presence,
  RemoteCredentials,
  RemoteModel,
  RemoteSession,
  RemoteSessionUsage,
  RemoteSkill,
  RemoteWorkspace,
  SessionFileListing,
  ThinkingLevel,
} from "./types";

interface RemoteContextValue extends ReturnType<typeof useDesktopManagement> {
  desktopSettingsRevision: number;
  skillsRevision: number;
  /** Capture navigation/connection ownership before starting asynchronous UI work. */
  captureConversationOperation(): () => boolean;
  phase: ConnectionPhase;
  error: string | null;
  credentials: RemoteCredentials | null;
  desktops: PairedDesktop[];
  /** Read another paired desktop. Instant: its runtime is already connected. */
  switchDesktop(desktopId: string): Promise<void>;
  renameDesktop(desktopId: string, name: string): Promise<void>;
  /** Set a desktop's local name/icon. Local-only; never leaves this phone. */
  labelDesktop(desktopId: string, patch: { name?: string; icon?: string }): Promise<void>;
  removeDesktop(desktopId: string): Promise<void>;
  presence: Presence | null;
  /**
   * Ask the open desktop to let this device drive it.
   *
   * Only meaningful while another paired device holds it (`presence.control ===
   * "standby"`); the desktop moves control for nobody who merely connected.
   */
  takeControl(): Promise<void>;
  desktopOnline: boolean;
  /**
   * Whether this desktop answers *reads*.
   *
   * Online is not enough: a machine another device drives refuses every command
   * before it reaches a handler (`superseded`), so a page that read it anyway
   * showed "could not read the desktop" over a machine that was answering
   * perfectly well — the user just had not taken control. Pages that load
   * something from the desktop gate on this and say why instead.
   */
  desktopReadable: boolean;
  hasConnectedContent: boolean;
  agentAvailable: boolean;
  connectionPresentation: ConnectionPresentation;
  catalogSync: CatalogSyncState;
  sessions: RemoteSession[];
  workspaces: RemoteWorkspace[];
  unreadSessions: Set<string>;
  models: RemoteModel[];
  selectedSessionId: string;
  selectedTitle: string;
  draft: boolean;
  draftMode: "chat" | "workspace";
  draftWorkspaceId: string;
  timeline: TimelineState;
  timelinePending: boolean;
  timelineSyncStatus: import("@future-os/remote-client/syncEngine").TimelineSyncStatus;
  timelineError: "timeout" | null;
  canLoadOlderTimeline: boolean;
  loadingOlderTimeline: boolean;
  /**
   * The last pull of older history failed (timeout, transport, cancelled).
   * Surfaces as a retry affordance; cleared by the next attempt. Distinct
   * from `loadingOlderTimeline`: this is a *settled* failure the user can act
   * on, not work still in flight.
   */
  olderHistoryFailed: boolean;
  modelId: string;
  thinkingLevel: ThinkingLevel;
  /** Token usage + amount for the open conversation (null until reported). */
  sessionUsage: RemoteSessionUsage | null;
  /**
   * Re-read the open conversation's state, so an amount is current when it is
   * about to be shown. Resolves once the read settles; a failure leaves the
   * last known figures in place.
   */
  refreshSessionUsage: () => Promise<void>;
  approvalTier: string;
  sandboxAvailable: boolean;
  sandboxCode?: string;
  busy: boolean;
  compacting: boolean;
  fileTransferSupported: boolean;
  capabilities: Set<string>;
  pair(code: string): Promise<void>;
  /**
   * Reach a machine again — the one named, or the one the phone reads.
   *
   * Every paired machine has its own runtime, so a machine the list is showing
   * but the phone is not reading can be reached on its own connection. Without
   * the id this is the connection sheet's own action, which is about the machine
   * the open conversation belongs to.
   */
  reconnect(desktopId?: string): Promise<void>;
  unpair(): Promise<void>;
  refreshSessions(): Promise<void>;
  refreshWorkspaces(): Promise<void>;
  /**
   * Re-read the connected desktop's model list.
   *
   * The list is what a new draft defaults its model to, so a caller that has
   * just moved the phone onto another machine reads it before building a draft
   * there — an empty list would leave the conversation on the model the machine
   * before it had. Resolves once the first probe settles; a failure leaves the
   * catalogue empty rather than throwing.
   */
  refreshModels(): Promise<void>;
  selectSession(sessionId: string): Promise<void>;
  /**
   * Open a new conversation on the machine that will run it.
   *
   * `newConversation` is the *open* machine's verb, and a screen's copy of it
   * belongs to the render that drew that screen: a screen that awaited
   * `switchDesktop(another)` and then called it would build the draft on the
   * machine it had just left. This names the machine instead, reads that
   * machine's runtime out of the registry — a ref, so it is whatever that
   * machine is *now* — and points the screens at it afterwards, so the draft is
   * what they then read.
   *
   * Rejects with `desktop_unavailable` when that machine has no live client: a
   * conversation with nowhere to be sent is worse than a message saying so.
   */
  newConversationOn(
    desktopId: string,
    mode?: "chat" | "workspace",
    workspaceId?: string,
  ): Promise<void>;
  /**
   * Open a conversation on the desktop that owns it.
   *
   * Every paired desktop holds a runtime, so this is not a reconnection: the
   * machine named is already connected and its own conversation state is
   * already there. What the call does is point the screens at that runtime and
   * open the conversation in it, which is why the row appears as fast as a row
   * on the desktop already being read.
   */
  openSessionOnDesktop(desktopId: string, sessionId: string): Promise<void>;
  retryTimeline(): Promise<void>;
  loadOlderTimeline(): Promise<false | string[]>;
  /** Pull-to-refresh: rebuild the visible timeline from durable history. */
  reloadTimeline(): void;
  newConversation(mode?: "chat" | "workspace", workspaceId?: string): Promise<void>;
  closeConversation(): void;
  sendMessage(
    text: string,
    attachments?: MobileAttachment[],
    onUploadProgress?: (completedBytes: number, totalBytes: number) => void,
  ): Promise<void>;
  /** Request manual context compaction; resolves with the operation to correlate. */
  compactContext(): Promise<{ sessionId: string; operationId: string }>;
  /** Await that operation's terminal outcome (never rejects; `timeout` on expiry). */
  awaitCompactionOutcome(
    sessionId: string,
    operationId: string,
    timeoutMs?: number,
    signal?: AbortSignal,
  ): Promise<import("./types").CompactionOutcome>;
  listSessionFiles(path?: string): Promise<SessionFileListing>;
  listSkills(): Promise<RemoteSkill[]>;
  /**
   * Fetch a tool call's display target on demand — the arguments a lean
   * history page omitted — and cache it for this conversation. Resolves null
   * when the call has no target; rejects on transport/route failure, which the
   * row treats as "leave the row as it was".
   */
  resolveToolCallTarget(toolCallId: string, runId: string, fusion?: { stepId?: string; offset?: number }): Promise<string | null>;
  prepareAttachment(
    attachment: HistoryAttachment,
    variant?: "preview" | "original",
    signal?: AbortSignal,
    onWaiting?: () => void,
  ): Promise<DownloadInfo>;
  cachedAttachment(
    attachment: HistoryAttachment,
    variant?: "preview" | "original",
  ): { info: DownloadInfo; file: import("expo-file-system").File } | null;
  downloadAttachment(
    info: DownloadInfo,
    onProgress?: (completedBytes: number, totalBytes: number) => void,
    signal?: AbortSignal,
    onWaiting?: () => void,
  ): Promise<import("expo-file-system").File>;
  abort(): Promise<void>;
  setModel(modelId: string): Promise<void>;
  setThinkingLevel(level: ThinkingLevel): Promise<void>;
  setApprovalTier(tier: string): Promise<void>;
  rename(sessionId: string, name: string): Promise<void>;
  generateTitle(sessionId: string, language: string): Promise<string>;
  deleteSession(sessionId: string, threadId: string): Promise<void>;
  deleteWorkspace(workspaceId: string): Promise<void>;
  /**
   * Register an existing directory on the desktop as a workspace. Resolves the
   * created row (a path that already names a workspace reopens it); rejects
   * with the desktop's reason when the directory does not exist.
   */
  createWorkspace(path: string, name: string): Promise<import("./types").RemoteWorkspace>;
  setSessionPinned(sessionId: string, threadId: string, pinned: boolean): Promise<void>;
  setWorkspacePinned(workspaceId: string, pinned: boolean): Promise<void>;
  /**
   * Rename a workspace on the connected desktop. The host applies its own name
   * rules and answers the fresh catalogue, so a resolved promise means the new
   * name is the accepted one; a refused name rejects.
   */
  renameWorkspace(workspaceId: string, name: string): Promise<void>;
  /** Fork the open conversation at a persisted user entry and open the child. */
  forkConversation(sourceEntryId: string): Promise<void>;
  decideApproval(id: string, decision: "approved" | "rejected"): Promise<void>;
  clearError(): void;
  continueRun(sessionId: string, runId: string): Promise<void>;
  /**
   * Every paired machine, in the shape the merged list and the status popover
   * read: the conversation catalogues of the machines other than the open one,
   * their own commands, and each one's status.
   *
   * A machine other than the open one used to be a catalogue-only observer;
   * it is now a full runtime, so this is a *view* of runtimes that already
   * exist rather than a pool this surface owns. Nothing here opens or closes a
   * connection: a machine's runtime belongs to the machine.
   */
  catalogs: {
    catalogs: CatalogEntry[];
    machines: MachineSummary[];
    refresh(): Promise<void>;
    reconnect(): void;
    commandsFor(desktopId: string): DesktopCommands | null;
    /** Ask `desktopId` to let this phone drive it (see `takeControl`). */
    takeControlFor(desktopId: string): Promise<void>;
    readWorkspaces(desktopId: string): Promise<RemoteWorkspace[]>;
  };
}

/** Commands a machine other than the open one answers, on its own runtime. */
export type { DesktopCommands } from "./desktopCommands";

type TimelineContextValue = Pick<RemoteContextValue,
  "timeline" | "timelinePending" | "timelineSyncStatus" | "timelineError" | "canLoadOlderTimeline" | "loadingOlderTimeline" | "olderHistoryFailed"
>;
export type RemoteControls = Omit<RemoteContextValue, keyof TimelineContextValue> & { streaming: boolean };
const RemoteContext = createContext<RemoteControls | null>(null);
const RemoteTimelineContext = createContext<TimelineContextValue | null>(null);
/** Every paired machine's own surface, by desktop id. See `DesktopScope`. */
const MachineScopesContext = createContext<Map<string, RemoteControls> | null>(null);

/**
 * One paired desktop's runtime, as the provider keeps it.
 *
 * The runtime is owned by a host component (one per paired desktop); this is
 * the handle the provider holds so the screens, the machine list and the
 * lifecycle can reach a machine that is not the open one.
 */
export interface RuntimeEntry extends DesktopRuntime {
  icon?: string;
}

/**
 * Whether anything the *provider* reads from a runtime changed.
 *
 * Function fields are deliberately not compared: they are rebuilt on every
 * render (they close over the current catalogue), so comparing them would
 * publish on every render and re-render the whole app in a loop. They are still
 * stored fresh on every render — only the re-render is skipped.
 *
 * The transcript's own contents are not here either: they change on every
 * streamed delta, and the controls must not be handed a new value for that.
 * They travel on their own signal (`sameTimelineData`) and reach only the
 * timeline context.
 */
function samePublishedData(previous: RuntimeEntry | undefined, next: RuntimeEntry): boolean {
  if (!previous) return false;
  const a = previous.connection;
  const b = next.connection;
  return previous.label === next.label
    && previous.icon === next.icon
    && previous.selectedSessionId === next.selectedSessionId
    && previous.selectedTitle === next.selectedTitle
    && previous.draft === next.draft
    && previous.draftMode === next.draftMode
    && previous.draftWorkspaceId === next.draftWorkspaceId
    && previous.streaming === next.streaming
    && previous.compacting === next.compacting
    && previous.busy === next.busy
    && previous.desktopSettingsRevision === next.desktopSettingsRevision
    && previous.skillsRevision === next.skillsRevision
    && a.phase === b.phase
    && a.error === b.error
    && a.credentials === b.credentials
    && a.desktopOnline === b.desktopOnline
    && a.hasConnectedContent === b.hasConnectedContent
    && a.presence === b.presence
    && a.capabilities === b.capabilities
    && previous.catalog.sessions === next.catalog.sessions
    && previous.catalog.workspaces === next.catalog.workspaces
    && previous.catalog.models === next.catalog.models
    && previous.catalog.unreadSessions === next.catalog.unreadSessions
    && previous.catalog.catalogSync === next.catalog.catalogSync
    && previous.catalog.approvalTier === next.catalog.approvalTier
    && previous.catalog.sandboxAvailable === next.catalog.sandboxAvailable
    && previous.catalog.sandboxCode === next.catalog.sandboxCode
    && previous.conversation.modelId === next.conversation.modelId
    && previous.conversation.thinkingLevel === next.conversation.thinkingLevel
    && previous.conversation.sessionUsage === next.conversation.sessionUsage;
}

/** Whether the open conversation's transcript changed. */
function sameTimelineData(previous: RuntimeEntry | undefined, next: RuntimeEntry): boolean {
  if (!previous) return false;
  // The leaves of the timeline controller, not the controller object: it is
  // rebuilt on every render, so comparing it would fire this signal on every
  // one of them — and fire it for reasons that have nothing to do with the
  // transcript.
  const a = previous.timeline;
  const b = next.timeline;
  return a.timeline === b.timeline
    && a.timelinePending === b.timelinePending
    && a.timelineSyncStatus === b.timelineSyncStatus
    && a.timelineError === b.timelineError
    && a.canLoadOlderTimeline === b.canLoadOlderTimeline
    && a.loadingOlderTimeline === b.loadingOlderTimeline
    && a.olderHistoryFailed === b.olderHistoryFailed;
}

/**
 * One paired desktop's runtime host.
 *
 * It renders nothing: it *is* the runtime — the hook that owns that machine's
 * connection, catalogue, conversations and outbox — and its only output is the
 * entry it publishes so the provider (and through it the screens) can read it.
 */
function DesktopRuntimeHost({
  desktopId,
  label,
  icon,
  active,
  foreground,
  onUnpaired,
  verifyPairing,
  publish,
}: {
  desktopId: string;
  label: string;
  icon?: string;
  active: boolean;
  foreground: boolean;
  onUnpaired(desktopId: string): void;
  verifyPairing(credentials: RemoteCredentials): Promise<unknown>;
  publish(entry: RuntimeEntry): void;
}): null {
  // Stable per machine: the connection rebuilds its callbacks when this one
  // changes, and a fresh arrow on every render would reconnect on every render.
  const unpaired = useCallback(() => onUnpaired(desktopId), [desktopId, onUnpaired]);
  const runtime = useDesktopRuntime({
    desktopId,
    active,
    foreground,
    label,
    onUnpaired: unpaired,
    verifyPairing,
  });
  const entry = useMemo<RuntimeEntry>(
    () => (icon ? { ...runtime, icon } : { ...runtime }),
    [icon, runtime],
  );
  // After every render, so the stored functions are always this render's; a
  // publish that changed nothing the provider reads does not re-render it.
  useEffect(() => { publish(entry); });
  return null;
}

/**
 * How often a machine the user is not reading is reached again after giving up.
 *
 * The open machine is the user's to retry — that is what the on-screen retry is
 * for, and a connection that has given up does not reopen itself. A machine
 * nobody is looking at has no such affordance, and a desktop that was asleep
 * when the phone started would read offline until the app restarted, which is
 * the one outcome "every paired desktop stays connected" cannot have.
 */
const MACHINE_BACKSTOP_MS = 60_000;

function machineStatusOf(entry: RuntimeEntry): MachineStatus {
  const { phase, desktopOnline, presence } = entry.connection;
  return peerMachineStatus({
    failed: phase === "failed" || phase === "revoked" || phase === "stopped",
    ready: phase === "ready",
    online: desktopOnline,
    agentAvailable: presence?.agentAvailable !== false,
  });
}

/** The runtime a machine has, or nothing while its host has not published. */
function runtimeFor(registry: Map<string, RuntimeEntry>, desktopId: string | null | undefined) {
  return desktopId ? registry.get(desktopId) ?? null : null;
}

const noop = async () => undefined;

/**
 * The verbs of a machine the phone holds no runtime for.
 *
 * Each one answers nothing: a page for a machine that is not there finds no
 * data and stays disabled, rather than reading the machine the phone *is*
 * reading.
 */
const NO_MANAGEMENT = {
  getDesktopSettings: noop as never,
  updateDesktopSettings: noop as never,
  listSettingsModels: noop as never,
  listInstalledSkills: noop as never,
  listAvailableSkills: noop as never,
  installSkill: noop as never,
  uninstallSkill: noop as never,
  suggestSkill: noop as never,
  skillRecoToday: noop as never,
  recordSkillReco: noop as never,
  listProviders: noop as never,
  updateBuiltinProvider: noop as never,
  upsertCustomProvider: noop as never,
  deleteCustomProvider: noop as never,
  listTasks: noop as never,
  getTask: noop as never,
  createTask: noop as never,
  updateTask: noop as never,
  deleteTask: noop as never,
  setTaskEnabled: noop as never,
  runTask: noop as never,
  listTaskRuns: noop as never,
};

/**
 * The machine-scoped half of the remote surface: everything that belongs to one
 * paired desktop rather than to the app around it.
 *
 * A function of a *machine*, not of the open one: the provider builds the
 * surface the screens read from the machine they read, and the surface of every
 * other paired machine from this same function (see `machineScopes`). What is
 * left out is what no machine owns — which machines are paired, which one is
 * being read, and pairing itself.
 *
 * `app` carries the app-level fallbacks a machine with no runtime falls back to:
 * a phone that has not booted yet is "booting" rather than "connecting", and the
 * pairing's own error is the app's to report.
 */
function machineSurface(
  runtime: RuntimeEntry | null,
  app: { phase: ConnectionPhase; error: string | null; clearError(): void },
) {
  const connection = runtime?.connection ?? null;
  const presence = connection?.presence ?? null;
  const agentAvailable = presence?.agentAvailable !== false;
  return {
    ...(runtime?.management ?? NO_MANAGEMENT),
    captureConversationOperation: runtime?.captureConversationOperation ?? (() => () => false),
    desktopSettingsRevision: runtime?.desktopSettingsRevision ?? 0,
    skillsRevision: runtime?.skillsRevision ?? 0,
    phase: connection?.phase ?? app.phase,
    error: connection?.error ?? app.error,
    presence,
    desktopOnline: connection?.desktopOnline ?? false,
    desktopReadable: (connection?.desktopOnline ?? false) && presence?.control !== "standby",
    hasConnectedContent: connection?.hasConnectedContent ?? false,
    catalogSync: runtime?.catalog.catalogSync ?? {
      sessions: "idle", workspaces: "idle", models: "idle", settings: "idle",
    },
    agentAvailable,
    connectionPresentation: buildConnectionPresentation({
      phase: connection?.phase ?? app.phase,
      desktopOnline: connection?.desktopOnline ?? false,
      agentAvailable,
      desktopDisconnected: presence?.disconnected,
      desktopDisconnectReason: presence?.reason,
      error: connection?.error ?? app.error,
    }),
    sessions: runtime?.catalog.sessions ?? [],
    workspaces: runtime?.catalog.workspaces ?? [],
    unreadSessions: runtime?.catalog.unreadSessions ?? new Set<string>(),
    models: runtime?.catalog.models ?? [],
    selectedSessionId: runtime?.selectedSessionId ?? "",
    selectedTitle: runtime?.selectedTitle ?? "",
    draft: runtime?.draft ?? false,
    draftMode: runtime?.draftMode ?? "chat",
    draftWorkspaceId: runtime?.draftWorkspaceId ?? "",
    streaming: runtime?.streaming ?? false,
    compacting: runtime?.compacting ?? false,
    modelId: runtime?.conversation.modelId ?? "",
    thinkingLevel: runtime?.conversation.thinkingLevel ?? DEFAULT_THINKING_LEVEL,
    sessionUsage: runtime?.conversation.sessionUsage ?? null,
    refreshSessionUsage: runtime?.conversation.refreshSessionUsage ?? noop,
    approvalTier: runtime?.catalog.approvalTier ?? "off",
    sandboxAvailable: runtime?.catalog.sandboxAvailable ?? false,
    sandboxCode: runtime?.catalog.sandboxCode,
    busy: runtime?.busy ?? false,
    fileTransferSupported: connection?.fileTransferSupported ?? false,
    capabilities: connection?.capabilities ?? new Set<string>(),
    refreshSessions: runtime?.catalog.refreshSessions ?? noop,
    refreshWorkspaces: runtime?.catalog.refreshWorkspaces ?? noop,
    refreshModels: runtime?.catalog.refreshModels ?? noop,
    selectSession: runtime
      ? runtime.conversation.selectSession
      : async () => undefined,
    retryTimeline: runtime?.timeline.retryTimeline ?? noop,
    loadOlderTimeline: runtime?.timeline.loadOlderTimeline ?? (async () => false),
    reloadTimeline: runtime?.timeline.reloadTimeline ?? (() => undefined),
    newConversation: runtime?.conversation.newConversation ?? noop,
    closeConversation: runtime?.closeConversation ?? (() => undefined),
    sendMessage: runtime?.outbox.sendMessage ?? noop,
    compactContext: runtime?.conversation.compactContext ?? (async () => {
      throw new Error("not_connected");
    }),
    awaitCompactionOutcome: runtime?.timeline.awaitCompactionOutcome
      ?? (async (_sessionId: string, _operationId: string): Promise<import("./types").CompactionOutcome> => ({ status: "timeout" })),
    listSkills: runtime?.conversation.listSkills ?? (async () => []),
    listSessionFiles: runtime?.conversation.listSessionFiles ?? (async () => {
      throw new Error("not_connected");
    }),
    resolveToolCallTarget: runtime?.conversation.resolveToolCallTarget ?? (async () => null),
    prepareAttachment: runtime?.conversation.prepareAttachment ?? (async () => {
      throw new Error("not_connected");
    }),
    cachedAttachment: runtime?.conversation.cachedAttachment ?? (() => null),
    downloadAttachment: runtime?.conversation.downloadAttachment ?? (async () => {
      throw new Error("not_connected");
    }),
    abort: runtime?.conversation.abort ?? noop,
    setModel: runtime?.conversation.setModel ?? noop,
    setThinkingLevel: runtime?.conversation.setThinkingLevel ?? noop,
    setApprovalTier: runtime?.conversation.setApprovalTier ?? noop,
    rename: runtime?.catalog.rename ?? noop,
    generateTitle: runtime?.catalog.generateTitle ?? (async () => {
      throw new Error("not_connected");
    }),
    deleteSession: runtime?.conversation.deleteSession ?? noop,
    deleteWorkspace: runtime?.conversation.deleteWorkspace ?? noop,
    createWorkspace: runtime?.catalog.createWorkspace ?? (async () => {
      throw new Error("not_connected");
    }),
    setSessionPinned: runtime?.catalog.setSessionPinned ?? noop,
    setWorkspacePinned: runtime?.catalog.setWorkspacePinned ?? noop,
    renameWorkspace: runtime?.catalog.renameWorkspace ?? noop,
    forkConversation: runtime?.conversation.forkConversation ?? noop,
    decideApproval: runtime?.conversation.decideApproval ?? noop,
    clearError: connection?.clearError ?? app.clearError,
    continueRun: runtime?.outbox.continueRun ?? noop,
    /**
     * Ask this desktop to let this device drive it.
     *
     * The machine does not move control for a device that merely connected;
     * this is the only call that does, and the UI gates it behind the user
     * reading who currently holds it.
     */
    takeControl: async (): Promise<void> => {
      const client = runtime?.connection.clientRef.current;
      if (!client) throw new Error("not_connected");
      const response = await client.request<{ control?: string }>({ type: "takeover" }, "list");
      if (response.success === false) throw new Error(response.error ?? "takeover_failed");
      // The answer is the machine handing control over; presence only repeats
      // it on its next tick. Record it now so the button that asked does not
      // reappear while the page still reads the previous heartbeat.
      if (response.data?.control === "active" && runtime?.connection.clientRef.current === client)
        runtime.connection.noteControlActive();
    },
  };
}

/**
 * The app's remote surface over *every* paired desktop.
 *
 * The phone runs one runtime per paired desktop (see `useDesktopRuntime`), and
 * exactly one of them is the one the screens read — the *active* desktop, a
 * selection rather than a connection. Selecting another desktop is therefore a
 * re-render, not a reconnection: that machine's runtime has been connected,
 * catalogued and (if it had a conversation open) reading it the whole time.
 *
 * The provider owns what belongs to the *app*: which desktops are paired, which
 * one is being read, and the process-wide lifecycle (foreground, network) that
 * drives every runtime at once.
 */
export function RemoteProvider({ children }: PropsWithChildren) {
  const networkAvailableRef = useRef<boolean | null>(null);
  const pairing = usePairedDesktops({ networkAvailableRef });
  /**
   * The runtimes, by desktop id. A ref because they hold sockets and callbacks;
   * `registryVersion` is what makes a change in them reach a render.
   */
  const registryRef = useRef(new Map<string, RuntimeEntry>());
  const [registryVersion, setRegistryVersion] = useState(0);
  /**
   * The transcript's own signal, kept apart from the machines'.
   *
   * A streamed delta changes what the transcript draws and nothing else, so it
   * must not re-render the machine list — nor, through it, every control on the
   * screen. Two signals are what make "the transcript follows the stream" and
   * "the composer does not re-render 100 times a second" both true.
   */
  const [timelineVersion, setTimelineVersion] = useState(0);

  const publish = useCallback((entry: RuntimeEntry) => {
    const previous = registryRef.current.get(entry.desktopId);
    // Stored whether or not the data changed: the functions in it are this
    // render's, and the screens should call the current ones.
    registryRef.current.set(entry.desktopId, entry);
    if (!samePublishedData(previous, entry)) setRegistryVersion((value) => value + 1);
    if (!sameTimelineData(previous, entry)) setTimelineVersion((value) => value + 1);
  }, []);

  // A machine that is no longer paired has no runtime: its entry is dropped
  // here so nothing can reach a removed desktop. The list the screens read is
  // filtered by the pairing set as well (below), so the removal needs no
  // re-render of its own — the pairing change already causes one.
  const pairedIds = useMemo(
    () => new Set(pairing.desktops.map((desktop) => desktop.desktopId)),
    [pairing.desktops],
  );
  useEffect(() => {
    for (const desktopId of [...registryRef.current.keys()]) {
      if (!pairedIds.has(desktopId)) registryRef.current.delete(desktopId);
    }
  }, [pairedIds]);

  // Depends on the callbacks it uses, never on the pairing surface itself: that
  // object is rebuilt on every render, and an unstable identity here would
  // re-register the app's AppState and network listeners on every one of them.
  const { drainRevokes } = pairing;
  const recoverAll = useCallback((reason: RecoveryReason) => {
    void drainRevokes();
    for (const entry of registryRef.current.values()) void entry.connection.recover(reason);
  }, [drainRevokes]);

  const setRuntimesActive = useCallback((active: boolean) => {
    for (const entry of registryRef.current.values()) entry.setAppActive(active);
  }, []);

  const { foreground } = useAppLifecycle({
    networkAvailableRef,
    setRuntimesActive,
    recoverAll,
  });

  const activeDesktopId = pairing.activeDesktopId;
  const activeRuntime = runtimeFor(registryRef.current, activeDesktopId);
  /**
   * The open desktop's credentials.
   *
   * From its runtime once it has them; before that, from the claim that just
   * created them — a desktop the user has just paired is readable at once, and
   * the screens must not fall back to "nothing selected" while its runtime is
   * still booting.
   */
  const credentials = activeRuntime?.connection.credentials
    ?? (pairing.claimedCredentials?.expectedDesktopId === activeDesktopId
      ? pairing.claimedCredentials
      : null);

  /**
   * Wait for a machine's runtime to have a live client.
   *
   * Normally there is nothing to wait for — the machine is connected because it
   * always is — but a desktop that was asleep when the user tapped its row is
   * still handshaking, and opening a conversation against no client would
   * silently do nothing. Bounded, so a machine that never answers fails the tap
   * rather than hanging it.
   */
  const waitForRuntime = useCallback(async (desktopId: string): Promise<RuntimeEntry | null> => {
    const deadline = Date.now() + 15_000;
    for (;;) {
      const entry = registryRef.current.get(desktopId);
      if (entry?.connection.clientRef.current) return entry;
      if (Date.now() >= deadline) return null;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }, []);

  /**
   * A desktop ended this phone's pairing.
   *
   * The desktop is the authority on its own pairings: once it says the device
   * was unpaired — by its own notice, or by refusing to renew the credentials —
   * there is nothing left for the phone to retry. Keeping the record produced a
   * "已断开 / 重试" screen against a pairing that could never come back, so the
   * record is dropped and the user is told why; the picker (or the pairing
   * screen, if it was the only one) is what is left, which is the one honest
   * answer.
   */
  const { forgetDesktop } = pairing;
  const forgetUnpaired = useCallback((desktopId: string) => {
    const name = pairing.desktops.find(desktop => desktop.desktopId === desktopId)?.name
      || desktopId;
    void forgetDesktop(desktopId).then(() => {
      AppAlert.alert(i18n.t("desktops.unpairedTitle"), i18n.t("desktops.unpaired", { name }));
    });
  }, [forgetDesktop, pairing.desktops]);

  const { selectDesktop } = pairing;
  const switchDesktop = useCallback(
    async (desktopId: string) => {
      await selectDesktop(desktopId);
    },
    [selectDesktop],
  );

  const openSessionOnDesktop = useCallback(
    async (desktopId: string, sessionId: string) => {
      const target = desktopId || activeDesktopId;
      if (!target) return;
      // A machine that is no longer paired has no runtime to wait for: the
      // caller gets the same answer an unreachable machine gives, without the
      // wait. A machine the phone is still *booting* into is a different state —
      // its runtime is about to appear — so the check waits for the pairing to
      // have been read.
      if (!pairing.booting && !pairedIds.has(target)) throw new Error("desktop_unavailable");
      const entry = await waitForRuntime(target);
      // A machine that cannot be reached must say so: the caller reports it,
      // rather than the row appearing to open onto an empty conversation.
      if (!entry) throw new Error("desktop_unavailable");
      // Moved only once the conversation has somewhere to open: a tap naming a
      // machine that never answers must not leave the screens reading a machine
      // with nothing on it.
      if (target !== activeDesktopId) await selectDesktop(target);
      await entry.conversation.selectSession(sessionId);
    },
    [activeDesktopId, pairedIds, pairing.booting, selectDesktop, waitForRuntime],
  );

  /**
   * Open a new conversation on the machine that will run it.
   *
   * The named machine is looked up in the registry on every call rather than
   * taken from this render's active runtime: the screens call this through a
   * value built from the render that drew them, so a screen that moved the phone
   * first and then asked for the conversation would otherwise build its draft on
   * the machine it left — the provider's own test measures both halves.
   */
  const newConversationOn = useCallback(
    async (desktopId: string, mode: "chat" | "workspace" = "chat", workspaceId = "") => {
      if (!registryRef.current.has(desktopId)) throw new Error("desktop_unavailable");
      const entry = await waitForRuntime(desktopId);
      if (!entry) throw new Error("desktop_unavailable");
      // Built on the machine that will run it and *before* the phone moves, so a
      // refused creation leaves the reader where they were rather than on a
      // machine with nothing on it. Each runtime reads its own catalogue as it
      // connects (models included), so the draft's own machine is the one whose
      // models it defaults to.
      await entry.conversation.newConversation(mode, workspaceId);
      if (desktopId !== activeDesktopId) await selectDesktop(desktopId);
    },
    [activeDesktopId, registryRef, selectDesktop, waitForRuntime],
  );

  const { reconnect: rebuildDesktop } = pairing;
  /**
   * Reach the machines nobody is looking at again.
   *
   * Rebuilding is per machine and only for a connection that gave up or a
   * desktop that announced it was going away: a revoked pairing is the user's
   * to re-establish, and re-running a rejected handshake every minute is noise.
   * Stopped here means the same as failed — the runtime is terminal either way,
   * and (unlike the open machine) nothing on screen offers the way back.
   */
  useEffect(() => {
    if (!foreground) return;
    const timer = setInterval(() => {
      for (const entry of registryRef.current.values()) {
        if (entry.desktopId === activeDesktopId) continue;
        const phase = entry.connection.phase;
        if (phase === "failed" || phase === "stopped") rebuildDesktop(entry.desktopId);
      }
    }, MACHINE_BACKSTOP_MS);
    return () => clearInterval(timer);
  }, [activeDesktopId, foreground, rebuildDesktop]);

  const disconnectDesktop = useCallback((desktopId: string) => {
    const entry = registryRef.current.get(desktopId);
    const phase = entry?.connection.phase;
    // A connection that gave up is not re-opened from the inside; a live one is
    // probed. The two failures need different answers, so the button picks.
    if (phase === "failed" || phase === "stopped" || phase === "revoked") rebuildDesktop(desktopId);
    else void entry?.connection.recover("request-failure");
  }, [rebuildDesktop]);

  const reconnect = useCallback(async (desktopId?: string) => {
    const target = desktopId ?? activeDesktopId;
    if (!target) return;
    disconnectDesktop(target);
  }, [activeDesktopId, disconnectDesktop]);

  /**
   * Remove the open pairing.
   *
   * The machine is told first, best effort and bounded: the local removal is
   * what the user asked for, and it must not wait on a desktop that may be
   * gone. Its runtime unmounts with the pairing, which closes the socket.
   */
  const { removeDesktop: removeStoredDesktop, unpair: unpairDesktop } = pairing;
  const unpair = useCallback(async () => {
    const entry = registryRef.current.get(activeDesktopId ?? "");
    await entry?.notifyUnpair();
    await unpairDesktop();
  }, [activeDesktopId, unpairDesktop]);

  const removeDesktop = useCallback(
    async (desktopId: string) => {
      if (desktopId === activeDesktopId) return unpair();
      await registryRef.current.get(desktopId)?.notifyUnpair();
      await removeStoredDesktop(desktopId);
    },
    [activeDesktopId, removeStoredDesktop, unpair],
  );

  /**
   * The machines other than the open one, in the shape the merged list reads:
   * their catalogues, their own commands, and their status.
   *
   * Every one of them is a live runtime, so a command here goes down that
   * machine's own connection — the same path the desktop app uses — and the
   * status is that runtime's own judged presence rather than a poll.
   */
  const catalogs = useMemo(() => {
    const others = [...registryRef.current.values()].filter(
      (entry) => entry.desktopId !== activeDesktopId && pairedIds.has(entry.desktopId),
    );
    const catalogEntries: CatalogEntry[] = others.map((entry) => ({
      desktopId: entry.desktopId,
      label: entry.label,
      // The same judgement the picker's row and the badge make, from the same
      // leaves: a machine that is not `connected` keeps its last catalogue here,
      // but the merged list does not draw it (see `CatalogEntry.status`).
      status: machineStatusOf(entry),
      sessions: entry.catalog.sessions,
    }));
    // The open machine contributes through its own runtime rather than a second
    // fetch: its catalogue is the same state the screens are reading.
    if (activeRuntime)
      catalogEntries.push({
        desktopId: activeRuntime.desktopId,
        label: activeRuntime.label,
        // The open machine is judged like any other, so a list that is showing
        // several machines cannot hold one machine's rows under a status its own
        // header contradicts.
        status: machineStatusOf(activeRuntime),
        sessions: activeRuntime.catalog.sessions,
      });
    const machines: MachineSummary[] = others.map((entry) => ({
      desktopId: entry.desktopId,
      label: entry.label,
      ...(entry.icon ? { icon: entry.icon } : {}),
      active: false,
      holder: entry.connection.presence?.holder ?? null,
      status: machineStatusOf(entry),
      ...(entry.connection.presence?.control ? { control: entry.connection.presence.control } : {}),
    }));
    return {
      catalogs: catalogEntries,
      machines,
      refresh: async () => {
        await Promise.allSettled(
          [...registryRef.current.values()].map((entry) => entry.catalog.refreshSessions()),
        );
      },
      reconnect: () => {
        for (const entry of registryRef.current.values()) {
          const phase = entry.connection.phase;
          if (phase === "failed" || phase === "stopped" || phase === "revoked")
            pairing.reconnect(entry.desktopId);
          else void entry.connection.recover("request-failure");
        }
      },
      /**
       * Ask one paired machine to let this phone drive it.
       *
       * Resolved through that machine's own runtime, not the open one: the
       * picker can offer the action for a device the phone is not currently
       * reading, and sending it down the open machine's connection would ask
       * the wrong desktop to hand over.
       */
      takeControlFor: async (desktopId: string): Promise<void> => {
        const entry = registryRef.current.get(desktopId);
        const client = entry?.connection.clientRef.current;
        if (!entry || !client) throw new Error("not_connected");
        const pairId = entry.connection.credentials?.pairId;
        const response = await client.request<{ control?: string }>({ type: "takeover" }, "list");
        if (response.success === false) throw new Error(response.error ?? "takeover_failed");
        // Controlled here rather than through presence: the machine answered
        // after it moved control, and the row that offered the action stops
        // being a standby row as soon as it is taken. Waiting for the next
        // heartbeat brought the button back for a tick, which reads as the
        // takeover failing and then working.
        //
        // Read the runtime back out of the registry: runtimes publish a new
        // entry object on every render, so the one captured above may be gone
        // by now. What must still be true is that this is the same pairing on
        // the same connection that sent the command.
        const latest = registryRef.current.get(desktopId);
        if (
          response.data?.control === "active"
          && latest
          && latest.connection.credentials?.pairId === pairId
          && latest.connection.clientRef.current === client
        ) latest.connection.noteControlActive();
      },
      commandsFor: (desktopId: string): DesktopCommands | null => {
        const entry = registryRef.current.get(desktopId);
        // Only from `ready`: while the machine's agent restarts it refuses, and
        // offering an action it will not take is what makes a menu lie.
        if (!entry || entry.connection.phase !== "ready") return null;
        const client = (): RemoteClient | null => entry.connection.clientRef.current;
        const accepted = async (): Promise<void> => {
          await entry.catalog.refreshSessions().catch(() => undefined);
        };
        return {
          capabilities: entry.connection.capabilities,
          renameSession: async (sessionId, name) => {
            await client()?.request(
              { type: "set_session_name", sessionId, name: name.trim() }, sessionId);
            await accepted();
          },
          generateTitle: async (sessionId, language) => {
            const response = await client()?.request<{ title?: string }>(
              { type: "generate_session_title", sessionId, mode: language },
              sessionId,
              65_000,
            );
            const title = response?.data?.title?.trim();
            if (!title) throw new Error("title_generation_failed");
            return title;
          },
          setSessionPinned: async (sessionId, threadId, pinned) => {
            await client()?.request(
              { type: "set_session_pinned", sessionId, threadId, pinned }, sessionId);
            await accepted();
          },
          deleteSession: async (sessionId, threadId) => {
            await client()?.request({ type: "delete_session", sessionId, threadId }, sessionId);
            await accepted();
          },
          renameWorkspace: async (workspaceId, name) => {
            await client()?.request(
              { type: "rename_workspace", workspaceId, name: name.trim() }, "list");
            await accepted();
          },
          setWorkspacePinned: async (workspaceId, pinned) => {
            await client()?.request({ type: "set_workspace_pinned", workspaceId, pinned }, "list");
            await accepted();
          },
          deleteWorkspace: async (workspaceId) => {
            await client()?.request({ type: "delete_workspace", workspaceId }, "list");
            await accepted();
          },
        };
      },
      readWorkspaces: async (desktopId: string): Promise<RemoteWorkspace[]> => {
        const client = registryRef.current.get(desktopId)?.connection.clientRef.current;
        if (!client) throw new Error("machine_unavailable");
        const response = await client.request<{ workspaces?: RemoteWorkspace[] }>(
          { type: "list_workspaces" },
          "list",
        );
        return Array.isArray(response.data?.workspaces) ? response.data.workspaces : [];
      },
    };
    // The dependency list is deliberately field-by-field: the pairing surface
    // and the runtime object are rebuilt on every render, so depending on
    // either would rebuild this on every streamed delta — which, through the
    // controls that read it, is a whole screen of re-renders per token.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the fields, not the objects they hang off
  }, [
    activeDesktopId,
    activeRuntime?.catalog.sessions,
    activeRuntime?.desktopId,
    activeRuntime?.label,
    pairedIds,
    pairing.reconnect,
    registryVersion,
  ]);

  useEffect(() => {
    if (activeRuntime?.connection.phase === "ready" && activeRuntime.connection.credentials)
      void prepareTaskNotifications();
  }, [activeRuntime?.connection.credentials, activeRuntime?.connection.phase]);

  const value = useMemo<RemoteControls>(() => {
    // A selected desktop whose runtime has not reported yet is *starting*, not
    // absent: the picker is for "which desktop", asked because there is none.
    const fallbackPhase: ConnectionPhase = pairing.booting
      ? "booting"
      : activeDesktopId
        ? "connecting"
        : "unpaired";
    return {
      // Everything that belongs to a machine comes from the machine the screens
      // read; everything that belongs to the app is added here. The split is what
      // lets the same machine surface be built for *another* machine (see
      // `machineScopes` below) instead of being written twice.
      ...machineSurface(activeRuntime, {
        phase: fallbackPhase,
        error: pairing.error,
        clearError: pairing.clearError,
      }),
      credentials,
      desktops: pairing.desktops,
      switchDesktop,
      renameDesktop: pairing.renameDesktop,
      labelDesktop: pairing.labelDesktop,
      removeDesktop,
      pair: pairing.pair,
      reconnect,
      unpair,
      openSessionOnDesktop,
      newConversationOn,
      catalogs,
    };
    // Field by field rather than "the runtime object": the runtime is rebuilt on
    // every render, so depending on it would hand every control a new value on
    // every streamed text delta — which is exactly the re-render storm the two
    // contexts exist to avoid. The timeline's own contents are deliberately not
    // here; they travel through the transcript context instead.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the leaves, not the objects they hang off
  }, [
    activeDesktopId,
    activeRuntime?.busy,
    credentials,
    activeRuntime?.catalog.approvalTier,
    activeRuntime?.catalog.catalogSync,
    activeRuntime?.catalog.createWorkspace,
    activeRuntime?.catalog.generateTitle,
    activeRuntime?.catalog.models,
    activeRuntime?.catalog.refreshModels,
    activeRuntime?.catalog.refreshSessions,
    activeRuntime?.catalog.refreshWorkspaces,
    activeRuntime?.catalog.rename,
    activeRuntime?.catalog.renameWorkspace,
    activeRuntime?.catalog.sandboxAvailable,
    activeRuntime?.catalog.sandboxCode,
    activeRuntime?.catalog.sessions,
    activeRuntime?.catalog.setSessionPinned,
    activeRuntime?.catalog.setWorkspacePinned,
    activeRuntime?.catalog.unreadSessions,
    activeRuntime?.catalog.workspaces,
    activeRuntime?.captureConversationOperation,
    activeRuntime?.closeConversation,
    activeRuntime?.compacting,
    activeRuntime?.connection.capabilities,
    activeRuntime?.connection.clearError,
    activeRuntime?.connection.credentials,
    activeRuntime?.connection.desktopOnline,
    activeRuntime?.connection.error,
    activeRuntime?.connection.fileTransferSupported,
    activeRuntime?.connection.hasConnectedContent,
    activeRuntime?.connection.phase,
    activeRuntime?.connection.presence,
    activeRuntime?.conversation.abort,
    activeRuntime?.conversation.cachedAttachment,
    activeRuntime?.conversation.compactContext,
    activeRuntime?.conversation.decideApproval,
    activeRuntime?.conversation.deleteSession,
    activeRuntime?.conversation.deleteWorkspace,
    activeRuntime?.conversation.downloadAttachment,
    activeRuntime?.conversation.forkConversation,
    activeRuntime?.conversation.listSessionFiles,
    activeRuntime?.conversation.listSkills,
    activeRuntime?.conversation.modelId,
    activeRuntime?.conversation.newConversation,
    activeRuntime?.conversation.prepareAttachment,
    activeRuntime?.conversation.refreshSessionUsage,
    activeRuntime?.conversation.resolveToolCallTarget,
    activeRuntime?.conversation.selectSession,
    activeRuntime?.conversation.sessionUsage,
    activeRuntime?.conversation.setApprovalTier,
    activeRuntime?.conversation.setModel,
    activeRuntime?.conversation.setThinkingLevel,
    activeRuntime?.conversation.thinkingLevel,
    activeRuntime?.desktopSettingsRevision,
    activeRuntime?.draft,
    activeRuntime?.draftMode,
    activeRuntime?.draftWorkspaceId,
    activeRuntime?.management,
    activeRuntime?.outbox.continueRun,
    activeRuntime?.outbox.sendMessage,
    activeRuntime?.selectedSessionId,
    activeRuntime?.selectedTitle,
    activeRuntime?.skillsRevision,
    activeRuntime?.streaming,
    activeRuntime?.timeline.awaitCompactionOutcome,
    activeRuntime?.timeline.loadOlderTimeline,
    activeRuntime?.timeline.reloadTimeline,
    activeRuntime?.timeline.retryTimeline,
    catalogs,
    openSessionOnDesktop,
    newConversationOn,
    pairing.booting,
    pairing.clearError,
    pairing.desktops,
    pairing.error,
    pairing.labelDesktop,
    pairing.pair,
    pairing.renameDesktop,
    reconnect,
    removeDesktop,
    switchDesktop,
    unpair,
  ]);

  /**
   * Every paired machine's own surface, by desktop id — what `DesktopScope`
   * reads.
   *
   * The value above is the surface of the machine the phone is reading. This is
   * the same surface built for each machine, so a screen that names a machine
   * (a device page) reads *that* machine: its connection state, its settings,
   * its models, its tasks. A machine whose runtime has not reported yet gets the
   * empty surface, never the open machine's — a page that quietly read the
   * machine the phone happens to be on is how a write lands somewhere the user
   * was not looking.
   *
   * Rebuilt whenever any machine's published state changes, because the machine
   * a page names is usually *not* the one the phone is reading, and its own
   * changes still have to reach that page.
   */
  const machineScopes = useMemo(() => {
    const scopes = new Map<string, RemoteControls>();
    for (const desktop of pairing.desktops) {
      scopes.set(desktop.desktopId, {
        ...value,
        ...machineSurface(registryRef.current.get(desktop.desktopId) ?? null, {
          // Not the open machine: a paired machine that has not reported yet is
          // starting, and its errors are its own.
          phase: "connecting",
          error: null,
          clearError: pairing.clearError,
        }),
      });
    }
    return scopes;
    // `registryVersion` is not read in here: the runtimes live in a ref, and this
    // is the signal that one of them published something new. It is load-bearing
    // — a machine the phone is *not* reading still has to update the page that
    // names it — so it stays, and the rule is told why.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the change signal, not a value
  }, [pairing.clearError, pairing.desktops, registryVersion, value]);

  const timelineValue = useMemo<TimelineContextValue>(() => (
    activeRuntime
      ? {
        timeline: activeRuntime.timeline.timeline,
        timelinePending: activeRuntime.timeline.timelinePending,
        timelineSyncStatus: activeRuntime.timeline.timelineSyncStatus,
        timelineError: activeRuntime.timeline.timelineError,
        canLoadOlderTimeline: activeRuntime.timeline.canLoadOlderTimeline,
        loadingOlderTimeline: activeRuntime.timeline.loadingOlderTimeline,
        olderHistoryFailed: activeRuntime.timeline.olderHistoryFailed,
      }
      : {
        timeline: EMPTY_TIMELINE,
        timelinePending: false,
        timelineSyncStatus: "idle" as const,
        timelineError: null,
        canLoadOlderTimeline: false,
        loadingOlderTimeline: false,
        olderHistoryFailed: false,
      }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the leaves, not the controller object
  ), [
    activeRuntime?.timeline.timeline,
    activeRuntime?.timeline.timelinePending,
    activeRuntime?.timeline.timelineSyncStatus,
    activeRuntime?.timeline.timelineError,
    activeRuntime?.timeline.canLoadOlderTimeline,
    activeRuntime?.timeline.loadingOlderTimeline,
    activeRuntime?.timeline.olderHistoryFailed,
    timelineVersion,
  ]);

  return (
    <RemoteContext.Provider value={value}>
      {/* One level above the timeline so a device page can read a machine other
          than the open one; the rest of the app never sees it change. */}
      <MachineScopesContext.Provider value={machineScopes}>
        <RemoteTimelineContext.Provider value={timelineValue}>
          {pairing.desktops.map((desktop) => (
            <DesktopRuntimeHost
              key={`${desktop.desktopId}:${pairing.generations[desktop.desktopId] ?? 0}`}
              desktopId={desktop.desktopId}
              label={desktop.name ?? desktop.desktopId}
              icon={desktop.icon}
              active={desktop.desktopId === activeDesktopId}
              foreground={foreground}
              onUnpaired={forgetUnpaired}
              verifyPairing={pairing.verifyPairing}
              publish={publish}
            />
          ))}
          {children}
        </RemoteTimelineContext.Provider>
      </MachineScopesContext.Provider>
    </RemoteContext.Provider>
  );
}

const EMPTY_TIMELINE: TimelineState = {
  items: [],
  seenEvents: new Set(),
  currentRunId: null,
  streaming: false,
  liveRuns: new Map(),
};

/**
 * Read one machine by id.
 *
 * Everything under this provider sees the named machine rather than the one the
 * phone is reading: its connection, its settings, its models and providers, its
 * tasks. A device page uses it because the page *is about that machine*, and
 * every paired desktop keeps its own live runtime — so the machine's settings
 * are readable there without the phone being pointed at it, and without a
 * connection step for the user to perform.
 *
 * The alternative — a page that reads whichever machine happens to be the open
 * one — can silently write to a machine the user was not looking at, which is
 * the failure this exists to close.
 *
 * An id with no scope (a machine that is no longer paired) renders its children
 * unchanged: the list that offered the id is the source of truth, and a page for
 * a machine that has just been removed is on its way out anyway.
 */
export function DesktopScope({ desktopId, children }: { desktopId: string; children?: ReactNode }) {
  const scopes = useContext(MachineScopesContext);
  const machine = scopes?.get(desktopId);
  return machine
    ? <RemoteContext.Provider value={machine}>{children}</RemoteContext.Provider>
    : children ?? null;
}

/** Navigation and controls should not subscribe to every streamed text delta. */
export function useRemoteControls(): RemoteControls {
  const value = useContext(RemoteContext);
  if (!value) throw new Error("useRemote must be used inside RemoteProvider");
  return value;
}

/** Compatibility surface for consumers that actually render the transcript. */
export function useRemote(): RemoteContextValue {
  const controls = useRemoteControls();
  const timeline = useContext(RemoteTimelineContext);
  if (!timeline) throw new Error("useRemote must be used inside RemoteProvider");
  return useMemo(() => ({ ...controls, ...timeline }), [controls, timeline]);
}
