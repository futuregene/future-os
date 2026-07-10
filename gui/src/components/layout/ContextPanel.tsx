import type { MouseEvent as ReactMouseEvent } from "react";
import type { ReviewBase } from "../../features/review/ReviewPanel";
import type { GitReview, StoredArtifact, StoredRun, StoredThread, StoredToolCall, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { WorkspaceReviewCapabilities } from "../../integrations/storage/types";
import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ArtifactDetailPanel } from "../../features/artifacts/ArtifactDetailPanel";
import { ArtifactsPanel } from "../../features/artifacts/ArtifactsPanel";
import { FileTreePanel } from "../../features/filetree/FileTreePanel";
import { upsertFutureReferenceEntries } from "../../features/markdown/futureReferenceStore";
import { ReviewPanel } from "../../features/review/ReviewPanel";
import { RunInspectPanel } from "../../features/runs/RunInspectPanel";
import { RunsPanel } from "../../features/runs/RunsPanel";
import {
  abortRun,
  clearFinishedRuns,
  ensureWorkspaceGit,
  getGitReview,
  getWorkspaceReviewCapabilities,
  listArtifacts,
  listRuns,
  listToolCalls,
} from "../../integrations/storage/threadStore";
import { onFutureEvent } from "../../lib/futureEvents";
import { usePolling } from "../../lib/usePolling";
import { startWindowDrag } from "../../lib/windowDrag";
import { EmptyState } from "../ui/EmptyState";
import { IconButton } from "../ui/IconButton";
import { Select } from "../ui/Select";

export type ContextTab = "runs" | "review" | "artifacts" | "files";
export type { ReviewBase };

// The Files tab appears for every thread — its root is the thread's workspace
// path (chat threads use their temporary workspace), which is available
// immediately, independent of the git/review capability probe.
const gitTabs = [
  { value: "review", labelKey: "contextPanel.review" },
  { value: "files", labelKey: "contextPanel.files" },
  { value: "runs", labelKey: "contextPanel.runs" },
] satisfies Array<{ value: ContextTab; labelKey: string }>;

const fileTabs = [
  { value: "artifacts", labelKey: "contextPanel.artifacts" },
  { value: "files", labelKey: "contextPanel.files" },
  { value: "runs", labelKey: "contextPanel.runs" },
] satisfies Array<{ value: ContextTab; labelKey: string }>;

const pendingTabs = [
  { value: "files", labelKey: "contextPanel.files" },
  { value: "runs", labelKey: "contextPanel.runs" },
] satisfies Array<{ value: ContextTab; labelKey: string }>;

