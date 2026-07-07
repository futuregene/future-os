import type { FormEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Folder,
  FolderOpen,
  MessageSquare,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { defaultAgentModelId } from "../../integrations/agent/agentClient";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { startWindowDrag } from "../../lib/windowDrag";
import { Composer } from "./Composer";

type ConversationMode = "workspace" | "chat";
type WorkspaceFormMode = "open" | null;

export interface NewConversationStart {
  attachments?: MessageAttachment[];
  content: string;
  mode: ConversationMode;
  modelId: string;
  thinkingLevel: string;
  workspace?: {
    id?: string;
    label: string;
    path: string;
  };
}

interface WorkspaceCreateRequest {
  name?: string | null;
  path: string;
  createDirectory: boolean;
}

interface NewConversationProps {
  initialWorkspaceForm?: WorkspaceFormMode;
  initialMode?: ConversationMode;
  initialWorkspaceId?: string | null;
  leftPanelExpanded: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  onModelChange: (modelId: string) => void;
  thinkingLevel: string;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  approvalTier: ApprovalTier;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  onStart: (input: NewConversationStart) => void;
  onAddWorkspace: (input: WorkspaceCreateRequest) => Promise<StoredWorkspace | null>;
  onToggleLeftPanel: () => void;
  workspaces: StoredWorkspace[];
}

export function NewConversation({
  initialWorkspaceForm,
  initialMode,
  initialWorkspaceId,
  leftPanelExpanded,
  modelId,
  modelOptions,
  onAddWorkspace,
  onModelChange,
  thinkingLevel,
  onThinkingLevelChange,
  approvalTier,
  onChangeApprovalTier,
  onStart,
  onToggleLeftPanel,
  workspaces,
}: NewConversationProps) {
  const { t } = useTranslation("agent");
  const workspaceOptions = useMemo(
    () =>
      workspaces.map(workspace => ({
        id: workspace.id,
        label: workspace.name,
        path: workspace.path,
      })),
    [workspaces],
  );
  const initialWorkspace = workspaceOptions.find(workspace => workspace.id === initialWorkspaceId);
  const [mode, setMode] = useState<ConversationMode>(
    initialMode ?? (initialWorkspace ? "workspace" : workspaceOptions.length > 0 ? "workspace" : "chat"),
  );
  const [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false);
  const [selectedWorkspace, setSelectedWorkspace] = useState(initialWorkspace?.id ?? workspaceOptions[0]?.id ?? "");
  const [workspaceFormMode, setWorkspaceFormMode] = useState<WorkspaceFormMode>(initialWorkspaceForm ?? null);
  const [workspaceDisplayName, setWorkspaceDisplayName] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [workspaceNotice, setWorkspaceNotice] = useState<string | null>(null);
  const [creatingWorkspace, setCreatingWorkspace] = useState(false);
  const workspaceMenuRef = useDismissableLayer<HTMLDivElement>({
    enabled: workspaceMenuOpen,
    onDismiss: () => setWorkspaceMenuOpen(false),
  });

  const activeWorkspace
    = workspaceOptions.find(workspace => workspace.id === selectedWorkspace) ?? workspaceOptions[0];

  useEffect(() => {
    const first = workspaceOptions[0];
    if (activeWorkspace || !first)
      return;

    setSelectedWorkspace(first.id);
  }, [activeWorkspace, workspaceOptions]);

  useEffect(() => {
    if (!initialWorkspaceId)
      return;
    const workspace = workspaceOptions.find(item => item.id === initialWorkspaceId);
    if (!workspace)
      return;

    setSelectedWorkspace(workspace.id);
    setMode("workspace");
  }, [initialWorkspaceId, workspaceOptions]);

  async function handleWorkspaceSubmit(event: FormEvent) {
    event.preventDefault();
    const path = workspacePath.trim();
    const displayName = workspaceDisplayName.trim();
    if (!path) {
      setWorkspaceError(t("newConversation.errorChooseDir"));
      return;
    }

    setCreatingWorkspace(true);
    setWorkspaceError(null);
    try {
      const workspace = await onAddWorkspace({
        name: displayName || null,
        path,
        createDirectory: false,
      });
      if (workspace) {
        setSelectedWorkspace(workspace.id);
        setMode("workspace");
      }
      setWorkspaceDisplayName("");
      setWorkspacePath("");
      setWorkspaceFormMode(null);
      setWorkspaceMenuOpen(false);
    }
    catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
    }
    finally {
      setCreatingWorkspace(false);
    }
  }

  function beginWorkspaceForm() {
    setWorkspaceFormMode("open");
    setWorkspaceMenuOpen(false);
    setWorkspaceError(null);
    setWorkspaceNotice(null);
    setWorkspaceDisplayName("");
    setWorkspacePath("");
  }

  // Cancelling the create-workspace dialog: fall back to the most-recently-used
  // workspace when one exists (list is ordered last-opened first), otherwise
  // there's nothing to land on, so switch to plain chat.
  function cancelWorkspaceForm() {
    setWorkspaceFormMode(null);
    setWorkspaceError(null);
    setWorkspaceNotice(null);
    setWorkspaceDisplayName("");
    setWorkspacePath("");
    const mostRecent = workspaceOptions[0];
    if (mostRecent) {
      setSelectedWorkspace(mostRecent.id);
      setMode("workspace");
    }
    else {
      setMode("chat");
    }
  }

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("newConversation.openWorkspaceDialog"),
    });

    if (typeof selected === "string") {
      setWorkspacePath(selected);
      // Opening a folder that already has a workspace record just reopens the
      // existing one (the backend dedupes by path) — flag it so the user knows.
      const existing = workspaces.find(workspace => samePath(workspace.path, selected));
      setWorkspaceNotice(existing ? t("newConversation.workspaceExists", { name: existing.name }) : null);
      if (!workspaceDisplayName.trim()) {
        setWorkspaceDisplayName(lastPathSegment(selected));
      }
    }
  }

  function handleSend({ attachments, content }: ComposerSendPayload) {
    if (mode === "workspace" && activeWorkspace) {
      onStart({
        attachments,
        content,
        mode,
        modelId: modelId || defaultAgentModelId,
        thinkingLevel,
        workspace: activeWorkspace,
      });
      return;
    }

    onStart({
      attachments,
      content,
      mode: "chat",
      modelId: modelId || defaultAgentModelId,
      thinkingLevel,
    });
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle
            expanded={leftPanelExpanded}
            onToggle={onToggleLeftPanel}
          />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-ink">{t("newConversation.headerTitle")}</div>
            <div className="truncate text-xs text-ink-muted">
              {mode === "workspace" && activeWorkspace ? activeWorkspace.label : t("newConversation.chatSubtitle")}
            </div>
          </div>
        </div>
      </header>
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-8">
        <div className="flex w-full max-w-4xl flex-col items-center">
          <h1 className="mb-8 text-center text-3xl font-semibold tracking-normal text-ink">{t("newConversation.welcome")}</h1>
          <div className="w-full max-w-3xl">
            <Composer
              className="w-full rounded-b-none bg-surface"
              modelId={modelId}
              modelOptions={modelOptions}
              onModelChange={onModelChange}
              thinkingLevel={thinkingLevel}
              onThinkingLevelChange={onThinkingLevelChange}
              approvalTier={approvalTier}
              onChangeApprovalTier={onChangeApprovalTier}
              onSend={handleSend}
              placeholder={t("newConversation.placeholder")}
              textareaClassName="h-16 text-base leading-6"
              workspaceId={mode === "workspace" ? activeWorkspace?.id : null}
            />
            <div className="relative flex flex-wrap items-center gap-1.5 rounded-b-lg bg-surface-subtle/80 p-1.5">
              <div className="relative" ref={workspaceMenuRef}>
                <button
                  className={cn(
                    "inline-flex h-8 max-w-64 items-center gap-2 rounded-md px-2.5 text-sm font-medium transition-colors",
                    mode === "workspace"
                      ? "bg-surface text-ink shadow-xs"
                      : "text-ink-soft hover:bg-surface/70 hover:text-ink",
                  )}
                  onClick={() => {
                    setMode("workspace");
                    setWorkspaceMenuOpen(open => !open);
                  }}
                  type="button"
                >
                  <Folder className="size-4 shrink-0" />
                  <span className="truncate">{activeWorkspace?.label ?? t("newConversation.workspace")}</span>
                </button>
                {workspaceMenuOpen
                  ? (
                      <div className="absolute left-0 top-9 z-30 w-72 rounded-lg border border-line-soft bg-surface p-1.5 shadow-panel">
                        <div className="max-h-36 space-y-0.5 overflow-auto">
                          {workspaceOptions.length === 0
                            ? (
                                <div className="p-2 text-sm text-ink-muted">{t("newConversation.noWorkspaces")}</div>
                              )
                            : null}
                          {workspaceOptions.map(workspace => (
                            <button
                              key={workspace.id}
                              className="flex h-9 w-full items-center gap-2 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle"
                              onClick={() => {
                                setSelectedWorkspace(workspace.id);
                                setMode("workspace");
                                setWorkspaceMenuOpen(false);
                              }}
                              type="button"
                            >
                              <Folder className="size-4 shrink-0 text-ink-soft" />
                              <span className="min-w-0 flex-1">
                                <span className="block truncate text-sm font-medium text-ink">{workspace.label}</span>
                                <span className="block truncate text-xs text-ink-muted">{workspace.path}</span>
                              </span>
                              {selectedWorkspace === workspace.id ? <Check className="size-4 text-ink-soft" /> : null}
                            </button>
                          ))}
                        </div>
                        <div className="mt-1 space-y-0.5 border-t border-line-soft pt-1">
                          <button
                            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                            onClick={beginWorkspaceForm}
                            type="button"
                          >
                            <FolderOpen className="size-3.5" />
                            <span>{t("newConversation.openExisting")}</span>
                          </button>
                          <button
                            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                            onClick={() => {
                              setMode("chat");
                              setWorkspaceMenuOpen(false);
                            }}
                            type="button"
                          >
                            <MessageSquare className="size-3.5" />
                            <span>{t("newConversation.chat")}</span>
                          </button>
                        </div>
                      </div>
                    )
                  : null}
              </div>
              <button
                className={cn(
                  "inline-flex h-8 items-center gap-2 rounded-md px-3 text-sm font-medium transition-colors",
                  mode === "chat"
                    ? "bg-surface text-ink shadow-xs"
                    : "text-ink-soft hover:bg-surface/70 hover:text-ink",
                )}
                onClick={() => {
                  setMode("chat");
                  setWorkspaceMenuOpen(false);
                }}
                type="button"
              >
                <MessageSquare className="size-4" />
                <span>{t("newConversation.chat")}</span>
              </button>
              <div className="min-w-56 flex-1 px-1 text-xs leading-5 text-ink-muted">
                {mode === "workspace" && activeWorkspace
                  ? t("newConversation.workspaceHint")
                  : t("newConversation.chatHint")}
              </div>
            </div>
          </div>
        </div>
      </div>
      {workspaceFormMode
        ? (
            <WorkspaceModal
              creating={creatingWorkspace}
              error={workspaceError}
              notice={workspaceNotice}
              displayName={workspaceDisplayName}
              path={workspacePath}
              onCancel={cancelWorkspaceForm}
              onDisplayNameChange={setWorkspaceDisplayName}
              onPickFolder={pickFolder}
              onSubmit={handleWorkspaceSubmit}
            />
          )
        : null}
    </div>
  );
}

