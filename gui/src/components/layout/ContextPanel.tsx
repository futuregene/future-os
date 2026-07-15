import type { MouseEvent as ReactMouseEvent } from "react";
import type { ReviewBase } from "../../integrations/storage/review";
import type { StoredRun, StoredThread, StoredToolCall, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ContextTab } from "./hooks/useContextData";
import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArtifactDetailPanel } from "../../features/artifacts/ArtifactDetailPanel";
import { ArtifactsPanel } from "../../features/artifacts/ArtifactsPanel";
import { FileTreePanel } from "../../features/filetree/FileTreePanel";
import { ReviewPanel } from "../../features/review/ReviewPanel";
import { RunInspectPanel } from "../../features/runs/RunInspectPanel";
import { RunsPanel } from "../../features/runs/RunsPanel";
import {
  abortRun,
  clearFinishedRuns,
} from "../../integrations/storage/threadStore";
import { onFutureEvent } from "../../lib/futureEvents";
import { startWindowDrag } from "../../lib/windowDrag";
import { EmptyState } from "../ui/EmptyState";
import { IconButton } from "../ui/IconButton";
import { Select } from "../ui/Select";
import { useContextData } from "./hooks/useContextData";

export type { ContextTab };
export type { ReviewBase };

// The Files tab appears for every thread — its root is the thread's workspace
// path (chat threads use their temporary workspace), which is available
// immediately, independent of the git/review capability probe.
const gitTabs = [
  { value: "files", labelKey: "contextPanel.files" },
  { value: "runs", labelKey: "contextPanel.runs" },
  { value: "review", labelKey: "contextPanel.review" },
] satisfies Array<{ value: ContextTab; labelKey: string }>;

// Artifacts is hidden pending a decision on what the tab is for (PRODUCT.md
// §4.8). Chat's workspace is a per-thread empty directory, so every file in it
// is by definition this conversation's output — Files already lists exactly
// that, from the same `activeWorkspace.path`, and it can't miss the ones bash
// produced. The artifacts table instead records write/edit tool calls, which is
// a strictly smaller, lossier view of the same directory, and the three things
// that once justified it are all gone: `@`-referencing artifacts is disabled
// (`isFutureReferenceType`), the cards deliberately show no summary or type
// badge, and Research (the "reusable output" consumer) is deferred.
//
// Scanning the directory instead would close the gap on recall, but the deeper
// problem is that "output" is a semantic claim and a filesystem only carries
// syntax (path, extension, mtime) — no scan can assert *this one matters*. If
// this tab comes back, it likely belongs on roadmap item 4 next to unified `@`
// references, and curated (the Agent or the user marks a deliverable) rather
// than scanned. Everything below it — panel, detail view, store, upload — is
// left intact and reachable only from here.
//   { value: "artifacts", labelKey: "contextPanel.artifacts" },
const fileTabs = [
  { value: "files", labelKey: "contextPanel.files" },
  { value: "runs", labelKey: "contextPanel.runs" },
] satisfies Array<{ value: ContextTab; labelKey: string }>;

const pendingTabs = [
  { value: "files", labelKey: "contextPanel.files" },
  { value: "runs", labelKey: "contextPanel.runs" },
] satisfies Array<{ value: ContextTab; labelKey: string }>;

interface ContextPanelProps {
  activeThread: StoredThread | null;
  activeWorkspace: StoredWorkspace | null;
  activeTab: ContextTab;
  expanded: boolean;
  /** Current panel width in px (drag-resized, session-persisted). */
  width: number;
  onResizeStart: (event: ReactMouseEvent) => void;
  /** Keyboard-driven resize (arrow keys on the divider), in px. */
  onResizeNudge: (deltaPx: number) => void;
  onTabChange: (tab: ContextTab) => void;
  onToggleExpanded: () => void;
}

