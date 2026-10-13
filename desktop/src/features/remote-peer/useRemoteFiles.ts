import type { RemoteFileEntry, RemoteFileListing } from "./remotePeerClient";
import { save } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
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
  /**
   * The paths being downloaded right now.
   *
   * A set rather than one path, because the panel disables each row's *own*
   * control from it: with a single value, starting a second download re-enabled
   * the first row and dropped its spinner while it was still running.
   */
  busyPaths: ReadonlySet<string>;
  /** The name the host saved as, once a download lands. */
  saved: string | null;
  open: (path?: string) => Promise<void>;
  refresh: () => Promise<void>;
  download: (entry: RemoteFileEntry) => Promise<void>;
  /** The paths being fetched for a preview, for the same reason. */
  previewingPaths: ReadonlySet<string>;
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

/** No path is in flight. Shared so an idle panel keeps one stable value. */
const NO_PATHS: ReadonlySet<string> = new Set();

/**
 * One path added to or removed from an in-flight set.
 *
 * A copy every time: the set on screen is never mutated, and the two calls a
 * transfer makes (start, finish) always change it, so there is no "nothing
 * changed" case to short-circuit — a guard for one would be a branch no test
 * could reach.
 */
function withPath(current: ReadonlySet<string>, path: string, present: boolean): ReadonlySet<string> {
  const next = new Set(current);
  if (present)
    next.add(path);
  else
    next.delete(path);
  return next;
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
  const [busyPaths, setBusyPaths] = useState<ReadonlySet<string>>(NO_PATHS);
  const [saved, setSaved] = useState<string | null>(null);
  const [previewingPaths, setPreviewingPaths] = useState<ReadonlySet<string>>(NO_PATHS);
  const [previewTarget, setPreviewTarget] = useState<RemotePreviewTarget | null>(null);
  /**
   * Which preview was asked for last.
   *
   * Two fetches can be in flight at once — nothing stops a reader clicking a
   * second file while the first is still coming — and the one they clicked last
   * is the one they are looking at, so a slower earlier answer must not replace
   * it. Bumped by a new preview and by closing the overlay, because a fetch that
   * lands after the reader closed it would otherwise open it again.
   */
  const previewGenerationRef = useRef(0);
  /** The same for a download's "saved as" line. */
  const downloadGenerationRef = useRef(0);

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
    // Whichever way this runs — the panel closing, or the reader moving to
    // another conversation — the fetches the *previous* one started are no
    // longer about what is on screen, so none of their answers may land: a
    // preview arriving late would put one machine's file under another's title,
    // and its spinner would belong to a row that is not drawn any more.
    previewGenerationRef.current += 1;
    setPreviewingPaths(NO_PATHS);
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

  /**
   * Download one file to a destination the user chooses.
   *
   * No in-flight guard, and deliberately not a *panel-wide* one: two files can
   * be coming down at once — a slow transfer left running while the reader picks
   * the next file is the normal case — so each row's own control is what says
   * whether that row is busy. What this does need is to keep the answers
   * straight, so a slow earlier download cannot label a later one.
   */
  const download = useCallback(async (entry: RemoteFileEntry) => {
    const generation = ++downloadGenerationRef.current;
    setBusyPaths(current => withPath(current, entry.path, true));
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
      if (generation === downloadGenerationRef.current)
        setSaved(savedAs);
    }
    catch (err) {
      if (generation === downloadGenerationRef.current)
        setError(errorMessage(err));
    }
    finally {
      setBusyPaths(current => withPath(current, entry.path, false));
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
    const generation = ++previewGenerationRef.current;
    setPreviewingPaths(current => withPath(current, entry.path, true));
    setError(null);
    try {
      const fetched = await previewRemoteFile({
        desktopId,
        sessionId,
        path: entry.path,
        name: entry.name,
      });
      // The last file the reader asked for is the one they are looking at, so an
      // earlier fetch that lands afterwards is dropped rather than shown.
      if (generation === previewGenerationRef.current)
        setPreviewTarget({ entry, name: fetched.name, path: fetched.path });
    }
    catch (err) {
      if (generation === previewGenerationRef.current)
        setError(errorMessage(err));
    }
    finally {
      setPreviewingPaths(current => withPath(current, entry.path, false));
    }
  }, [desktopId, sessionId]);

  return {
    busyPaths,
    /**
     * Close the overlay, and give up on the fetches still running for it.
     *
     * Otherwise a file the reader already dismissed lands a moment later and
     * opens itself again over whatever they moved on to.
     */
    closePreview: () => {
      previewGenerationRef.current += 1;
      setPreviewTarget(null);
    },
    download,
    entries: listing?.entries ?? [],
    error,
    listing,
    loading,
    open,
    parent: listing ? parentOf(listing.path, listing.rootPath) : null,
    preview,
    previewTarget,
    previewingPaths,
    refresh: readRoot,
    saved,
  };
}
