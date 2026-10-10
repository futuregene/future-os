import type { TaskBackend } from "../tasks/taskBackend";
import type { TaskInput, TaskRunView, TaskView } from "../tasks/useTasks";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * The task editor pointed at a paired host.
 *
 * The commands and their payloads are the host's, not this app's, so the names
 * (`run_task`, `taskId`) and the envelopes (`{ tasks: [...] }`) are adapted here
 * rather than leaking into the editor. That adaptation is the whole job of this
 * file: everything above it — the list, the form, the dependency reconciliation
 * — is the same screen the local tasks panel uses.
 */

/** The host's list row: identity and trigger state, and deliberately no prompt. */
interface RemoteTaskRow {
  id: string;
  name: string;
  enabled: boolean;
  triggerKind?: string;
  trigger?: Record<string, unknown>;
  depJoin?: string;
  depCount?: number;
  nextDueAt?: number | null;
  queued?: boolean;
  latestRun?: unknown;
}

/** The host's detail record, which is where the prompt crosses the wire. */
interface RemoteTaskDetail extends RemoteTaskRow {
  prompt?: string;
  promptVersion?: number;
  cwd?: string;
  modelId?: string | null;
  thinkingLevel?: string | null;
  sessionPolicy?: string;
  sessionRetention?: string;
  conversationMode?: string;
}

function runView(raw: unknown): TaskRunView | null {
  if (typeof raw !== "object" || raw === null)
    return null;
  const run = raw as Record<string, unknown>;
  const text = (key: string): string => (typeof run[key] === "string" ? run[key] as string : "");
  return {
    id: text("id"),
    kind: text("kind"),
    origin: text("origin"),
    status: text("status"),
    // The host's run summary does not carry a due time; the run itself is
    // already past its trigger, so there is nothing to show here.
    dueAt: typeof run.dueAt === "number" ? run.dueAt : null,
    threadId: typeof run.threadId === "string" ? run.threadId : null,
    sessionId: typeof run.sessionId === "string" ? run.sessionId : null,
    startedAt: typeof run.startedAt === "number" ? run.startedAt : null,
    finishedAt: typeof run.finishedAt === "number" ? run.finishedAt : null,
    promptVersion: typeof run.promptVersion === "number" ? run.promptVersion : null,
    resultSummary: typeof run.resultSummary === "string" ? run.resultSummary : null,
    errorMessage: typeof run.errorMessage === "string" ? run.errorMessage : null,
    sessionDeleted: run.sessionDeleted === true,
  };
}

function taskView(detail: RemoteTaskDetail): TaskView {
  return {
    id: detail.id,
    name: detail.name,
    enabled: detail.enabled,
    prompt: detail.prompt ?? "",
    promptVersion: detail.promptVersion ?? 0,
    cwd: detail.cwd ?? "",
    modelId: detail.modelId ?? null,
    thinkingLevel: detail.thinkingLevel ?? null,
    sessionPolicy: detail.sessionPolicy ?? "new",
    sessionRetention: detail.sessionRetention ?? "keep",
    conversationMode: detail.conversationMode ?? "workspace",
    triggerKind: detail.triggerKind ?? "manual",
    trigger: detail.trigger ?? {},
    depJoin: detail.depJoin ?? "all",
    depCount: detail.depCount ?? 0,
    nextDueAt: detail.nextDueAt ?? null,
    queued: detail.queued === true,
    latestRun: runView(detail.latestRun),
  };
}

/**
 * The payload `create_task` / `update_task` take on the host.
 *
 * `null` means "clear it" there, which is how the form's "default" choice is
 * expressed; an omitted key would mean "leave it as it was".
 */
function taskPayload(input: TaskInput): Record<string, unknown> {
  return {
    name: input.name,
    prompt: input.prompt,
    cwd: input.cwd,
    modelId: input.modelId ?? null,
    thinkingLevel: input.thinkingLevel ?? null,
    sessionPolicy: input.sessionPolicy,
    sessionRetention: input.sessionRetention,
    conversationMode: input.conversationMode,
    enabled: input.enabled,
    depJoin: input.depJoin,
    ...(input.triggerKind === undefined ? {} : { triggerKind: input.triggerKind }),
    ...(input.trigger === undefined ? {} : { trigger: input.trigger }),
  };
}

