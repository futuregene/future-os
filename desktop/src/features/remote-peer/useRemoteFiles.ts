import type { RemoteFileEntry, RemoteFileListing } from "./remotePeerClient";
import { save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useState } from "react";
import { errorMessage } from "../../lib/errors";
import { downloadRemoteFile, listRemoteSessionFiles, previewRemoteFile } from "./remotePeerClient";
import { parentOf } from "./remotePeerFiles";

/**
 * Browsing one conversation's files on another machine.
 *
 * Extracted so the panel that shows them is the only thing that decides how they
 * look*: the reading, the navigation and the download are the same whether they
 * are drawn in a panel or anywhere else, and a second copy would be a second set
 * of rules for what "up" means on a host whose separators are not this machine's.
 *
 * Nothing here joins a path itself: every level comes from the host, because the
 * separator and the case rules belong to the machine the files are on.
 */
export interface RemoteFilesState {
  listing: RemoteFileListing | null;
  loading: boolean;
  error: string | null;
  /** The path being downloaded, so its own row can say so. */
  busyPath: string | null;
  /** The name the host saved as, once a download lands. */
  saved: string | null;
  open: (path?: string) => Promise<void>;
  refresh: () => Promise<void>;
  download: (entry: RemoteFileEntry) => Promise<void>;
  /** The path being fetched for a preview, so its own row can say so. */
  previewingPath: string | null;
  /** What the overlay should show, or `null` when nothing is open. */
  previewTarget: RemotePreviewTarget | null;
  /** Fetch one file into the local cache and show it. */
  preview: (entry: RemoteFileEntry) => Promise<void>;
  closePreview: () => void;
  /** One level up, or null at the root. */
  parent: string | null;
  entries: RemoteFileEntry[];
}

/**
 * A file being previewed: the local cache path the overlay reads, plus the source
 * entry, so a preview that cannot be rendered can fall back to saving the file.
 */
export interface RemotePreviewTarget {
  entry: RemoteFileEntry;
  name: string;
  path: string;
}

export function useRemoteFiles(
  desktopId: string,
  sessionId: string,
  /**
   * Whether anything is showing these files.
   *
   * The panel collapses to a toggle over the conversation header, and a listing
   * read for a panel nobody opened is a request the reader did not ask for — on
   * another machine, over the network, for a directory they may never look at.
   */
  enabled: boolean,
): RemoteFilesState {
  // `null` until the host answers: the browser reads its own root rather than
  // making every caller pre-read one, and an empty state while that is in flight
  // would read as "this conversation has no files".
  const [listing, setListing] = useState<RemoteFileListing | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [previewingPath, setPreviewingPath] = useState<string | null>(null);
  const [previewTarget, setPreviewTarget] = useState<RemotePreviewTarget | null>(null);

  /**
   * Read a directory.
   *
   * No in-flight guard, deliberately: every control that calls this is disabled
   * while a read is outstanding, so a second call cannot be produced from the
   * UI, and a guard no test can reach is a guard no test can trust.
   */
  const open = useCallback(async (path?: string) => {
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
  }, [desktopId, sessionId]);

  const readRoot = useCallback(async () => {
    setLoading(true);
    try {
      setListing(await listRemoteSessionFiles(desktopId, sessionId));
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setLoading(false);
    }
  }, [desktopId, sessionId]);

  // The root on open. Written out rather than calling `open` so this effect's
  // dependencies are the two it really uses, and so a view closed while the read
  // is in flight cannot set state on an unmounted component.
  useEffect(() => {
    if (!enabled) {
      setListing(null);
      setError(null);
      // Another conversation's file must not stay on screen under this one: the
      // overlay would show one machine's screenshot under another's title.
      setPreviewTarget(null);
      return;
    }
    setPreviewTarget(null);
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
  }, [desktopId, sessionId, enabled]);

  /** No in-flight guard here either; see `open`. */
  const download = useCallback(async (entry: RemoteFileEntry) => {
    setBusyPath(entry.path);
    setError(null);
    setSaved(null);
    try {
      // The save dialog is what makes the destination the user's choice; a
      // cancelled dialog returns null, and nothing is fetched.
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
      // The host may rename the file (a preview variant), so the name it reports
      // is the one to show.
      setSaved(savedAs);
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setBusyPath(null);
    }
  }, [desktopId, sessionId]);

  /**
   * Fetch one file into the local cache and show it.
   *
   * No save dialog, deliberately: looking at a file is not copying it, and a
   * panel where every click opened a "where should this go?" dialog would make the
   * quickest thing on the screen the slowest. The bytes land in this app's own
   * cache, and the same overlay the local panel uses reads them from there.
   */
  const preview = useCallback(async (entry: RemoteFileEntry) => {
    setPreviewingPath(entry.path);
    setError(null);
    try {
      const fetched = await previewRemoteFile({
        desktopId,
        sessionId,
        path: entry.path,
        name: entry.name,
      });
      setPreviewTarget({ entry, name: fetched.name, path: fetched.path });
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setPreviewingPath(null);
    }
  }, [desktopId, sessionId]);

  return {
    busyPath,
    closePreview: () => setPreviewTarget(null),
    download,
    entries: listing?.entries ?? [],
    error,
    listing,
    loading,
    open,
    parent: listing ? parentOf(listing.path, listing.rootPath) : null,
    preview,
    previewTarget,
    previewingPath,
    refresh: readRoot,
    saved,
  };
}
