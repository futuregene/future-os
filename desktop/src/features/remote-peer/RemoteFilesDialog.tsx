import type { RemoteFileEntry, RemoteFileListing } from "./remotePeerClient";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Download, File, Folder } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { errorMessage } from "../../lib/errors";
import { downloadRemoteFile, listRemoteSessionFiles } from "./remotePeerClient";
import { formatSize, parentOf } from "./remotePeerFiles";

/**
 * Browse and download the files in a conversation on another machine.
 *
 * The listing comes from the host and is navigated by the host's own paths: this
 * component never joins a path itself, because the separator and the case rules
 * are the host's, not this machine's.
 *
 * A download always goes through a save dialog. Guessing a destination would
 * mean writing to a directory the user did not choose on a machine whose
 * conventions we are guessing at, and a file that lands somewhere unexpected is
 * indistinguishable from one that was never saved.
 */
export function RemoteFilesDialog({
  desktopId,
  onClose,
  peerName,
  sessionId,
}: {
  desktopId: string;
  onClose: () => void;
  peerName: string;
  sessionId: string;
}) {
  const { t } = useTranslation("remotePeer");
  // `null` until the host answers: the browser reads its own root rather than
  // making every caller pre-read one, and an empty state while that is in flight
  // would read as "this conversation has no files".
  const [listing, setListing] = useState<RemoteFileListing | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  /**
   * Read a directory.
   *
   * No in-flight guard, deliberately: every control that calls this is disabled
   * while a read is outstanding, so a second call cannot be produced from the
   * UI, and a guard no test can reach is a guard no test can trust.
   */
  async function open(path?: string) {
    setLoading(true);
    setError(null);
    try {
      setListing(await listRemoteSessionFiles(desktopId, sessionId, path));
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setLoading(false);
    }
  }

  // The root on open. Written out rather than calling `open` so the effect's
  // dependencies are the two it really uses, and so a dialog closed while the
  // read is in flight cannot set state on an unmounted component.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      try {
        const next = await listRemoteSessionFiles(desktopId, sessionId);
        if (!cancelled)
          setListing(next);
      }
      catch (err) {
        if (!cancelled)
          setError(errorMessage(err));
      }
      finally {
        if (!cancelled)
          setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [desktopId, sessionId]);

  /** No in-flight guard here either; see `open`. */
  async function download(entry: RemoteFileEntry) {
    setBusyPath(entry.path);
    setError(null);
    setSaved(null);
    try {
      // The dialog is what makes the destination the user's choice; a cancelled
      // dialog returns null, and nothing is fetched.
      const destination = await save({ defaultPath: entry.name });
      if (!destination)
        return;
      const savedAs = await downloadRemoteFile({
        desktopId,
        sessionId,
        path: entry.path,
        name: entry.name,
        destination,
      });
      // The host may have renamed the file (a preview variant), so the name it
      // reports is the one to show.
      setSaved(savedAs);
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setBusyPath(null);
    }
  }

  /** One level up, by trimming the host's path — never by rebuilding it. */
  const parent = listing ? parentOf(listing.path, listing.rootPath) : null;
  const entries = listing?.entries ?? [];

  return (
    <Dialog
      description={t("filesOn", { name: peerName })}
      onClose={onClose}
      open
      title={t("filesTitle")}
    >
      <div className="mb-3 flex items-center gap-2">
        <Button
          disabled={loading || parent === null}
          onClick={() => void open(parent ?? undefined)}
          size="sm"
          variant="ghost"
        >
          <ArrowUp className="size-3.5" />
          {t("filesUp")}
        </Button>
        <span className="min-w-0 flex-1 truncate text-xs text-ink-muted" title={listing?.path ?? undefined}>
          {listing?.path || listing?.rootPath || ""}
        </span>
      </div>

      {error ? <p className="mb-2 text-xs text-danger">{error}</p> : null}
      {saved ? <p className="mb-2 text-xs text-ink-muted">{t("filesSaved", { name: saved })}</p> : null}

      <div className="max-h-80 overflow-y-auto rounded-md border border-line-soft">
        {entries.length === 0
          ? <p className="p-3 text-xs text-ink-muted">{listing === null || loading ? t("loadingHistory") : t("filesEmpty")}</p>
          : entries.map(entry => (
              <div
                className="flex items-center gap-2 border-b border-line-soft/50 px-3 py-2 last:border-b-0"
                key={entry.path}
              >
                {entry.isDir
                  ? <Folder aria-hidden className="size-4 shrink-0 text-ink-muted" />
                  : <File aria-hidden className="size-4 shrink-0 text-ink-muted" />}
                <button
                  className="min-w-0 flex-1 truncate text-left text-sm text-ink hover:underline disabled:hover:no-underline"
                  disabled={!entry.isDir || loading}
                  onClick={() => void open(entry.path)}
                  title={entry.path}
                  type="button"
                >
                  {entry.name}
                </button>
                {entry.isDir
                  ? null
                  : <span className="shrink-0 text-xs text-ink-muted">{formatSize(entry.size)}</span>}
                {entry.isDir
                  ? null
                  : (
                      <Button
                        disabled={busyPath !== null}
                        onClick={() => void download(entry)}
                        size="sm"
                        variant="ghost"
                      >
                        <Download className="size-3.5" />
                        {busyPath === entry.path ? t("filesDownloading") : t("filesDownload")}
                      </Button>
                    )}
              </div>
            ))}
      </div>
    </Dialog>
  );
}