/** One command on one host. */
function send<T>(desktopId: string, command: Record<string, unknown>, lane = "list"): Promise<T> {
  return requestRemotePeer<T>(desktopId, command, lane);
}

export function remoteTaskBackend(desktopId: string): TaskBackend {
  return {
    kind: "remote",

    /**
     * The host's list omits the prompt, and the editor cannot open a task whose
     * prompt it does not have — showing an empty field would invite the user to
     * "save" an erased one. So every row is completed before the list is shown.
     *
     * This is one read per task, in parallel. It is safe to be eager here
     * because the reads happen once, when the panel is opened, and because a
     * failure has to be fatal rather than partial: a half-read list is a list
     * with an editable empty prompt in it.
     */
    async list() {
      const data = await send<{ tasks?: RemoteTaskRow[] }>(desktopId, { type: "list_tasks" });
      const rows = Array.isArray(data?.tasks) ? data.tasks : [];
      const details = await Promise.all(
        rows.map(row => send<RemoteTaskDetail>(desktopId, { type: "get_task", taskId: row.id })),
      );
      return details.map(taskView);
    },

    async create(input) {
      const detail = await send<RemoteTaskDetail>(
        desktopId,
        { type: "create_task", task: taskPayload(input) },
      );
      return taskView(detail);
    },

    async update(id, input) {
      const detail = await send<RemoteTaskDetail>(
        desktopId,
        { type: "update_task", taskId: id, task: taskPayload(input) },
      );
      return taskView(detail);
    },

    async remove(id) {
      await send(desktopId, { type: "delete_task", taskId: id });
    },

    async setEnabled(id, enabled) {
      await send(desktopId, { type: "set_task_enabled", taskId: id, enabled });
    },

    async runNow(id) {
      await send(desktopId, { type: "run_task", taskId: id });
    },

    async listRuns(id, limit) {
      const data = await send<{ runs?: unknown[] }>(
        desktopId,
        { type: "list_task_runs", taskId: id, limit },
      );
      return (Array.isArray(data?.runs) ? data.runs : [])
        .map(runView)
        .filter((run): run is TaskRunView => run !== null);
    },

    async listDeps(id) {
      const data = await send<{ deps?: unknown[] }>(desktopId, { type: "list_task_deps", taskId: id });
      return (Array.isArray(data?.deps) ? data.deps : []).flatMap((raw) => {
        if (typeof raw !== "object" || raw === null)
          return [];
        const dep = raw as Record<string, unknown>;
        return [{
          upstreamTaskId: typeof dep.upstreamTaskId === "string" ? dep.upstreamTaskId : "",
          upstreamName: typeof dep.upstreamName === "string" ? dep.upstreamName : "",
          on: typeof dep.on === "string" ? dep.on : "",
          satisfied: dep.satisfied === true,
        }];
      });
    },

    async setDep(id, upstreamTaskId, on) {
      await send(desktopId, { type: "set_task_dep", taskId: id, upstreamTaskId, on });
    },

    async removeDep(id, upstreamTaskId) {
      await send(desktopId, { type: "remove_task_dep", taskId: id, upstreamTaskId });
    },

    async listRevisions(id) {
      const data = await send<{ revisions?: unknown[] }>(
        desktopId,
        { type: "list_task_revisions", taskId: id },
      );
      return (Array.isArray(data?.revisions) ? data.revisions : []).flatMap((raw) => {
        if (typeof raw !== "object" || raw === null)
          return [];
        const revision = raw as Record<string, unknown>;
        const text = (key: string): string => (typeof revision[key] === "string" ? revision[key] as string : "");
        return [{
          id: text("id"),
          version: typeof revision.version === "number" ? revision.version : 0,
          // The host sends both the whole prompt and a preview; the editor shows
          // the prompt, and the preview is what a phone's narrow row needs.
          prompt: text("prompt") || text("promptPreview"),
          source: text("source"),
          status: text("status"),
          reason: typeof revision.reason === "string" ? revision.reason : null,
          confidence: typeof revision.confidence === "number" ? revision.confidence : null,
          createdAt: typeof revision.createdAt === "number" ? revision.createdAt : 0,
        }];
      });
    },

    async applyRevision(id, revisionId) {
      // The host answers the whole task, and it is the same implementation this
      // app's own command uses — accepting a suggestion is one write either way.
      await send(desktopId, { type: "apply_task_revision", taskId: id, revisionId });
    },
  };
}
