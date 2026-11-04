/**
 * Fixture backend for the UI harness.
 *
 * The real desktop UI talks to Tauri `invoke(command, args)`. Under the
 * harness that boundary is replaced by an HTTP endpoint (see
 * `desktop/shot/mock/core.ts`), and this server answers the same commands from
 * a **copy of the user's real `app.db`** — same schema, same row counts, same
 * long titles and paths — instead of the synthetic `shot/mock/data.ts` fixture.
 *
 * Why a server and not a generated TS fixture:
 *  - writes are real (`create_thread`, `delete_thread`, `create_workspace`)
 *    so a case can delete a conversation and the next read has to reflect it;
 *  - every command and its duration is logged, which is an observation channel
 *    of its own (the UI's "did it call the right thing?" is invisible in a
 *    screenshot);
 *  - a case gets a fresh DB copy, so cases cannot leak state into each other.
 *
 * Commands this server does not implement answer `{handled:false}` and the page
 * falls back to the in-page mock fixture. That keeps non-data surfaces
 * (settings panels, remote peers, task list) working while the data surface is
 * real. Unhandled names are logged so adding coverage is mechanical.
 *
 * Fidelity boundary: this reimplements the *store* commands in JS for test
 * purposes. It does not exercise `host/src/store/*.rs`, so a backend bug in a
 * query will not be found here — only UI behaviour against real-shaped data.
 */