function WorkspaceModal({
  creating,
  displayName,
  error,
  notice,
  path,
  onCancel,
  onDisplayNameChange,
  onPickFolder,
  onSubmit,
}: {
  creating: boolean;
  displayName: string;
  error: string | null;
  notice: string | null;
  path: string;
  onCancel: () => void;
  onDisplayNameChange: (value: string) => void;
  onPickFolder: () => void;
  onSubmit: (event: FormEvent) => void;
}) {
  const { t } = useTranslation("agent");
  const pathPlaceholder = t("newConversation.modal.selectExisting");

  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-ink-strong/20 px-6">
      <form className="w-full max-w-md rounded-lg border border-line-soft bg-surface p-4 shadow-panel" onSubmit={onSubmit}>
        <div className="mb-3">
          <div className="text-sm font-semibold text-ink">{t("newConversation.modal.openTitle")}</div>
          <div className="mt-1 text-xs leading-5 text-ink-muted">{t("newConversation.modal.openDescription")}</div>
        </div>
        <div className="space-y-2">
          <div className="flex h-9 overflow-hidden rounded-md border border-line-soft bg-surface">
            <button
              aria-label={t("newConversation.modal.chooseWorkspaceAria")}
              className="inline-flex h-full w-10 shrink-0 items-center justify-center border-r border-line-soft text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
              onClick={onPickFolder}
              title={t("newConversation.modal.chooseWorkspaceAria")}
              type="button"
            >
              <FolderOpen className="size-4" />
            </button>
            <div
              className={cn(
                "flex min-w-0 flex-1 items-center px-2 text-sm",
                path ? "text-ink" : "text-ink-muted",
              )}
              title={path || pathPlaceholder}
            >
              <span className="truncate">{path || pathPlaceholder}</span>
            </div>
          </div>
          <input
            className="h-9 w-full rounded-md border border-line-soft bg-surface px-2 text-sm text-ink outline-none focus:border-accent"
            onChange={event => onDisplayNameChange(event.target.value)}
            placeholder={t("newConversation.modal.displayNameOpen")}
            value={displayName}
          />
          <div className="min-h-5 truncate px-1 text-xs text-ink-muted" title={path || undefined}>
            {path
              ? t("newConversation.modal.pathPreview", { path })
              : t("newConversation.modal.chooseExisting")}
          </div>
        </div>
        {error ? <div className="mt-2 text-xs leading-5 text-danger">{error}</div> : null}
        {!error && notice ? <div className="mt-2 text-xs leading-5 text-warning">{notice}</div> : null}
        <div className="mt-4 flex justify-end gap-2">
          <button
            className="h-8 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            onClick={onCancel}
            type="button"
          >
            {t("newConversation.modal.cancel")}
          </button>
          <Button
            disabled={creating}
            size="sm"
            type="submit"
            variant="primary"
          >
            {t("newConversation.modal.open")}
          </Button>
        </div>
      </form>
    </div>
  );
}

function lastPathSegment(path: string) {
  // Split on both separators so Windows paths (C:\Users\project) work too,
  // mirroring fileNameFromPath in attachments.ts.
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? "Workspace";
}

/**
 * Compare filesystem paths ignoring a trailing separator and case (macOS /
 * Windows are case-insensitive; the backend also dedupes by exact string).
 */
function samePath(a: string, b: string) {
  const normalize = (value: string) => value.replace(/[/\\]+$/, "").toLowerCase();
  return normalize(a) === normalize(b);
}
