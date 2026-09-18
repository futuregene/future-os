/**
 * Screenshot harness stand-in for `src/remote/RemoteContext`.
 *
 * Supplies the same context surface the real provider does ("useRemote" /
 * "useRemoteControls") from a fixed dataset, so every screen, dialog and
 * composer under `src/` renders its real implementation while the Expo web
 * build runs with no desktop, no NATS connection and no pairing.
 *
 * Only the data source and the transport are replaced. Selection, draft state
 * and pinning are real local state here, so tapping through the UI in the
 * harness behaves like the app.
 *
 * Unknown members return a no-op async function (with a console warning) so a
 * new field added to the real context never hard-fails a capture run.
 */
import type { PropsWithChildren } from "react";
import type {
  AvailableSkill,
  DesktopSettings,
  DownloadInfo,
  HistoryAttachment,
  HistoryEntry,
  InstalledSkill,
  MobileAttachment,
  RemoteCredentials,
  RemoteModel,
  SessionFileListing,
} from "../../src/remote/types";
import { createContext, useContext, useMemo, useState } from "react";
import { connectionPresentation as buildConnectionPresentation } from "../../src/remote/connectionPresentation";
import { applyStreamEvents, timelineFromEntries } from "../../src/remote/projection";
import {
  demoCredentials,
  demoDesktops,
  demoEntries,
  demoFiles,
  demoInstalledSkills,
  demoAvailableSkills,
  demoModels,
  demoSkills,
  demoWorkspaces,
  sessions,
} from "./data";

type MockValue = Record<string, any>;

const warned = new Set<string>();

/** Fixed at module load: the value only needs to look recent. */
const BOOT_TIME = Date.now();

/**
 * Fill in members the mock does not define: a capture run should degrade to a
 * no-op rather than crash the whole screen. Every miss is logged once so the
 * fix (adding a real value here) stays obvious.
 */
function withFallback(value: MockValue): MockValue {
  return new Proxy(value, {
    get(target, property) {
      if (typeof property === "string" && property in target)
        return target[property];
      if (typeof property === "string" && !warned.has(property)) {
        warned.add(property);
        console.warn(`[shot] mock RemoteContext has no "${property}"; returning a no-op`);
      }
      return async () => undefined;
    },
  });
}

