/**
 * A browser-side stand-in for the desktop's Tauri (Rust) backend.
 *
 * Answers the same command names the real backend does, from an in-memory
 * dataset (`./data`), so the real frontend renders realistic content.
 *
 * Coverage follows what the shipped UI actually calls. When the app starts
 * asking for a command that is missing here, the console prints
 * `[mock] UNHANDLED COMMAND <name>` — add a handler for it and re-capture.
 */
import type { MockEntry } from "./data";
import {
  appSettings,
  availableSkills,
  buildInfo,
  compactResumeEntries,
  demoFileContents,
  entriesByThread,
  HOME,
  installedSkills,
  models,
  pendingApproval,
  providersView,
  refreshedSessionUsage,
  remoteConversationState,
  remoteFileListing,
  remotePeers,
  remoteSessions,
  remoteToolArgs,
  remoteWorkspaces,
  requestedThread,
  reviewFiles,
  runs,
  sessionUsage,
  taskDeps,
  taskRuns,
  tasks,
  threads,
  toolCalls,
  toolOutputs,
  unpricedSessionUsage,
  workspaceFiles,
  workspaces,
} from "./data";
import { replayPage, replayState, sessionEntries, streamRemoteRun } from "./remoteStream";

// The console output doubles as the "what is missing" list while iterating on
// the harness, which is why this one log call is intentional.
// eslint-disable-next-line no-console
const log = (...args: unknown[]) => console.log("[mock]", ...args);

let settings: typeof appSettings = { ...appSettings };

/**
 * Threads the harness has put into a *running* state.
 *
 * A plain page load has nothing running (`list_streaming_thread_ids` is empty),
 * so the rail's reserved indicator slot could only ever show its placeholder:
 * the spinning ring, the waiting-approval badge and the completed/failed dots
 * were all unreachable in a capture. The seeding helpers below are the entry
 * points the harness calls from `shot/main.tsx` — harness-only, never part of
 * the fixture's default state.
 */
let harnessStreamingThreadIds: string[] = [];
/** Thread ids `?pinned=` added to the fixture's own pin (see `HarnessState`). */
let harnessPinnedThreadIds: string[] = [];
let harnessUpdate = "";

export interface HarnessState {
  /** Update screen fixtures, including recovery after the main UI stops. */
  update?: string | null;
  /** Thread ids the agent reports as streaming (drives the row's running ring). */
  streaming?: string[];
  /**
   * Thread ids whose finished run is unread (drives the completed/failed dot).
   *
   * Written straight into `sessionStorage` because that is where the app keeps
   * the set, and it is read at mount — seeding it from a React effect would be
   * too late. Each id also needs a terminal run in `runs`, since the dot is
   * gated on the run's own status.
   */
  unread?: string[];
  /**
   * Thread ids to pin for this capture (drives the Vibe panel's membership).
   *
   * The fixture ships one pinned conversation, which is what the rail's own
   * pinned section needs to be reachable. A panel with a single cell would say
   * nothing about the grid, so a capture that opens the panel pins a few more —
   * here, and only for that page load.
   */
  pinned?: string[];
}

export function seedHarnessState(state: HarnessState) {
  harnessUpdate = state.update ?? "";
  // Recorded, not applied: `threadList` is built below this function, so the flag
  // is stamped on when the list is read (see `list_threads`). Stamping is
  // idempotent, which is what makes that safe to do per read.
  harnessStreamingThreadIds = state.streaming ?? [];
  harnessPinnedThreadIds = state.pinned ?? [];
  if (state.unread) {
    try {
      sessionStorage.setItem("future.unreadThreads", JSON.stringify(state.unread));
    }
    catch {
      // Best-effort, as in the app's own hook.
    }
  }
}

/**
 * The keys the fixture's host has hidden.
 *
 * Read from the same mutable `settings` the mock answers `get_desktop_settings`
 * with, so a scenario that flips a switch and one that lists models cannot
 * disagree about what is hidden.
 */
function hiddenModelKeys(): string[] {
  const hidden: unknown = (settings as { hiddenModels?: unknown }).hiddenModels;
  return Array.isArray(hidden) ? hidden.filter((item): item is string => typeof item === "string") : [];
}

/**
 * Whether this page asked for a host whose *reads* take a moment (`?remoteSlowRead=1`).
 *
 * A host answers in a few milliseconds here, so the state a reader is in *while*
 * a read is in flight never reaches a capture — and that is exactly the state a
 * control with nothing to show lives in: one that filled its own label into the
 * value slot looked identical to one that said it was loading. This flag is the
 * fixture for that moment; nothing else about the host changes.
 */
function slowReadsEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("remoteSlowRead") === "1";
}

/** One of the host's reads, held back to the fixture's `?remoteSlowRead=1`. */
function slow<T>(answer: T): T | Promise<T> {
  if (!slowReadsEnabled())
    return answer;
  return new Promise(resolve => setTimeout(resolve, 1500, answer));
}
const threadList = threads.map(thread => ({ ...thread }));
const workspaceList = workspaces.map(workspace => ({ ...workspace }));
/** Staged session-history pages, read back in 64 KiB chunks (see `begin_session_history_transfer`). */
const historyTransfers = new Map<string, Uint8Array>();
let historyTransferSeq = 0;
/** The chunk size `historyTransfer.ts` insists on. */
const HISTORY_CHUNK_BYTES = 64 * 1024;
/** How many times each thread's agent state has been read (see the handler). */
const stateReads = new Map<string, number>();

