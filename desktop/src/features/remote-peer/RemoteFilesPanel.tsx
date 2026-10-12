import type { RemotePeer } from "./remotePeerClient";
import { ArrowUp, Download, File, Folder, PanelRightClose, PanelRightOpen, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { cn } from "../../lib/cn";
import { peerBadgeText } from "./peerIcons";
import { formatSize } from "./remotePeerFiles";
import { useRemoteFiles } from "./useRemoteFiles";

/**
 * The right panel for a remote conversation: that machine's files, and nothing
 * else.
 *
 * Only files, with no tab strip: the panel's other tabs describe *this* machine
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
  onToggleExpanded,
  peer,
  sessionId,
}: {
  desktopId: string;
  expanded: boolean;
  onToggleExpanded: () => void;
  peer: RemotePeer | undefined;
  sessionId: string;
}) {
  const { t } = useTranslation("remotePeer");
  const files = useRemoteFiles(desktopId, sessionId);

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

  return (
    <aside className="relative flex w-80 shrink-0 flex-col border-l border-line-soft bg-surface-panel">
      <header className="flex h-12 shrink-0 items-center gap-1 px-4">
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">
          {t("filesOn", { name: peer ? peerBadgeText(peer, peer.desktopId) : desktopId })}
        </span>
        <IconButton
          disabled={files.loading}
          icon={<RefreshCw className={cn("size-3.5", files.loading && "animate-spin")} />}
          label={t("filesRefresh")}
          onClick={() => void files.refresh()}
        />
        <IconButton
          icon={<PanelRightClose className="size-3.5" />}
          label={t("filesClose")}
          onClick={onToggleExpanded}
        />
      </header>

      <div className="flex min-h-0 flex-1 flex-col px-3 pb-3">
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
            : files.entries.map(entry => (
                <div className="flex items-center gap-2 border-b border-line-soft/50 px-2 py-1.5 last:border-b-0" key={entry.path}>
                  {entry.isDir
                    ? <Folder aria-hidden className="size-3.5 shrink-0 text-ink-muted" />
                    : <File aria-hidden className="size-3.5 shrink-0 text-ink-muted" />}
                  <button
                    className="min-w-0 flex-1 truncate text-left text-sm text-ink hover:underline disabled:hover:no-underline"
                    disabled={!entry.isDir || files.loading}
                    onClick={() => void files.open(entry.path)}
                    title={entry.path}
                    type="button"
                  >
                    {entry.name}
                  </button>
                  {entry.isDir
                    ? null
                    : (
                        <>
                          <span className="shrink-0 text-[11px] text-ink-muted">{formatSize(entry.size)}</span>
                          {/* Download goes through a save dialog: guessing a
                              destination would write to a directory the user did
                              not choose, on a machine whose conventions we are
                              guessing at. */}
                          <IconButton
                            disabled={files.busyPath !== null}
                            icon={<Download className="size-3.5" />}
                            label={t("filesDownload", { name: entry.name })}
                            onClick={() => void files.download(entry)}
                          />
                        </>
                      )}
                </div>
              ))}
        </div>
      </div>
    </aside>
  );
}
