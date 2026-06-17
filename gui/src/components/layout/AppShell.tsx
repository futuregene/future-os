import type { NewConversationStart } from "../../features/agent/NewConversation";
import type { MessageAttachment } from "../../features/agent/types";
import type { AgentModelOption } from "../../integrations/agent/models";
import type { StoredApprovalRequest, StoredRun, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ActivitySection } from "./ActivityRail";
import type { DeleteDialogState, RenameDialogState } from "./AppShellDialogs";
import type { ContextTab } from "./ContextPanel";
import { useCallback, useEffect, useMemo, useState } from "react";
import { AgentThread } from "../../features/agent/AgentThread";
import { NewConversation } from "../../features/agent/NewConversation";
import { ResearchView } from "../../features/research/ResearchView";
import { defaultAgentModelId, defaultModelId, loadAgentModelOptions } from "../../integrations/agent/models";
import {
  cancelStaleApprovalRequests,
  createDefaultChatThread,
  createThread,
  createWorkspace,
  decideApprovalRequest,
  deleteThread,
  getRecentThread,
  getThreadCleanupSummary,
  initializeAppStore,
  listApprovalRequests,
  listRuns,
  listThreads,
  listWorkspaces,
  pinThread,
  renameThread,
  restoreThread,
  updateThreadModel,
} from "../../integrations/storage/threadStore";
import { ActivityRail } from "./ActivityRail";
import { AppShellDialogs } from "./AppShellDialogs";
import { ContextPanel } from "./ContextPanel";

interface PendingPrompt {
  attachments?: MessageAttachment[];
  id: string;
  content: string;
}

interface WorkspaceCreateRequest {
  name?: string | null;
  path: string;
  createDirectory: boolean;
}