import { readFileSync, existsSync, appendFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { DatabaseSync } from "node:sqlite";
import { createServer } from "node:http";

const now = () => Date.now();

function createId(prefix) {
  const d = new Date();
  const pad = n => String(n).padStart(2, "0");
  const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
  return `${prefix}-${stamp}-${randomUUID().slice(0, 6)}`;
}

function threadRow(row) {
  if (!row)
    return null;
  return {
    id: row.id,
    workspaceId: row.workspace_id,
    mode: row.mode,
    title: row.title,
    status: row.status,
    pinned: !!row.pinned,
    readonly: !!row.readonly,
    agentSessionId: row.agent_session_id ?? null,
    parentSessionId: row.parent_session_id ?? null,
    lastMessageAt: row.last_message_at ?? null,
    lastOpenedAt: row.last_opened_at ?? null,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    archivedAt: row.archived_at ?? null,
  };
}

function workspaceRow(row) {
  if (!row)
    return null;
  return {
    id: row.id,
    name: row.name,
    kind: row.kind,
    path: row.path,
    description: row.description ?? null,
    pinned: !!row.pinned,
    cleanupStatus: row.cleanup_status,
    cleanupRequestedAt: row.cleanup_requested_at ?? null,
    cleanedAt: row.cleaned_at ?? null,
    lastOpenedAt: row.last_opened_at ?? null,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
    deletedAt: row.deleted_at ?? null,
    archivedAt: row.archived_at ?? null,
  };
}

const THREAD_COLUMNS = `id, workspace_id, mode, title, status, pinned, readonly,
  agent_session_id, parent_session_id, last_message_at, last_opened_at,
  created_at, updated_at, archived_at`;
const WORKSPACE_COLUMNS = `id, name, kind, path, description, cleanup_status,
  cleanup_requested_at, cleaned_at, last_opened_at, created_at, updated_at,
  deleted_at, archived_at, pinned`;

/**
 * Models come from the agent's own `models.json`, so the picker shows what the
 * machine really has configured (usually a handful) instead of invented ones.
 */
function loadModels(modelsPath) {
  if (!modelsPath || !existsSync(modelsPath))
    return [];
  const { providers = {} } = JSON.parse(readFileSync(modelsPath, "utf8"));
  const models = [];
  for (const [provider, config] of Object.entries(providers)) {
    for (const model of config.models ?? []) {
      const reasoning = !!model.reasoning;
      models.push({
        id: model.id,
        label: model.name ?? model.id,
        provider,
        supportsImages: Array.isArray(model.modalities) && model.modalities.includes("image"),
        reasoning,
        thinkingLevels: model.thinkingLevels ?? (reasoning ? ["default", "off", "low", "medium", "high"] : ["default"]),
        defaultThinkingLevel: "default",
        contextWindow: model.contextWindow ?? null,
        description: model.description ?? null,
        descriptionEn: model.descriptionEn ?? null,
      });
    }
  }
  return models;
}

function readAppSettings(db) {
  const rows = db.prepare("SELECT key, value FROM app_settings").all();
  const get = key => rows.find(row => row.key === key)?.value;
  const parse = (key, fallback) => {
    const raw = get(key);
    if (raw == null)
      return fallback;
    try {
      return JSON.parse(raw);
    }
    catch {
      return raw;
    }
  };
  return {
    approvalTier: parse("approval_tier", "off"),
    hiddenModels: parse("hidden_models", []),
    autoUpgradeSkills: parse("auto_upgrade_skills", true),
    autoConnectRemote: parse("auto_connect_remote", true),
    skillGuideDismissed: parse("skill_guide_dismissed", false),
    skillIntroDismissed: parse("skill_intro_dismissed", false),
    bellOnComplete: parse("bell_on_complete", true),
    autoTitleFirstTurn: parse("auto_title_first_turn", true),
    titleLanguage: parse("title_language", "zh"),
    communityEdition: parse("community_edition", false),
    skillRecommend: parse("skill_recommend", true),
    showThinking: parse("show_thinking", false),
  };
}

function providerView(models) {
  const byProvider = new Map();
  for (const model of models) {
    const entry = byProvider.get(model.provider) ?? { id: model.provider, name: model.provider, baseUrl: "", hasApiKey: true, modelCount: 0, requiresBaseUrl: false };
    entry.modelCount += 1;
    byProvider.set(model.provider, entry);
  }
  return { builtin: [...byProvider.values()], custom: [] };
}

/**
 * One deterministic transcript per thread, derived from its title.
 *
 * Real transcripts live in the 1 GB agent DB behind a block/projection layer
 * this harness does not reconstruct. These entries are shaped like the real
 * projection and clearly derived, so an opened conversation reads as a
 * conversation rather than as an empty pane. Marked in the case manifest as a
 * known fidelity gap.
 */
function syntheticEntries(thread) {
  if (!thread)
    return [];
  const created = thread.lastMessageAt ?? thread.updatedAt ?? now();
  return [
    {
      id: `harness_u_${thread.id}`,
      kind: "message",
      role: "user",
      createdAtMs: created - 60_000,
      blocks: [{ kind: "text", text: `继续处理「${thread.title}」这件事，先看一下工作区里现有的材料。` }],
      metadata: {},
    },
    {
      id: `harness_a_${thread.id}`,
      kind: "message",
      role: "assistant",
      createdAtMs: created,
      run: { status: "completed", durationMs: 42_000 },
      usage: { inputTokens: 12_400, outputTokens: 820, cacheReadTokens: 6_200 },
      blocks: [{ kind: "text", text: `已读取工作区的文件。关于「${thread.title}」，我先把现状和下一步整理如下：\n\n1. 现有材料已经覆盖主线；\n2. 还需要一处交叉验证；\n3. 下一步我会补上验证并给出结论。` }],
      metadata: {},
    },
  ];
}

export function createFixtureServer({ dbPath, modelsPath, logPath, slowModelsMs = 0 }) {
  const db = new DatabaseSync(dbPath);
  db.exec("PRAGMA journal_mode=WAL");
  db.exec("PRAGMA foreign_keys=ON");

  const models = loadModels(modelsPath);
  let settings = readAppSettings(db);
  /** threadId → { model, thinkingLevel } for the agent-backed part of state. */
  const sessionState = new Map();
  const historyTransfers = new Map();
  const commandLog = [];
  const transferSeq = { value: 0 };

  const writeLog = entry => {
    commandLog.push(entry);
    if (logPath)
      appendFileSync(logPath, `${JSON.stringify(entry)}\n`);
  };

  const modelFor = (threadId) => {
    const state = sessionState.get(threadId);
    if (state?.model)
      return state;
    if (!models.length)
      return { model: null, thinkingLevel: null };
    let hash = 0;
    for (const ch of String(threadId))
      hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
    const model = models[hash % models.length];
    return {
      model: `${model.provider}/${model.id}`,
      thinkingLevel: model.thinkingLevels?.[0] === "default" ? "medium" : (model.thinkingLevels?.[0] ?? "medium"),
    };
  };

  const threadById = id => threadRow(db.prepare(`SELECT ${THREAD_COLUMNS} FROM threads WHERE id = ?`).get(id));
  const workspaceById = id => workspaceRow(db.prepare(`SELECT ${WORKSPACE_COLUMNS} FROM workspaces WHERE id = ?`).get(id));

  const listThreads = (scope = "recent", { excludePinned = false } = {}) => {
    let where = "deleted_at IS NULL AND status != 'deleted' AND archived_at IS NULL";
    if (excludePinned)
      where += " AND pinned = 0";
    if (scope === "chat")
      where += " AND mode = 'chat'";
    else if (scope === "archived")
      where = "deleted_at IS NULL AND status != 'deleted' AND archived_at IS NOT NULL";
    else if (scope.startsWith("workspace:"))
      where += ` AND workspace_id = '${scope.slice("workspace:".length).replace(/'/g, "''")}'`;
    const rows = db.prepare(`SELECT ${THREAD_COLUMNS} FROM threads WHERE ${where}
      ORDER BY pinned DESC, COALESCE(last_message_at, updated_at) DESC`).all();
    return rows.map(threadRow);
  };

  const listWorkspaces = (archived = false, { excludePinned = false, userOnly = false } = {}) => {
    const clauses = ["deleted_at IS NULL"];
    if (archived)
      clauses.push("archived_at IS NOT NULL");
    else
      clauses.push("archived_at IS NULL");
    if (excludePinned)
      clauses.push("pinned = 0");
    if (userOnly)
      clauses.push("kind = 'user'");
    return db.prepare(`SELECT ${WORKSPACE_COLUMNS} FROM workspaces WHERE ${clauses.join(" AND ")}
      ORDER BY pinned DESC, COALESCE(last_opened_at, updated_at) DESC`).all().map(workspaceRow);
  };

  /**
   * Stable keyset pagination.
   *
   * Offset pagination over a live `ORDER BY` duplicates rows when the sort key
   * moves between pages (opening a workspace updates `last_opened_at`), and the
   * UI then renders the same workspace twice — a `React key` warning that looks
   * like a product defect but is the fixture's. The cursor is the last row's
   * (pinned, timestamp, id) instead.
   */
  const page = (rows, cursor, limit) => {
    let start = 0;
    if (cursor != null && String(cursor).trim() !== "") {
      const index = rows.findIndex(row => row.id === String(cursor));
      // The cursor row disappearing (deleted between pages) restarts at the
      // top rather than silently returning rows the caller already has.
      start = index >= 0 ? index + 1 : 0;
    }
    const slice = rows.slice(start, start + limit);
    return {
      slice,
      next: start + slice.length < rows.length ? (slice.at(-1)?.id ?? null) : null,
    };
  };

  const createChatWorkspace = (threadId, title) => {
    const id = createId("ws");
    const t = now();
    const path = `/tmp/future-harness/workspaces/${threadId}`;
    db.prepare(`INSERT INTO workspaces (id, name, kind, path, description, cleanup_status, created_at, updated_at)
      VALUES (?, ?, 'temporary', ?, NULL, 'active', ?, ?)`)
      .run(id, `${title} Workspace`, path, t, t);
    return workspaceById(id);
  };

  const handlers = {
    // ── Boot / agent liveness ────────────────────────────────────────────
    get_agent_status: () => ({ phase: "ready", desktopVersion: "harness", agentVersion: "harness" }),
    check_app_update: () => ({ currentVersion: "harness", latestVersion: "harness", hasUpdate: false, platformSupported: true, canInstallInApp: false, downloadUrl: null }),
    app_maintenance_status: () => ({ cli: null, update: null }),
    get_skill_guide: () => ({ links: {}, skills: {} }),
    app_build_info: () => ({ version: "0.0.0-harness", buildNumber: "harness", isRelease: false }),
    get_app_settings: () => settings,
    update_app_settings: (args) => {
      settings = { ...settings, ...(args?.settings ?? args ?? {}) };
      return settings;
    },
    list_installed_skills: () => [],
    list_available_skills: () => [],
    list_pending_approval_requests: () => [],
    list_approval_requests: () => [],
    list_agent_providers: () => providerView(models),
    get_future_auth_state: () => ({ status: "authenticated", profile: null }),
    get_future_balance: () => ({ credits: 0 }),
    /** One machine under test: no paired devices, so "new chat" cannot ask which. */
    remote_status: () => ({
      agentAvailable: true,
      phase: "stopped",
      reason: null,
      recovery: null,
      natsUrl: "tls://www.future-os.cn:4222",
      pairId: "",
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_harness",
      desktopPublicKey: "",
      webUrl: null,
      webLanUrl: null,
      warningCode: null,
    }),
    remote_peer_list: () => [],
    remote_peer_sessions: () => ({ desktopId: "", sessions: [] }),
    remote_peer_workspaces: () => ({ desktopId: "", workspaces: [] }),
    initialize_app_store: () => null,
    refresh_skills: () => null,
    sync_skills: () => null,
    list_streaming_thread_ids: () => [],
    list_agent_models: async () => {
      if (slowModelsMs > 0)
        await new Promise(resolve => setTimeout(resolve, slowModelsMs));
      return models;
    },
    list_models: () => ({ models }),
    get_available_models: () => ({ models }),
    list_settings_models: () => ({ models }),
    list_providers: () => providerView(models),

    // ── Workspaces ───────────────────────────────────────────────────────
    list_workspaces: () => listWorkspaces(false),
    list_workspaces_page: (args) => {
      const archived = !!args?.archived;
      const { slice, next } = page(listWorkspaces(archived, { excludePinned: !archived, userOnly: true }), args?.cursor, Number(args?.limit ?? 50));
      return { workspaces: slice, nextCursor: next };
    },
    list_pinned_workspaces: () => listWorkspaces(false, { userOnly: true }).filter(w => w.pinned),
    create_workspace: (args) => {
      const input = args?.input ?? {};
      const id = createId("ws");
      const t = now();
      const path = String(input.path ?? "").replace(/^~(?=\/)/, process.env.HOME ?? "~");
      const name = input.name?.trim() || path.split("/").filter(Boolean).pop() || "Workspace";
      db.prepare(`INSERT INTO workspaces (id, name, kind, path, description, cleanup_status, created_at, updated_at)
        VALUES (?, ?, 'user', ?, ?, 'active', ?, ?)`)
        .run(id, name, path, input.description ?? null, t, t);
      return workspaceById(id);
    },
    rename_workspace: (args) => {
      const id = args?.input?.workspaceId;
      db.prepare("UPDATE workspaces SET name = ?, updated_at = ? WHERE id = ?").run(args?.input?.name ?? "", now(), id);
      return workspaceById(id);
    },
    delete_workspace: (args) => {
      const t = now();
      db.prepare("UPDATE workspaces SET deleted_at = ?, updated_at = ? WHERE id = ?").run(t, t, args?.workspaceId);
      db.prepare("UPDATE threads SET deleted_at = ?, status = 'deleted', updated_at = ? WHERE workspace_id = ?").run(t, t, args?.workspaceId);
      return workspaceById(args?.workspaceId);
    },
    pin_workspace: (args) => {
      db.prepare("UPDATE workspaces SET pinned = ?, updated_at = ? WHERE id = ?")
        .run(args?.input?.pinned ? 1 : 0, now(), args?.input?.workspaceId);
      return workspaceById(args?.input?.workspaceId);
    },
    archive_workspace: (args) => {
      db.prepare("UPDATE workspaces SET archived_at = ?, updated_at = ? WHERE id = ?").run(now(), now(), args?.workspaceId);
      return workspaceById(args?.workspaceId);
    },
    restore_workspace: (args) => {
      db.prepare("UPDATE workspaces SET archived_at = NULL, updated_at = ? WHERE id = ?").run(now(), args?.workspaceId);
      return workspaceById(args?.workspaceId);
    },
    ensure_workspace_git: () => true,
    get_workspace_cleanup_summary: () => null,

    // ── Threads ──────────────────────────────────────────────────────────
    list_threads: () => listThreads("recent"),
    list_threads_page: (args) => {
      const rows = listThreads(String(args?.scope ?? "recent"), { excludePinned: true });
      const { slice, next } = page(rows, args?.cursor, Number(args?.limit ?? 50));
      return { threads: slice, nextCursor: next, totalRoots: rows.length };
    },
    list_pinned_threads: (args) => listThreads(String(args?.scope ?? "recent")).filter(t => t.pinned),
    list_active_threads: () => [],
    search_threads: (args) => {
      const q = `%${String(args?.query ?? "").replace(/[%_]/g, "")}%`;
      const rows = db.prepare(`SELECT ${THREAD_COLUMNS} FROM threads
        WHERE deleted_at IS NULL AND status != 'deleted' AND title LIKE ?
        ORDER BY COALESCE(last_message_at, updated_at) DESC LIMIT ?`).all(q, Number(args?.limit ?? 100));
      return { active: rows.filter(r => !r.archived_at).map(threadRow), archived: rows.filter(r => r.archived_at).map(threadRow) };
    },
    get_recent_thread: () => {
      const row = db.prepare(`SELECT ${THREAD_COLUMNS} FROM threads
        WHERE deleted_at IS NULL AND status != 'deleted' AND archived_at IS NULL
        ORDER BY COALESCE(last_opened_at, last_message_at, updated_at) DESC LIMIT 1`).get();
      return threadRow(row);
    },
    mark_thread_opened: (args) => {
      db.prepare("UPDATE threads SET last_opened_at = ?, updated_at = updated_at WHERE id = ?").run(now(), args?.threadId);
      return null;
    },
    create_thread: (args) => {
      const input = args?.input ?? {};
      const mode = input.mode === "workspace" ? "workspace" : "chat";
      const id = createId("thread");
      const t = now();
      const title = input.title || (mode === "chat" ? "新对话" : "新会话");
      db.exec("BEGIN IMMEDIATE");
      try {
        let workspace;
        if (mode === "chat") {
          workspace = createChatWorkspace(id, title);
        }
        else if (input.workspaceId) {
          workspace = workspaceById(input.workspaceId);
        }
        else {
          const path = String(input.workspacePath ?? "").replace(/^~(?=\/)/, process.env.HOME ?? "~");
          const name = input.workspaceName || path.split("/").filter(Boolean).pop() || "Workspace";
          const existing = db.prepare(`SELECT ${WORKSPACE_COLUMNS} FROM workspaces WHERE path = ? AND deleted_at IS NULL LIMIT 1`).get(path);
          if (existing) {
            workspace = workspaceRow(existing);
          }
          else {
            const wsId = createId("ws");
            db.prepare(`INSERT INTO workspaces (id, name, kind, path, cleanup_status, created_at, updated_at)
              VALUES (?, ?, 'user', ?, 'active', ?, ?)`).run(wsId, name, path, t, t);
            workspace = workspaceById(wsId);
          }
        }
        db.prepare(`INSERT INTO threads (id, workspace_id, mode, title, title_seed, status, pinned, readonly,
            agent_session_id, asset_root_id, last_opened_at, created_at, updated_at)
          VALUES (?, ?, ?, ?, ?, 'active', 0, 0, ?, ?, ?, ?, ?)`)
          .run(id, workspace.id, mode, title, input.agentSessionId ? null : title, input.agentSessionId ?? `sess_${id}`, id, t, t, t);
        db.exec("COMMIT");
        if (input.modelId)
          sessionState.set(id, {
            model: input.modelId,
            thinkingLevel: input.thinkingLevel || "default",
          });
      }
      catch (error) {
        db.exec("ROLLBACK");
        throw error;
      }
      return threadById(id);
    },
    rename_thread: (args) => {
      db.prepare("UPDATE threads SET title = ?, updated_at = ? WHERE id = ?").run(args?.input?.title ?? "", now(), args?.input?.threadId);
      return threadById(args?.input?.threadId);
    },
    update_thread_model: (args) => {
      const id = args?.input?.threadId;
      const current = modelFor(id);
      sessionState.set(id, { ...current, model: args?.input?.modelId ?? current.model });
      return threadById(id);
    },
    update_thread_thinking_level: (args) => {
      const id = args?.input?.threadId;
      const current = modelFor(id);
      sessionState.set(id, { ...current, thinkingLevel: args?.input?.thinkingLevel ?? current.thinkingLevel });
      return threadById(id);
    },
    pin_thread: (args) => {
      db.prepare("UPDATE threads SET pinned = ?, updated_at = ? WHERE id = ?")
        .run(args?.input?.pinned ? 1 : 0, now(), args?.input?.threadId);
      return threadById(args?.input?.threadId);
    },
    archive_thread_tree: (args) => {
      db.prepare("UPDATE threads SET archived_at = ?, status = 'archived', pinned = 0, updated_at = ? WHERE id = ?").run(now(), now(), args?.threadId);
      return threadById(args?.threadId);
    },
    restore_thread_tree: (args) => {
      db.prepare("UPDATE threads SET archived_at = NULL, status = 'active', updated_at = ? WHERE id = ?").run(now(), args?.threadId);
      return threadById(args?.threadId);
    },
    batch_archive_threads: (args) => {
      let count = 0;
      for (const id of args?.threadIds ?? []) {
        const result = db.prepare("UPDATE threads SET archived_at = ?, status = 'archived', pinned = 0, updated_at = ? WHERE id = ? AND deleted_at IS NULL").run(now(), now(), id);
        count += Number(result.changes ?? 0);
      }
      return { archivedCount: count, skipped: [] };
    },
    delete_thread: (args) => {
      const id = args?.input?.threadId;
      const t = now();
      db.prepare("UPDATE threads SET deleted_at = ?, status = 'deleted', updated_at = ? WHERE id = ?").run(t, t, id);
      const thread = threadById(id);
      if (thread?.mode === "chat") {
        db.prepare("UPDATE workspaces SET deleted_at = ?, updated_at = ? WHERE id = ? AND kind = 'temporary'").run(t, t, thread.workspaceId);
      }
      return thread;
    },
    batch_delete_threads: (args) => {
      let deletedCount = 0;
      const failed = [];
      for (const id of args?.input?.threadIds ?? []) {
        const t = now();
        const res = db.prepare("UPDATE threads SET deleted_at = ?, status = 'deleted', updated_at = ? WHERE id = ? AND deleted_at IS NULL").run(t, t, id);
        if (Number(res.changes ?? 0) > 0)
          deletedCount += 1;
        else failed.push(id);
      }
      return { deletedCount, failed };
    },
    get_thread_cleanup_summary: (args) => {
      const thread = threadById(args?.threadId);
      return thread
        ? { threadId: thread.id, workspaceId: thread.workspaceId, workspaceKind: "user", workspacePath: "", cleanupStatus: "active", artifactCount: 0, workspaceFileCount: 0 }
        : null;
    },
    get_thread_agent_state: (args) => {
      const thread = threadById(args?.threadId);
      if (!thread)
        return null;
      const { model, thinkingLevel } = modelFor(thread.id);
      return {
        model,
        thinkingLevel,
        sessionName: thread.title,
        sessionId: thread.agentSessionId || thread.id,
        cwd: "/workspace",
        parentSessionId: thread.parentSessionId,
        isStreaming: false,
        isCompacting: false,
        sandboxTier: settings.approvalTier,
        usage: { inputTokens: 12_400, outputTokens: 820, cacheReadTokens: 6_200, cacheWriteTokens: 0, costCny: 0.31, costInputCny: 0.22, costOutputCny: 0.06, costCacheReadCny: 0.03, costCacheWriteCny: 0 },
        activeRun: null,
      };
    },

    // ── Agent transcript (synthetic but deterministic) ───────────────────
    get_session_entries: (args) => {
      const thread = threadById(args?.threadId);
      const entries = syntheticEntries(thread);
      return { entries, hasMore: false, nextOffset: entries.length };
    },
    get_session_entries_page: (args) => {
      const thread = threadById(args?.threadId);
      const entries = syntheticEntries(thread);
      return { entries, hasMore: false, nextOffset: entries.length };
    },
    begin_session_history_transfer: (args) => {
      const thread = threadById(args?.threadId);
      const entries = syntheticEntries(thread);
      const payload = { entries, hasMore: false, nextOffset: entries.length };
      const bytes = Buffer.from(JSON.stringify(payload), "utf8");
      const id = `harness_transfer_${++transferSeq.value}`;
      historyTransfers.set(id, bytes);
      return { id, totalBytes: bytes.length, chunkBytes: 64 * 1024 };
    },
    read_session_history_transfer: (args) => {
      const bytes = historyTransfers.get(String(args?.transferId ?? ""));
      if (!bytes)
        return [];
      const offset = Math.max(0, Number(args?.offset ?? 0));
      return Array.from(bytes.subarray(offset, offset + 64 * 1024));
    },
    release_session_history_transfer: (args) => {
      historyTransfers.delete(String(args?.transferId ?? ""));
      return null;
    },

    // ── Settings / misc surfaces ─────────────────────────────────────────
    get_desktop_settings: () => settings,
    update_desktop_settings: (args) => {
      settings = { ...settings, ...(args?.settings ?? {}) };
      return settings;
    },
    get_settings: () => ({ approvalTier: settings.approvalTier, sandboxAvailable: true }),
    list_skills: () => [],
    list_tasks: () => [],
    list_session_files: () => ({ files: [] }),
    list_runs: () => [],
    get_latest_run: () => null,
    list_latest_run_infos: (args) => (args?.threadIds ?? []).map(threadId => ({ threadId, run: null })),
    list_artifacts: () => ({ artifacts: [] }),
    image_url: () => "",
    convert_file_src: (args) => args?.path ?? "",
    get_git_status: () => ({ branch: "main", dirty: false, files: [] }),
    get_thread_usage: () => null,
  };

  const server = createServer((req, res) => {
    const cors = {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Headers": "content-type",
      "Access-Control-Allow-Methods": "POST, GET, OPTIONS",
    };
    if (req.method === "OPTIONS") {
      res.writeHead(204, cors);
      res.end();
      return;
    }
    if (req.url === "/health") {
      res.writeHead(200, { ...cors, "content-type": "application/json" });
      res.end(JSON.stringify({ ok: true, threads: listThreads().length, workspaces: listWorkspaces().length }));
      return;
    }
    if (req.url === "/log") {
      res.writeHead(200, { ...cors, "content-type": "application/json" });
      res.end(JSON.stringify(commandLog));
      return;
    }
    if (req.url === "/query" && req.method === "POST") {
      readBody(req).then((body) => {
        try {
          const sql = String(body?.sql ?? "");
          if (!/^\s*(select|with)\b/i.test(sql))
            throw new Error("only SELECT queries are allowed");
          const rows = db.prepare(sql).all();
          res.writeHead(200, { ...cors, "content-type": "application/json" });
          res.end(JSON.stringify({ rows }));
        }
        catch (error) {
          res.writeHead(400, { ...cors, "content-type": "application/json" });
          res.end(JSON.stringify({ error: String(error?.message ?? error) }));
        }
      });
      return;
    }
    if (req.url !== "/invoke" || req.method !== "POST") {
      res.writeHead(404, cors);
      res.end("not found");
      return;
    }

    readBody(req).then(async (body) => {
      const command = String(body?.command ?? "");
      const args = body?.args ?? {};
      const started = performance.now();
      const handler = handlers[command];
      let payload;
      if (!handler) {
        payload = { handled: false };
      }
      else {
        try {
          const value = await handler(args);
          payload = { handled: true, value: value ?? null };
        }
        catch (error) {
          payload = { handled: true, error: String(error?.message ?? error) };
        }
      }
      const entry = {
        ts: Date.now(),
        command,
        handled: payload.handled,
        ms: Math.round((performance.now() - started) * 100) / 100,
        args: summarizeArgs(args),
        error: payload.error ?? null,
      };
      writeLog(entry);
      if (!payload.handled)
        process.stderr.write(`[fixture] fallback: ${command}\n`);
      res.writeHead(200, { ...cors, "content-type": "application/json" });
      res.end(JSON.stringify(payload));
    }).catch((error) => {
      res.writeHead(500, { ...cors, "content-type": "application/json" });
      res.end(JSON.stringify({ handled: true, error: String(error?.message ?? error) }));
    });
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolve({
        url: `http://127.0.0.1:${port}`,
        port,
        log: commandLog,
        db,
        close: () => new Promise((done) => {
          // A browser keep-alive socket would otherwise hold `server.close()`
          // open until Chrome drops it, which can outlast the run.
          server.closeAllConnections?.();
          server.close(() => {
            db.close();
            done();
          });
          setTimeout(done, 2000).unref();
        }),
      });
    });
  });
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", chunk => chunks.push(chunk));
    req.on("end", () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}"));
      }
      catch (error) {
        reject(error);
      }
    });
    req.on("error", reject);
  });
}

function summarizeArgs(args) {
  if (!args || typeof args !== "object")
    return null;
  const clone = {};
  for (const [key, value] of Object.entries(args)) {
    if (typeof value === "function")
      continue;
    if (value && typeof value === "object" && !Array.isArray(value))
      clone[key] = Object.fromEntries(Object.entries(value).filter(([, v]) => typeof v !== "function").slice(0, 8));
    else
      clone[key] = value;
  }
  const text = JSON.stringify(clone);
  return text.length > 400 ? `${text.slice(0, 400)}…` : clone;
}