/** Seed app settings before the app boots (`?settings=` in `main.tsx`). */
export function patchSettings(patch: Record<string, unknown>) {
  settings = { ...settings, ...patch };
  return settings;
}

export function entriesForThread(threadId: string): MockEntry[] {
  // `?compactHistory=1` swaps in the history of a run that compacted mid-turn,
  // so a capture can assert the reply after the divider survives the durable
  // projection (the schemaVersion 3 path).
  if (
    typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("compactHistory") === "1"
  ) {
    return compactResumeEntries;
  }
  return entriesByThread[threadId] ?? [];
}

/**
 * Demo files that exist as real PNGs under `shot/assets/`; a path maps to the
 * URL the browser can load.
 */
export function assetUrl(path: string): string {
  const known: Record<string, string> = {
    "effect-size.png": "/shot/assets/effect-size.png",
    "forest-plot.png": "/shot/assets/forest-plot.png",
  };
  return known[path.split("/").pop() ?? ""] ?? path;
}

const EMPTY_REMOTE_STATUS = {
  agentAvailable: true,
  phase: "stopped",
  reason: null,
  recovery: null,
  natsUrl: "tls://www.future-os.cn:4222",
  pairId: "",
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "desktop_C76HV2R4TKPGCC55",
  desktopPublicKey: "MCowBQYDK2VwAyEA...",
  webUrl: null,
  webLanUrl: null,
  warningCode: null,
};

/**
 * The host side with several paired devices (`?remoteDevices=<mode>`).
 *
 * A plain load answers `EMPTY_REMOTE_STATUS`, which is the single-device page
 * the host section used to be. The device list only renders when the backend
 * reports `devices`, so a capture of it needs a fixture that has them — and the
 * three rows it can show are exactly the three states the list distinguishes:
 * the device in control, one connected but standing by (it stays connected so
 * it can take back over without re-pairing), and one that is only paired.
 */
function remoteStatusWithDevices(mode: string): Record<string, unknown> {
  const now = Math.floor(Date.now() / 1000);
  const devices = [
    {
      pairId: "pair_8F3A21C4D5E64B77",
      phase: "ready",
      control: "active",
      connected: true,
      client: { name: "iPhone 17 Pro", kind: "mobile" },
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    },
    {
      pairId: "pair_2D9E4B1A7C3F8052",
      phase: "ready",
      control: "standby",
      connected: true,
      client: { name: "实验室的 MacBook Air", kind: "desktop" },
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    },
    {
      pairId: "pair_6B7C0E5A9D21438F",
      phase: "stopped",
      control: "standby",
      connected: false,
      client: { name: "iPad mini", kind: "mobile" },
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    },
  ];

  if (mode === "reconnect") {
    // The bridge has lost its connection and is retrying: the same device list,
    // with the transport down. `reason: "network"` is what turns the status row
    // into "网络异常" plus a "重新连接" action.
    return {
      ...EMPTY_REMOTE_STATUS,
      phase: "reconnecting",
      reason: "network",
      pairId: devices[0]!.pairId,
      devices: devices.map(device => ({ ...device, connected: false, control: "standby" })),
    };
  }

  if (mode === "waiting") {
    // The invitation for a fourth device is on screen: the list carries the
    // pending row, and the QR block below it is drawn from that row.
    const pending = {
      pairId: "pair_C41F7A08E62B4D19",
      phase: "connecting" as const,
      control: "standby" as const,
      connected: false,
      client: null,
      pairingCode: "futureos://remote/pair?code=mock&desktopId=desktop_C76HV2R4TKPGCC55",
      pairingCodeExpiresAt: now + 540,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    };
    return {
      ...EMPTY_REMOTE_STATUS,
      phase: "ready",
      pairId: devices[0]!.pairId,
      devices: [...devices, pending],
      pairingCode: pending.pairingCode,
      pairingCodeExpiresAt: pending.pairingCodeExpiresAt,
    };
  }

  return {
    ...EMPTY_REMOTE_STATUS,
    phase: "ready",
    pairId: devices[0]!.pairId,
    devices,
  };
}

/** `?remoteDevices=multi|waiting|reconnect`, or `""` for the plain host page. */
function remoteDevicesMode(): string {
  if (typeof window === "undefined")
    return "";
  return new URLSearchParams(window.location.search).get("remoteDevices") ?? "";
}

/**
 * The paired computers this machine connects *out* to, with the states a screen
 * has to draw.
 *
 * `?peers=held` makes the first one a machine another paired device is driving:
 * its row says who holds it and refuses Settings until somebody takes it over.
 * The plain list is two peers — one reachable, one not — because "a host that
 * cannot be reached" is the state the merged list has to survive.
 */