// On a thread switch, hold off blanking + showing the loading spinner: a local
// fetch usually resolves in tens of ms, so the previous thread's data simply
// swaps for the new one with no flash. Only if the fetch is still running after
// this delay do we blank and show the spinner...
const LOADING_SPINNER_DELAY_MS = 200;
// ...and once shown, keep it visible at least this long so it can't itself flash.
const LOADING_SPINNER_MIN_MS = 200;

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
  const [runs, setRuns] = useState<StoredRun[]>([]);
  const [toolsByRun, setToolsByRun] = useState<Record<string, StoredToolCall[]>>({});
  const [artifacts, setArtifacts] = useState<StoredArtifact[]>([]);
  const [gitReview, setGitReview] = useState<GitReview | null>(null);
  const [reviewCapabilities, setReviewCapabilities] = useState<WorkspaceReviewCapabilities | null>(null);
  const [reviewBase, setReviewBase] = useState<ReviewBase>("head");
  const [reviewCustomBase, setReviewCustomBase] = useState("");
  // Debounced so typing a custom base doesn't refire the whole-tree diff (and
  // listRuns / N×listToolCalls) on every keystroke — only the settled value does.
  const [debouncedReviewCustomBase, setDebouncedReviewCustomBase] = useState("");
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [selectedToolId, setSelectedToolId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const refreshGenerationRef = useRef(0);
  // Guards the once-per-open default-tab seed: armed (false) while the panel is
  // closed, tripped (true) after seeding so a thread switch mid-open never
  // re-seeds. See the seeding effect below.
  const seededForOpenRef = useRef(false);
  const activeThreadId = activeThread?.id ?? null;
  const activeThreadMode = activeThread?.mode ?? null;
  const activeWorkspaceId = activeWorkspace?.id ?? activeThread?.workspaceId ?? null;
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
  const selectedRun = selectedRunId
    ? runs.find(run => run.id === selectedRunId) ?? null
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

  const refreshContext = useCallback(async (options?: { ensureGit?: boolean }) => {
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

    // Note: blanking + the loading spinner are driven by the thread-switch
    // bootstrap effect (delayed + min-duration), not here — so a fast local
    // refresh just swaps data in without a flash, and polls never blank.
    try {
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

      const [nextRuns, nextGitReview, nextCapabilities] = await Promise.all([
        listRuns(activeThreadId),
        // C3: only run the whole-tree git diff while the Review tab is showing it.
        activeWorkspaceId && activeTab === "review"
          ? getGitReview({
              base: reviewBase,
              customBase: debouncedReviewCustomBase,
              workspaceId: activeWorkspaceId,
            })
          : Promise.resolve(null),
        activeWorkspaceId && activeThreadMode === "workspace"
          ? getWorkspaceReviewCapabilities(activeWorkspaceId)
          : Promise.resolve(null),
      ]);
      // Only chat threads use Artifacts; workspace threads show Review (§14.6).
      const nextArtifacts = activeThreadMode === "workspace" ? [] : await listArtifacts(activeThreadId);
      const toolEntries = await Promise.all(nextRuns.map(async run => [run.id, await listToolCalls(run.id)] as const));
      const toolCalls = toolEntries.flatMap(([, tools]) => tools);

      if (!isCurrentRefresh())
        return;

      setRuns(nextRuns);
      setToolsByRun(Object.fromEntries(toolEntries));
      setArtifacts(nextArtifacts);
      setGitReview(nextGitReview);
      setReviewCapabilities(nextCapabilities);
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
  }, [activeTab, activeThreadId, activeThreadMode, activeWorkspaceId, reviewBase, debouncedReviewCustomBase]);

  // Poll the latest refreshContext through a ref so the interval never restarts
  // when the callback's identity changes. refreshContext depends on
  // activeTab/reviewBase/debouncedReviewCustomBase, so keying the poll on it
  // would restart (immediate tick + reset) on every tab/base change — firing a
  // second fetch on top of the parameter-driven effect below. usePolling always
  // invokes the latest callback, so the ref keeps the tick current for free.
  const refreshContextRef = useRef(refreshContext);
  refreshContextRef.current = refreshContext;

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
    const timer = setTimeout(setDebouncedReviewCustomBase, 300, reviewCustomBase);
    return () => clearTimeout(timer);
  }, [reviewCustomBase]);

  // Thread-changed bootstrap: fetch the new thread's context (and ensure git),
  // but avoid the loading flash on fast local switches. We keep showing the
  // previous thread's data and only blank + show the spinner if the fetch is
  // still running after LOADING_SPINNER_DELAY_MS; once shown, the spinner stays
  // for at least LOADING_SPINNER_MIN_MS so it can't flash off immediately.
  useEffect(() => {
    if (activeThreadId === null) {
      void refreshContext();
      return;
    }
    let cancelled = false;
    let spinnerShownAt: number | null = null;
    let minTimer: ReturnType<typeof setTimeout> | undefined;

    const spinnerTimer = setTimeout(() => {
      if (cancelled)
        return;
      spinnerShownAt = performance.now();
      setRuns([]);
      setToolsByRun({});
      setArtifacts([]);
      setGitReview(null);
      setLoading(true);
    }, LOADING_SPINNER_DELAY_MS);

    void refreshContext({ ensureGit: true }).finally(() => {
      if (cancelled)
        return;
      clearTimeout(spinnerTimer);
      if (spinnerShownAt === null) {
        // Fast path: spinner never appeared — data just swapped in. (Also
        // clears any spinner stranded true by a rapid earlier switch.)
        setLoading(false);
        return;
      }
      const remaining = LOADING_SPINNER_MIN_MS - (performance.now() - spinnerShownAt);
      if (remaining <= 0) {
        setLoading(false);
        return;
      }
      minTimer = setTimeout(() => {
        if (!cancelled)
          setLoading(false);
      }, remaining);
    });

    return () => {
      cancelled = true;
      clearTimeout(spinnerTimer);
      if (minTimer)
        clearTimeout(minTimer);
    };
    // Keyed on the active thread only; refreshContext is intentionally omitted.
    // eslint-disable-next-line react/exhaustive-deps
  }, [activeThreadId]);

  // Parameter-driven refresh: re-fetch for the current tab / diff base without
  // blanking already-loaded state.
  useEffect(() => {
    void refreshContext();
    // eslint-disable-next-line react/exhaustive-deps
  }, [activeTab, reviewBase, debouncedReviewCustomBase]);

  useEffect(() => {
    setSelectedArtifactId(null);
    setSelectedRunId(null);
    setSelectedToolId(null);
  }, [activeThreadId]);

  // Seed the content tab (Review for workspace threads, Artifacts for chat)
  // each time the panel OPENS — not per thread. So opening the panel lands on
  // the work-output tab, but while it stays open, switching threads keeps
  // whatever tab you're on. Only seeds after the workspace kind resolves, so we
  // pick the real tab (not the runs-only pending set).
  useEffect(() => {
    if (!expanded) {
      seededForOpenRef.current = false; // Re-arm for the next open.
      return;
    }
    if (activeThreadId === null || workspaceKindPending || seededForOpenRef.current)
      return;
    seededForOpenRef.current = true;
    const preferred: ContextTab = isWorkspaceThread ? "review" : "artifacts";
    if (activeTab !== preferred)
      onTabChange(preferred);
  }, [expanded, activeThreadId, isWorkspaceThread, workspaceKindPending, activeTab, onTabChange]);

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
  }, [expanded, handleSelectArtifact, handleSelectRun, onTabChange, onToggleExpanded]);

  usePolling(() => refreshContextRef.current(), 1500, {
    enabled: Boolean(activeThreadId) && expanded,
    // Intentionally no refreshContext dep: the parameter-driven effect above
    // owns param-change fetches; the poll only needs to tick periodically.
    deps: [],
  });

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
            : selectedRun
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
                  threadId={activeThread.id}
                  onChanged={refreshContext}
                  onSelectArtifact={handleSelectArtifact}
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
