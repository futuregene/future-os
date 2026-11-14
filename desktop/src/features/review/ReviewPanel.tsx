import type {
  GitReview,
  LastRunReviewData,
} from "../../integrations/storage/types";
import type { GitHistorySource } from "../git-history/gitHistorySource";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getLastRunReview,
  retryRunReview,
} from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { onFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { GitHistoryPanel } from "../git-history/GitHistoryPanel";
import { WorkingTreeReview } from "./GitChangesReview";
import { LastRunReview } from "./LastRunReview";

type ReviewView = "history" | "branch" | "uncommitted" | "last_run";

export function ReviewPanel({
  changePreview = "ready",
  branchReview,
  threadId,
  uncommittedReview,
  isGitWorkspace,
  workspacePath,
}: {
  branchReview: GitReview | null;
  changePreview?: "ready" | "unsupported_too_large";
  isGitWorkspace: boolean | null;
  threadId: string;
  uncommittedReview: GitReview | null;
  /**
   * The thread's own workspace path, for the history view. The *thread's*, not
   * the store's active workspace: the store falls back to the first user
   * workspace while a thread's own is unresolved, and reading another project's
   * log under this conversation's title is worse than showing nothing
   * (issue #1541 is the same mistake on the Files tab).
   */
  workspacePath: string | null;
}) {
  const { t } = useTranslation("review");
  const review = uncommittedReview ?? branchReview;
  // Capabilities arrive on the lightweight, non-diff context refresh. Use that
  // stable fact while the two expensive git diffs load, rather than briefly
  // rendering this as the non-git last-run-only view.
  const reviewKind
    = isGitWorkspace === null
      ? review === null
        ? "loading"
        : review.isGitWorkspace
          ? "git"
          : "non_git"
      : isGitWorkspace
        ? "git"
        : "non_git";
  const isGit = reviewKind === "git";
  // The history is offered only where there is one, and only once the thread's
  // own workspace path is known: the read is rooted at that path, and offering
  // the tab before it resolves would open onto another workspace's log (see the
  // `workspacePath` prop).
  const historySource: GitHistorySource | null
    = isGit && workspacePath ? { kind: "local", path: workspacePath } : null;
  const [activeView, setActiveView] = useState<ReviewView>("branch");
  /**
   * The thread whose view the reader has chosen by hand.
   *
   * A thread's workspace path arrives with its first context refresh, so the
   * opening view has to be seeded *after* mount — and a seed that ignored a
   * click would yank the view out from under a reader who reached for "Branch"
   * during those first frames.
   */
  const chosenForRef = useRef<string | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [retryError, setRetryError] = useState<string | null>(null);

  // Cancellation-safe per-thread load: a slow getLastRunReview for a previous
  // thread can no longer land under the thread we've since switched to.
  const runResource = useAsyncResource<LastRunReviewData | null>(
    () =>
      changePreview === "unsupported_too_large"
        ? Promise.resolve(null)
        : getLastRunReview(threadId),
    [threadId, changePreview],
    null,
  );
  const runReview = runResource.data;
  const { reload } = runResource;

  // A Git workspace opens on its history — the lane graph is what this panel is
  // for, and it leads the strip. Until the thread's own path resolves there is
  // no log to read, so those first frames stay on the branch delta rather than
  // highlighting a tab that is not in the strip yet. Non-git workspaces only
  // have the last-run view.
  //
  // Runs before the seeding effect below, so a new thread starts unchosen.
  useEffect(() => {
    chosenForRef.current = null;
  }, [threadId]);
  useEffect(() => {
    if (reviewKind === "git") {
      if (chosenForRef.current !== threadId)
        setActiveView(workspacePath ? "history" : "branch");
    }
    else if (reviewKind === "non_git") {
      setActiveView("last_run");
    }
  }, [threadId, reviewKind, workspacePath]);

  /** Record a view the reader picked, so a late workspace path cannot override it. */
  function choose(view: ReviewView) {
    chosenForRef.current = threadId;
    setActiveView(view);
  }

  // Refresh when a Run on this thread finishes (its changeset just landed).
  useEffect(
    () =>
      onFutureEvent("review-updated", (detail) => {
        if (detail.threadId === threadId)
          reload();
      }),
    [threadId, reload],
  );

  async function handleRetry() {
    const runId = runReview?.run?.id ?? runReview?.changeset.runId;
    if (!runId)
      return;
    setRetrying(true);
    setRetryError(null);
    try {
      await retryRunReview(runId);
      reload();
    }
    catch (error) {
      setRetryError(errorMessage(error));
    }
    finally {
      setRetrying(false);
    }
  }

  const lastRun = (
    <LastRunReview
      changePreview={changePreview}
      error={retryError ?? runResource.error}
      loading={runResource.loading}
      retrying={retrying}
      review={runReview}
      onRetry={handleRetry}
    />
  );

  // Non-git Workspace: just the last-run view under a static heading (§3.2).
  if (!isGit) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="shrink-0 px-4 pb-3 text-xs font-medium text-ink-muted">
          {t("lastRunHeading")}
        </div>
        {lastRun}
      </div>
    );
  }

  const reviewData = activeView === "branch" ? branchReview : uncommittedReview;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        className={`mx-4 grid shrink-0 gap-1 rounded-md border border-line-soft bg-transparent p-1 ${
          historySource ? "grid-cols-4" : "grid-cols-3"
        }`}
      >
        {/* The history leads the strip: the lane graph is what a reader opens
            this panel to see, and it was the last of four tabs. It still only
            appears where there is a log to read (see `historySource`), so a
            workspace without one keeps the same three-view strip. */}
        {historySource
          ? (
              <ViewTab
                active={activeView === "history"}
                label={t("tab.history")}
                onClick={() => choose("history")}
              />
            )
          : null}
        <ViewTab
          active={activeView === "branch"}
          label={t("tab.branch")}
          onClick={() => choose("branch")}
        />
        <ViewTab
          active={activeView === "uncommitted"}
          label={t("tab.uncommitted")}
          onClick={() => choose("uncommitted")}
        />
        <ViewTab
          active={activeView === "last_run"}
          label={t("tab.lastRun")}
          onClick={() => choose("last_run")}
        />
      </div>
      {activeView === "last_run"
        ? (
            lastRun
          )
        : activeView === "history" && historySource
          ? <GitHistoryPanel source={historySource} />
          : reviewData
            ? (
                <WorkingTreeReview
                  review={reviewData}
                  showBranch={activeView === "branch"}
                />
              )
            : null}
    </div>
  );
}

function ViewTab({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className={
        active
          ? "h-8 rounded bg-accent-soft/70 text-sm font-medium text-accent"
          : "h-8 rounded text-sm font-medium text-ink-muted transition-colors hover:text-ink"
      }
      onClick={onClick}
      type="button"
    >
      {label}
    </button>
  );
}
