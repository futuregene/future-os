import { useCallback, useEffect, useState } from "react";
import { invokeCommand } from "../../integrations/tauri/invoke";

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
  nextDueAt: number | null;
  /** An explicit request is waiting for the tick (the task was busy). */
  queued: boolean;
  reflection: string;
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
    setDep,
    removeDep,
    listRevisions,
    applyRevision,
  };
}
