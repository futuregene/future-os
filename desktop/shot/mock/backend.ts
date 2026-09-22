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
  providersView,
  refreshedSessionUsage,
  reviewFiles,
  runs,
  sessionUsage,
  threads,
  toolCalls,
  toolOutputs,
  unpricedSessionUsage,
  workspaceFiles,
  workspaces,
} from "./data";

// The console output doubles as the "what is missing" list while iterating on
// the harness, which is why this one log call is intentional.
// eslint-disable-next-line no-console
const log = (...args: unknown[]) => console.log("[mock]", ...args);

let settings: typeof appSettings = { ...appSettings };
const threadList = threads.map(thread => ({ ...thread }));
const workspaceList = workspaces.map(workspace => ({ ...workspace }));
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

/** Command name → implementation. Anything missing goes to the fallback. */
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
    phase: "ready",
    desktopVersion: buildInfo.version,
    agentVersion: buildInfo.version,
  }),
  list_streaming_thread_ids: () => [],
  check_app_update: () => ({ available: false, version: null, notes: null }),
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
  list_threads: () => threadList,
  get_recent_thread: () => threadList.find(thread => thread.id === "th_review") ?? null,
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
      model: "future/deepseek-v4-pro",
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
  list_approval_requests: () => [],
  list_pending_approval_requests: () => [],
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

  // ── Remote (phone control) ─────────────────────────────────────────────
  remote_status: () => EMPTY_REMOTE_STATUS,
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

  // ── Embedded terminal ──────────────────────────────────────────────────
  // Points at `shot/terminal-server.mjs`, a tiny stand-in for the loopback PTY
  // server that replays a canned shell session.
  terminal_server_info: () => ({
    url: "http://127.0.0.1:7391",
    token: "shot-token",
    port: 7391,
    maxSessions: 8,
  }),

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
