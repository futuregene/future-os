import type { StoredApprovalRequest, StoredArtifact, StoredReviewChangeset, StoredReviewFileChange, StoredRun, StoredRunEvent, StoredThread, StoredToolCall, StoredToolOutput } from "../../integrations/storage/threadStore";
import { PanelRightClose, PanelRightOpen } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import {
  listApprovalRequests,
  listArtifacts,
  listReviewChangesets,
  listReviewFileChanges,
  listRunEvents,
  listRuns,
  listToolCalls,
  listToolOutputs,
} from "../../integrations/storage/threadStore";
import { startWindowDrag } from "../../lib/windowDrag";
import { IconButton } from "../ui/IconButton";
import { Tabs } from "../ui/Tabs";
import { ApprovalsPanel } from "./context-panel/ApprovalsPanel";
import { ArtifactsPanel } from "./context-panel/ArtifactsPanel";
import { EmptyState } from "./context-panel/ContextEmptyState";
import { ReviewPanel } from "./context-panel/ReviewPanel";
import { RunsPanel } from "./context-panel/RunsPanel";

export type ContextTab = "runs" | "approvals" | "review" | "artifacts";

const tabs = [
  { value: "runs", label: "Runs" },
  { value: "approvals", label: "Approvals" },
  { value: "review", label: "Review" },
  { value: "artifacts", label: "Artifacts" },
] satisfies Array<{ value: ContextTab; label: string }>;

interface ContextPanelProps {
  activeThread: StoredThread | null;
  activeTab: ContextTab;
  expanded: boolean;
  pendingApprovalCount: number;
  onTabChange: (tab: ContextTab) => void;
  onToggleExpanded: () => void;
}

export function ContextPanel({
  activeThread,
  activeTab,
  expanded,
  pendingApprovalCount,
  onTabChange,
  onToggleExpanded,
}: ContextPanelProps) {
  const [runs, setRuns] = useState<StoredRun[]>([]);
  const [eventsByRun, setEventsByRun] = useState<Record<string, StoredRunEvent[]>>({});
  const [toolsByRun, setToolsByRun] = useState<Record<string, StoredToolCall[]>>({});
  const [outputsByTool, setOutputsByTool] = useState<Record<string, StoredToolOutput[]>>({});
  const [approvals, setApprovals] = useState<StoredApprovalRequest[]>([]);
  const [artifacts, setArtifacts] = useState<StoredArtifact[]>([]);
  const [changesets, setChangesets] = useState<StoredReviewChangeset[]>([]);
  const [filesByChangeset, setFilesByChangeset] = useState<Record<string, StoredReviewFileChange[]>>({});
  const [loading, setLoading] = useState(false);
  const activeThreadId = activeThread?.id ?? null;
  const hasContextData = runs.length > 0
    || approvals.length > 0
    || artifacts.length > 0
    || changesets.length > 0;
  const showInitialLoading = loading && !hasContextData;

  const refreshContext = useCallback(async (options?: { showLoading?: boolean }) => {
    if (!activeThreadId) {
      setRuns([]);
      setEventsByRun({});
      setToolsByRun({});
      setOutputsByTool({});
      setApprovals([]);
      setArtifacts([]);
      setChangesets([]);
      setFilesByChangeset({});
      setLoading(false);
      return;
    }

    const showLoading = options?.showLoading ?? false;
    if (showLoading) {
      setRuns([]);
      setEventsByRun({});
      setToolsByRun({});
      setOutputsByTool({});
      setApprovals([]);
      setArtifacts([]);
      setChangesets([]);
      setFilesByChangeset({});
      setLoading(true);
    }
    try {
      const [nextRuns, nextApprovals, nextChangesets, nextArtifacts] = await Promise.all([
        listRuns(activeThreadId),
        listApprovalRequests(activeThreadId),
        listReviewChangesets(activeThreadId),
        listArtifacts(activeThreadId),
      ]);
      const eventEntries = await Promise.all(
        nextRuns.slice(0, 8).map(async run => [run.id, await listRunEvents(run.id)] as const),
      );
      const toolEntries = await Promise.all(
        nextRuns.slice(0, 8).map(async run => [run.id, await listToolCalls(run.id)] as const),
      );
      const toolCalls = toolEntries.flatMap(([, tools]) => tools);
      const outputEntries = await Promise.all(
        toolCalls.slice(0, 24).map(async tool => [tool.id, await listToolOutputs(tool.id)] as const),
      );
      const fileEntries = await Promise.all(
        nextChangesets.slice(0, 8).map(async changeset => [
          changeset.id,
          await listReviewFileChanges(changeset.id),
        ] as const),
      );

      setRuns(nextRuns);
      setEventsByRun(Object.fromEntries(eventEntries));
      setToolsByRun(Object.fromEntries(toolEntries));
      setOutputsByTool(Object.fromEntries(outputEntries));
      setApprovals(nextApprovals);
      setArtifacts(nextArtifacts);
      setChangesets(nextChangesets);
      setFilesByChangeset(Object.fromEntries(fileEntries));
    }
    finally {
      if (showLoading) {
        setLoading(false);
      }
    }
  }, [activeThreadId]);

  useEffect(() => {
    void refreshContext({ showLoading: true });
  }, [refreshContext]);

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
        {pendingApprovalCount > 0
          ? (
              <span className="absolute right-1 top-1 size-2 rounded-full bg-amber-500 ring-2 ring-white" />
            )
          : null}
      </button>
    );
  }

  return (
    <aside className="flex w-96 shrink-0 flex-col border-l border-line-soft bg-surface-subtle">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="min-w-0 flex-1" data-tauri-drag-region>
          <h2 className="truncate text-sm font-semibold text-ink">Run Context</h2>
          <p className="truncate text-xs text-ink-muted">
            {activeThread ? activeThread.title : "No active thread"}
          </p>
        </div>
        <IconButton
          icon={<PanelRightClose className="size-3.5" />}
          label="Collapse context panel"
          onClick={onToggleExpanded}
        />
      </header>
      <div className="shrink-0 px-4 pb-3 pt-1">
        <Tabs
          className="overflow-x-auto rounded-lg bg-surface/70 p-1"
          items={tabs}
          value={activeTab}
          onChange={onTabChange}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto px-4 pb-4 pt-1">
        {showInitialLoading ? <div className="py-4 text-sm text-ink-muted">Loading context...</div> : null}
        {!showInitialLoading && !activeThread ? <EmptyState title="No thread selected" /> : null}
        {!showInitialLoading && activeThread && activeTab === "runs"
          ? (
              <RunsPanel
                eventsByRun={eventsByRun}
                outputsByTool={outputsByTool}
                runs={runs}
                toolsByRun={toolsByRun}
              />
            )
          : null}
        {!showInitialLoading && activeThread && activeTab === "approvals"
          ? (
              <ApprovalsPanel approvals={approvals} onDecision={refreshContext} />
            )
          : null}
        {!showInitialLoading && activeThread && activeTab === "review"
          ? (
              <ReviewPanel changesets={changesets} filesByChangeset={filesByChangeset} />
            )
          : null}
        {!showInitialLoading && activeThread && activeTab === "artifacts"
          ? (
              <ArtifactsPanel artifacts={artifacts} onChanged={refreshContext} />
            )
          : null}
      </div>
    </aside>
  );
}
