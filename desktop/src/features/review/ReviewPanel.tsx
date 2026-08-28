import type {
  GitReview,
  LastRunReviewData,
} from "../../integrations/storage/types";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getLastRunReview,
  retryRunReview,
} from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { onFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { WorkingTreeReview } from "./GitChangesReview";
import { LastRunReview } from "./LastRunReview";

type ReviewView = "branch" | "uncommitted" | "last_run";

export function ReviewPanel({
  changePreview = "ready",
  branchReview,
  threadId,
  uncommittedReview,
  isGitWorkspace,
}: {
  branchReview: GitReview | null;
  changePreview?: "ready" | "unsupported_too_large";
  isGitWorkspace: boolean | null;
  threadId: string;
  uncommittedReview: GitReview | null;
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
  const [activeView, setActiveView] = useState<ReviewView>("branch");
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

  // Git workspaces open on the committed branch delta; non-git workspaces only
  // have the last-run view.
  useEffect(() => {
    if (reviewKind === "git")
      setActiveView("branch");
    else if (reviewKind === "non_git")
      setActiveView("last_run");
  }, [threadId, reviewKind]);

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
      <div className="mx-4 grid shrink-0 grid-cols-3 gap-1 rounded-md border border-line-soft bg-transparent p-1">
        <ViewTab
          active={activeView === "branch"}
          label={t("tab.branch")}
          onClick={() => setActiveView("branch")}
        />
        <ViewTab
          active={activeView === "uncommitted"}
          label={t("tab.uncommitted")}
          onClick={() => setActiveView("uncommitted")}
        />
        <ViewTab
          active={activeView === "last_run"}
          label={t("tab.lastRun")}
          onClick={() => setActiveView("last_run")}
        />
      </div>
      {activeView === "last_run"
        ? (
            lastRun
          )
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
