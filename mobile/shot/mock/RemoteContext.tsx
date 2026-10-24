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
import type { PropsWithChildren, ReactNode } from "react";
import type {
  AvailableSkill,
  DesktopSettings,
  DownloadInfo,
  HistoryAttachment,
  HistoryEntry,
  InstalledSkill,
  MobileAttachment,
  PairedDesktop,
  RemoteCredentials,
  RemoteModel,
  RemoteSessionUsage,
  SessionFileListing,
} from "../../src/remote/types";
import { createContext, useContext, useMemo, useState } from "react";
import { connectionPresentation as buildConnectionPresentation } from "../../src/remote/connectionPresentation";
import { applyStreamEvents, timelineFromEntries } from "@future-os/remote-client/projection";
import { leanRenderMode, leanRenderTimeline, leanTargetForToolCall } from "./leanRender";
import {
  compactResumeEntries,
  demoCredentials,
  demoDesktops,
  demoEntries,
  demoFiles,
  demoInstalledSkills,
  demoAvailableSkills,
  demoModels,
  demoProviders,
  demoRefreshedSessionUsage,
  demoSessionUsage,
  demoSkills,
  demoTaskDeps,
  demoTaskDetail,
  demoTaskRuns,
  demoTasks,
  demoUnpricedSessionUsage,
  demoWorkspaces,
  multiTurnEntries,
  manyRecentSessions,
  multiTurnOlderEntries,
  recentOnlySessions,
  sessions,
} from "./data";
import { seedComposerDraft, withApprovalRequest } from "./smallestType";

type MockValue = Record<string, any>;

const warned = new Set<string>();

/**
 * Seed the standing fixtures before the first render: a screen reads its stored
 * state in a mount effect, and a seed that landed after it would be missed.
 * Both are opt-in through the URL (`?draft=attachments`, `?approval=1`).
 */
