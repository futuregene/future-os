import type { TaskBackend } from "./taskBackend";
import { useCallback, useEffect, useState } from "react";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { localTaskBackend } from "./taskBackend";

/** Task rows as the Tauri backend serializes them. */
export interface TaskRunView {
  id: string;
  kind: string;
  origin: string;
  status: string;
  dueAt: number | null;
  threadId: string | null;
  sessionId: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  promptVersion: number | null;
  resultSummary: string | null;
  errorMessage: string | null;
  /** This run's conversation was deleted after it settled. */
  sessionDeleted: boolean;
}

export interface TaskView {
  id: string;
  name: string;
  enabled: boolean;
  prompt: string;
  promptVersion: number;
  cwd: string;
  modelId: string | null;
  thinkingLevel: string | null;
  /**
   * IANA zone the schedule's wall-clock times are read in (`Asia/Shanghai`).
   * Always concrete: a task that stores none reports the zone it resolves to.
   */
  timezone: string;
  sessionPolicy: string;
  /** `keep` (default) or `delete`; only meaningful with `sessionPolicy: "new"`. */
  sessionRetention: string;
  conversationMode: string;
  triggerKind: string;
  trigger: Record<string, unknown>;
  /**
   * Always 0. Task dependencies were removed; the field stays because a phone
   * of an older build reads `depCount` to label a task's trigger, and an absent
   * field would leave it showing a trigger kind that no longer exists.
   */
  depCount: number;
  nextDueAt: number | null;
  /** An explicit request is waiting for the tick (the task was busy). */
  queued: boolean;
  latestRun: TaskRunView | null;
}

export interface TaskInput {
  name: string;
  prompt: string;
  cwd: string;
  modelId?: string | null;
  thinkingLevel?: string | null;
  /** Empty or absent = this machine's zone (host-resolved, Beijing fallback). */
  timezone?: string | null;
  sessionPolicy?: string;
  sessionRetention?: string;
  conversationMode?: string;
  triggerKind?: string;
  trigger?: Record<string, unknown>;
  enabled?: boolean;
}

/**
 * The task list and the actions on it, over whichever backend it was given.
 * The backend defaults to this app's own store, so every existing call site
 * keeps working unchanged; passing another one points the same state machine at
 * a paired host. `reload` is part of the result so a remote panel can refresh on
 * that host's pushes instead of this app's.
 */
export function useTasks(backend: TaskBackend = localTaskBackend) {
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const next = await backend.list();
      setTasks(next);
      setError(null);
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setLoading(false);
    }
  }, [backend]);

  useEffect(() => {
    void reload();
  }, [reload]);

  // A run changes the task's state (queued → running → finished) and can also
  // produce a conversation; the host already announces both with
  // "threads-updated". Without this the panel kept saying "queued" at a task
  // that had been running for minutes, until the user left and came back.
  //
  // Only for this app's own tasks: the event says nothing about a remote host's,
  // and reloading a remote panel on a local event would show the wrong reason
  // for the refresh.
  useTauriEvent("threads-updated", () => {
    if (backend.kind === "local")
      void reload();
  });

  const createTask = useCallback(async (input: TaskInput) => {
    const created = await backend.create(input);
    await reload();
    return created;
  }, [backend, reload]);

  const updateTask = useCallback(async (id: string, input: TaskInput) => {
    const updated = await backend.update(id, input);
    await reload();
    return updated;
  }, [backend, reload]);

  const deleteTask = useCallback(async (id: string) => {
    await backend.remove(id);
    await reload();
  }, [backend, reload]);

  const setEnabled = useCallback(async (id: string, enabled: boolean) => {
    await backend.setEnabled(id, enabled);
    await reload();
  }, [backend, reload]);

  const listRuns = useCallback(
    (id: string, limit = 20) => backend.listRuns(id, limit),
    [backend],
  );

  return {
    backend,
    tasks,
    loading,
    error,
    reload,
    createTask,
    updateTask,
    deleteTask,
    setEnabled,
    listRuns,
  };
}