function peerList(): typeof remotePeers {
  if (typeof window === "undefined")
    return remotePeers;
  if (new URLSearchParams(window.location.search).get("peers") !== "held")
    return remotePeers;
  return remotePeers.map((peer, index) => index === 0
    ? {
        ...peer,
        control: "standby" as const,
        holder: { pairId: "pair_phone", name: "LNA-AL00", kind: "mobile" },
      }
    : { ...peer, control: "active" as const });
}

/** Command name → implementation. Anything missing goes to the fallback. */
/** Mutable copy so a capture can show a task being toggled or deleted. */
const taskList = tasks.map(task => ({ ...task }));

const handlers: Record<string, (args: any) => unknown> = {
  // ── App bootstrap ──────────────────────────────────────────────────────
  initialize_app_store: () => null,
  app_build_info: () => buildInfo,
  get_app_settings: () => settings,
  update_app_settings: args => patchSettings(args?.input ?? {}),
  get_future_environment: () => ({ environment: "production", platformUrl: "https://api.future-os.cn" }),
  set_future_environment: () => ({ environment: "production", platformUrl: "https://api.future-os.cn" }),
  probe_sandbox: () => ({ available: true, tier: "sandbox", platform: "macos" }),
  // The desktop gates all account/provider work behind this handshake; without
  // it the app never leaves its splash ("请稍候").
  get_agent_status: () => ({
    phase: harnessUpdate.startsWith("stopped-") ? "stopping" : "ready",
    ...(harnessUpdate.startsWith("stopped-") ? { shutdown: { operationId: "shutdown-raw-id", reason: "update", state: "stopped", stage: "complete", elapsedMs: 500, errors: [] } } : {}),
    desktopVersion: buildInfo.version,
    agentVersion: buildInfo.version,
  }),
  // Seeded by the harness (`?streaming=`), so a row's *running* state — the
  // reserved slot's spinning ring — is reachable in a capture. Empty by
  // default: a plain page load has nothing running.
  list_streaming_thread_ids: () => harnessStreamingThreadIds,
  check_app_update: () => ({ currentVersion: buildInfo.version, latestVersion: "2.1.0", hasUpdate: Boolean(harnessUpdate), platformSupported: true, canInstallInApp: true, downloadUrl: "https://future-os.cn/" }),
  app_maintenance_status: () => ({
    cli: { tool: { path: "C:\\Program Files\\FutureOS\\future.exe", source: "bundled", version: buildInfo.version }, error: "CLI diagnostic must stay private" },
    update: !harnessUpdate || harnessUpdate === "available"
      ? null
      : {
          operationId: "update-raw-id",
          fromVersion: buildInfo.version,
          targetVersion: "2.1.0",
          stage: "finished",
          outcome: harnessUpdate.includes("repair") ? "repair-required" : harnessUpdate.includes("ready") ? "pending-validation" : "failed-restored",
          diagnostic: "Cannot start update observer: os error 50; C:\\Program Files\\FutureOS",
          restartSafe: !harnessUpdate.includes("repair"),
          repairAcknowledged: false,
        },
  }),
  get_skill_guide: () => ({
    links: { help: "https://future-os.cn/docs" },
    skills: {
      coachPrompt: { zh: "帮我把今天的文献整理成综述初稿", en: "Draft a review from today's papers" },
      manual: { zh: "https://future-os.cn/docs/skills", en: "https://future-os.cn/docs/skills" },
    },
  }),

  // ── Workspaces & threads ───────────────────────────────────────────────
  list_workspaces: () => workspaceList,
  pin_workspace: (args) => {
    const target = workspaceList.find(workspace => workspace.id === args?.input?.workspaceId);
    if (target)
      target.pinned = args.input.pinned;
    return target ?? null;
  },
  rename_workspace: (args) => {
    const target = workspaceList.find(workspace => workspace.id === args?.input?.workspaceId);
    if (target)
      target.name = args.input.name;
    return target ?? null;
  },
  delete_workspace: args => workspaceList.find(workspace => workspace.id === args?.workspaceId) ?? null,
  create_workspace: args => ({
    id: `ws_${Date.now()}`,
    name: args?.input?.name ?? "新工作区",
    kind: "user",
    path: args?.input?.path ?? `${HOME}/Research/new`,
    cleanupStatus: "active",
    createdAt: Date.now(),
    updatedAt: Date.now(),
  }),
  ensure_workspace_git: () => true,
  list_threads: () => {
    for (const id of harnessPinnedThreadIds) {
      const target = threadList.find(thread => thread.id === id);
      if (target)
        target.pinned = true;
    }
    return threadList;
  },
  get_recent_thread: () => threadList.find(thread => thread.id === (requestedThread() ?? "th_review")) ?? null,
  mark_thread_opened: () => null,
  pin_thread: (args) => {
    const target = threadList.find(thread => thread.id === args?.input?.threadId);
    if (target)
      target.pinned = args.input.pinned;
    return target ?? null;
  },
  rename_thread: (args) => {
    const target = threadList.find(thread => thread.id === args?.input?.threadId);
    if (target)
      target.title = args.input.title;
    return target ?? null;
  },
  generate_thread_title: () => ({ title: "多巴胺与风险决策：任务不确定性下的效应方向", model: "deepseek-v4" }),
  update_thread_model: args => threadList.find(thread => thread.id === args?.input?.threadId) ?? null,
  update_thread_thinking_level: args => threadList.find(thread => thread.id === args?.input?.threadId) ?? null,
  delete_thread: () => null,
  restore_thread: () => null,
  batch_delete_threads: () => ({ deletedCount: 0, failed: [] }),
  get_thread_cleanup_summary: args => ({
    threadId: args?.threadId ?? "",
    workspaceId: "ws_dopamine",
    workspaceKind: "user",
    workspacePath: `${HOME}/Research/dopamine-decision`,
    cleanupStatus: "active",
    artifactCount: 3,
    workspaceFileCount: 12,
  }),
  fork_thread: () => `sess_fork_${Date.now()}`,
  observe_session: () => null,
  get_thread_agent_state: (args) => {
    const target = threadList.find(thread => thread.id === args?.threadId) ?? threadList[0];
    if (!target)
      return null;
    // The first read for a thread answers with the state the header already
    // shows; later reads (the app re-reads when the usage panel opens) answer
    // with a larger figure, so a capture can prove the panel refreshed.
    const reads = (stateReads.get(target.id) ?? 0) + 1;
    stateReads.set(target.id, reads);
    const chatMode = target.mode === "chat";
    const usage = chatMode
      ? unpricedSessionUsage
      : reads > 1
        ? refreshedSessionUsage
        : sessionUsage;
    return {
      model: target.model ?? "future/deepseek-v4-pro",
      thinkingLevel: "medium",
      sessionName: target.title,
      sessionId: target.agentSessionId ?? null,
      cwd: workspaceList.find(workspace => workspace.id === target.workspaceId)?.path ?? `${HOME}/Research/dopamine-decision`,
      parentSessionId: target.parentSessionId ?? null,
      isStreaming: false,
      isCompacting: false,
      activeRun: null,
      // Chat-mode conversations stand in for a model with no prices on file, so
      // the usage dialog's tokens-only fallback is capturable.
      usage,
    };
  },
  reconcile_thread_workspace: () => null,
  attach_remote_stream: () => ({ runId: null }),

  // ── Session history ────────────────────────────────────────────────────
  /**
   * A history read, staged and then read back in chunks.
   *
   * This is the lane `integrations/storage/historyTransfer.ts` uses: the backend
   * stages the page, the client reads 64 KiB slices of it and composes the JSON
   * cooperatively. Only the inline command was answered here, so every
   * conversation in every capture came out as `invalid_history_transfer` and a
   * retry link — the transcript a screenshot exists to show never rendered at
   * all. The bytes are UTF-8 JSON, which is what `decodeJsonBytes` expects.
   */
  begin_session_history_transfer: (args) => {
    const entries = entriesForThread(args?.threadId ?? "");
    const page = { entries, hasMore: false, nextOffset: entries.length };
    const bytes = new TextEncoder().encode(JSON.stringify(page));
    const id = `shot_transfer_${++historyTransferSeq}`;
    historyTransfers.set(id, bytes);
    return { id, totalBytes: bytes.length, chunkBytes: HISTORY_CHUNK_BYTES };
  },
  read_session_history_transfer: (args) => {
    const bytes = historyTransfers.get(String(args?.transferId ?? ""));
    if (!bytes)
      return [];
    const offset = Math.max(0, Number(args?.offset ?? 0));
    return Array.from(bytes.subarray(offset, offset + HISTORY_CHUNK_BYTES));
  },
  release_session_history_transfer: (args) => {
    historyTransfers.delete(String(args?.transferId ?? ""));
    return null;
  },
  get_session_entries_page: (args) => {
    const entries = entriesForThread(args?.threadId ?? "");
    return { entries, hasMore: false, nextOffset: entries.length };
  },
  get_session_entries: args => ({ entries: entriesForThread(args?.threadId ?? "") }),

  // ── Runs, tools, approvals ─────────────────────────────────────────────
  list_runs: args => runs.filter(run => run.threadId === args?.threadId),
  get_latest_run: args => runs.find(run => run.threadId === args?.threadId) ?? null,
  get_run: args => runs.find(run => run.id === args?.runId) ?? null,
  list_latest_run_infos: (args: any) =>
    (args?.threadIds ?? [])
      .map((threadId: string) => {
        const run = runs.find(candidate => candidate.threadId === threadId);
        return run ? { threadId, runId: run.id, status: run.status, endedAt: run.endedAt ?? null } : null;
      })
      .filter(Boolean),
  create_run: args => ({
    id: `run_${Date.now()}`,
    threadId: args?.input?.threadId,
    status: "running",
    createdAt: Date.now(),
    updatedAt: Date.now(),
  }),
  update_run_status: () => null,
  abort_run: () => null,
  archive_finished_runs: () => 2,
  list_run_events: () => [],
  list_run_events_since: () => [],
  list_run_events_bulk: (args: any) => (args?.runIds ?? []).map((id: string) => [id, []]),
  list_tool_calls: args => toolCalls.filter(call => call.runId === args?.runId),
  list_tool_calls_bulk: (args: any) =>
    (args?.runIds ?? []).map((id: string) => [id, toolCalls.filter(call => call.runId === id)]),
  list_tool_outputs: (args) => {
    const content = toolOutputs[args?.toolCallId];
    return content
      ? [{ id: `out_${args.toolCallId}`, toolCallId: args.toolCallId, kind: "text", content, createdAt: Date.now() }]
      : [];
  },
  list_approval_requests: args => (args?.threadId === pendingApproval.threadId ? [pendingApproval] : []),
  /**
   * A run's auto-approval assessments: none here, but the *handler* has to exist.
   *
   * The run inspector renders them unconditionally, so an unanswered command
   * reaches it as `null` and `data.filter` throws — which unmounts the whole app.
   * Opening a step row (or the panel) is how a capture reaches that screen, so a
   * missing answer here cost every frame that clicked a tool row, not a detail.
   */
  list_approval_assessments: () => [],
  list_pending_approval_requests: () => [pendingApproval],
  decide_approval_request: () => null,
  save_approval_rule: () => null,
  save_approval_rules: () => null,

  // ── Review ─────────────────────────────────────────────────────────────
  get_workspace_review_capabilities: () => ({
    isGitWorkspace: true,
    views: ["git_changes", "last_run"],
    defaultView: "git_changes",
    changePreview: "ready",
  }),
  get_git_review: () => ({
    isGitWorkspace: true,
    workspacePath: `${HOME}/Research/dopamine-decision`,
    branch: "main",
    upstream: "origin/main",
    diffBase: null,
    diffBaseLabel: "工作区未提交的改动",
    additions: 18,
    deletions: 2,
    files: reviewFiles.map(file => ({
      path: file.path,
      status: file.changeType,
      additions: file.additions,
      deletions: file.deletions,
      diff: file.diff,
      binary: file.path.endsWith(".png"),
      diffTruncated: false,
    })),
  }),
  get_last_run_review: () => ({
    changeset: {
      id: "cs1",
      threadId: "th_review",
      runId: "run_1",
      title: "本次运行的改动",
      status: "applied",
      filesChanged: 2,
      additions: 18,
      deletions: 2,
      sourceKind: "run_snapshot",
      workspaceId: "ws_dopamine",
      binaryFiles: 1,
      omittedFiles: 0,
      completeness: "complete",
      confidence: "normal",
      overlapped: false,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    },
    files: reviewFiles,
    run: runs[0],
    snapshotStatus: "complete",
    confidence: "normal",
    overlapped: false,
  }),
  retry_run_review: () => null,

  // ── Files & artifacts ──────────────────────────────────────────────────
  list_directory: args => workspaceFiles[args?.path] ?? [],
  search_workspace_files: () => [],
  // `validUtf8` and `size` are part of the command's real contract: without
  // `validUtf8` the text preview classifies every file as binary and bails to
  // the OS handler. Demo files also preview as themselves — a `.py` shows the
  // source the reply quotes, not the demo markdown — so the code preview's
  // syntax highlighting has real input.
  read_text_file_preview: (args) => {
    const name = (args?.path ?? "").split("/").pop() ?? "";
    const content = demoFileContents[name] ?? "# 多巴胺与风险决策：结论对比\n\n见下方表格。\n";
    return {
      path: args?.path ?? "",
      name,
      content,
      size: content.length,
      truncated: false,
      validUtf8: true,
    };
  },
  open_path: () => null,
  open_external_url: () => null,
  open_url: () => null,
  resolve_preview_link_path: args => ({ path: args?.target ?? "", name: "file.md" }),
  resolve_markdown_references: (args) => {
    const references = args?.input?.references ?? [];
    return references.map((reference: any) => ({
      targetId: reference.targetId,
      targetType: reference.targetType,
      status: reference.targetType === "file" ? "resolved" : "missing",
      data: reference.targetType === "file"
        ? {
            path: reference.targetId,
            name: String(reference.targetId).split("/").pop() ?? "file",
            relativePath: String(reference.targetId).replace(`${HOME}/Research/dopamine-decision/`, ""),
            insideWorkspace: String(reference.targetId).startsWith(`${HOME}/Research/dopamine-decision/`),
          }
        : null,
    }));
  },
  list_artifacts: () => [],
  delete_artifact: () => null,
  export_artifact_file: () => null,
  import_attachment_artifact: () => null,
  import_ephemeral_attachment: () => null,
  inspect_attachment: args => ({ path: args?.path, name: "attachment", kind: "file", size: 1024 }),
  validate_image_attachment: () => ({ valid: true }),
  save_pasted_file: () => null,
  save_pasted_image: () => null,
  delete_temp_attachment: () => null,
  read_native_clipboard_file_paths: () => [],
  generate_image_thumbnail: () => null,
  prepare_image_preview: args => ({ path: assetUrl(args?.path ?? ""), version: "1" }),

  // ── Agent: models, providers, account ──────────────────────────────────
  list_agent_models: () => models,
  sync_future_models: () => ({ synced: true, modelCount: models.length, revision: 42 }),
  list_agent_providers: () => providersView,
  upsert_custom_provider: () => providersView,
  update_builtin_provider: () => providersView,
  update_builtin_provider_key: () => providersView,
  set_builtin_provider_base_url: () => providersView,
  delete_custom_provider: () => providersView,
  // Thinking-level detection: every level is accepted, so a screenshot scenario
  // that clicks Detect shows a fully selected row.
  probe_thinking_level: (args: { input?: { level?: string } }) => ({
    level: args?.input?.level || "default",
    ok: true,
  }),
  set_default_model: () => null,
  get_future_auth_state: () => ({
    status: "authenticated",
    profile: { email: "lixin@lab.example.edu", userId: "u_8841", emailVerified: true, createdAt: "2026-03-02T08:00:00Z" },
  }),
  get_future_profile: () => ({
    email: "lixin@lab.example.edu",
    userId: "u_8841",
    emailVerified: true,
    createdAt: "2026-03-02T08:00:00Z",
  }),
  get_future_balance: () => ({ credits: 842.5 }),
  start_future_login: () => ({ loginUrl: "https://future-os.cn/login", expiresAt: Date.now() + 300_000 }),
  poll_future_login: () => ({ status: "pending" }),
  logout_future_provider: () => null,

  // ── Skills ─────────────────────────────────────────────────────────────
  list_installed_skills: () => installedSkills,
  list_available_skills: () => availableSkills,
  install_skill: () => null,
  uninstall_skill: () => true,
  refresh_skills: () => null,
  bootstrap_builtin_skills: () => null,

  // ── Skill recommendation (PRD "技能推荐") ─────────────────────────────
  // The agent answers `suggest_skill`; the daily-budget state lives in the
  // Desktop store. Both are simulated here so a scenario can render the
  // recommendation card without a Jev key. `suggest_skill` returns the first
  // uninstalled catalogue skill so the card has content to show.
  suggest_skill: () => {
    const installedIds = new Set((installedSkills as { id: string }[]).map(row => row.id));
    const pick = (availableSkills as { id: string; description: string }[])
      .find(entry => !installedIds.has(entry.id));
    return pick ? { name: pick.id, description: pick.description } : null;
  },
  skill_reco_today: () => ({ count: 0, skillIds: [], messageHashes: [] }),
  record_skill_reco: () => null,

  // ── Remote (phone control) ─────────────────────────────────────────────
  // A plain load is the single-device host page; `?remoteDevices=` swaps in the
  // multi-device fixture (see `remoteStatusWithDevices`).
  remote_status: () =>
    remoteDevicesMode() ? remoteStatusWithDevices(remoteDevicesMode()) : EMPTY_REMOTE_STATUS,
  remote_pairing_status: () => ({ paired: false, deviceName: null }),
  remote_start: () => ({
    ...EMPTY_REMOTE_STATUS,
    phase: "ready",
    pairId: "E59C28514F3F453CB4E91949930396F5",
    pairingCode: "eyJkZXNrdG9wIjoiZGVza3RvcF9DNzZIVjJSNFRLUEdDQzU1In0",
    pairingCodeExpiresAt: Math.floor(Date.now() / 1000) + 600,
  }),
  remote_stop: () => EMPTY_REMOTE_STATUS,
  remote_unpair: () => null,
  // The three actions the device list offers. Serving them keeps a capture that
  // presses one from silently doing nothing while it works in the app.
  remote_add_device: () => remoteStatusWithDevices("waiting"),
  remote_takeover: () => remoteStatusWithDevices("multi"),
  remote_unpair_device: () => remoteStatusWithDevices("multi"),

  // ── Remote, the client role: this Desktop connecting *out* ─────────────
  //
  // Everything here answers the opposite question to the host handlers above:
  // what this machine is connected to, not what is connected to it. The two
  // halves are separate screens reading separate commands, so a fixture that
  // served both from one shape would hide a screen reading the wrong one.
  remote_peer_list: () => peerList(),
  // A freshly claimed host has no name yet and is not connected until the
  // supervisor gets there — which is the state the pairing screen shows.
  remote_peer_pair: () => ({
    ...remotePeers[0]!,
    desktopId: "desktop_paired",
    name: null,
    pairId: "pair_paired",
    connected: false,
    error: null,
    bridgeInstanceId: null,
    features: [],
    agentAvailable: false,
  }),
  remote_peer_connect: (args: { desktopId: string }) => ({
    ...(remotePeers.find(peer => peer.desktopId === args.desktopId) ?? remotePeers[0]),
    connected: true,
    error: null,
  }),
  /**
   * The app's own liveness pass, and "reconnect all".
   *
   * The fixture answers it with the same list: a capture's state *is* the
   * fixture, and the list below is static, so a machine the fixture calls down
   * stays down on screen however often the app asks for it. Serving it at all is
   * what keeps a screen that clicks "reconnect all" from silently doing nothing
   * in a capture while it works in the app.
   */
  remote_peer_connect_all: () => remotePeers,
  remote_peer_disconnect: () => null,
  remote_peer_unpair: () => null,
  remote_peer_set_label: () => null,
  /**
   * Stage a file on the host.
   *
   * Absent until now, and its absence did not degrade gracefully: `attach()`
   * awaited `null` and then pushed it into `uploads`, so the very next render
   * read `upload.uploadId` off nothing and unmounted the whole page. Every
   * capture that tried to show a staged attachment on the remote side
   * therefore came out blank.
   */
  remote_peer_upload_file: (args: { path?: string; name?: string }) => {
    const path = args?.path ?? "attachment";
    const name = args?.name ?? path.split("/").pop() ?? path;
    return { uploadId: `upl_${name}`, name, contentHash: `hash_${name}` };
  },
  remote_peer_sessions: (args: { desktopId: string }) => ({
    desktopId: args.desktopId,
    sessions: remoteSessions[args.desktopId] ?? [],
  }),
  remote_peer_workspaces: (args: { desktopId: string }) => ({
    ...remoteWorkspaces,
    desktopId: args.desktopId,
  }),
  /**
   * That host's directory listing, and a preview of one file in it.
   *
   * The preview answers a *local cache* path whose name is the host's, which is
   * what the overlay is handed: the name is what selects a renderer, and the
   * asset map turns these two demo names into real PNGs under `shot/assets/`.
   */
  remote_peer_list_files: (args: { desktopId: string; sessionId: string; path?: string }) => ({
    ...remoteFileListing,
    path: args?.path ?? remoteFileListing.path,
  }),
  remote_peer_preview_file: (args: { name?: string }) => ({
    name: args?.name ?? "preview",
    path: `${HOME}/.future/app/remote-previews/mock__${args?.name ?? "preview"}`,
  }),
  /**
   * The command channel to a host.
   *
   * Routed by command type here exactly as the host routes it: a fixture that
   * answered everything with one blob would not reveal a screen asking for the
   * wrong command.
   */
  remote_peer_request: (args: { command?: { type?: string; [key: string]: unknown }; desktopId: string }) => {
    const command = args.command ?? {};
    if (command.type === "prompt") {
      // The host answers the prompt, then streams the run it started (a fixture
      // the `remote-streaming` capture drives; see `remoteStream.ts`).
      streamRemoteRun(String(command.sessionId ?? ""), String(command.message ?? ""));
      return { sessionId: String(command.sessionId ?? ""), threadId: "th_forest" };
    }
    switch (command.type) {
      case "get_session_entries":
        return {
          entries: sessionEntries(String(command.sessionId)),
          hasMore: false,
          nextOffset: 0,
        };
      /**
       * The host's state: its settings *and* the run it has in flight.
       *
       * `activeRun` is what a client reads to learn there is a reply already being
       * written — the pushes alone cannot say so until a frame arrives, and a long
       * tool call pushes nothing for minutes.
       */
      case "get_state":
        return slow({ ...remoteConversationState, ...replayState(String(command.sessionId ?? "")) });
      /**
       * The run's log, for the client that reads it rather than guessing.
       *
       * Served for the fixture's in-flight run only; anything else answers an
       * empty read, which is what a host with nothing to add says.
       */
      case "get_events_since":
        return replayPage(
          String(command.sessionId ?? ""),
          String(command.runId ?? ""),
          Number(command.sinceIdx ?? -1),
        );
      /**
       * The host's own unit: one conversation, addressed by its *thread* id.
       *
       * Applied to the fixture, so the list stops serving the row and a capture
       * of the panel after a delete shows what the delete actually did. Leaving
       * it out made a deleted conversation stay in the list — the fixture
       * disagreeing with the command rather than the screen.
       */
      case "delete_session": {
        const threadId = String(command.threadId ?? "");
        const rows = remoteSessions[args.desktopId] as Array<{ threadId?: string }> | undefined;
        const index = rows?.findIndex(row => row.threadId === threadId) ?? -1;
        if (rows && index >= 0)
          rows.splice(index, 1);
        return { deleted: true };
      }
      // The host answers these two differently on purpose: `list_models` is the
      // picker's list and omits what the user hid, `list_settings_models` is the
      // settings page's and includes it. The fixture mirrors that split, or a
      // page fed from the wrong one would still look right here.
      case "list_models":
      case "get_available_models":
        return slow({
          // The host answers with the level the *agent* declares for a model,
          // which is a capability marker rather than a default anybody chose:
          // `high` for every reasoning model and `off` for the rest
          // (`agent/src/rpc/commands/providers.rs`). The fixture's own `models`
          // carry the level this app's pickers start at, so the host's answer is
          // mapped rather than reused: a client that seeded a draft from this
          // field would look right here and run at high effort against a real
          // machine (see `useRemoteConversationSettings`).
          models: models
            .filter(model => !hiddenModelKeys().includes(`${model.provider}/${model.id}`))
            .map(model => ({ ...model, thinkingLevel: model.reasoning === false ? "off" : "high" })),
        });
      case "list_settings_models":
        return { models };
      case "get_desktop_settings":
        return { ...settings };
      case "update_desktop_settings": {
        const patch = (command.settings ?? {}) as Record<string, unknown>;
        settings = { ...settings, ...patch };
        return { ...settings };
      }
      case "get_settings":
        return { approvalTier: "sandbox", sandboxAvailable: true };
      case "list_workspaces":
        return remoteWorkspaces;
      case "get_tool_call_args":
        return remoteToolArgs[String(command.toolCallId)] ?? null;
      case "list_session_files":
        return { files: [] };
      case "list_skills":
        return { skills: installedSkills };
      case "list_available_skills":
        return { skills: availableSkills };
      case "list_tasks":
        return { tasks: tasks.map(task => ({
          id: task.id,
          name: task.name,
          enabled: task.enabled,
          triggerKind: task.triggerKind,
          trigger: task.trigger,
          depCount: task.depCount,
          nextDueAt: task.nextDueAt,
          queued: task.queued,
          latestRun: task.latestRun,
        })) };
      // The host's list row omits the prompt; its detail record is what carries
      // it, which is why the client completes each row with this read.
      case "get_task": {
        const task = tasks.find(row => row.id === String(command.taskId));
        return task ? { ...task } : null;
      }
      case "list_providers":
        return providersView;
      case "list_task_runs":
        return { runs: taskRuns[String(command.taskId)] ?? [] };
      case "list_task_deps":
        return { deps: taskDeps[String(command.taskId)] ?? [] };
      case "skill_reco_today":
        return { today: { count: 0, skillIds: [], messageHashes: [] } };
      case "suggest_skill":
        return { skill: null };
      case "generate_session_title":
        return { title: "重画森林图并核对置信区间（已修正区间水平）" };
      default:
        return null;
    }
  },

  // ── Embedded terminal ──────────────────────────────────────────────────
  // Points at `shot/terminal-server.mjs`, a tiny stand-in for the loopback PTY
  // server that replays a canned shell session.
  terminal_server_info: () => ({
    url: "http://127.0.0.1:7391",
    token: "shot-token",
    port: 7391,
    maxSessions: 8,
  }),

  // ── Tasks ──────────────────────────────────────────────────────────────
  // The panel reads through these; mutations are applied to the in-memory list
  // so a capture shows the same list a real write would produce.
  list_tasks: () => taskList,
  create_task: (args) => {
    const input = args?.input ?? {};
    const created = {
      id: `tsk_${Date.now()}`,
      name: input.name,
      enabled: input.enabled ?? true,
      prompt: input.prompt,
      promptVersion: 1,
      cwd: input.cwd,
      modelId: input.modelId ?? null,
      thinkingLevel: input.thinkingLevel ?? null,
      sessionPolicy: input.sessionPolicy ?? "new",
      sessionRetention: input.sessionRetention ?? "keep",
      conversationMode: input.conversationMode ?? "workspace",
      triggerKind: input.triggerKind ?? "manual",
      trigger: input.trigger ?? {},
      depJoin: input.depJoin ?? "all",
      nextDueAt: input.triggerKind === "schedule" ? Date.now() + 60 * 60_000 : null,
      queued: false,
      depCount: 0,
      latestRun: null,
    };
    taskList.push(created);
    return created;
  },
  update_task: (args) => {
    const target = taskList.find(task => task.id === args?.id);
    if (!target)
      throw new Error("task not found");
    const input = args?.input ?? {};
    const promptChanged = input.prompt !== undefined && input.prompt !== target.prompt;
    Object.assign(target, {
      name: input.name ?? target.name,
      prompt: input.prompt ?? target.prompt,
      cwd: input.cwd ?? target.cwd,
      modelId: input.modelId ?? null,
      thinkingLevel: input.thinkingLevel ?? null,
      sessionPolicy: input.sessionPolicy ?? target.sessionPolicy,
      sessionRetention: input.sessionRetention ?? target.sessionRetention,
      triggerKind: input.triggerKind ?? target.triggerKind,
      trigger: input.trigger ?? target.trigger,
      depJoin: input.depJoin ?? target.depJoin,
      enabled: input.enabled ?? target.enabled,
      promptVersion: promptChanged ? target.promptVersion + 1 : target.promptVersion,
      nextDueAt: (input.triggerKind ?? target.triggerKind) === "schedule" ? Date.now() + 60 * 60_000 : null,
    });
    return target;
  },
  delete_task: (args) => {
    const index = taskList.findIndex(task => task.id === args?.id);
    if (index >= 0)
      taskList.splice(index, 1);
    return null;
  },
  set_task_enabled: (args) => {
    const target = taskList.find(task => task.id === args?.id);
    if (!target)
      throw new Error("task not found");
    target.enabled = !!args?.enabled;
    if (target.enabled && target.triggerKind === "schedule")
      target.nextDueAt = Date.now() + 60 * 60_000;
    return target;
  },
  run_task_now: args => taskList.find(task => task.id === args?.id) ?? null,
  list_task_runs: args => taskRuns[args?.id] ?? [],
  list_task_deps: args => taskDeps[args?.id] ?? [],

  // ── Settings pages ─────────────────────────────────────────────────────
  clear_app_data: () => null,
  reset_windows_sandbox: () => 0,
  install_app_update: () => null,
  restart_after_app_update: () => null,
  compact_thread_context: () => ({ operationId: "op_1", accepted: true }),
};

export function dispatch(command: string, args: any): unknown {
  const handler = handlers[command];
  if (!handler) {
    log("UNHANDLED COMMAND", command, JSON.stringify(args ?? {}).slice(0, 400));
    return null;
  }
  const value = handler(args);
  log(command, JSON.stringify(args ?? {}).slice(0, 200));
  return value;
}
