import { beforeEach, expect, it, vi } from "vitest";
import { remoteTaskBackend } from "./remoteTaskBackend";

/**
 * The task editor pointed at a host.
 *
 * Everything above this adapter is the local editor, so what has to be right
 * here is the adaptation: the host's command names and envelopes, and the two
 * differences that would corrupt something if they were got wrong — the list
 * omitting the prompt, and the run's conversation being named by a different id
 * than the task's.
 */

const request = vi.fn<(desktopId: string, command: Record<string, unknown>, lane: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: Parameters<typeof request>) => request(...args),
}));

/** The host's detail record, which is what the editor is built from. */
function detail(overrides: Record<string, unknown> = {}) {
  return {
    id: "task_1",
    name: "Nightly",
    enabled: true,
    prompt: "do the thing",
    promptVersion: 3,
    cwd: "/work",
    modelId: "deepseek/deepseek-chat",
    thinkingLevel: "medium",
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { every: "1h" },
    depJoin: "all",
    depCount: 0,
    nextDueAt: 1234,
    queued: false,
    latestRun: null,
    ...overrides,
  };
}

/** The host's list row: identity and trigger state, and no prompt. */
function row(overrides: Record<string, unknown> = {}) {
  return {
    id: "task_1",
    name: "Nightly",
    enabled: true,
    triggerKind: "schedule",
    trigger: { every: "1h" },
    depCount: 0,
    nextDueAt: 1234,
    queued: false,
    latestRun: null,
    ...overrides,
  };
}

beforeEach(() => {
  request.mockReset();
});

it("completes every row from the host's detail record", async () => {
  request.mockImplementation(async (_desktop, command) => {
    if (command.type === "list_tasks")
      return { tasks: [row()] };
    if (command.type === "get_task")
      return detail();
    return undefined;
  });

  const [task] = await remoteTaskBackend("desktop_a").list();

  // The prompt is not in the list, and the editor cannot open a task without it.
  expect(task!.prompt).toBe("do the thing");
  expect(task!.cwd).toBe("/work");
  expect(request).toHaveBeenCalledWith("desktop_a", { type: "get_task", taskId: "task_1" }, "list");
});

/**
 * A half-read list is not shown.
 *
 * If one detail read failed and the row were kept, the editor would offer that
 * task with an empty prompt — and "save" would erase the real one.
 */
it("fails the whole list rather than showing a task with no prompt", async () => {
  request.mockImplementation(async (_desktop, command) => {
    if (command.type === "list_tasks")
      return { tasks: [row({ id: "a" }), row({ id: "b" })] };
    if (command.type === "get_task")
      throw new Error("task not found");
    return undefined;
  });

  await expect(remoteTaskBackend("desktop_a").list()).rejects.toThrow("task not found");
});

/** The host's own command names and payloads, not this app's. */
it("writes through the host's command names", async () => {
  const backend = remoteTaskBackend("desktop_a");
  const input = {
    name: "Nightly",
    prompt: "do it",
    cwd: "/work",
    modelId: null,
    thinkingLevel: null,
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    enabled: true,
    depJoin: "all",
    triggerKind: "schedule",
    trigger: { every: "1h" },
  };

  request.mockResolvedValue(detail());
  await backend.create(input);
  expect(request).toHaveBeenCalledWith("desktop_a", {
    type: "create_task",
    task: expect.objectContaining({ name: "Nightly", prompt: "do it", triggerKind: "schedule" }),
  }, "list");

  request.mockReset();
  request.mockResolvedValue(undefined);
  await backend.remove("t1");
  await backend.setEnabled("t1", false);
  await backend.runNow("t1");
  await backend.setDep("t1", "up", "failure");
  await backend.removeDep("t1", "up");
  expect(request.mock.calls.map(([, command]) => command)).toEqual([
    { type: "delete_task", taskId: "t1" },
    { type: "set_task_enabled", taskId: "t1", enabled: false },
    // The host's name for "run it now" is `run_task`; `run_task_now` is this
    // app's own command and would be refused there.
    { type: "run_task", taskId: "t1" },
    { type: "set_task_dep", taskId: "t1", upstreamTaskId: "up", on: "failure" },
    { type: "remove_task_dep", taskId: "t1", upstreamTaskId: "up" },
  ]);
});