export function ContextPanel({
  activeThread,
  activeWorkspace,
  activeTab,
  expanded,
  width,
  onResizeStart,
  onResizeNudge,
  onTabChange,
  onToggleExpanded,
}: ContextPanelProps) {
  const { t } = useTranslation("layout");
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [selectedToolId, setSelectedToolId] = useState<string | null>(null);
  // Guards the once-per-open default-tab seed: armed (false) while the panel is
  // closed, tripped (true) after seeding so a thread switch mid-open never
  // re-seeds. See the seeding effect below.
  const seededForOpenRef = useRef(false);
  const activeThreadId = activeThread?.id ?? null;
  const activeThreadMode = activeThread?.mode ?? null;
  const activeWorkspaceId = activeWorkspace?.id ?? activeThread?.workspaceId ?? null;

  const {
    runs,
    toolsByRun,
    artifacts,
    gitReview,
    reviewCapabilities,
    loading,
    reviewBase,
    setReviewBase,
    reviewCustomBase,
    setReviewCustomBase,
    refreshContext,
  } = useContextData({ activeThreadId, activeThreadMode, activeWorkspaceId, activeTab, expanded });

  // Workspace-mode threads (git or not) show Review (§14.6); chat keeps Artifacts.
  // Tab choice is driven by capabilities (cheap), not the whole-tree git diff (C3).
  const isWorkspaceThread = activeThreadMode === "workspace";
  const workspaceKindPending = activeThreadId !== null && isWorkspaceThread && reviewCapabilities === null;
  const tabs = workspaceKindPending ? pendingTabs : isWorkspaceThread ? gitTabs : fileTabs;
  const changePreview = reviewCapabilities?.changePreview ?? "ready";
  const hasContextData = runs.length > 0
    || artifacts.length > 0
    || (gitReview?.files.length ?? 0) > 0;
  const showInitialLoading = loading && (!hasContextData || workspaceKindPending);
  const selectedArtifact = selectedArtifactId
    ? artifacts.find(artifact => artifact.id === selectedArtifactId) ?? null
    : null;
  // The Runs panel drills into a single tool call; find it (and its owning run)
  // across the per-run tool map so the inspector reuses the run detail view.
  const selectedTool = selectedToolId
    ? Object.entries(toolsByRun).reduce<{ run: StoredRun; tool: StoredToolCall } | null>((found, [runId, tools]) => {
        if (found)
          return found;
        const tool = tools.find(entry => entry.id === selectedToolId);
        if (!tool)
          return null;
        const run = runs.find(entry => entry.id === runId);
        return run ? { run, tool } : null;
      }, null)
    : null;

  useEffect(() => {
    if (!tabs.some(tab => tab.value === activeTab)) {
      const first = tabs[0];
      if (first)
        onTabChange(first.value);
    }
  }, [activeTab, onTabChange, tabs]);

  async function handleTerminateRun(run: StoredRun) {
    if (!activeThreadId)
      return;

    await abortRun({ runId: run.id, threadId: activeThreadId });
    await refreshContext();
  }

  async function handleClearFinishedRuns() {
    if (!activeThreadId)
      return;

    await clearFinishedRuns(activeThreadId);
    await refreshContext();
  }

  const handleSelectRun = useCallback((runId: string) => {
    setSelectedRunId(runId);
    setSelectedToolId(null);
    setSelectedArtifactId(null);
    if (activeTab !== "runs") {
      onTabChange("runs");
    }
  }, [activeTab, onTabChange]);

  const handleSelectTool = useCallback((toolId: string) => {
    setSelectedToolId(toolId);
    setSelectedRunId(null);
    setSelectedArtifactId(null);
    if (activeTab !== "runs") {
      onTabChange("runs");
    }
  }, [activeTab, onTabChange]);

  const handleSelectArtifact = useCallback((artifactId: string) => {
    setSelectedArtifactId(artifactId);
    setSelectedRunId(null);
    if (activeTab !== "artifacts") {
      onTabChange("artifacts");
    }
    setSelectedToolId(null);
  }, [activeTab, onTabChange]);

  useEffect(() => {
    setSelectedArtifactId(null);
    setSelectedRunId(null);
    setSelectedToolId(null);
  }, [activeThreadId]);

  // Seed the content tab (Review for workspace threads, Files for chat) each
  // time the panel is OPENED MANUALLY (toggle button). When an inspect-run or
  // inspect-tool event opens the panel, the event handler already set the tab;
  // don't override it.
  useEffect(() => {
    if (!expanded) {
      seededForOpenRef.current = false; // Re-arm for the next open.
      return;
    }
    if (activeThreadId === null || workspaceKindPending || seededForOpenRef.current)
      return;
    seededForOpenRef.current = true;
    // If a tool / run inspection just opened the panel it already picked the
    // runs tab — leave that choice alone.
    if (selectedToolId || selectedRunId)
      return;
    const preferred: ContextTab = "files";
    if (activeTab !== preferred)
      onTabChange(preferred);
  }, [expanded, activeThreadId, workspaceKindPending, activeTab, onTabChange, selectedToolId, selectedRunId]);

  useEffect(() => {
    const unsubscribers = [
      onFutureEvent("inspect-run", (detail) => {
        handleSelectRun(detail.runId);
        if (!expanded) {
          onToggleExpanded();
        }
      }),
      onFutureEvent("inspect-tool", (detail) => {
        handleSelectTool(detail.toolId);
        setSelectedRunId(detail.runId);
        if (!expanded) {
          onToggleExpanded();
        }
      }),
      onFutureEvent("inspect-artifact", (detail) => {
        handleSelectArtifact(detail.artifactId);
        if (!expanded) {
          onToggleExpanded();
        }
      }),
      onFutureEvent("open-review", () => {
        setSelectedArtifactId(null);
        setSelectedRunId(null);
        setSelectedToolId(null);
        onTabChange("review");
        if (!expanded) {
          onToggleExpanded();
        }
      }),
    ];
    return () => unsubscribers.forEach(unsubscribe => unsubscribe());
  }, [expanded, handleSelectArtifact, handleSelectRun, handleSelectTool, onTabChange, onToggleExpanded]);

  if (!expanded) {
    return (
      <button
        aria-label={t("contextPanel.expand")}
        title={t("contextPanel.expand")}
        className="absolute right-3 top-2 z-30 inline-flex size-8 items-center justify-center rounded-md border border-transparent bg-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={onToggleExpanded}
        type="button"
      >
        <PanelRightOpen className="size-3.5" />
      </button>
    );
  }

  return (
    <aside
      className="relative flex shrink-0 flex-col border-l border-line-soft bg-surface-subtle"
      style={{ width }}
    >
      {/* Divider: drag to resize the center/right split. Sits astride the left
          border with a wider invisible hit area — no visual line, just the
          resize cursor. */}
      <div
        aria-label={t("contextPanel.resize")}
        aria-orientation="vertical"
        className="absolute -left-1 top-0 z-20 h-full w-2 cursor-ew-resize"
        onMouseDown={onResizeStart}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") {
            event.preventDefault();
            onResizeNudge(16);
          }
          else if (event.key === "ArrowRight") {
            event.preventDefault();
            onResizeNudge(-16);
          }
        }}
        role="separator"
        tabIndex={0}
      />
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="inline-block max-w-full">
          <label className="sr-only" htmlFor="context-panel-view">{t("contextPanel.panelView")}</label>
          <Select
            className="w-fit min-w-24 max-w-full py-0 font-normal hover:border-line"
            id="context-panel-view"
            onChange={event => onTabChange(event.target.value as ContextTab)}
            size="sm"
            value={activeTab}
            wrapperClassName="max-w-full"
          >
            {tabs.map(tab => (
              <option key={tab.value} value={tab.value}>{t(tab.labelKey)}</option>
            ))}
          </Select>
        </div>
        <IconButton
          icon={<PanelRightClose className="size-3.5" />}
          label={t("contextPanel.collapse")}
          onClick={onToggleExpanded}
        />
      </header>
      <div className="min-h-0 flex-1 overflow-auto px-4 pb-4 pt-2">
        {showInitialLoading ? <div className="py-4 text-sm text-ink-muted">{t("contextPanel.loading")}</div> : null}
        {!showInitialLoading && !activeThread ? <EmptyState title={t("contextPanel.noThreadSelected")} /> : null}
        {!showInitialLoading && activeThread && activeTab === "runs"
          ? selectedTool
            ? (
                <RunInspectPanel
                  compact
                  run={selectedTool.run}
                  tools={[selectedTool.tool]}
                  onBack={() => setSelectedToolId(null)}
                />
              )
            : selectedToolId
              ? <div className="py-4 text-sm text-ink-muted">{t("contextPanel.loading")}</div>
              : (
                  <RunsPanel
                    runs={runs}
                    toolsByRun={toolsByRun}
                    workspacePath={activeWorkspace?.path ?? null}
                    onClearFinished={handleClearFinishedRuns}
                    onInspectTool={handleSelectTool}
                    onTerminateRun={handleTerminateRun}
                  />
                )
          : null}
        {!showInitialLoading && activeThread && activeTab === "review"
          ? (
              <ReviewPanel
                capabilities={reviewCapabilities}
                changePreview={changePreview}
                customBase={reviewCustomBase}
                review={gitReview}
                reviewBase={reviewBase}
                threadId={activeThread.id}
                onCustomBaseChange={setReviewCustomBase}
                onReviewBaseChange={setReviewBase}
              />
            )
          : null}
        {/* Unreachable while Artifacts is hidden: no tab selects it, and the
            only other way in — `inspect-artifact` from ArtifactEmbed — needs the
            disabled `@`-reference renderer. Kept wired so restoring the tab is a
            one-line change. */}
        {!showInitialLoading && activeThread && activeTab === "artifacts"
          ? selectedArtifact
            ? (
                <ArtifactDetailPanel
                  artifact={selectedArtifact}
                  onBack={() => setSelectedArtifactId(null)}
                  onChanged={refreshContext}
                />
              )
            : (
                <ArtifactsPanel
                  artifacts={artifacts}
                  onChanged={refreshContext}
                  onSelectArtifact={handleSelectArtifact}
                  threadId={activeThread.id}
                  workspacePath={activeWorkspace?.path ?? null}
                />
              )
          : null}
        {!showInitialLoading && activeThread && activeTab === "files"
          ? <FileTreePanel isWorkspace={isWorkspaceThread} rootPath={activeWorkspace?.path ?? null} />
          : null}
      </div>
    </aside>
  );
}