export function AppShell() {
  const [section, setSection] = useState<ActivitySection>("chat");
  const [centerMode, setCenterMode] = useState<"thread" | "new-chat">("thread");
  const [leftExpanded, setLeftExpanded] = useState(true);
  const [leftOverlayOpen, setLeftOverlayOpen] = useState(false);
  const [rightExpanded, setRightExpanded] = useState(false);
  const [contextTab, setContextTab] = useState<ContextTab>("runs");
  const [pendingApprovals, setPendingApprovals] = useState<StoredApprovalRequest[]>([]);
  const [modelOptions, setModelOptions] = useState<AgentModelOption[]>([]);
  const [selectedModelId, setSelectedModelId] = useState(defaultAgentModelId);
  const [threads, setThreads] = useState<StoredThread[]>([]);
  const [threadRunStatuses, setThreadRunStatuses] = useState<Record<string, StoredRun["status"] | undefined>>({});
  const [workspaces, setWorkspaces] = useState<StoredWorkspace[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [newChatWorkspaceId, setNewChatWorkspaceId] = useState<string | null>(null);
  const [loadingStore, setLoadingStore] = useState(true);
  const [storeError, setStoreError] = useState<string | null>(null);
  const [pendingPrompt, setPendingPrompt] = useState<PendingPrompt | null>(null);
  const [deleteDialog, setDeleteDialog] = useState<DeleteDialogState | null>(null);
  const [renameDialog, setRenameDialog] = useState<RenameDialogState | null>(null);

  const activeThread = useMemo(
    () => threads.find(thread => thread.id === activeThreadId) ?? null,
    [activeThreadId, threads],
  );
  const activeWorkspace = useMemo(
    () =>
      workspaces.find(workspace => workspace.id === activeThread?.workspaceId)
      ?? workspaces.find(workspace => workspace.kind === "user")
      ?? null,
    [activeThread?.workspaceId, workspaces],
  );
  const activeApproval = useMemo(
    () =>
      [...pendingApprovals]
        .filter(approval => approval.status === "pending")
        .sort((left, right) => left.createdAt - right.createdAt)[0] ?? null,
    [pendingApprovals],
  );

  const refreshThreadRunStatuses = useCallback(async (nextThreads: StoredThread[]) => {
    const entries = await Promise.all(
      nextThreads.map(async (thread) => {
        const runs = await listRuns(thread.id);
        return [thread.id, runs[0]?.status] as const;
      }),
    );
    setThreadRunStatuses(Object.fromEntries(entries));
  }, []);

  const refreshStore = useCallback(async (nextActiveThreadId?: string) => {
    const [nextThreads, nextWorkspaces] = await Promise.all([listThreads(), listWorkspaces()]);
    const selectableThreads = nextThreads.filter(thread => thread.status === "active");
    setThreads(nextThreads);
    setWorkspaces(nextWorkspaces);
    void refreshThreadRunStatuses(selectableThreads);
    if (nextActiveThreadId && selectableThreads.some(thread => thread.id === nextActiveThreadId)) {
      setActiveThreadId(nextActiveThreadId);
    }
    else if (activeThreadId && selectableThreads.some(thread => thread.id === activeThreadId)) {
      setActiveThreadId(activeThreadId);
    }
    else {
      setActiveThreadId(selectableThreads[0]?.id ?? null);
    }
  }, [activeThreadId, refreshThreadRunStatuses]);

  useEffect(() => {
    let cancelled = false;

    async function bootstrapStore() {
      setLoadingStore(true);
      try {
        await initializeAppStore();
        await cancelStaleApprovalRequests();
        const recentThread = (await getRecentThread()) ?? (await createDefaultChatThread());
        const [nextThreads, nextWorkspaces] = await Promise.all([listThreads(), listWorkspaces()]);
        if (cancelled)
          return;
        setThreads(nextThreads);
        setWorkspaces(nextWorkspaces);
        void refreshThreadRunStatuses(nextThreads.filter(thread => thread.status === "active"));
        setActiveThreadId(recentThread.id);
        setStoreError(null);
      }
      catch (error) {
        if (!cancelled) {
          setStoreError(error instanceof Error ? error.message : String(error));
        }
      }
      finally {
        if (!cancelled) {
          setLoadingStore(false);
        }
      }
    }

    void bootstrapStore();

    return () => {
      cancelled = true;
    };
  }, [refreshThreadRunStatuses]);

  useEffect(() => {
    let cancelled = false;

    async function refreshAgentModels() {
      try {
        const nextModels = await loadAgentModelOptions();
        if (cancelled)
          return;
        setModelOptions(nextModels);
        setSelectedModelId(current =>
          nextModels.some(model => model.id === current)
            ? current
            : defaultModelId(nextModels),
        );
      }
      catch {
        if (!cancelled) {
          setModelOptions([]);
        }
      }
    }

    void refreshAgentModels();
    const timer = window.setInterval(() => {
      void refreshAgentModels();
    }, 10000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  const activeThreads = useMemo(
    () => threads.filter(thread => thread.status === "active"),
    [threads],
  );
  const activeThreadIdForApprovals = activeThread?.id ?? null;

  useEffect(() => {
    if (activeThreads.length === 0) {
      setThreadRunStatuses({});
      return;
    }

    void refreshThreadRunStatuses(activeThreads);
    const timer = window.setInterval(() => {
      void refreshThreadRunStatuses(activeThreads);
    }, 1500);

    return () => window.clearInterval(timer);
  }, [activeThreads, refreshThreadRunStatuses]);

  useEffect(() => {
    let cancelled = false;
    async function refreshPendingApprovals() {
      if (!activeThreadIdForApprovals) {
        setPendingApprovals([]);
        return;
      }

      try {
        const approvals = await listApprovalRequests(activeThreadIdForApprovals);
        if (cancelled)
          return;
        const pending = approvals.filter(approval => approval.status === "pending");
        setPendingApprovals(pending);
      }
      catch {
        if (!cancelled) {
          setPendingApprovals([]);
        }
      }
    }

    void refreshPendingApprovals();
    const timer = window.setInterval(() => {
      void refreshPendingApprovals();
    }, 1500);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeThreadIdForApprovals]);

  function handleSectionChange(nextSection: ActivitySection) {
    setSection(nextSection);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }

  function handleSelectThread(thread: StoredThread) {
    setSection(thread.mode === "workspace" ? "workspace" : "chat");
    setActiveThreadId(thread.id);
    setCenterMode("thread");
    setNewChatWorkspaceId(null);
  }

  function handleSelectWorkspace(_workspace: StoredWorkspace, workspaceThreads: StoredThread[]) {
    const latestThread = workspaceThreads[0];
    setSection("workspace");
    setNewChatWorkspaceId(null);
    if (latestThread) {
      setActiveThreadId(latestThread.id);
      setCenterMode("thread");
    }
    else {
      setActiveThreadId(null);
      setCenterMode("thread");
    }
  }

  function handleOpenNewChat(workspaceId?: string) {
    setSection(workspaceId ? "workspace" : "chat");
    setNewChatWorkspaceId(workspaceId ?? null);
    setCenterMode("new-chat");
  }

  async function handleStartNewConversation(input: NewConversationStart) {
    const title = deriveThreadTitle(input.content);
    const thread = await createThread({
      mode: input.mode,
      title,
      workspaceId: input.workspace?.id,
      workspaceName: input.workspace?.label,
      workspacePath: input.workspace?.path,
      modelId: input.modelId,
    });
    setSelectedModelId(input.modelId);
    await refreshStore(thread.id);
    setSection(thread.mode === "workspace" ? "workspace" : "chat");
    setCenterMode("thread");
    setPendingPrompt({
      attachments: input.attachments,
      id: `${thread.id}:${Date.now()}`,
      content: input.content,
    });
  }

  async function handleAddWorkspace(input: WorkspaceCreateRequest) {
    const workspace = await createWorkspace(input);
    await refreshStore(activeThread?.id ?? undefined);
    return workspace;
  }

  function handleRenameThread(thread: StoredThread) {
    setRenameDialog({
      error: null,
      submitting: false,
      thread,
      value: thread.title,
    });
  }

  async function handleConfirmRenameThread() {
    if (!renameDialog || renameDialog.submitting)
      return;

    const nextTitle = renameDialog.value.trim();
    if (!nextTitle) {
      setRenameDialog(current => current ? { ...current, error: "Title cannot be empty." } : current);
      return;
    }
    if (nextTitle === renameDialog.thread.title) {
      setRenameDialog(null);
      return;
    }

    setRenameDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await renameThread({ threadId: renameDialog.thread.id, title: nextTitle });
      await refreshStore(renameDialog.thread.id);
      setRenameDialog(null);
    }
    catch (error) {
      setRenameDialog(current =>
        current
          ? {
              ...current,
              error: error instanceof Error ? error.message : String(error),
              submitting: false,
            }
          : current,
      );
    }
  }

  async function handleTogglePinThread(thread: StoredThread) {
    await pinThread({ threadId: thread.id, pinned: !thread.pinned });
    await refreshStore(thread.id);
  }

  async function handleModelChange(modelId: string) {
    setSelectedModelId(modelId);
    if (!activeThread)
      return;

    await updateThreadModel({
      threadId: activeThread.id,
      modelId,
    });
    await refreshStore(activeThread.id);
  }

  async function handleApprovalDecision(
    approval: StoredApprovalRequest,
    status: "approved" | "rejected",
  ) {
    await decideApprovalRequest({
      approvalRequestId: approval.id,
      decisionNote: status === "approved" ? "Approved in GUI." : "Rejected in GUI.",
      status,
    });
    if (activeThreadIdForApprovals) {
      const approvals = await listApprovalRequests(activeThreadIdForApprovals);
      const pending = approvals.filter(item => item.status === "pending");
      setPendingApprovals(pending);
    }
    await refreshStore(activeThread?.id ?? undefined);
  }

  function handleDeleteThread(thread: StoredThread) {
    setDeleteDialog({
      cleanupSummary: null,
      error: null,
      loadingSummary: thread.mode === "chat",
      submitting: false,
      thread,
    });

    if (thread.mode === "chat") {
      void getThreadCleanupSummary(thread.id)
        .then((summary) => {
          setDeleteDialog(current =>
            current?.thread.id === thread.id
              ? { ...current, cleanupSummary: summary, loadingSummary: false }
              : current,
          );
        })
        .catch(() => {
          setDeleteDialog(current =>
            current?.thread.id === thread.id
              ? { ...current, loadingSummary: false }
              : current,
          );
        });
    }
  }

  async function handleConfirmDeleteThread() {
    if (!deleteDialog || deleteDialog.submitting)
      return;

    setDeleteDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await deleteThread(deleteDialog.thread.id);
      await refreshStore(deleteDialog.thread.id === activeThreadId ? undefined : activeThreadId ?? undefined);
      setDeleteDialog(null);
    }
    catch (error) {
      setDeleteDialog(current =>
        current
          ? {
              ...current,
              error: error instanceof Error ? error.message : String(error),
              submitting: false,
            }
          : current,
      );
    }
  }

  async function handleRestoreThread(thread: StoredThread) {
    const restoredThread = await restoreThread(thread.id);
    await refreshStore(restoredThread.id);
    setSection(restoredThread.mode === "workspace" ? "workspace" : "chat");
    setCenterMode("thread");
  }

  function handleToggleLeftPanel() {
    setLeftExpanded((expanded) => {
      const nextExpanded = !expanded;
      setLeftOverlayOpen(false);
      return nextExpanded;
    });
  }

  function handlePreviewLeftPanel(open: boolean) {
    if (leftExpanded)
      return;
    setLeftOverlayOpen(open);
  }

  const activityRailProps = {
    active: section,
    activeThreadId,
    threads,
    threadRunStatuses,
    workspaces,
    onChange: handleSectionChange,
    onNewChat: handleOpenNewChat,
    onDeleteThread: handleDeleteThread,
    onRenameThread: handleRenameThread,
    onRestoreThread: handleRestoreThread,
    onSelectWorkspace: handleSelectWorkspace,
    onSelectThread: handleSelectThread,
    onTogglePinThread: handleTogglePinThread,
    onToggleExpanded: handleToggleLeftPanel,
  };

  return (
    <div className="relative flex h-full min-h-0 overflow-hidden bg-canvas text-ink">
      {leftExpanded ? <ActivityRail expanded {...activityRailProps} /> : null}
      {!leftExpanded
        ? (
            <div
              aria-hidden="true"
              className="absolute left-0 top-0 z-30 h-full w-2 cursor-ew-resize"
              onMouseEnter={() => handlePreviewLeftPanel(true)}
            />
          )
        : null}
      {!leftExpanded && leftOverlayOpen
        ? (
            <div
              className="absolute left-0 top-0 z-40 h-full w-56 md:w-64 xl:w-72"
              onMouseEnter={() => handlePreviewLeftPanel(true)}
              onMouseLeave={() => handlePreviewLeftPanel(false)}
            >
              <ActivityRail expanded floating {...activityRailProps} />
            </div>
          )
        : null}
      <main className="min-w-0 flex-1 bg-surface">
        {centerMode === "new-chat"
          ? (
              <NewConversation
                initialWorkspaceId={newChatWorkspaceId}
                leftPanelExpanded={leftExpanded}
                modelId={selectedModelId}
                modelOptions={modelOptions}
                onAddWorkspace={handleAddWorkspace}
                onModelChange={setSelectedModelId}
                onStart={handleStartNewConversation}
                onToggleLeftPanel={handleToggleLeftPanel}
                workspaces={workspaces.filter(workspace => workspace.kind === "user")}
              />
            )
          : section === "research"
            ? (
                <ResearchView
                  workspaceId={activeWorkspace?.id ?? null}
                  workspaceName={activeWorkspace?.name ?? "No workspace selected"}
                />
              )
            : section === "data" || section === "skill" || section === "settings"
              ? (
                  <ModulePlaceholder section={section} />
                )
              : storeError
                ? (
                    <div className="flex h-full items-center justify-center p-8 text-sm text-ink-soft">
                      FutureOS 本地存储初始化失败：
                      {storeError}
                    </div>
                  )
                : (
                    <AgentThread
                      activeApproval={activeApproval}
                      loadingStore={loadingStore}
                      modelId={activeThread?.modelId ?? selectedModelId}
                      modelOptions={modelOptions}
                      onModelChange={handleModelChange}
                      pendingPrompt={pendingPrompt}
                      thread={activeThread}
                      onApprovalDecision={handleApprovalDecision}
                      leftPanelExpanded={leftExpanded}
                      onToggleLeftPanel={handleToggleLeftPanel}
                      onPromptConsumed={(id) => {
                        setPendingPrompt(current => (current?.id === id ? null : current));
                      }}
                      onThreadActivity={() => {
                        void refreshStore(activeThread?.id ?? undefined);
                      }}
                    />
                  )}
      </main>
      <ContextPanel
        activeThread={activeThread}
        activeWorkspace={activeWorkspace}
        activeTab={contextTab}
        expanded={rightExpanded}
        onTabChange={setContextTab}
        onToggleExpanded={() => setRightExpanded(value => !value)}
      />
      <AppShellDialogs
        deleteDialog={deleteDialog}
        renameDialog={renameDialog}
        setDeleteDialog={setDeleteDialog}
        setRenameDialog={setRenameDialog}
        onConfirmDeleteThread={() => void handleConfirmDeleteThread()}
        onConfirmRenameThread={() => void handleConfirmRenameThread()}
      />
    </div>
  );
}

function ModulePlaceholder({ section }: { section: ActivitySection }) {
  const title = section === "data" ? "Data" : section === "skill" ? "Skill" : "Settings";
  const detail = section === "data"
    ? "Data sources and credentials are planned as the next resource module."
    : section === "skill"
      ? "Skills will manage built-in and workspace-level agent capabilities."
      : "Application and model preferences will live here.";

  return (
    <section className="flex h-full min-h-0 items-center justify-center bg-surface p-8">
      <div className="max-w-md rounded-lg border border-line-soft bg-surface-subtle p-6 text-center">
        <div className="text-base font-semibold text-ink">{title}</div>
        <p className="mt-2 text-sm leading-6 text-ink-muted">{detail}</p>
      </div>
    </section>
  );
}

function deriveThreadTitle(content: string) {
  const compact = content.replace(/\s+/g, " ").trim();
  if (!compact)
    return "New Chat";
  return compact.length > 28 ? `${compact.slice(0, 28)}...` : compact;
}
