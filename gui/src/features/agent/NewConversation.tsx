import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import type { WorkspaceCreateRequest, WorkspaceFormMode } from "./useWorkspaceForm";
import {
  Check,
  Folder,
  FolderOpen,
  MessageSquare,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { defaultAgentModelId } from "../../integrations/agent/agentClient";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { startWindowDrag } from "../../lib/windowDrag";
import { Composer } from "./Composer";
import { WorkspaceModal } from "./NewConversationWorkspaceForm";
import { useWorkspaceForm } from "./useWorkspaceForm";

export type ConversationMode = "workspace" | "chat";

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

interface NewConversationProps {
  initialWorkspaceForm?: WorkspaceFormMode;
  initialMode?: ConversationMode;
  initialWorkspaceId?: string | null;
  leftPanelExpanded: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  modelsEmptyReason?: "no_models" | "all_disabled";
  onModelChange: (modelId: string) => void;
  thinkingLevel: string;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  approvalTier: ApprovalTier;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  /**
   * May return a promise: the composer then keeps the draft until it resolves
   * (a failed thread creation must not wipe the composed first message).
   */
  onStart: (input: NewConversationStart) => void | Promise<void>;
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
  modelsEmptyReason,
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
  const workspaceForm = useWorkspaceForm({
    initialWorkspaceForm,
    workspaces,
    onAddWorkspace,
    onSelectWorkspace: setSelectedWorkspace,
    onModeChange: setMode,
    onCloseMenu: () => setWorkspaceMenuOpen(false),
  });
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

  // Adopt the caller-requested workspace exactly once (when it appears in the
  // options, which may lag the mount). Without the ref guard this effect
  // re-fires whenever `workspaceOptions` changes identity — AppShell re-renders
  // at least every run-status poll tick — and would snap a user-chosen
  // mode/workspace back to the initial one mid-composition.
  const adoptedInitialWorkspaceRef = useRef(false);
  useEffect(() => {
    if (!initialWorkspaceId || adoptedInitialWorkspaceRef.current)
      return;
    const workspace = workspaceOptions.find(item => item.id === initialWorkspaceId);
    if (!workspace)
      return;

    adoptedInitialWorkspaceRef.current = true;
    setSelectedWorkspace(workspace.id);
    setMode("workspace");
  }, [initialWorkspaceId, workspaceOptions]);

  function handleSend({ attachments, content }: ComposerSendPayload) {
    // Return onStart's promise so the composer clears only after the thread is
    // actually created (see ComposerProps.onSend).
    if (mode === "workspace" && activeWorkspace) {
      return onStart({
        attachments,
        content,
        mode,
        modelId: modelId || defaultAgentModelId,
        thinkingLevel,
        workspace: activeWorkspace,
      });
    }

    return onStart({
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
              modelsEmptyReason={modelsEmptyReason}
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
                  title={activeWorkspace?.label ?? t("newConversation.workspace")}
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
                            onClick={workspaceForm.begin}
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
      {workspaceForm.mode
        ? (
            <WorkspaceModal
              creating={workspaceForm.creating}
              error={workspaceForm.error}
              notice={workspaceForm.notice}
              displayName={workspaceForm.displayName}
              path={workspaceForm.path}
              onCancel={workspaceForm.cancel}
              onDisplayNameChange={workspaceForm.setDisplayName}
              onPickFolder={() => void workspaceForm.pickFolder()}
              onSubmit={workspaceForm.submit}
            />
          )
        : null}
    </div>
  );
}
