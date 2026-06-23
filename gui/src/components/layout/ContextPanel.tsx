import type { ReviewBase } from "../../features/review/ReviewPanel";
import type { GitReview, StoredArtifact, StoredRun, StoredThread, StoredToolCall, StoredWorkspace } from "../../integrations/storage/threadStore";
import { ChevronDown, PanelRightClose, PanelRightOpen } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ArtifactDetailPanel } from "../../features/artifacts/ArtifactDetailPanel";
import { ArtifactsPanel } from "../../features/artifacts/ArtifactsPanel";
import { upsertFutureReferenceEntries } from "../../features/markdown/futureReferenceStore";
import { ReviewPanel } from "../../features/review/ReviewPanel";
import { RunInspectPanel } from "../../features/runs/RunInspectPanel";
import { RunsPanel } from "../../features/runs/RunsPanel";
import {
  abortRun,
  clearFinishedRuns,
  ensureWorkspaceGit,
  getGitReview,
  listArtifacts,
  listRuns,
  listToolCalls,
} from "../../integrations/storage/threadStore";
import { onFutureEvent } from "../../lib/futureEvents";
import { startWindowDrag } from "../../lib/windowDrag";
import { EmptyState } from "../ui/EmptyState";
import { IconButton } from "../ui/IconButton";

export type ContextTab = "runs" | "review" | "artifacts";
export type { ReviewBase };

const gitTabs = [
  { value: "runs", label: "Runs" },
  { value: "review", label: "Review" },
] satisfies Array<{ value: ContextTab; label: string }>;

const fileTabs = [
  { value: "runs", label: "Runs" },
  { value: "artifacts", label: "Artifacts" },
] satisfies Array<{ value: ContextTab; label: string }>;

const pendingTabs = [
  { value: "runs", label: "Runs" },
] satisfies Array<{ value: ContextTab; label: string }>;

interface ContextPanelProps {
  activeThread: StoredThread | null;
  activeWorkspace: StoredWorkspace | null;
  activeTab: ContextTab;
  expanded: boolean;
  onTabChange: (tab: ContextTab) => void;
  onToggleExpanded: () => void;
}

