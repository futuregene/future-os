import type { ReviewBase } from "../../../integrations/storage/review";
import type { GitReview, StoredArtifact, StoredRun, StoredThread, StoredToolCall } from "../../../integrations/storage/threadStore";
import type { WorkspaceReviewCapabilities } from "../../../integrations/storage/types";
import { useCallback, useEffect, useRef, useState } from "react";
import { upsertFutureReferenceEntries } from "../../../features/markdown/futureReferenceStore";
import {
  ensureWorkspaceGit,
  getGitReview,
  getWorkspaceReviewCapabilities,
  listArtifacts,
  listRuns,
  listToolCalls,
} from "../../../integrations/storage/threadStore";
import { usePolling } from "../../../lib/usePolling";

export type ContextTab = "runs" | "review" | "artifacts" | "files";

// On a thread switch, hold off blanking + showing the loading spinner: a local
// fetch usually resolves in tens of ms, so the previous thread's data simply
// swaps for the new one with no flash. Only if the fetch is still running after
// this delay do we blank and show the spinner...
const LOADING_SPINNER_DELAY_MS = 200;
// ...and once shown, keep it visible at least this long so it can't itself flash.
const LOADING_SPINNER_MIN_MS = 200;

interface UseContextDataInput {
  activeThreadId: string | null;
  activeThreadMode: StoredThread["mode"] | null;
  activeWorkspaceId: string | null;
  activeTab: ContextTab;
  expanded: boolean;
}

/**
 * Owns the right-context data pipeline: fetches runs/tools/artifacts/git-review
 * for the active thread, drives the flash-free loading spinner, debounces the
 * custom review base, polls while the panel is open, and mirrors the fetched
 * objects into the markdown reference store. Kept out of `ContextPanel` so the
 * panel is a pure view over this data (GUI principle 5).
 */
export function useContextData({
  activeThreadId,
  activeThreadMode,
  activeWorkspaceId,
  activeTab,
  expanded,
}: UseContextDataInput) {
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
  const [loading, setLoading] = useState(false);
  const refreshGenerationRef = useRef(0);

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

  usePolling(() => refreshContextRef.current(), 1500, {
    enabled: Boolean(activeThreadId) && expanded,
    // Intentionally no refreshContext dep: the parameter-driven effect above
    // owns param-change fetches; the poll only needs to tick periodically.
    deps: [],
  });

  return {
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
  };
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