/** A cleared model or thinking level is `null` on the wire, not omitted. */
it("says null for a cleared choice rather than leaving the key out", async () => {
  request.mockResolvedValue(detail());
  await remoteTaskBackend("desktop_a").update("t1", {
    name: "n",
    prompt: "p",
    cwd: "/c",
    modelId: null,
    thinkingLevel: null,
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    enabled: true,
    depJoin: "all",
  });

  const [, command] = request.mock.calls[0]!;
  const task = (command as { task: Record<string, unknown> }).task;
  expect(task.modelId).toBeNull();
  expect(task.thinkingLevel).toBeNull();
  // An omitted key would mean "leave it", which is not what clearing is.
  expect("modelId" in task).toBe(true);
});

it("unwraps the host's envelopes for runs, dependencies and revisions", async () => {
  request.mockImplementation(async (_desktop, command) => {
    if (command.type === "list_task_runs") {
      return {
        runs: [
          {
            id: "r1",
            kind: "scheduled",
            origin: "tick",
            status: "succeeded",
            threadId: "th1",
            sessionId: "s1",
            startedAt: 10,
            finishedAt: 20,
            promptVersion: 2,
            resultSummary: "ok",
            errorMessage: null,
            sessionDeleted: true,
          },
          "not a run",
        ],
      };
    }
    if (command.type === "list_task_deps") {
      return { deps: [{ upstreamTaskId: "u1", upstreamName: "Build", on: "completed", satisfied: true }] };
    }
    if (command.type === "list_task_revisions") {
      return {
        revisions: [{
          id: "rev1",
          version: 4,
          source: "agent",
          status: "pending",
          reason: "why",
          confidence: 0.5,
          createdAt: 99,
          prompt: "the whole prompt",
          promptPreview: "the whole…",
        }],
      };
    }
    return undefined;
  });

  const backend = remoteTaskBackend("desktop_a");
  const runs = await backend.listRuns("t1", 5);
  expect(runs).toHaveLength(1);
  expect(runs[0]).toMatchObject({ id: "r1", sessionId: "s1", sessionDeleted: true, dueAt: null });
  // The host's `limit` is carried through, not dropped.
  expect(request).toHaveBeenCalledWith("desktop_a", { type: "list_task_runs", taskId: "t1", limit: 5 }, "list");

  expect(await backend.listDeps("t1")).toEqual([
    { upstreamTaskId: "u1", upstreamName: "Build", on: "completed", satisfied: true },
  ]);

  // The whole prompt is what the editor shows; the preview is for narrow rows.
  expect((await backend.listRevisions("t1"))[0]!.prompt).toBe("the whole prompt");
});

/** A revision whose prompt is missing falls back to the preview rather than blank. */
it("falls back to the revision preview when the host sends no prompt", async () => {
  request.mockResolvedValue({ revisions: [{ id: "r", version: 1, promptPreview: "preview only" }] });
  const [revision] = await remoteTaskBackend("desktop_a").listRevisions("t1");
  expect(revision!.prompt).toBe("preview only");
});

/** Malformed entries are dropped, not rendered as empty rows. */
it("drops entries the host sent in a shape it does not use", async () => {
  request.mockImplementation(async (_desktop, command) => {
    if (command.type === "list_task_runs")
      return { runs: ["nonsense", 7, { id: "ok" }] };
    if (command.type === "list_task_deps")
      return { deps: [null, { upstreamTaskId: "u" }] };
    if (command.type === "list_task_revisions")
      return { revisions: [42, { id: "rev", prompt: "p" }] };
    return undefined;
  });

  const backend = remoteTaskBackend("desktop_a");
  expect((await backend.listRuns("t1", 20)).map(run => run.id)).toEqual(["ok"]);
  expect(await backend.listDeps("t1")).toEqual([
    { upstreamTaskId: "u", upstreamName: "", on: "", satisfied: false },
  ]);
  expect((await backend.listRevisions("t1")).map(revision => revision.id)).toEqual(["rev"]);
});

/** A host that answers nothing for the list is empty, not a crash. */
it("reads an absent list as empty", async () => {
  request.mockResolvedValue(undefined);
  expect(await remoteTaskBackend("desktop_a").list()).toEqual([]);
});

it("applies a revision through the host's own write", async () => {
  request.mockResolvedValue(detail());
  await remoteTaskBackend("desktop_a").applyRevision("t1", "rev1");
  expect(request).toHaveBeenCalledWith(
    "desktop_a",
    { type: "apply_task_revision", taskId: "t1", revisionId: "rev1" },
    "list",
  );
});
