import type { TaskInput, TaskRunView, TaskView } from "./useTasks";
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
  listRuns: (id: string, limit: number) => Promise<TaskRunView[]>;
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
  listRuns(id, limit) {
    return invokeCommand<TaskRunView[]>("list_task_runs", { id, limit });
  },
};
