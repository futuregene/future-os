import type { StoredThread } from "../../../integrations/storage/threadStore";
import { useCallback, useEffect, useMemo, useState } from "react";

export function useRailSelection({
  onBatchDeleteThreads,
  onSelectThread,
  threadScopes,
  visibleThreads,
}: {
  onBatchDeleteThreads: (threads: StoredThread[]) => void;
  onSelectThread: (thread: StoredThread) => void;
  threadScopes: Map<string, string>;
  visibleThreads: StoredThread[];
}) {
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectionScope, setSelectionScope] = useState("chat");
  const [selectedThreadIds, setSelectedThreadIds] = useState<Set<string>>(() => new Set());

  const scopedThreads = useMemo(
    () => visibleThreads.filter(thread => threadScopes.get(thread.id) === selectionScope),
    [selectionScope, threadScopes, visibleThreads],
  );

  const exitSelectionMode = useCallback(() => {
    setSelectionMode(false);
    setSelectedThreadIds(new Set());
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
  // checkbox state never include stale IDs.
  useEffect(() => {
    if (!selectionMode)
      return;
    const allowed = new Set(scopedThreads.map(thread => thread.id));
    setSelectedThreadIds((current) => {
      const next = new Set([...current].filter(id => allowed.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [scopedThreads, selectionMode]);

  const enterSelectionMode = useCallback((scope: string) => {
    setSelectionScope(scope);
    setSelectedThreadIds(new Set());
    setSelectionMode(true);
  }, []);

  const isThreadInScope = useCallback(
    (thread: StoredThread) => threadScopes.get(thread.id) === selectionScope,
    [selectionScope, threadScopes],
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

  const handleRowSelect = useCallback((thread: StoredThread) => {
    if (selectionMode) {
      if (isThreadInScope(thread))
        toggleThreadSelection(thread);
      return;
    }
    onSelectThread(thread);
  }, [isThreadInScope, onSelectThread, selectionMode, toggleThreadSelection]);

  const selectAll = useCallback(() => {
    setSelectedThreadIds(new Set(scopedThreads.map(thread => thread.id)));
  }, [scopedThreads]);

  const deselectAll = useCallback(() => setSelectedThreadIds(new Set()), []);

  const deleteSelected = useCallback(() => {
    const selectedThreads = scopedThreads.filter(thread => selectedThreadIds.has(thread.id));
    if (selectedThreads.length === 0)
      return;
    onBatchDeleteThreads(selectedThreads);
    exitSelectionMode();
  }, [exitSelectionMode, onBatchDeleteThreads, scopedThreads, selectedThreadIds]);

  return {
    deleteSelected,
    deselectAll,
    enterSelectionMode,
    exitSelectionMode,
    handleRowSelect,
    isThreadInScope,
    scopedThreads,
    selectAll,
    selectedThreadIds,
    selectionMode,
    toggleThreadSelection,
  };
}
