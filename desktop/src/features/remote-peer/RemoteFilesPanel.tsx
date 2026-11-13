import type { FutureEventMap } from "../../lib/futureEvents";
import type { GitHistorySource } from "../git-history/gitHistorySource";
import type { RemotePeer } from "./remotePeerClient";
import { ArrowUp, Download, File, Folder, Loader2, PanelRightClose, PanelRightOpen, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { cn } from "../../lib/cn";
import { onFutureEvent } from "../../lib/futureEvents";
import { FilePreviewOverlay } from "../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../filepreview/previewKind";
import { FusionDiscussionPanel } from "../fusion/FusionDiscussionPanel";
import { GitHistoryPanel } from "../git-history/GitHistoryPanel";
import { peerBadgeText } from "./peerIcons";
import { listRemoteGitCommits } from "./remotePeerClient";
import { formatSize } from "./remotePeerFiles";
import { useRemoteFiles } from "./useRemoteFiles";

/**
 * Remote files, or a selected Fusion discussion read from that same host.
 *
 * The local panel's other tabs describe *this* machine
 * — its review of a diff, its runs, its artifacts — and none of them have a
 * remote counterpart to show. Offering them would mean either an empty panel or
 * one machine's facts under another's title.
 *
 * Collapsed it is the same floating affordance the local conversation has, in
 * the same corner, so the control is where a reader already reaches for it.
 */
export function RemoteFilesPanel({
  desktopId,
  expanded,
  floating,
  onToggleExpanded,
  peer,
  sessionId,
}: {
  desktopId: string;
  expanded: boolean;
  /**
   * Draw the panel over the conversation rather than as a column: the window
   * is too narrow to host the third column, so an overlay is the only way to
   * show the remote files without squeezing the conversation. Same contract as
   * `ContextPanel.floating`.
   */
  floating?: boolean;
  onToggleExpanded: () => void;
  peer: RemotePeer | undefined;
  sessionId: string;
}) {
  const { t } = useTranslation("remotePeer");
  const scope = `${desktopId}:${sessionId}`;
  const [view, setView] = useState<"files" | "history">("files");
  /**
   * Whether the conversation's workspace on that machine is a Git repository.
   *
   * Asked once per conversation because it decides whether the History tab
   * exists at all — the same read the history itself uses, so a plain directory
   * costs one probe and no round trip on every open. `null` while in flight:
   * the tab appears when the answer arrives rather than offering a page that
   * would only say "not a repository".
   */
  const [isRepo, setIsRepo] = useState<boolean | null>(null);
  // Re-asked each time the panel is opened, not once per conversation: a
  // workspace that is a plain directory when the panel first mounts may be
  // `git init`-ed by the agent a moment later, and a tab that never appears
  // again would leave the reader with a stale answer they cannot correct.
  useEffect(() => {
    if (!expanded) {
      // Closed, nothing is offered from it; keeping the last answer is right,
      // so re-opening does not flash the tab strip out and back in.
      return;
    }
    let current = true;
    void listRemoteGitCommits(desktopId, sessionId).then(
      (listing) => {
        if (current)
          setIsRepo(listing.isRepo);
      },
      // A peer that cannot answer (offline, older build) simply gets no History
      // tab: there is nothing to show and no action a reader could take here.
      () => {
        if (current)
          setIsRepo(false);
      },
    );
    return () => {
      current = false;
    };
  }, [desktopId, sessionId, expanded]);
  const historySource = useMemo<GitHistorySource | null>(
    () => (isRepo ? { kind: "remote", desktopId, sessionId } : null),
    [isRepo, desktopId, sessionId],
  );
  const [selection, setSelection] = useState<{ scope: string; detail: FutureEventMap["inspect-fusion"] } | null>(null);
  const fusion = selection?.scope === scope ? selection.detail : null;
  const files = useRemoteFiles(desktopId, sessionId, expanded && !fusion);
  useEffect(() => onFutureEvent("inspect-fusion", (detail) => {
    if (!detail.resolveTarget)
      return;
    setSelection({ scope, detail });
    if (!expanded)
      onToggleExpanded();
  }), [scope, expanded, onToggleExpanded]);
  // The view can outlive its tab: a re-probe may answer "not a repository" while
  // "history" is the selected view, and the tab strip is then gone. Derived
  // rather than reset in an effect, so the two cannot disagree for a frame.
  const showingHistory = view === "history" && historySource !== null;
  const preview = files.previewTarget;
  const previewKind = preview ? previewKindForPath(preview.name) : null;

  if (!expanded) {
    // Collapsed, this toggle floats over the conversation header instead of
    // living in a panel header, with the inset the header's 56px (`pr-14`)
    // reserve — the same placement the local one has.
    return (
      <button
        aria-label={t("filesOpen")}
        className="absolute right-4 top-2 z-30 inline-flex size-8 items-center justify-center rounded-md border border-transparent bg-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={onToggleExpanded}
        title={t("filesOpen")}
        type="button"
      >
        <PanelRightOpen className="size-3.5" />
      </button>
    );
  }

  if (fusion) {
    return (
      <aside className={cn("flex w-80 flex-col border-l border-line-soft bg-surface-panel", floating ? "absolute inset-y-0 right-0 z-40 shadow-panel" : "relative shrink-0")}>
        <header className="flex h-12 shrink-0 items-center gap-1 px-4">
          <span className="min-w-0 flex-1 truncate text-sm text-ink-muted">{peer ? peerBadgeText(peer, peer.desktopId) : desktopId}</span>
          <IconButton icon={<PanelRightClose className="size-3.5" />} label={t("filesClose")} onClick={onToggleExpanded} />
        </header>
        <div className="min-h-0 flex-1">
          <FusionDiscussionPanel key={`${scope}:${fusion.runId}:${fusion.toolCallId}`} {...fusion} onBack={() => setSelection(null)} />
        </div>
      </aside>
    );
  }

  return (
    <aside
      className={
        floating
          ? "absolute inset-y-0 right-0 z-40 flex w-80 flex-col border-l border-line-soft bg-surface-panel shadow-panel"
          : "relative flex w-80 shrink-0 flex-col border-l border-line-soft bg-surface-panel"
      }
    >
      <header className="flex h-12 shrink-0 items-center gap-1 px-4">
        {historySource
          ? (
              // Two pages of one conversation on that machine: what is in its
              // workspace now, and how that workspace got here. The switch takes
              // the title's place rather than sitting under it — the panel is a
              // column, and a title plus a switch would cost two rows of it.
              <div className="grid min-w-0 flex-1 grid-cols-2 gap-1 rounded-md border border-line-soft p-0.5">
                <PeerPanelTab
                  active={!showingHistory}
                  label={t("filesTab")}
                  onClick={() => setView("files")}
                />
                <PeerPanelTab
                  active={showingHistory}
                  label={t("historyTab")}
                  onClick={() => setView("history")}
                />
              </div>
            )
          : (
              <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">
                {t("filesOn", { name: peer ? peerBadgeText(peer, peer.desktopId) : desktopId })}
              </span>
            )}
        {!showingHistory
          ? (
              <IconButton
                disabled={files.loading}
                icon={<RefreshCw className={cn("size-3.5", files.loading && "animate-spin")} />}
                label={t("filesRefresh")}
                onClick={() => void files.refresh()}
              />
            )
          : null}
        <IconButton
          icon={<PanelRightClose className="size-3.5" />}
          label={t("filesClose")}
          onClick={onToggleExpanded}
        />
      </header>

      {showingHistory && historySource
        ? (
            <div className="min-h-0 flex-1">
              <GitHistoryPanel source={historySource} />
            </div>
          )
        : null}

      <div
        className={cn(
          "flex min-h-0 flex-1 flex-col px-3 pb-3",
          showingHistory && "hidden",
        )}
      >
        <div className="mb-2 flex items-center gap-1">
          <IconButton
            disabled={files.loading || files.parent === null}
            icon={<ArrowUp className="size-3.5" />}
            label={t("filesUp")}
            onClick={() => void files.open(files.parent ?? undefined)}
          />
          <span className="min-w-0 flex-1 truncate text-xs text-ink-muted" title={files.listing?.path ?? undefined}>
            {files.listing?.path || files.listing?.rootPath || ""}
          </span>
        </div>

        {files.error ? <p className="mb-2 text-xs text-danger">{files.error}</p> : null}
        {files.saved ? <p className="mb-2 text-xs text-ink-muted">{t("filesSaved", { name: files.saved })}</p> : null}

        <div className="min-h-0 flex-1 overflow-y-auto rounded-md border border-line-soft">
          {files.entries.length === 0
            ? (
                <p className="p-3 text-xs text-ink-muted">
                  {files.listing === null || files.loading ? t("loadingHistory") : t("filesEmpty")}
                </p>
              )
            : files.entries.map((entry) => {
                // A folder opens; a file the overlay can render opens *here*,
                // after being fetched into this machine's cache — the same click,
                // the same overlay and the same file types as the local panel,
                // which is what makes the two panels one behaviour. Anything else
                // is downloaded through its own control rather than opened with a
                // handler this machine does not have for another machine's file.
                // Each row answers for itself. Gating every row on "anything in
                // this panel is busy" made a second file unclickable while the
                // first was still coming down — the transfer of one file has
                // nothing to do with whether the reader may open another, and on
                // a slow link it read as the panel being broken.
                const previewable = !entry.isDir && previewKindForPath(entry.name) !== null;
                const fetching = files.previewingPaths.has(entry.path);
                const downloading = files.busyPaths.has(entry.path);
                // One row height for both kinds: a folder row carries neither a
                // size label nor a download button, so leaving the height to its
                // contents made it visibly shorter than the file rows around it.
                // The floor is stated here, and each row's controls stay below
                // it (the download icon is `size-7`, not the default `size-9`),
                // so text scaling still grows a row instead of being clipped.
                return (
                  <div className="flex min-h-10 items-center gap-2 border-b border-line-soft/50 px-2 py-1 last:border-b-0" key={entry.path}>
                    {entry.isDir
                      ? <Folder aria-hidden className="size-3.5 shrink-0 text-ink-muted" />
                      : fetching
                        ? <Loader2 aria-hidden className="size-3.5 shrink-0 animate-spin text-ink-muted" />
                        : <File aria-hidden className="size-3.5 shrink-0 text-ink-muted" />}
                    <button
                      className="min-w-0 flex-1 truncate text-left text-sm text-ink hover:underline disabled:hover:no-underline"
                      disabled={(!entry.isDir && !previewable) || files.loading || fetching}
                      onClick={() => void (entry.isDir ? files.open(entry.path) : files.preview(entry))}
                      title={previewable ? t("filesPreview", { name: entry.name }) : entry.path}
                      type="button"
                    >
                      {entry.name}
                    </button>
                    {entry.isDir
                      ? null
                      : (
                          <>
                            <span className="shrink-0 text-2xs text-ink-muted">{formatSize(entry.size)}</span>
                            {/* Download goes through a save dialog: guessing a
                                destination would write to a directory the user did
                                not choose, on a machine whose conventions we are
                                guessing at. */}
                            <IconButton
                              className="size-7"
                              disabled={downloading || fetching}
                              icon={<Download className="size-3.5" />}
                              label={t("filesDownload", { name: entry.name })}
                              onClick={() => void files.download(entry)}
                            />
                          </>
                        )}
                  </div>
                );
              })}
        </div>
      </div>

      {/* The same overlay the local panel shows, over the file this machine just
          fetched: one preview implementation, so a file cannot look like one
          thing here and another there. `onOpenExternal` is that panel's
          "open it with the platform" fallback, which for another machine's file
          is the save dialog — the download control's own action. */}
      {preview && previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={preview.name}
              onClose={() => files.closePreview()}
              onOpenExternal={() => void files.download(preview.entry)}
              open
              path={preview.path}
            />
          )
        : null}
    </aside>
  );
}

/** One page of the peer panel: the workspace's files, or its history. */
function PeerPanelTab({
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
      type="button"
      aria-pressed={active}
      className={cn(
        "h-7 truncate rounded px-2 text-xs font-medium transition-colors",
        active ? "bg-accent-soft/70 text-accent" : "text-ink-muted hover:text-ink",
      )}
      onClick={onClick}
    >
      {label}
    </button>
  );
}
