import type { TaskBackend } from "./taskBackend";
import { useCallback, useEffect, useState } from "react";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { localTaskBackend, reconcileDeps } from "./taskBackend";

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
  sessionPolicy: string;
  /** `keep` (default) or `delete`; only meaningful with `sessionPolicy: "new"`. */
  sessionRetention: string;
  conversationMode: string;
  triggerKind: string;
  trigger: Record<string, unknown>;
  depJoin: string;
  /** How many upstream dependencies this task waits on. */
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
  sessionPolicy?: string;
  sessionRetention?: string;
  conversationMode?: string;
  triggerKind?: string;
  trigger?: Record<string, unknown>;
  depJoin?: string;
  enabled?: boolean;
}

export interface TaskDepView {
  upstreamTaskId: string;
  upstreamName: string;
  on: string;
  satisfied: boolean;
}

/** One dependency edge as the form holds it (upstream + condition). */
export interface TaskDepInput {
  upstreamTaskId: string;
  on: string;
}

export interface TaskRevisionView {
  id: string;
  version: number;
  prompt: string;
  source: string;
  status: string;
  reason: string | null;
  confidence: number | null;
  createdAt: number;
}

/**
 * The task list and the actions on it, over whichever backend it was given.
 *
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

  const runNow = useCallback(async (id: string) => {
    await backend.runNow(id);
    await reload();
  }, [backend, reload]);

  const listRuns = useCallback(
    (id: string, limit = 20) => backend.listRuns(id, limit),
    [backend],
  );

  const listDeps = useCallback(
    (id: string) => backend.listDeps(id),
    [backend],
  );

  const setDep = useCallback(
    (id: string, upstreamTaskId: string, on: string) =>
      backend.setDep(id, upstreamTaskId, on),
    [backend],
  );

  const removeDep = useCallback(
    (id: string, upstreamTaskId: string) =>
      backend.removeDep(id, upstreamTaskId),
    [backend],
  );

  /**
   * Make a task's dependency edges say exactly this, and nothing else.
   *
   * A form holds the whole set the user wants, so the write is a reconciliation
   * rather than add/remove calls poured out of the UI: an edge that is kept as
   * it was is not written again (rewriting it would be a no-op that still
   * touches the row), a changed condition is one call, and an edge the user
   * dropped is removed. The comparison runs against the store's current edges,
   * not the ones the form was opened with — a CLI or phone write in between must
   * not be silently reverted.
   */
  const saveDeps = useCallback(async (id: string, wanted: TaskDepInput[]) => {
    await reconcileDeps(backend, id, wanted);
    await reload();
  }, [backend, reload]);

  const listRevisions = useCallback(
    (id: string) => backend.listRevisions(id),
    [backend],
  );

  const applyRevision = useCallback(async (id: string, revisionId: string) => {
    await backend.applyRevision(id, revisionId);
    await reload();
  }, [backend, reload]);

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
    runNow,
    listRuns,
    listDeps,
    saveDeps,
    setDep,
    removeDep,
    listRevisions,
    applyRevision,
  };
}
