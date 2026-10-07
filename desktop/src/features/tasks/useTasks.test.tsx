import type { TaskView } from "./useTasks";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useTasks } from "./useTasks";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({ invokeCommand: vi.fn() }));

vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invokeCommand }));

function task(overrides: Partial<TaskView> = {}): TaskView {
  return {
    id: "tsk_1",
    name: "daily",
    enabled: true,
    prompt: "summarize",
    promptVersion: 1,
    cwd: "/tmp",
    modelId: null,
    thinkingLevel: null,
    sessionPolicy: "new",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "daily", time: "09:00" },
    depJoin: "all",
    nextDueAt: 1_000,
    queued: false,
    reflection: "ask",
    latestRun: null,
    ...overrides,
  };
}

/** Mount the hook and expose its API plus the latest render's snapshot. */
async function renderHook() {
  const seen: { tasks: TaskView[]; error: string | null; loading: boolean } = {
    tasks: [],
    error: null,
    loading: false,
  };
  let api: ReturnType<typeof useTasks> | null = null;
  const container = document.createElement("div");
  const root = createRoot(container);
  function Probe() {
    const value = useTasks();
    api = value;
    seen.tasks = value.tasks;
    seen.error = value.error;
    seen.loading = value.loading;
    return null;
  }
  await act(async () => {
    root.render(<Probe />);
  });
  return {
    seen,
    api: () => api!,
    unmount: () => act(() => root.unmount()),
  };
}

beforeEach(() => {
  mocks.invokeCommand.mockReset();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("useTasks", () => {
  it("loads the list on mount", async () => {
    mocks.invokeCommand.mockResolvedValueOnce([task()]);
    const hook = await renderHook();
    expect(mocks.invokeCommand).toHaveBeenCalledWith("list_tasks");
    expect(hook.seen.tasks.map(t => t.id)).toEqual(["tsk_1"]);
    expect(hook.seen.error).toBeNull();
    hook.unmount();
  });

  it("surfaces a load failure as a message instead of throwing", async () => {
    mocks.invokeCommand.mockRejectedValueOnce(new Error("boom"));
    const hook = await renderHook();
    expect(hook.seen.error).toBe("boom");
    expect(hook.seen.tasks).toEqual([]);
    expect(hook.seen.loading).toBe(false);
    hook.unmount();
  });

  it("stringifies a non-Error rejection", async () => {
    mocks.invokeCommand.mockRejectedValueOnce("plain failure");
    const hook = await renderHook();
    expect(hook.seen.error).toBe("plain failure");
    hook.unmount();
  });

  it("reloads after every mutation so the list reflects the backend", async () => {
    mocks.invokeCommand.mockResolvedValue([]);
    const hook = await renderHook();
    mocks.invokeCommand.mockClear();

    mocks.invokeCommand.mockResolvedValueOnce(task());
    await act(async () => {
      await hook.api().createTask({ name: "daily", prompt: "p", cwd: "/tmp" });
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(1, "create_task", {
      input: { name: "daily", prompt: "p", cwd: "/tmp" },
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(2, "list_tasks");

    mocks.invokeCommand.mockClear();
    mocks.invokeCommand.mockResolvedValueOnce(task());
    await act(async () => {
      await hook.api().updateTask("tsk_1", { name: "x", prompt: "p", cwd: "/tmp" });
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(1, "update_task", {
      id: "tsk_1",
      input: { name: "x", prompt: "p", cwd: "/tmp" },
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(2, "list_tasks");

    mocks.invokeCommand.mockClear();
    mocks.invokeCommand.mockResolvedValue(undefined);
    await act(async () => {
      await hook.api().deleteTask("tsk_1");
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(1, "delete_task", { id: "tsk_1" });

    mocks.invokeCommand.mockClear();
    mocks.invokeCommand.mockResolvedValue(task({ enabled: false }));
    await act(async () => {
      await hook.api().setEnabled("tsk_1", false);
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(1, "set_task_enabled", {
      id: "tsk_1",
      enabled: false,
    });

    mocks.invokeCommand.mockClear();
    mocks.invokeCommand.mockResolvedValue(task());
    await act(async () => {
      await hook.api().runNow("tsk_1");
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(1, "run_task_now", { id: "tsk_1" });

    mocks.invokeCommand.mockClear();
    mocks.invokeCommand.mockResolvedValue(task({ prompt: "p2", promptVersion: 2 }));
    await act(async () => {
      await hook.api().applyRevision("tsk_1", "rev_1");
    });
    expect(mocks.invokeCommand).toHaveBeenNthCalledWith(1, "apply_task_revision", {
      id: "tsk_1",
      revisionId: "rev_1",
    });
    hook.unmount();
  });

  it("passes the read commands straight through", async () => {
    mocks.invokeCommand.mockResolvedValue([]);
    const hook = await renderHook();
    mocks.invokeCommand.mockClear();

    await hook.api().listRuns("tsk_1");
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("list_task_runs", { id: "tsk_1", limit: 20 });
    await hook.api().listRuns("tsk_1", 5);
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("list_task_runs", { id: "tsk_1", limit: 5 });
    await hook.api().listDeps("tsk_1");
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("list_task_deps", { id: "tsk_1" });
    await hook.api().setDep("tsk_1", "tsk_up", "failure");
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("set_task_dep", {
      id: "tsk_1",
      upstreamTaskId: "tsk_up",
      on: "failure",
    });
    await hook.api().removeDep("tsk_1", "tsk_up");
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("remove_task_dep", {
      id: "tsk_1",
      upstreamTaskId: "tsk_up",
    });
    await hook.api().listRevisions("tsk_1");
    expect(mocks.invokeCommand).toHaveBeenLastCalledWith("list_task_revisions", { id: "tsk_1" });
    hook.unmount();
  });
});
