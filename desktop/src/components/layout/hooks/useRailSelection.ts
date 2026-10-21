import type { MergedConversation } from "../../../features/remote-peer/mergeConversations";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { useCallback, useEffect, useMemo, useState } from "react";

/**
 * The rail's batch selection, over every kind of row the list can hold.
 *
 * A batch is a *scope* — the chat section, or one workspace — and either kind of
 * row can be in it: this machine's threads and the ones a paired host reports.
 * The two are selected and deleted by different code (one store write here, one
 * command per conversation there), which is why they are held in two sets, but
 * they are one selection to the user: one toolbar, one count, one "select all",
 * and one Delete that does the right thing per row.
 */
export function useRailSelection({
  onBatchDeleteThreads,
  onDeleteRemoteConversations,
  onSelectThread,
  pinnedAreShortcuts = true,
  /** The scope each remote row is in, keyed by its merged-list row key. */
  remoteScopes,
  threadScopes,
  visibleRemoteRows,
  visibleThreads,
}: {
  onBatchDeleteThreads: (threads: StoredThread[]) => void;
  /**
   * Delete a batch of conversations on the hosts that own them.
   *
   * Separate from `onBatchDeleteThreads` because it is a different operation on
   * a different machine: one is this app's store, the other is a command per
   * conversation to the host. Handing a remote row to the local batch would
   * delete a thread this machine does not have.
   */
  onDeleteRemoteConversations: (rows: MergedConversation[]) => void;
  onSelectThread: (thread: StoredThread) => void;
  /**
   * Whether a pinned row belongs to the shortcut section above the groups.
   *
   * True for the tree, where it does: select-all in a group must not sweep up the
   * conversations the user pinned on purpose, so those rows carry no checkbox
   * while a batch is open. False for the recent view, which opens no such
   * section — it ranks by one clock and draws the pin as a marker — so a pinned
   * conversation there is an ordinary row, and leaving it out put one row of that
   * very list beyond the batch for no reason the reader could see.
   */
  pinnedAreShortcuts?: boolean;
  remoteScopes: Map<string, string>;
  threadScopes: Map<string, string>;
  /** The remote rows the list is showing, for selection and counts. */
  visibleRemoteRows: MergedConversation[];
  visibleThreads: StoredThread[];
}) {
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectionScope, setSelectionScope] = useState("chat");
  const [selectedThreadIds, setSelectedThreadIds] = useState<Set<string>>(() => new Set());
  /** Remote rows are keyed by `desktopId::sessionId` (their merged-list key). */
  const [selectedRemoteKeys, setSelectedRemoteKeys] = useState<Set<string>>(() => new Set());

  // Pinned conversations are hoisted into their own global section, outside
  // every group, so they are never part of a batch: select-all in a group must
  // not sweep up the conversations the user pinned on purpose, and the pinned
  // rows carry no checkbox while selection mode is open. The same holds for a
  // pinned *host* row, for the same reason.
  const scopedThreads = useMemo(
    () => visibleThreads.filter(
      thread => (!pinnedAreShortcuts || !thread.pinned) && threadScopes.get(thread.id) === selectionScope,
    ),
    [pinnedAreShortcuts, selectionScope, threadScopes, visibleThreads],
  );
  const scopedRemoteRows = useMemo(
    () => visibleRemoteRows.filter(
      row => (!pinnedAreShortcuts || !row.pinned) && remoteScopes.get(row.key) === selectionScope,
    ),
    [pinnedAreShortcuts, remoteScopes, selectionScope, visibleRemoteRows],
  );

  const exitSelectionMode = useCallback(() => {
    setSelectionMode(false);
    setSelectedThreadIds(new Set());
    setSelectedRemoteKeys(new Set());
  }, []);

  useEffect(() => {
    if (!selectionMode)
      return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape")
        exitSelectionMode();
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [exitSelectionMode, selectionMode]);

  // Threads may be archived or deleted by another client while selection mode
  // is open. Keep the selection inside the live scope so counts and tri-state
  // checkbox state never include stale IDs. The same for a host's rows: a
  // catalogue read that drops one (deleted there, or the link went down) must
  // drop it from the count, or Delete would address a conversation the user can
  // no longer see.
  useEffect(() => {
    if (!selectionMode)
      return;
    const allowed = new Set(scopedThreads.map(thread => thread.id));
    setSelectedThreadIds((current) => {
      const next = new Set([...current].filter(id => allowed.has(id)));
      return next.size === current.size ? current : next;
    });
    const allowedRemote = new Set(scopedRemoteRows.map(row => row.key));
    setSelectedRemoteKeys((current) => {
      const next = new Set([...current].filter(key => allowedRemote.has(key)));
      return next.size === current.size ? current : next;
    });
  }, [scopedRemoteRows, scopedThreads, selectionMode]);

  const enterSelectionMode = useCallback((scope: string) => {
    setSelectionScope(scope);
    setSelectedThreadIds(new Set());
    setSelectedRemoteKeys(new Set());
    setSelectionMode(true);
  }, []);

  const isThreadInScope = useCallback(
    (thread: StoredThread) =>
      (!pinnedAreShortcuts || !thread.pinned) && threadScopes.get(thread.id) === selectionScope,
    [pinnedAreShortcuts, selectionScope, threadScopes],
  );

  const isRemoteRowInScope = useCallback(
    (row: MergedConversation) =>
      (!pinnedAreShortcuts || !row.pinned) && remoteScopes.get(row.key) === selectionScope,
    [pinnedAreShortcuts, remoteScopes, selectionScope],
  );

  const toggleThreadSelection = useCallback((thread: StoredThread) => {
    setSelectedThreadIds((current) => {
      const next = new Set(current);
      if (next.has(thread.id))
        next.delete(thread.id);
      else
        next.add(thread.id);
      return next;
    });
  }, []);

  const toggleRemoteSelection = useCallback((row: MergedConversation) => {
    setSelectedRemoteKeys((current) => {
      const next = new Set(current);
      if (next.has(row.key))
        next.delete(row.key);
      else
        next.add(row.key);
      return next;
    });
  }, []);

  const handleRowSelect = useCallback((thread: StoredThread) => {
    if (selectionMode) {
      if (isThreadInScope(thread))
        toggleThreadSelection(thread);
      return;
    }
    onSelectThread(thread);
  }, [isThreadInScope, onSelectThread, selectionMode, toggleThreadSelection]);

  /**
   * Whether activating this remote row means ticking it rather than opening it.
   *
   * The list composes the click itself (opening a remote conversation is the
   * shell's job — it sets which host and session the window shows), so the hook
   * exports the *rule* and not a handler: in selection mode a row in the scope is
   * ticked, and a row outside it is not in this batch at all.
   */
  const isRemoteRowSelectable = useCallback(
    (row: MergedConversation) => selectionMode && isRemoteRowInScope(row),
    [isRemoteRowInScope, selectionMode],
  );

  const selectAll = useCallback(() => {
    setSelectedThreadIds(new Set(scopedThreads.map(thread => thread.id)));
    setSelectedRemoteKeys(new Set(scopedRemoteRows.map(row => row.key)));
  }, [scopedRemoteRows, scopedThreads]);

  const deselectAll = useCallback(() => {
    setSelectedThreadIds(new Set());
    setSelectedRemoteKeys(new Set());
  }, []);

  const deleteSelected = useCallback(() => {
    const selectedThreads = scopedThreads.filter(thread => selectedThreadIds.has(thread.id));
    const selectedRemote = scopedRemoteRows.filter(row => selectedRemoteKeys.has(row.key));
    if (selectedThreads.length === 0 && selectedRemote.length === 0)
      return;
    if (selectedThreads.length > 0)
      onBatchDeleteThreads(selectedThreads);
    if (selectedRemote.length > 0)
      onDeleteRemoteConversations(selectedRemote);
    exitSelectionMode();
  }, [exitSelectionMode, onBatchDeleteThreads, onDeleteRemoteConversations, scopedRemoteRows, scopedThreads, selectedRemoteKeys, selectedThreadIds]);

  return {
    deleteSelected,
    deselectAll,
    enterSelectionMode,
    exitSelectionMode,
    handleRowSelect,
    isRemoteRowInScope,
    isRemoteRowSelectable,
    isThreadInScope,
    scopedThreads,
    selectAll,
    selectedCount: selectedThreadIds.size + selectedRemoteKeys.size,
    selectedRemoteKeys,
    selectedThreadIds,
    selectionMode,
    toggleRemoteSelection,
    toggleThreadSelection,
    totalCount: scopedThreads.length + scopedRemoteRows.length,
  };
}
