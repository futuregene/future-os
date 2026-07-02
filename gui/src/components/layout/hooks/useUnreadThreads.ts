import type { ThreadRunInfo } from "./useThreadStore";
import { useEffect, useRef, useState } from "react";

const STORAGE_KEY = "future.unreadThreads";

function loadUnread(): Set<string> {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  }
  catch {
    return new Set();
  }
}

function saveUnread(ids: Set<string>): void {
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]));
  }
  catch {
    // Best-effort; the in-memory set still applies this session.
  }
}

function isRunning(status: ThreadRunInfo["status"]): boolean {
  return status === "running" || status === "queued" || status === "waiting_approval";
}

function isFinished(status: ThreadRunInfo["status"]): boolean {
  return status === "completed" || status === "failed";
}

/**
 * Tracks which threads have an unread finished run. A thread becomes unread when
 * we *observe* its run transition from in-progress → finished this session —
 * including the thread currently being viewed, so the dot appears and lingers
 * until you leave it. The mark is cleared only on the edges: the instant a thread
 * is opened (enter) and the instant it is switched away from (leave). Staying on
 * a thread continuously does nothing. State lives in sessionStorage — a small id
 * set, not a per-thread history — so an app restart starts with a clean slate.
 */
export function useUnreadThreads(
  runInfo: Record<string, ThreadRunInfo | undefined>,
  activeThreadId: string | null,
): Set<string> {
  const [unread, setUnread] = useState<Set<string>>(loadUnread);
  // Last status we saw per thread, to detect the in-progress → finished edge.
  const lastStatusRef = useRef<Record<string, ThreadRunInfo["status"]>>({});
  // Previous active thread, so a change can clear both the entered and left one.
  const prevActiveRef = useRef<string | null>(activeThreadId);

  useEffect(() => {
    const seen = lastStatusRef.current;
    const justFinished: string[] = [];
    for (const [id, info] of Object.entries(runInfo)) {
      if (!info)
        continue;
      const before = seen[id];
      if (before !== undefined && isRunning(before) && isFinished(info.status))
        justFinished.push(id);
      seen[id] = info.status;
    }
    // Drop status entries for threads no longer present so the ref doesn't grow
    // unbounded across a long session.
    for (const id of Object.keys(seen)) {
      if (!(id in runInfo))
        delete seen[id];
    }
    if (justFinished.length === 0)
      return;
    setUnread((current) => {
      const next = new Set(current);
      for (const id of justFinished)
        next.add(id);
      saveUnread(next);
      return next;
    });
  }, [runInfo]);

  // Clear on the enter edge (thread just opened) and the leave edge (thread just
  // switched away from). Declared after the marking effect so, if a completion
  // and a switch land together, the clear wins for the edges.
  useEffect(() => {
    const previous = prevActiveRef.current;
    prevActiveRef.current = activeThreadId;
    const edges = [previous, activeThreadId].filter((id): id is string => Boolean(id));
    if (edges.length === 0)
      return;
    setUnread((current) => {
      let changed = false;
      const next = new Set(current);
      for (const id of edges) {
        if (next.delete(id))
          changed = true;
      }
      if (!changed)
        return current;
      saveUnread(next);
      return next;
    });
  }, [activeThreadId]);

  return unread;
}