export function ContextPanel({
  activeThread,
  activeWorkspace,
  activeTab,
  expanded,
  onTabChange,
  onToggleExpanded,
}: ContextPanelProps) {
  const [runs, setRuns] = useState<StoredRun[]>([]);
  const [toolsByRun, setToolsByRun] = useState<Record<string, StoredToolCall[]>>({});
  const [artifacts, setArtifacts] = useState<StoredArtifact[]>([]);
  const [gitReview, setGitReview] = useState<GitReview | null>(null);
  const [reviewBase, setReviewBase] = useState<ReviewBase>("head");
  const [reviewCustomBase, setReviewCustomBase] = useState("");
  const [selectedReviewId, setSelectedReviewId] = useState<string | null>(null);
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const refreshGenerationRef = useRef(0);
  const activeThreadId = activeThread?.id ?? null;
  const activeThreadMode = activeThread?.mode ?? null;
  const activeWorkspaceId = activeWorkspace?.id ?? activeThread?.workspaceId ?? null;
  const workspaceKindPending = activeThreadId !== null && activeWorkspaceId !== null && gitReview === null;
  const isGitWorkspace = gitReview?.isGitWorkspace ?? false;
  const tabs = workspaceKindPending ? pendingTabs : isGitWorkspace ? gitTabs : fileTabs;
  const hasContextData = runs.length > 0
    || artifacts.length > 0
    || (gitReview?.files.length ?? 0) > 0;
  const showInitialLoading = loading && (!hasContextData || workspaceKindPending);
  const selectedArtifact = selectedArtifactId
    ? artifacts.find(artifact => artifact.id === selectedArtifactId) ?? null
    : null;
  const selectedRun = selectedRunId
    ? runs.find(run => run.id === selectedRunId) ?? null
    : null;

  const refreshContext = useCallback(async (options?: { showLoading?: boolean; ensureGit?: boolean }) => {
    const refreshGeneration = refreshGenerationRef.current + 1;
    refreshGenerationRef.current = refreshGeneration;
    const isCurrentRefresh = () => refreshGenerationRef.current === refreshGeneration;

    if (!activeThreadId) {
      setRuns([]);
      setToolsByRun({});
      setArtifacts([]);
      setGitReview(null);
      setLoading(false);
      return;
    }

    const showLoading = options?.showLoading ?? false;
    if (showLoading) {
      setRuns([]);
      setToolsByRun({});
      setArtifacts([]);
      setGitReview(null);
      setLoading(true);
    }

    // When a workspace thread is opened, make sure its directory is under git
    // before the review query runs, so the Review tab and branch changes show
    // up on first load instead of after the next poll. Best-effort: a missing
    // git binary or a temporary chat workspace is a no-op on the backend.
    if (options?.ensureGit && activeWorkspaceId && activeThreadMode === "workspace") {
      try {
        await ensureWorkspaceGit(activeWorkspaceId);
      }
      catch {
        // git is optional; fall through to the review query regardless.
      }
      if (!isCurrentRefresh())
        return;
    }

    try {
      const [nextRuns, nextGitReview] = await Promise.all([
        listRuns(activeThreadId),
        activeWorkspaceId
          ? getGitReview({
              base: reviewBase,
              customBase: reviewCustomBase,
              workspaceId: activeWorkspaceId,
            })
          : Promise.resolve(null),
      ]);
      const nextArtifacts = nextGitReview?.isGitWorkspace ? [] : await listArtifacts(activeThreadId);
      const toolEntries = await Promise.all(nextRuns.map(async run => [run.id, await listToolCalls(run.id)] as const));
      const toolCalls = toolEntries.flatMap(([, tools]) => tools);

      if (!isCurrentRefresh())
        return;

      setRuns(nextRuns);
      setToolsByRun(Object.fromEntries(toolEntries));
      setArtifacts(nextArtifacts);
      setGitReview(nextGitReview);
      upsertContextReferences(activeWorkspaceId, {
        artifacts: nextArtifacts,
        runs: nextRuns,
        tools: toolCalls,
      });
    }
    catch {
      if (isCurrentRefresh()) {
        setRuns([]);
        setToolsByRun({});
        setArtifacts([]);
        setGitReview(null);
      }
    }
    finally {
      if (showLoading && isCurrentRefresh()) {
        setLoading(false);
      }
    }
  }, [activeThreadId, activeThreadMode, activeWorkspaceId, reviewBase, reviewCustomBase]);

  useEffect(() => {
    if (!tabs.some(tab => tab.value === activeTab)) {
      onTabChange(tabs[0].value);
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
  }, [activeTab, onTabChange]);

  useEffect(() => {
    void refreshContext({ showLoading: true, ensureGit: true });
  }, [refreshContext]);

  useEffect(() => {
    setSelectedArtifactId(null);
    setSelectedRunId(null);
  }, [activeThreadId]);

  useEffect(() => {
    const unsubscribers = [
      onFutureEvent("inspect-run", (detail) => {
        handleSelectRun(detail.runId);
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
      onFutureEvent("open-review", (detail) => {
        setSelectedReviewId(detail.reviewId);
        setSelectedArtifactId(null);
        setSelectedRunId(null);
        onTabChange("review");
        if (!expanded) {
          onToggleExpanded();
        }
      }),
    ];
    return () => unsubscribers.forEach(unsubscribe => unsubscribe());
  }, [expanded, handleSelectArtifact, handleSelectRun, onTabChange, onToggleExpanded]);

  useEffect(() => {
    if (!activeThreadId || !expanded) {
      return;
    }
    const timer = window.setInterval(() => {
      void refreshContext();
    }, 1500);
    return () => window.clearInterval(timer);
  }, [activeThreadId, expanded, refreshContext]);

  if (!expanded) {
    return (
      <button
        aria-label="Expand context panel"
        title="Expand context panel"
        className="absolute right-3 top-2 z-30 inline-flex size-8 items-center justify-center rounded-md border border-transparent bg-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={onToggleExpanded}
        type="button"
      >
        <PanelRightOpen className="size-3.5" />
      </button>
    );
  }

  return (
    <aside className="flex w-96 shrink-0 flex-col border-l border-line-soft bg-surface-subtle">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="relative inline-block max-w-full">
          <label className="sr-only" htmlFor="context-panel-view">Context panel view</label>
          <select
            id="context-panel-view"
            className="h-8 w-fit min-w-24 max-w-full appearance-none rounded-md border border-line-soft bg-surface py-0 pl-3 pr-8 text-sm font-normal text-ink outline-none transition-colors hover:border-line focus:border-blue-300 focus:ring-2 focus:ring-blue-100"
            value={activeTab}
            onChange={event => onTabChange(event.target.value as ContextTab)}
          >
            {tabs.map(tab => (
              <option key={tab.value} value={tab.value}>{tab.label}</option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-4 -translate-y-1/2 text-ink-muted" />
        </div>
        <IconButton
          icon={<PanelRightClose className="size-3.5" />}
          label="Collapse context panel"
          onClick={onToggleExpanded}
        />
      </header>
      <div className="min-h-0 flex-1 overflow-auto px-4 pb-4 pt-2">
        {showInitialLoading ? <div className="py-4 text-sm text-ink-muted">Loading context...</div> : null}
        {!showInitialLoading && !activeThread ? <EmptyState title="No thread selected" /> : null}
        {!showInitialLoading && activeThread && activeTab === "runs"
          ? selectedRun
            ? (
                <RunInspectPanel
                  run={selectedRun}
                  tools={toolsByRun[selectedRun.id] ?? []}
                  onBack={() => setSelectedRunId(null)}
                />
              )
            : (
                <RunsPanel
                  runs={runs}
                  toolsByRun={toolsByRun}
                  onClearFinished={handleClearFinishedRuns}
                  onInspectRun={handleSelectRun}
                  onTerminateRun={handleTerminateRun}
                />
              )
          : null}
        {!showInitialLoading && activeThread && activeTab === "review" && isGitWorkspace
          ? (
              <ReviewPanel
                customBase={reviewCustomBase}
                review={gitReview}
                reviewBase={reviewBase}
                selectedReviewId={selectedReviewId}
                threadId={activeThread.id}
                onCustomBaseChange={setReviewCustomBase}
                onReviewBaseChange={setReviewBase}
              />
            )
          : null}
        {!showInitialLoading && activeThread && activeTab === "artifacts" && !isGitWorkspace
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
                />
              )
          : null}
      </div>
    </aside>
  );
}

function upsertContextReferences(
  workspaceId: string | null,
  {
    artifacts,
    runs,
    tools,
  }: {
    artifacts: StoredArtifact[];
    runs: StoredRun[];
    tools: StoredToolCall[];
  },
) {
  upsertFutureReferenceEntries(workspaceId, [
    ...runs.map(run => ({ data: run, targetId: run.id, targetType: "run" as const })),
    ...tools.map(tool => ({ data: tool, targetId: tool.id, targetType: "tool" as const })),
    ...artifacts.map(artifact => ({ data: artifact, targetId: artifact.id, targetType: "artifact" as const })),
  ]);
}
