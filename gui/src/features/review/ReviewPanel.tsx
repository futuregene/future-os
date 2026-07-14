import type { ReviewBase } from "../../integrations/storage/review";
import type { GitReview, LastRunReviewData, WorkspaceReviewCapabilities } from "../../integrations/storage/types";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getLastRunReview, retryRunReview } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { onFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { ReviewHeader, WorkingTreeReview } from "./GitChangesReview";
import { LastRunReview } from "./LastRunReview";

type ReviewView = "git_changes" | "last_run";

export function ReviewPanel({
  capabilities,
  changePreview = "ready",
  customBase,
  onCustomBaseChange,
  onReviewBaseChange,
  review,
  reviewBase,
  threadId,
}: {
  capabilities: WorkspaceReviewCapabilities | null;
  changePreview?: "ready" | "unsupported_too_large";
  customBase: string;
  onCustomBaseChange: (value: string) => void;
  onReviewBaseChange: (value: ReviewBase) => void;
  review: GitReview | null;
  reviewBase: ReviewBase;
  threadId: string;
}) {
  const { t } = useTranslation("review");
  const isGit = review?.isGitWorkspace ?? false;
  const [activeView, setActiveView] = useState<ReviewView>("last_run");
  const [retrying, setRetrying] = useState(false);
  const [retryError, setRetryError] = useState<string | null>(null);

  // Cancellation-safe per-thread load: a slow getLastRunReview for a previous
  // thread can no longer land under the thread we've since switched to.
  const runResource = useAsyncResource<LastRunReviewData | null>(
    () => changePreview === "unsupported_too_large"
      ? Promise.resolve(null)
      : getLastRunReview(threadId),
    [threadId, changePreview],
    null,
  );
  const runReview = runResource.data;
  const { reload } = runResource;

  // A non-git Workspace only ever shows the last-run view.
  useEffect(() => {
    if (!isGit)
      setActiveView("last_run");
  }, [isGit]);

  // Capabilities load after mount, so seed the active view from the backend's
  // default once per thread — but never override a later manual tab choice.
  const appliedDefaultThreadRef = useRef<string | null>(null);
  useEffect(() => {
    if (capabilities && appliedDefaultThreadRef.current !== threadId) {
      appliedDefaultThreadRef.current = threadId;
      setActiveView(capabilities.defaultView);
    }
  }, [capabilities, threadId]);

  // Refresh when a Run on this thread finishes (its changeset just landed).
  useEffect(() => onFutureEvent("review-updated", (detail) => {
    if (detail.threadId === threadId)
      reload();
  }), [threadId, reload]);

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
      <div className="space-y-3">
        <div className="border-b border-line-soft pb-3 text-xs font-medium text-ink-muted">{t("lastRunHeading")}</div>
        {lastRun}
      </div>
    );
  }

  const reviewData = review!;

  return (
    <div className="space-y-3">
      {activeView === "git_changes"
        ? (
            <ReviewHeader
              customBase={customBase}
              review={reviewData}
              reviewBase={reviewBase}
              onCustomBaseChange={onCustomBaseChange}
              onReviewBaseChange={onReviewBaseChange}
            />
          )
        : null}
      <div className="grid grid-cols-2 gap-1 rounded-md bg-surface p-1">
        <ViewTab active={activeView === "git_changes"} label={t("tab.gitChanges")} onClick={() => setActiveView("git_changes")} />
        <ViewTab active={activeView === "last_run"} label={t("tab.lastRun")} onClick={() => setActiveView("last_run")} />
      </div>
      {activeView === "last_run"
        ? lastRun
        : <WorkingTreeReview files={reviewData.files} />}
    </div>
  );
}

function ViewTab({ active, label, onClick }: { active: boolean; label: string; onClick: () => void }) {
  return (
    <button
      className={active
        ? "h-8 rounded bg-surface-subtle text-sm font-medium text-ink"
        : "h-8 rounded text-sm font-medium text-ink-muted transition-colors hover:text-ink"}
      onClick={onClick}
      type="button"
    >
      {label}
    </button>
  );
}
