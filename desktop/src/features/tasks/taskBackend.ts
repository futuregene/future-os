import type {
  TaskDepInput,
  TaskDepView,
  TaskInput,
  TaskRevisionView,
  TaskRunView,
  TaskView,
} from "./useTasks";
import { invokeCommand } from "../../integrations/tauri/invoke";

/**
 * Where a task lives, so the task editor can be pointed at another computer.
 *
 * The editor is the same screen either way — only the errand underneath differs.
 * The local implementation calls this app's commands; the remote one speaks to a
 * paired host (see `remoteTaskBackend.ts`). Keeping this an interface rather than
 * a conditional inside `useTasks` is what lets one 1100-line editor serve both:
 * a branch in the hook would leave every call site wondering which machine it
 * was about to write to.
 */
export interface TaskBackend {
  /**
   * `local` for this app's own store. The editor's live refresh is driven by
   * this app's `threads-updated` event, which says nothing about a remote
   * host's tasks — a remote panel is refreshed by that host's own pushes.
   */
  kind: "local" | "remote";
  list: () => Promise<TaskView[]>;
  create: (input: TaskInput) => Promise<TaskView>;
  update: (id: string, input: TaskInput) => Promise<TaskView>;
  remove: (id: string) => Promise<void>;
  setEnabled: (id: string, enabled: boolean) => Promise<void>;
  runNow: (id: string) => Promise<void>;
  listRuns: (id: string, limit: number) => Promise<TaskRunView[]>;
  listDeps: (id: string) => Promise<TaskDepView[]>;
  setDep: (id: string, upstreamTaskId: string, on: string) => Promise<void>;
  removeDep: (id: string, upstreamTaskId: string) => Promise<void>;
  listRevisions: (id: string) => Promise<TaskRevisionView[]>;
  applyRevision: (id: string, revisionId: string) => Promise<void>;
}

/**
 * The lazy reconciliation both backends share: make a task's edges say exactly
 * `wanted` and nothing else.
 *
 * It compares against what is stored *now*, not what the form was opened with,
 * so an edit made elsewhere in between is not silently reverted.
 */
export async function reconcileDeps(
  backend: Pick<TaskBackend, "listDeps" | "setDep" | "removeDep">,
  id: string,
  wanted: TaskDepInput[],
): Promise<void> {
  const current = await backend.listDeps(id);
  const byUpstream = new Map(wanted.map(dep => [dep.upstreamTaskId, dep.on]));
  for (const dep of current) {
    const on = byUpstream.get(dep.upstreamTaskId);
    if (on === undefined)
      await backend.removeDep(id, dep.upstreamTaskId);
    else if (on !== dep.on)
      await backend.setDep(id, dep.upstreamTaskId, on);
    byUpstream.delete(dep.upstreamTaskId);
  }
  // What is left in the map is new: it was not among the stored edges.
  for (const [upstreamTaskId, on] of byUpstream)
    await backend.setDep(id, upstreamTaskId, on);
}

/** This app's own task store, through the Tauri commands. */
export const localTaskBackend: TaskBackend = {
  kind: "local",
  async list() {
    return invokeCommand<TaskView[]>("list_tasks");
  },
  async create(input) {
    return invokeCommand<TaskView>("create_task", { input });
  },
  async update(id, input) {
    return invokeCommand<TaskView>("update_task", { id, input });
  },
  async remove(id) {
    await invokeCommand<void>("delete_task", { id });
  },
  async setEnabled(id, enabled) {
    await invokeCommand<TaskView>("set_task_enabled", { id, enabled });
  },
  async runNow(id) {
    await invokeCommand<TaskView>("run_task_now", { id });
  },
  listRuns(id, limit) {
    return invokeCommand<TaskRunView[]>("list_task_runs", { id, limit });
  },
  listDeps(id) {
    return invokeCommand<TaskDepView[]>("list_task_deps", { id });
  },
  async setDep(id, upstreamTaskId, on) {
    await invokeCommand<void>("set_task_dep", { id, upstreamTaskId, on });
  },
  async removeDep(id, upstreamTaskId) {
    await invokeCommand<void>("remove_task_dep", { id, upstreamTaskId });
  },
  listRevisions(id) {
    return invokeCommand<TaskRevisionView[]>("list_task_revisions", { id });
  },
  async applyRevision(id, revisionId) {
    await invokeCommand<TaskView>("apply_task_revision", { id, revisionId });
  },
};
