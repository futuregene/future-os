import { useCallback, useEffect, useState } from "react";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useTauriEvent } from "../../lib/useTauriEvent";

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
  conversationMode: string;
  triggerKind: string;
  trigger: Record<string, unknown>;
  depJoin: string;
  /** How many upstream dependencies this task waits on. */
  depCount: number;
  nextDueAt: number | null;
  /** An explicit request is waiting for the tick (the task was busy). */
  queued: boolean;
  reflection: string;
  /** Prompt suggestions awaiting a decision (0 when there are none). */
  pendingProposals: number;
  latestRun: TaskRunView | null;
}

export interface TaskInput {
  name: string;
  prompt: string;
  cwd: string;
  modelId?: string | null;
  thinkingLevel?: string | null;
  sessionPolicy?: string;
  conversationMode?: string;
  reflection?: string;
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

/** Thin client over the task Tauri commands (no local state). */
export function useTasks() {
  const [tasks, setTasks] = useState<TaskView[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const next = await invokeCommand<TaskView[]>("list_tasks");
      setTasks(next);
      setError(null);
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // A run changes the task's state (queued → running → finished) and can also
  // produce a conversation; the host already announces both with
  // "threads-updated". Without this the panel kept saying "queued" at a task
  // that had been running for minutes, until the user left and came back.
  useTauriEvent("threads-updated", () => {
    void reload();
  });

  const createTask = useCallback(async (input: TaskInput) => {
    const created = await invokeCommand<TaskView>("create_task", { input });
    await reload();
    return created;
  }, [reload]);

  const updateTask = useCallback(async (id: string, input: TaskInput) => {
    const updated = await invokeCommand<TaskView>("update_task", { id, input });
    await reload();
    return updated;
  }, [reload]);

  const deleteTask = useCallback(async (id: string) => {
    await invokeCommand<void>("delete_task", { id });
    await reload();
  }, [reload]);

  const setEnabled = useCallback(async (id: string, enabled: boolean) => {
    await invokeCommand<TaskView>("set_task_enabled", { id, enabled });
    await reload();
  }, [reload]);

  const runNow = useCallback(async (id: string) => {
    await invokeCommand<TaskView>("run_task_now", { id });
    await reload();
  }, [reload]);

  const listRuns = useCallback(
    (id: string, limit = 20) => invokeCommand<TaskRunView[]>("list_task_runs", { id, limit }),
    [],
  );

  const listDeps = useCallback(
    (id: string) => invokeCommand<TaskDepView[]>("list_task_deps", { id }),
    [],
  );

  const setDep = useCallback(
    (id: string, upstreamTaskId: string, on: string) =>
      invokeCommand<void>("set_task_dep", { id, upstreamTaskId, on }),
    [],
  );

  const removeDep = useCallback(
    (id: string, upstreamTaskId: string) =>
      invokeCommand<void>("remove_task_dep", { id, upstreamTaskId }),
    [],
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
    const current = await invokeCommand<TaskDepView[]>("list_task_deps", { id });
    const byUpstream = new Map(wanted.map(dep => [dep.upstreamTaskId, dep.on]));
    for (const dep of current) {
      const on = byUpstream.get(dep.upstreamTaskId);
      if (on === undefined)
        await invokeCommand<void>("remove_task_dep", { id, upstreamTaskId: dep.upstreamTaskId });
      else if (on !== dep.on)
        await invokeCommand<void>("set_task_dep", { id, upstreamTaskId: dep.upstreamTaskId, on });
      byUpstream.delete(dep.upstreamTaskId);
    }
    // What is left in the map is new: it was not among the stored edges.
    for (const [upstreamTaskId, on] of byUpstream)
      await invokeCommand<void>("set_task_dep", { id, upstreamTaskId, on });
    await reload();
  }, [reload]);

  const listRevisions = useCallback(
    (id: string) => invokeCommand<TaskRevisionView[]>("list_task_revisions", { id }),
    [],
  );

  const applyRevision = useCallback(async (id: string, revisionId: string) => {
    await invokeCommand<TaskView>("apply_task_revision", { id, revisionId });
    await reload();
  }, [reload]);

  return {
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
