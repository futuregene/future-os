import type { FormEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Folder,
  FolderOpen,
  FolderPlus,
  MessageSquare,
  Search,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { defaultAgentModelId } from "../../integrations/agent/agentClient";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { startWindowDrag } from "../../lib/windowDrag";
import { Composer } from "./Composer";

type ConversationMode = "workspace" | "chat";
type WorkspaceFormMode = "open" | "create" | null;

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
  initialCreateWorkspace?: boolean;
  initialMode?: ConversationMode;
  initialWorkspaceId?: string | null;
  leftPanelExpanded: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  onModelChange: (modelId: string) => void;
  thinkingLevel: string;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  onStart: (input: NewConversationStart) => void;
  onAddWorkspace: (input: WorkspaceCreateRequest) => Promise<StoredWorkspace | null>;
  onToggleLeftPanel: () => void;
  workspaces: StoredWorkspace[];
}

export function NewConversation({
  initialCreateWorkspace,
  initialMode,
  initialWorkspaceId,
  leftPanelExpanded,
  modelId,
  modelOptions,
  onAddWorkspace,
  onModelChange,
  thinkingLevel,
  onThinkingLevelChange,
  onStart,
  onToggleLeftPanel,
  workspaces,
}: NewConversationProps) {
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
  const [workspaceFormMode, setWorkspaceFormMode] = useState<WorkspaceFormMode>(initialCreateWorkspace ? "create" : null);
  const [workspaceName, setWorkspaceName] = useState("");
  const [workspaceDisplayName, setWorkspaceDisplayName] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [creatingWorkspace, setCreatingWorkspace] = useState(false);
  const workspaceMenuRef = useDismissableLayer<HTMLDivElement>({
    enabled: workspaceMenuOpen,
    onDismiss: () => setWorkspaceMenuOpen(false),
  });

  const activeWorkspace
    = workspaceOptions.find(workspace => workspace.id === selectedWorkspace) ?? workspaceOptions[0];

  useEffect(() => {
    if (activeWorkspace || workspaceOptions.length === 0)
      return;

    setSelectedWorkspace(workspaceOptions[0].id);
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
    const rawPath = workspacePath.trim();
    const name = workspaceName.trim();
    const displayName = workspaceDisplayName.trim();
    const path = workspaceFormMode === "create" && name ? joinPath(rawPath, name) : rawPath;
    if (!path) {
      setWorkspaceError("Choose a workspace directory.");
      return;
    }
    if (workspaceFormMode === "create" && !name) {
      setWorkspaceError("Enter a workspace name.");
      return;
    }

    setCreatingWorkspace(true);
    setWorkspaceError(null);
    try {
      const workspace = await onAddWorkspace({
        name: displayName || (workspaceFormMode === "create" ? name : null),
        path,
        createDirectory: workspaceFormMode === "create",
      });
      if (workspace) {
        setSelectedWorkspace(workspace.id);
        setMode("workspace");
      }
      setWorkspaceName("");
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

  function beginWorkspaceForm(nextMode: Exclude<WorkspaceFormMode, null>) {
    setWorkspaceFormMode(nextMode);
    setWorkspaceMenuOpen(false);
    setWorkspaceError(null);
    setWorkspaceName("");
    setWorkspaceDisplayName("");
    setWorkspacePath("");
  }

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: workspaceFormMode === "create" ? "Choose parent workspace" : "Open workspace",
    });

    if (typeof selected === "string") {
      setWorkspacePath(selected);
      if (workspaceFormMode === "open" && !workspaceDisplayName.trim()) {
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
            <div className="truncate text-sm font-semibold text-ink">新对话</div>
            <div className="truncate text-xs text-ink-muted">
              {mode === "workspace" && activeWorkspace ? activeWorkspace.label : "普通对话"}
            </div>
          </div>
        </div>
      </header>
      <div className="flex min-h-0 flex-1 items-center justify-center overflow-auto p-8">
        <div className="flex w-full max-w-4xl flex-col items-center">
          <h1 className="mb-8 text-center text-3xl font-semibold tracking-normal text-ink">欢迎使用</h1>
          <div className="w-full max-w-3xl">
            <Composer
              className="w-full rounded-b-none bg-surface"
              modelId={modelId}
              modelOptions={modelOptions}
              onModelChange={onModelChange}
              thinkingLevel={thinkingLevel}
              onThinkingLevelChange={onThinkingLevelChange}
              onSend={handleSend}
              placeholder="随心输入"
              textareaClassName="h-16 text-base leading-6"
              workspaceId={mode === "workspace" ? activeWorkspace?.id : null}
            />
            <div className="relative flex flex-wrap items-center gap-1.5 rounded-b-lg bg-surface-subtle/80 p-1.5">
              <div className="relative" ref={workspaceMenuRef}>
                <button
                  className={cn(
                    "inline-flex h-8 max-w-64 items-center gap-2 rounded-md px-2.5 text-sm font-medium transition-colors",
                    mode === "workspace"
                      ? "bg-surface text-ink shadow-sm"
                      : "text-ink-soft hover:bg-surface/70 hover:text-ink",
                  )}
                  onClick={() => {
                    setMode("workspace");
                    setWorkspaceMenuOpen(open => !open);
                  }}
                  type="button"
                >
                  <Folder className="size-4 shrink-0" />
                  <span className="truncate">{activeWorkspace?.label ?? "Workspace"}</span>
                </button>
                {workspaceMenuOpen
                  ? (
                      <div className="absolute left-0 top-9 z-30 w-72 rounded-lg border border-line-soft bg-surface p-1.5 shadow-panel">
                        <div className="flex h-8 items-center gap-2 rounded-md px-2 text-sm text-ink-muted">
                          <Search className="size-3.5" />
                          <span>Search workspace</span>
                        </div>
                        <div className="mt-1 max-h-36 space-y-0.5 overflow-auto">
                          {workspaceOptions.length === 0
                            ? (
                                <div className="p-2 text-sm text-ink-muted">No workspaces yet.</div>
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
                            onClick={() => beginWorkspaceForm("open")}
                            type="button"
                          >
                            <FolderOpen className="size-3.5" />
                            <span>Open Existing Workspace</span>
                          </button>
                          <button
                            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                            onClick={() => beginWorkspaceForm("create")}
                            type="button"
                          >
                            <FolderPlus className="size-3.5" />
                            <span>Create New Workspace</span>
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
                            <span>Chat</span>
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
                    ? "bg-surface text-ink shadow-sm"
                    : "text-ink-soft hover:bg-surface/70 hover:text-ink",
                )}
                onClick={() => {
                  setMode("chat");
                  setWorkspaceMenuOpen(false);
                }}
                type="button"
              >
                <MessageSquare className="size-4" />
                <span>Chat</span>
              </button>
              <div className="min-w-56 flex-1 px-1 text-xs leading-5 text-ink-muted">
                {mode === "workspace" && activeWorkspace
                  ? "Uses this project folder and its workspace resources."
                  : "Starts quickly with a temporary workspace managed by FutureOS."}
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
              mode={workspaceFormMode}
              directoryName={workspaceName}
              displayName={workspaceDisplayName}
              path={workspacePath}
              onCancel={() => setWorkspaceFormMode(null)}
              onDirectoryNameChange={setWorkspaceName}
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
  directoryName,
  displayName,
  error,
  mode,
  path,
  onCancel,
  onDirectoryNameChange,
  onDisplayNameChange,
  onPickFolder,
  onSubmit,
}: {
  creating: boolean;
  directoryName: string;
  displayName: string;
  error: string | null;
  mode: Exclude<WorkspaceFormMode, null>;
  path: string;
  onCancel: () => void;
  onDirectoryNameChange: (value: string) => void;
  onDisplayNameChange: (value: string) => void;
  onPickFolder: () => void;
  onSubmit: (event: FormEvent) => void;
}) {
  const pathPlaceholder = mode === "open" ? "Select existing workspace" : "Select parent workspace";
  const previewPath = mode === "create" && path && directoryName.trim()
    ? joinPath(path, directoryName.trim())
    : path;

  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-ink-strong/20 px-6">
      <form className="w-full max-w-md rounded-lg border border-line-soft bg-surface p-4 shadow-panel" onSubmit={onSubmit}>
        <div className="mb-3">
          <div className="text-sm font-semibold text-ink">
            {mode === "open" ? "Open Existing Workspace" : "Create New Workspace"}
          </div>
          <div className="mt-1 text-xs leading-5 text-ink-muted">
            {mode === "open"
              ? "Use an existing workspace directory."
              : "Choose a parent workspace directory, then name the new workspace."}
          </div>
        </div>
        <div className="space-y-2">
          <div className="flex h-9 overflow-hidden rounded-md border border-line-soft bg-surface">
            <button
              aria-label={mode === "create" ? "Choose parent workspace" : "Choose workspace"}
              className="inline-flex h-full w-10 shrink-0 items-center justify-center border-r border-line-soft text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
              onClick={onPickFolder}
              title={mode === "create" ? "Choose parent workspace" : "Choose workspace"}
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
            {mode === "create"
              ? (
                  <>
                    <div className="flex h-full shrink-0 items-center border-l border-line-soft px-1 text-sm text-ink-muted">
                      /
                    </div>
                    <input
                      className="h-full min-w-0 flex-[0.55] border-0 bg-surface px-2 text-sm text-ink outline-none placeholder:text-ink-muted"
                      onChange={event => onDirectoryNameChange(event.target.value)}
                      placeholder="Workspace directory"
                      value={directoryName}
                    />
                  </>
                )
              : null}
          </div>
          <input
            className="h-9 w-full rounded-md border border-line-soft bg-surface px-2 text-sm text-ink outline-none focus:border-accent"
            onChange={event => onDisplayNameChange(event.target.value)}
            placeholder={
              mode === "create"
                ? "Display name (optional, defaults to workspace directory)"
                : "Display name (optional, defaults to selected directory)"
            }
            value={displayName}
          />
          <div className="min-h-5 truncate px-1 text-xs text-ink-muted" title={previewPath || undefined}>
            {mode === "create"
              ? previewPath
                ? `Workspace path: ${previewPath}`
                : "Workspace path will be created under the selected directory."
              : path
                ? `Workspace path: ${path}`
                : "Choose an existing workspace directory."}
          </div>
        </div>
        {error ? <div className="mt-2 text-xs leading-5 text-danger">{error}</div> : null}
        <div className="mt-4 flex justify-end gap-2">
          <button
            className="h-8 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            onClick={onCancel}
            type="button"
          >
            Cancel
          </button>
          <Button
            disabled={creating}
            size="sm"
            type="submit"
            variant="primary"
          >
            {mode === "open" ? "Open" : "Create"}
          </Button>
        </div>
      </form>
    </div>
  );
}

function lastPathSegment(path: string) {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? "Workspace";
}

function joinPath(parent: string, child: string) {
  if (!parent)
    return child;
  if (parent.endsWith("/"))
    return `${parent}${child}`;
  return `${parent}/${child}`;
}