export function RemoteProvider({ children }: PropsWithChildren) {
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [draft, setDraft] = useState(false);
  const [desktopId, setDesktopId] = useState(demoDesktops[0]?.desktopId ?? "");
  const [sessionPins, setSessionPins] = useState<Record<string, boolean>>({});
  const [workspacePins, setWorkspacePins] = useState<Record<string, boolean>>({});
  const [desktopSettings, setDesktopSettings] = useState<DesktopSettings>({
    autoUpgradeSkills: true,
    autoTitleFirstTurn: true,
    autoConnectRemote: false,
    hiddenModels: [],
  });

  // A cheap stand-in for the real per-session timeline: the demo conversation
  // is fully projected, any other session shows its first turns.
  // `?compacted=1` additionally folds the exact wire sequence a manual
  // compaction produces (standalone phase, stamped with the session's last run
  // id) through the *real* reducer, so the composer state in a capture is the
  // state the app would have.
  const delayedCompaction = new URLSearchParams(window.location.search).get("compactionDelay") === "1";
  const [compactionFinished, setCompactionFinished] = useState(false);
  const scriptedCompaction = compactionFinished || new URLSearchParams(window.location.search).get("compacted") === "1";
  const baseTimeline = useMemo(() => {
    const history = timelineFromEntries(demoEntries as unknown as HistoryEntry[]);
    if (!scriptedCompaction) return history;
    const manualRun = "run_1";
    return applyStreamEvents(history, [
      { type: "compaction_started", runId: manualRun, idx: 900, data: JSON.stringify({ operation_id: "cmp_shot", trigger: "manual", phase: "standalone" }) },
      { type: "compaction_committed", runId: manualRun, idx: 901, data: JSON.stringify({ operation_id: "cmp_shot", checkpoint_id: "cp_shot", trigger: "manual", phase: "standalone", tokens_before: 33064 }) },
    ]);
  }, [scriptedCompaction]);
  const timeline = useMemo(
    () => (selectedSessionId === "" || selectedSessionId === "sess_dopamine_review"
      ? baseTimeline
      : { ...baseTimeline, items: baseTimeline.items.slice(0, 2) }),
    [baseTimeline, selectedSessionId],
  );

  const credentials: RemoteCredentials = {
    ...demoCredentials,
    expectedDesktopId: desktopId,
  };

  const value = withFallback({
    // Connection
    phase: "ready",
    error: null,
    credentials,
    desktops: demoDesktops,
    presence: {
      online: true,
      pairId: credentials.pairId,
      bridgeInstanceId: "bridge_1",
      lastHeartbeatTs: BOOT_TIME,
      agentAvailable: true,
    },
    desktopOnline: true,
    hasConnectedContent: true,
    agentAvailable: true,
    connectionPresentation: buildConnectionPresentation({
      phase: "ready",
      desktopOnline: true,
      agentAvailable: true,
    }),

    // Catalogue
    catalogSync: { state: "ready", epoch: "epoch_1" },
    sessions: sessions.map(session =>
      sessionPins[session.sessionId] === undefined
        ? session
        : { ...session, pinned: sessionPins[session.sessionId] },
    ),
    workspaces: demoWorkspaces.map(workspace =>
      workspacePins[workspace.id] === undefined
        ? workspace
        : { ...workspace, pinned: workspacePins[workspace.id] },
    ),
    unreadSessions: new Set<string>(["sess_dopamine_review"]),
    models: demoModels,
    skillsRevision: 0,
    desktopSettingsRevision: 0,

    // Conversation
    selectedSessionId,
    selectedTitle: sessions.find(session => session.sessionId === selectedSessionId)?.title ?? "",
    draft,
    draftMode: "chat",
    draftWorkspaceId: "",
    timeline,
    timelinePending: false,
    timelineSyncStatus: "idle",
    timelineError: null,
    canLoadOlderTimeline: false,
    loadingOlderTimeline: false,
    streaming: baseTimeline.streaming,
    compacting: scriptedCompaction ? baseTimeline.compacting === true : new URLSearchParams(window.location.search).get("compacting") === "1",

    // Composer settings
    modelId: "future/deepseek-v4-pro",
    thinkingLevel: "medium",
    approvalTier: "off",
    sandboxAvailable: true,
    busy: false,
    fileTransferSupported: true,
    capabilities: new Set<string>([
      "file_transfer_v1",
      "prompt_receipt_v1",
      "selective_events_v1",
      "session_files_v1",
      "skills_v1",
      "desktop_settings_v1",
      "skill_management_v1",
      "workspace_pinning_v1",
      "compaction_v1",
    ]),

    // Actions the harness drives for real
    selectSession: async (sessionId: string) => {
      setDraft(false);
      setSelectedSessionId(sessionId);
    },
    newConversation: async () => setDraft(true),
    closeConversation: () => {
      setDraft(false);
      setSelectedSessionId("");
    },
    switchDesktop: async (id: string) => setDesktopId(id),
    setSessionPinned: async (sessionId: string, _threadId: string, pinned: boolean) =>
      setSessionPins(current => ({ ...current, [sessionId]: pinned })),
    setWorkspacePinned: async (workspaceId: string, pinned: boolean) =>
      setWorkspacePins(current => ({ ...current, [workspaceId]: pinned })),

    // Desktop settings & skills pages
    getDesktopSettings: async () => desktopSettings,
    updateDesktopSettings: async (patch: Partial<DesktopSettings>) => {
      setDesktopSettings(current => ({ ...current, ...patch }));
      return { ...desktopSettings, ...patch };
    },
    listSettingsModels: async (): Promise<RemoteModel[]> => demoModels,
    listInstalledSkills: async (): Promise<InstalledSkill[]> => demoInstalledSkills,
    listAvailableSkills: async (): Promise<AvailableSkill[]> => demoAvailableSkills,
    installSkill: async () => undefined,
    uninstallSkill: async () => true,

    // Files
    listSessionFiles: async (): Promise<SessionFileListing> => demoFiles,
    listSkills: async () => demoSkills,

    // Attachments: hand back an inline preview image so image rendering
    // (auto-load, zoom, preview overlay) can be captured without a desktop.
    prepareAttachment: async (_attachment: HistoryAttachment, variant?: string): Promise<DownloadInfo> => ({
      transferId: "t1",
      name: "effect-size.png",
      mimeType: "image/png",
      size: 43_309,
      contentHash: "shot",
      previewKind: "image",
      variant: variant === "original" ? "original" : "preview",
      chunkBytes: 1024,
    }),
    cachedAttachment: () => null,
    // Absolute URL: the capture script serves the demo figures over HTTP
    // (see `assetsPort` in scripts/screenshots/scenarios.json).
    downloadAttachment: async () => ({ uri: "http://127.0.0.1:7392/effect-size.png" }) as any,

    rename: async () => undefined,
    generateTitle: async () => "多巴胺与风险决策：任务不确定性下的效应方向",
    // Manual compaction: the harness shows the request being accepted, then
    // reports the outcome the divider cannot (here: nothing to compact).
    compactContext: async () => {
      if (delayedCompaction) await new Promise(resolve => setTimeout(resolve, 1500));
      return { sessionId: selectedSessionId, operationId: "cmp_shot" };
    },
    awaitCompactionOutcome: async (_session: string, _operation: string, _timeout?: number, signal?: AbortSignal) => {
      if (!delayedCompaction) return { status: "unchanged", alreadyCompacted: false, reused: false };
      // No started event: the real hook must show pending before the mock ACK,
      // and keep it until the scenario explicitly completes the operation.
      return new Promise(resolve => {
        const cleanup = () => {
          window.removeEventListener("shot:complete-compaction", complete);
          signal?.removeEventListener("abort", cancel);
        };
        const complete = () => {
          cleanup(); setCompactionFinished(true); resolve({ status: "committed" });
        };
        const cancel = () => { cleanup(); resolve({ status: "cancelled" }); };
        window.addEventListener("shot:complete-compaction", complete);
        signal?.addEventListener("abort", cancel, { once: true });
        if (signal?.aborted) cancel();
      });
    },
  });

  const timelineValue = useMemo(() => ({
    timeline,
    timelinePending: false,
    timelineSyncStatus: "idle",
    timelineError: null,
    canLoadOlderTimeline: false,
    loadingOlderTimeline: false,
  }), [timeline]);

  return (
    <ControlsContext.Provider value={value}>
      <TimelineContext.Provider value={timelineValue}>{children}</TimelineContext.Provider>
    </ControlsContext.Provider>
  );
}

const ControlsContext = createContext<MockValue | null>(null);
const TimelineContext = createContext<MockValue | null>(null);

export type RemoteControls = MockValue;

/** Navigation and controls should not subscribe to streamed text deltas. */
export function useRemoteControls(): any {
  const value = useContext(ControlsContext);
  if (!value)
    throw new Error("useRemote must be used inside RemoteProvider");
  return value;
}

/** Compatibility surface for consumers that render the transcript. */
export function useRemote(): any {
  const controls = useRemoteControls();
  const timeline = useContext(TimelineContext);
  if (!timeline)
    throw new Error("useRemote must be used inside RemoteProvider");
  return useMemo(() => ({ ...controls, ...timeline }), [controls, timeline]);
}

export type { MobileAttachment };
