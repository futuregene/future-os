import type { RemoteWorkspaceRow } from "./remotePeerClient";
import { useCallback, useEffect, useRef, useState } from "react";
import { listRemoteWorkspaces } from "./remotePeerClient";

/**
 * Another machine's workspaces, for the screens that offer to join one.
 *
 * A workspace is a directory on *that* machine, so this is a read of the host's
 * own store and nothing more: the local new-conversation screen can browse this
 * disk, while this side can only name what the host already knows about.
 *
 * `desktopId` is nullable because the caller's question is "which host's
 * workspaces, if any" — a local draft has none to ask for, and passing `null`
 * clears the list instead of leaving another machine's folders on screen.
 *
 * The read is repeated when `desktopId` changes and through `reload`, which the
 * caller uses after it changes something on the host (a workspace created from
 * the draft's own menu). It is deliberately not cached per host the way
 * `remoteHostCatalog` caches models and skills: a workspace list is small, it is
 * read only while a draft is open, and a stale entry here is a name the user
 * just picked that the menu does not show.
 */
export function useRemoteWorkspaceRows(desktopId: string | null): {
  error: string | null;
  loading: boolean;
  reload: () => Promise<void>;
  rows: RemoteWorkspaceRow[];
} {
  const [rows, setRows] = useState<RemoteWorkspaceRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /**
   * Which read is the current one.
   *
   * A machine's list is an identity for the caller: choosing between two
   * folders means the ids belong to the host on screen. Two reads can be in
   * flight at once — a reader opening a draft on one machine and then another
   * replaces one with the other — and the slower answer must not overwrite the
   * question that is now being asked.
   */
  const generationRef = useRef(0);

  const reload = useCallback(async () => {
    const current = ++generationRef.current;
    if (!desktopId) {
      setRows([]);
      setLoading(false);
      setError(null);
      return;
    }
    setLoading(true);
    try {
      const read = await listRemoteWorkspaces(desktopId);
      if (current !== generationRef.current)
        return;
      setRows(read);
      setError(null);
    }
    catch (caught) {
      if (current !== generationRef.current)
        return;
      // Reported rather than swallowed: a menu that offered nothing would say
      // "that computer has no workspaces", which is a different fact from "it
      // did not answer".
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      if (current === generationRef.current)
        setLoading(false);
    }
  }, [desktopId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { error, loading, reload, rows };
}