seedComposerDraft(window.location.search);

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
  // `?single=1` leaves the phone with one pairing. It is the state where the
  // list has no other machine to fall back to, so `?held=1&single=1` is the one
  // that asks for the takeover page instead of the conversations.
  const singlePairing = new URLSearchParams(window.location.search).get("single") === "1";
  const pairedDesktops = singlePairing ? demoDesktops.slice(0, 1) : demoDesktops;
  const [selectedSessionId, setSelectedSessionId] = useState("");
  const [draft, setDraft] = useState(false);
  const [desktopId, setDesktopId] = useState(pairedDesktops[0]?.desktopId ?? "");
  // The phone's own labels for its desktops (name and icon): stored state here
  // so the device page and the list it sits on move together, as they do once
  // the real store answers.
  const [desktops, setDesktops] = useState<PairedDesktop[]>(pairedDesktops);
  const [sessionPins, setSessionPins] = useState<Record<string, boolean>>({});
  const [workspacePins, setWorkspacePins] = useState<Record<string, boolean>>({});
  const [usageOverride, setUsageOverride] = useState<RemoteSessionUsage | null>(null);
  const [providers, setProviders] = useState(demoProviders);
  const [desktopSettings, setDesktopSettings] = useState<DesktopSettings>({
    autoUpgradeSkills: true,
    autoTitleFirstTurn: true,
    autoConnectRemote: true,
    // The desktop always reports this one; omitting it would make the harness
    // exercise the phone's older-desktop fallback instead of the real path.
    skillRecommend: true,
    hiddenModels: [],
  });

  // A cheap stand-in for the real per-session timeline: the demo conversation
  // is fully projected, any other session shows its first turns.
  // `?compacted=1` additionally folds the exact wire sequence a manual
  // compaction produces (standalone phase, stamped with the session's last run
  // id) through the *real* reducer, so the composer state in a capture is the
  // state the app would have.
  const delayedCompaction = new URLSearchParams(window.location.search).get("compactionDelay") === "1";
  // `?peers=1` makes the second paired desktop answer as well — see
  // `peerMachines` below. It is what puts the new-conversation dialog on its
  // machine step, which only exists while more than one machine can take the
  // conversation.
  const withPeers = new URLSearchParams(window.location.search).get("peers") === "1";
  // `?recent=1` adds the states the recency page exists to separate: a second
  // run producing output, and a run blocked on the reader's approval. Every
  // other capture keeps the catalogue it had.
  const withRecent = new URLSearchParams(window.location.search).get("recent") === "1";
  // `?recentMany=1` adds forty older conversations, which is what the recency
  // page's window is for: it draws the newest and offers the rest.
  const withRecentMany = new URLSearchParams(window.location.search).get("recentMany") === "1";
  const catalogue = [
    ...(withRecent ? recentOnlySessions : []),
    ...sessions,
    ...(withRecentMany ? manyRecentSessions : []),
  ];
  const [compactionFinished, setCompactionFinished] = useState(false);
  const scriptedCompaction = compactionFinished || new URLSearchParams(window.location.search).get("compacted") === "1";
  // `?multiTurnOlder=1` keeps one page of history above the multi-turn
  // conversation: the question-nav ↑ can then page it in for real.
  const withOlder = new URLSearchParams(window.location.search).get("multiTurnOlder") === "1";
  // `?connection=dropped` makes the paired desktop answer as one that has gone
  // away: the phone keeps its pairing and its cached catalogue, but the badge
  // and its sheet have to say the connection is gone and offer a retry. Every
  // other capture runs against a live desktop, so this is opt-in.
  const connectionDropped = new URLSearchParams(window.location.search).get("connection") === "dropped";
  // `?held=1` makes another paired device the one driving this desktop: the
  // phone is connected and in standby, so it can show who holds the machine and
  // offer to take it back. This is the state connecting must NOT silently
  // resolve, so it needs a fixture of its own.
  const heldByAnother = new URLSearchParams(window.location.search).get("held") === "1";
  const [olderLoaded, setOlderLoaded] = useState(false);
  // `?leanRender=<mode>` renders a lean (or, for the control, full) lane slice
  // instead of the demo conversation — see shot/mock/leanRender.ts.
  const renderMode = leanRenderMode(window.location.search);
  const baseTimeline = useMemo(() => {
    // `?compactHistory=1` swaps in the history of a run that compacted
    // mid-turn, so a capture can assert the reply after the divider survives
    // the durable projection (the schemaVersion 3 path). `?multiTurn=1` swaps in
    // a several-turn conversation, which is what the controls that jump between
    // questions need in order to have somewhere to jump.
    const search = new URLSearchParams(window.location.search);
    const renderFixture = leanRenderTimeline(renderMode);
    if (renderFixture) return renderFixture;
    const historyEntries = search.get("compactHistory") === "1"
      ? compactResumeEntries
      : search.get("multiTurn") === "1"
        ? olderLoaded
          ? [...multiTurnOlderEntries, ...multiTurnEntries]
          : multiTurnEntries
        : demoEntries;
    const history = timelineFromEntries(historyEntries as unknown as HistoryEntry[]);
    // `?approval=1` docks a run blocked on the reader's approval (see
    // `smallestType.ts`); every other capture gets the demo timeline untouched.
    if (!scriptedCompaction) return withApprovalRequest(search.toString(), history);
    const manualRun = "run_1";
    return withApprovalRequest(search.toString(), applyStreamEvents(history, [
      { type: "compaction_started", runId: manualRun, idx: 900, data: JSON.stringify({ operation_id: "cmp_shot", trigger: "manual", phase: "standalone" }) },
      { type: "compaction_committed", runId: manualRun, idx: 901, data: JSON.stringify({ operation_id: "cmp_shot", checkpoint_id: "cp_shot", trigger: "manual", phase: "standalone", tokens_before: 33064, tokens_after: 9250 }) },
    ]));
  }, [scriptedCompaction, renderMode, olderLoaded]);
  const timeline = useMemo(
    () => (selectedSessionId === "" || selectedSessionId === "sess_dopamine_review"
      ? baseTimeline
      : { ...baseTimeline, items: baseTimeline.items.slice(0, 2) }),
    [baseTimeline, selectedSessionId],
  );

  // Switching desktops moves the whole pairing, as the real `switchDesktop`
  // does: screens read `credentials.pairId` to tell which machine is current,
  // and the settings modal is keyed by it. The real app reconnects nothing —
  // every paired desktop keeps its own runtime — so a switch here is the
  // pointer this state holds; the harness has one catalogue, not one per
  // machine, which is what its `catalogs` below reflects.
  const pairedDesktop = desktops.find(desktop => desktop.desktopId === desktopId);
  const credentials: RemoteCredentials = {
    ...demoCredentials,
    expectedDesktopId: desktopId,
    pairId: pairedDesktop?.pairId ?? demoCredentials.pairId,
  };

  // The second machine, live or absent — see the `machines` note below. Its
  // commands are the same answers the open machine gives, which is what a
  // paired desktop really offers its client, so a capture can act on a peer's
  // row exactly as it can on the phone's own.
  const peerId = demoDesktops[1]?.desktopId ?? "";
  // `?held=1` also makes the *other* machine the one that is driving, which is
  // what the picker has to show: the open machine connected and in standby, the
  // peer holding it. Without a second machine there is nothing to pick between.
  const peerMachines = withPeers && peerId
    ? [{
        desktopId: peerId,
        label: demoDesktops[1]!.name,
        active: false,
        status: "connected" as const,
        control: "active" as const,
      }]
    : [];
  const peerCommands = {
    capabilities: new Set<string>(),
    renameSession: async () => {},
    generateTitle: async () => {},
    setSessionPinned: async () => {},
    deleteSession: async () => {},
    renameWorkspace: async () => {},
    setWorkspacePinned: async () => {},
    deleteWorkspace: async () => {},
  };

  const value = withFallback({
    // Connection
    phase: connectionDropped ? "stopped" : "ready",
    error: null,
    credentials,
    desktops,
    presence: {
      online: !connectionDropped,
      pairId: credentials.pairId,
      bridgeInstanceId: "bridge_1",
      lastHeartbeatTs: BOOT_TIME,
      agentAvailable: true,
      control: heldByAnother ? "standby" : "active",
      holder: heldByAnother
        ? { pairId: "pair_lab_desktop", name: "实验室的 MacBook Air", kind: "desktop" }
        : null,
      ...(connectionDropped ? { disconnected: true } : {}),
    },
    // Taking over is an action, not a state: the mock answers it the way the
    // desktop does (success), and the banner disappearing is what a capture's
    // assertion can see.
    takeControl: async () => undefined,
    desktopOnline: !connectionDropped,
    hasConnectedContent: true,
    agentAvailable: true,
    connectionPresentation: buildConnectionPresentation(
      connectionDropped
        ? {
            // The desktop stopped answering and said so: the pairing survives,
            // the cached content is still readable, and the only way forward is
            // to ask again. This is the state the badge and its sheet describe.
            phase: "stopped",
            desktopOnline: false,
            agentAvailable: true,
            desktopDisconnected: true,
          }
        : { phase: "ready", desktopOnline: true, agentAvailable: true },
    ),

    // Catalogue
    catalogSync: { state: "ready", epoch: "epoch_1" },
    sessions: catalogue.map(session =>
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
    /**
     * Every *other* machine's catalogue, as the merged list reads it.
     *
     * The real provider reads these from the runtime each paired desktop has;
     * the harness has one machine's data, so this is the open machine's own
     * entry and no peers — the list then renders its workspace tree exactly as
     * it does on a phone paired to one desktop.
     */
    catalogs: {
      catalogs: [{
        desktopId,
        label: pairedDesktop?.name ?? desktopId,
        // The harness always renders a machine that is answering: its catalogue
        // is the one this file builds, not a mirror of a connection that could
        // be down. The merged list draws only `connected` machines.
        status: "connected" as const,
        sessions,
      }],
      // `?peers=1` gives the second paired desktop a live connection, the state
      // that puts the dialog below on its machine step; every other capture
      // wants the ordinary one-machine phone this file describes above.
      machines: peerMachines,
      refresh: async () => {},
      reconnect: () => {},
      commandsFor: (id: string) => (withPeers && id === peerId ? peerCommands : null),
      takeControlFor: async () => undefined,
      readWorkspaces: async () => demoWorkspaces,
    },
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
    canLoadOlderTimeline: withOlder && !olderLoaded,
    loadingOlderTimeline: false,
    loadOlderTimeline: async (): Promise<false | string[]> => {
      if (!withOlder || olderLoaded) return false;
      setOlderLoaded(true);
      // In conversation order, matching the remote contract: the *host* reverses
      // its own descending scan before replying, so a backward page is
      // oldest-first and its first id is the row the page-up jump lands on.
      return multiTurnOlderEntries.map(entry => entry.id);
    },
    streaming: baseTimeline.streaming,
    compacting: scriptedCompaction ? baseTimeline.compacting === true : new URLSearchParams(window.location.search).get("compacting") === "1",

    // Composer settings
    modelId: "future/deepseek-v4-pro",
    thinkingLevel: "medium",
    // The amount in the top bar, refreshed by every state read in the app.
    sessionUsage: usageOverride ?? (selectedSessionId === "sess_chat_pvalue" ? demoUnpricedSessionUsage : demoSessionUsage),
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
      "provider_management_v1",
      "workspace_pinning_v1",
      "compaction_v1",
      "fork_v1",
      "tasks_v1",
      // What the shipping client declares (src/remote/client.ts); the lean
      // fixtures are the feed a desktop serves a client that asked for it.
      "lean_events_v1",
    ]),

    // Actions the harness drives for real
    selectSession: async (sessionId: string) => {
      setDraft(false);
      setSelectedSessionId(sessionId);
    },
    // The app re-reads the session when the usage sheet opens. Answer with a
    // larger figure, the way a session that spent more since the last read
    // would — the top bar and the sheet share this state, so a capture shows
    // both moving to the refreshed amount.
    refreshSessionUsage: async () => {
      setUsageOverride(demoRefreshedSessionUsage);
    },
    newConversation: async () => setDraft(true),
    // Creating on a machine that was named outright. The harness holds one
    // desktop's catalogue, so the machine is what moves and the draft opens there
    // — the same two things the real provider does (see `newConversationOn`).
    newConversationOn: async (desktopId: string) => {
      if (desktopId) setDesktopId(desktopId);
      setDraft(true);
    },
    closeConversation: () => {
      setDraft(false);
      setSelectedSessionId("");
    },
    // A lean history row with no target fetches its command on open: the app
    // goes through the bridge (`get_tool_call_args`), the harness answers from
    // the full-feed fixture with the shipping target derivation.
    resolveToolCallTarget: async (toolCallId: string) => leanTargetForToolCall(toolCallId),
    switchDesktop: async (id: string) => setDesktopId(id),
    // Local-only labels, like the real store: a blank value clears the field.
    renameDesktop: async (id: string, name: string) => setDesktops(current => current.map(item =>
      item.desktopId === id ? { desktopId: item.desktopId, pairId: item.pairId, ...(name.trim() ? { name: name.trim() } : {}), ...(item.icon ? { icon: item.icon } : {}) } : item)),
    labelDesktop: async (id: string, patch: { name?: string; icon?: string }) => setDesktops(current => current.map((item) => {
      if (item.desktopId !== id) return item;
      const name = patch.name === undefined ? item.name : patch.name.trim() || undefined;
      const icon = patch.icon === undefined ? item.icon : patch.icon || undefined;
      return { desktopId: item.desktopId, pairId: item.pairId, ...(name ? { name } : {}), ...(icon ? { icon } : {}) };
    })),
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
    // Providers and models, as the desktop reports them. Mutations are applied
    // locally so the harness shows the same list a real write would produce.
    listProviders: async () => providers,
    updateBuiltinProvider: async (provider: any) => {
      setProviders(current => ({
        ...current,
        builtin: current.builtin.map(item => item.id === provider.id
          ? { ...item, hasApiKey: provider.updateApiKey ? !!provider.apiKey : item.hasApiKey, baseUrl: provider.baseUrl ?? item.baseUrl }
          : item),
      }));
      return providers;
    },
    upsertCustomProvider: async (provider: any) => {
      const entry = { id: provider.id, name: provider.name || provider.id, api: provider.api, baseUrl: provider.baseUrl, hasApiKey: providers.custom.find(item => item.id === provider.id)?.hasApiKey ?? false, models: provider.models };
      setProviders(current => ({
        ...current,
        custom: current.custom.some(item => item.id === provider.id)
          ? current.custom.map(item => (item.id === provider.id ? entry : item))
          : [...current.custom, entry],
      }));
      return providers;
    },
    deleteCustomProvider: async (providerId: string) => {
      setProviders(current => ({ ...current, custom: current.custom.filter(item => item.id !== providerId) }));
      return providers;
    },
    // Tasks: the phone manages the paired desktop's tasks (list rows carry no
    // prompt body, matching the wire contract; the detail does).
    listTasks: async () => demoTasks,
    getTask: async () => demoTaskDetail,
    listTaskRuns: async () => demoTaskRuns,
    listTaskDeps: async () => demoTaskDeps,
    createTask: async () => demoTaskDetail,
    updateTask: async () => demoTaskDetail,
    deleteTask: async () => undefined,
    setTaskEnabled: async () => demoTaskDetail,
    runTask: async () => demoTaskDetail,
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
    // Fork is a Desktop write; the harness only needs the affordance to exist.
    forkConversation: async () => undefined,
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
    canLoadOlderTimeline: withOlder && !olderLoaded,
    loadingOlderTimeline: false,
  }), [timeline, withOlder, olderLoaded]);

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

/**
 * The machine a device page names.
 *
 * The real `DesktopScope` hands its subtree the named machine's own surface,
 * read from that machine's runtime. The harness holds *one* machine's data (one
 * catalogue, one settings snapshot, one `desktopId`), so this is the identity:
 * the page renders its machine's real rows, over the demo desktop the phone is
 * on, with no connection step in between — which is the state being captured.
 */
export function DesktopScope({ children }: { desktopId: string; children?: ReactNode }) {
  return children ?? null;
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
