import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { TaskRunView, TaskView } from "./useTasks";
// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { TasksView } from "./TasksView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  invokeCommand: vi.fn(),
  openDialog: vi.fn(),
  tauriEvents: {} as Record<string, Array<(payload: unknown) => void>>,
}));

vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invokeCommand }));
// The host announces task-list and run changes with "threads-updated". The
// double mirrors `lib/useTauriEvent` itself: one subscription per mount (in an
// effect, with the latest handler behind a ref) — registering per render would
// fire one reload per render and let a stale answer win.
vi.mock("../../lib/useTauriEvent", async () => {
  const { useEffect, useRef } = await import("react");
  return {
    useTauriEvent: (name: string, handler: (payload: unknown) => void) => {
      const handlerRef = useRef(handler);
      handlerRef.current = handler;
      useEffect(() => {
        const listener = (payload: unknown) => handlerRef.current(payload);
        (mocks.tauriEvents[name] ??= []).push(listener);
        return () => {
          mocks.tauriEvents[name] = (mocks.tauriEvents[name] ?? []).filter(item => item !== listener);
        };
      }, [name]);
    },
  };
});
// The working-directory picker opens the OS directory chooser; the fake stands
// in for the native tree so the field's contract can be asserted.
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.openDialog }));

/** The enabled-model list the view is handed (Settings → Models decides it). */
const MODELS: AgentModelOption[] = [{ id: "gpt-5", label: "GPT-5", provider: "future" }];

function task(overrides: Partial<TaskView> = {}): TaskView {
  return {
    id: "tsk_1",
    name: "daily report",
    enabled: true,
    prompt: "summarize yesterday",
    promptVersion: 3,
    cwd: "/tmp/repo",
    modelId: "future/gpt-5",
    thinkingLevel: "high",
    sessionPolicy: "new",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "daily", time: "09:00" },
    depJoin: "all",
    depCount: 0,
    nextDueAt: 1_700_000_000_000,
    queued: false,
    reflection: "ask",
    pendingProposals: 0,
    latestRun: null,
    ...overrides,
  };
}

function run(overrides: Partial<TaskRunView> = {}): TaskRunView {
  return {
    id: "trn_1",
    kind: "main",
    origin: "schedule",
    status: "completed",
    dueAt: 1,
    threadId: "thr_1",
    sessionId: "sess_1",
    startedAt: 1_700_000_000_000,
    finishedAt: 1_700_000_060_000,
    promptVersion: 1,
    resultSummary: "all good",
    errorMessage: null,
    ...overrides,
  };
}

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  void i18n.changeLanguage("en");
  mocks.invokeCommand.mockReset();
  mocks.openDialog.mockReset();
  mocks.tauriEvents = {};
});

/**
 * Fire a host event at every listener the mounted view registered, and let the
 * reload it triggers land (the handlers start async reads they do not await).
 */
async function fireHostEvent(name: string) {
  await act(async () => {
    for (const handler of mocks.tauriEvents[name] ?? []) {
      handler({});
    }
    await new Promise(resolve => setTimeout(resolve, 0));
  });
}

afterEach(() => {
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
});

/**
 * Route each command to a canned answer. `list_tasks` is served from a mutable
 * array so a mutation can be observed the way the real backend would report it.
 */
function backend(tasks: TaskView[], runs: TaskRunView[] = [run()], revisions: unknown[] = [defaultRevision()]) {
  mocks.invokeCommand.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => backendFor(tasks, command, args, runs, revisions),
  );
}

/** The canned answers of the read commands, so a test can wrap them. */
function backendFor(
  tasks: TaskView[],
  command: string,
  args?: Record<string, unknown>,
  runs: TaskRunView[] = [run()],
  revisions: unknown[] = [defaultRevision()],
) {
  switch (command) {
    case "list_tasks":
      return tasks;
    // A write answers with the stored task (the shape the backend returns), so
    // a save that creates one can go on to write its dependency edges.
    case "create_task":
    case "update_task":
      return tasks[0];
    case "list_task_runs":
      return args?.id === "tsk_1" ? runs : [];
    case "list_task_deps":
      return args?.id === "tsk_1"
        ? [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: false }]
        : [];
    case "list_task_revisions":
      return args?.id === "tsk_1" ? revisions : [];
    default:
      return undefined;
  }
}

/** A second live task, so a dependency has somewhere to point. */
function upstreamTask(overrides: Partial<TaskView> = {}): TaskView {
  return task({ id: "tsk_up2", name: "upstream two", depJoin: "all", ...overrides });
}

/** The `[command, args]` pairs a mock recorded, in order. */
function calls() {
  return mocks.invokeCommand.mock.calls.map(([command, args]) => ({
    command: command as string,
    args: args as Record<string, unknown> | undefined,
  }));
}

/** A stored prompt version with its source. */
function revision(overrides: Record<string, unknown> = {}) {
  return {
    id: "rev_2",
    version: 2,
    prompt: "newer prompt",
    source: "reflection",
    status: "active",
    reason: "shorter",
    confidence: 0.8,
    createdAt: 2,
    ...overrides,
  };
}

function defaultRevision() {
  return revision();
}

async function renderView(
  tasks: TaskView[] = [task()],
  onOpenThread = vi.fn(),
  modelOptions: AgentModelOption[] = MODELS,
  revisions: unknown[] = [defaultRevision()],
) {
  backend(tasks, [run()], revisions);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(
      <TasksView
        leftPanelExpanded
        modelOptions={modelOptions}
        onToggleLeftPanel={() => {}}
        onOpenThread={onOpenThread}
      />,
    );
  });
  return { container, onOpenThread };
}

function buttonByText(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

function rows(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLButtonElement>("div.w-72 > button")];
}

/** The control under the labelled field, e.g. the input after the "Name" span. */
function field(container: HTMLElement, label: string) {
  const span = [...container.querySelectorAll("span")]
    .find(node => (node.textContent ?? "").trim() === label);
  if (!span)
    throw new Error(`no label ${label}`);
  const control = span.parentElement!.querySelector<HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement>("input, textarea, select");
  if (!control)
    throw new Error(`no control after ${label}`);
  return control;
}

/** The trigger-mode select lives in the form's fieldset, not the option selects. */
function triggerModeSelect(container: HTMLElement) {
  const select = container.querySelector<HTMLSelectElement>("fieldset select");
  if (!select)
    throw new Error(`no trigger select; body=${container.innerHTML.slice(0, 400)}`);
  return select;
}

/** A control by its aria-label (the dependency rows carry names in theirs). */
function labelled(container: HTMLElement, label: string) {
  const control = container.querySelector<HTMLInputElement | HTMLSelectElement>(
    `[aria-label="${label}"]`,
  );
  if (!control)
    throw new Error(`no control labelled ${label}`);
  return control;
}

/**
 * The dependency editor, when it is on screen: it lives inside the trigger
 * fieldset and appears with the `dependency` trigger mode (or when the task
 * already has upstreams).
 */
function dependencyFieldset(container: HTMLElement): HTMLElement | undefined {
  const picker = container.querySelector("[aria-label=\"Add an upstream task\"]");
  return picker?.closest("fieldset") ?? undefined;
}

async function click(button: HTMLButtonElement | undefined) {
  expect(button, "button must exist").toBeDefined();
  await act(async () => {
    button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

/** Set a controlled input/textarea/select through React's own value setter. */
async function setValue(
  control: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement,
  value: string,
) {
  const proto = control instanceof HTMLTextAreaElement
    ? HTMLTextAreaElement.prototype
    : control instanceof HTMLSelectElement
      ? HTMLSelectElement.prototype
      : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(control, value);
    control.dispatchEvent(new Event(control instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}

/**
 * The model and the thinking level are the user's explicit decisions, so every
 * save that is meant to succeed has to make them first.
 */
async function chooseModelAndThinking(container: HTMLElement, model = "future/gpt-5", thinking = "high") {
  await setValue(field(container, "Model") as HTMLSelectElement, model);
  await setValue(field(container, "Thinking level") as HTMLSelectElement, thinking);
}

describe("tasksView", () => {
  it("lists tasks with their trigger summary and state", async () => {
    const { container } = await renderView();
    const labels = rows(container).map(row => row.textContent ?? "");
    expect(labels).toHaveLength(1);
    expect(labels[0]).toContain("daily report");
    expect(labels[0]).toContain("Daily");
  });

  it("says so when there are no tasks", async () => {
    const { container } = await renderView([]);
    expect(container.textContent).toContain("No tasks yet");
  });

  // Pressing "run now" while a run is in flight leaves the request queued for
  // the tick. The row has to say so: otherwise the button looks like it did
  // nothing until the in-flight run ends.
  it("shows a queued request instead of the last run's status", async () => {
    const { container } = await renderView([
      task({ queued: true, latestRun: run({ status: "running" }) }),
    ]);
    const row = rows(container)[0]?.textContent ?? "";
    expect(row).toContain("queued");
    expect(row).not.toContain("running");
  });

  it("shows the last run's status when nothing is waiting", async () => {
    const { container } = await renderView([
      task({ queued: false, latestRun: run({ status: "completed" }) }),
    ]);
    expect(rows(container)[0]?.textContent ?? "").toContain("done");
  });

  it("shows the load failure", async () => {
    mocks.invokeCommand.mockImplementation(async (command: string) => {
      if (command === "list_tasks")
        throw new Error("store unreadable");
      return undefined;
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    roots.push({ container, root });
    await act(async () => {
      root.render(<TasksView leftPanelExpanded modelOptions={MODELS} onToggleLeftPanel={() => {}} onOpenThread={vi.fn()} />);
    });
    expect(container.textContent).toContain("store unreadable");
  });

  it("opens a task's detail with runs, dependencies and prompt versions", async () => {
    const { container, onOpenThread } = await renderView();
    await click(rows(container)[0]);

    expect(container.textContent).toContain("summarize yesterday");
    // Dependencies show as waiting until the upstream run lands.
    expect(container.textContent).toContain("upstream");
    expect(container.textContent).toContain("waiting");
    // The run ledger shows the result summary and links to its conversation.
    expect(container.textContent).toContain("all good");
    await click(buttonByText(container, "Open"));
    expect(onOpenThread).toHaveBeenCalledWith("thr_1");
    // Applied prompt versions are listed with their source.
    expect(container.textContent).toContain("v2");
    expect(container.textContent).toContain("Reflection");
  });

  it("shows the model, thinking level and working directory of a task", async () => {
    const { container } = await renderView([task({ modelId: "future/gpt-5", thinkingLevel: "high" })]);
    await click(rows(container)[0]);
    const text = container.textContent ?? "";
    // A detail page that omits these cannot answer "what will this run on?".
    expect(text).toContain("Model");
    expect(text).toContain("future/gpt-5");
    expect(text).toContain("Thinking");
    expect(text).toContain("High");
    expect(text).toContain("Working directory");
    expect(text).toContain("/tmp/repo");
  });

  // A task can be created from the editor with the upstream it waits on. The
  // edges are written once the task exists (an edge needs both ids).
  it("creates a task that depends on another task", async () => {
    const { container } = await renderView([task(), upstreamTask()]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "downstream");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);

    // The dependency editor belongs to the trigger choice: it is not there
    // while the task runs on a schedule or on request.
    expect(dependencyFieldset(container)).toBeUndefined();
    await setValue(triggerModeSelect(container), "dependency");
    const fieldset = dependencyFieldset(container)!;

    const picker = labelled(fieldset, "Add an upstream task") as HTMLSelectElement;
    expect([...picker.options].map(option => option.textContent)).toEqual([
      "Pick an upstream task",
      "daily report",
      "upstream two",
    ]);
    await setValue(picker, "tsk_up2");
    await click(buttonByText(fieldset, "Add"));
    expect(dependencyFieldset(container)!.textContent).toContain("upstream two");

    mocks.invokeCommand.mockClear();
    backend([task(), upstreamTask()]);
    await click(buttonByText(container, "Save"));

    const sent = calls();
    expect(sent.find(call => call.command === "create_task")?.args).toMatchObject({
      input: { depJoin: "all" },
    });
    expect(sent).toContainEqual({
      command: "set_task_dep",
      args: { id: "tsk_1", upstreamTaskId: "tsk_up2", on: "success" },
    });
  });

  it("edits and removes a task's dependencies", async () => {
    const { container } = await renderView([
      task({ triggerKind: "manual", trigger: {}, nextDueAt: null, depJoin: "all", depCount: 1 }),
      upstreamTask(),
      task({ id: "tsk_up3", name: "upstream three", depJoin: "all" }),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));

    // A task started by its upstreams opens on the dependency trigger, with the
    // stored edge and its condition.
    expect((triggerModeSelect(container) as HTMLSelectElement).value).toBe("dependency");
    const fieldset = dependencyFieldset(container)!;
    expect(fieldset.textContent).toContain("upstream");
    const condition = labelled(fieldset, "When upstream finishes") as HTMLSelectElement;
    expect(condition.value).toBe("success");
    await setValue(condition, "failure");

    // A second upstream makes the join policy meaningful, so it appears.
    await setValue(labelled(fieldset, "Add an upstream task") as HTMLSelectElement, "tsk_up3");
    await click(buttonByText(fieldset, "Add"));
    const join = labelled(dependencyFieldset(container)!, "With several upstreams, run") as HTMLSelectElement;
    await setValue(join, "any");

    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));

    const sent = calls();
    expect(sent).toContainEqual({
      command: "set_task_dep",
      args: { id: "tsk_1", upstreamTaskId: "tsk_up", on: "failure" },
    });
    expect(sent).toContainEqual({
      command: "set_task_dep",
      args: { id: "tsk_1", upstreamTaskId: "tsk_up3", on: "success" },
    });
    expect(sent.find(call => call.command === "update_task")?.args).toMatchObject({
      input: { depJoin: "any" },
    });

    // Dropping the stored edge removes it rather than leaving it behind.
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    await click(buttonByText(dependencyFieldset(container)!, "Remove"));
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    expect(calls()).toContainEqual({
      command: "remove_task_dep",
      args: { id: "tsk_1", upstreamTaskId: "tsk_up" },
    });
  });

  // A refused edge (the backend sees a cycle the editor cannot) must not make
  // the user create the task again: the task is stored, so the editor stays on
  // update.
  it("keeps a created task when its dependency is refused", async () => {
    const { container } = await renderView([task(), upstreamTask()]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "downstream");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(triggerModeSelect(container), "dependency");
    await setValue(labelled(dependencyFieldset(container)!, "Add an upstream task") as HTMLSelectElement, "tsk_up2");
    await click(buttonByText(dependencyFieldset(container)!, "Add"));

    mocks.invokeCommand.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "create_task")
        return task({ id: "tsk_new", name: "downstream" });
      if (command === "set_task_dep")
        throw new Error("dependency cycle detected");
      return backendFor([task(), upstreamTask()], command, args);
    });
    await click(buttonByText(container, "Save"));

    expect(container.textContent).toContain("dependency cycle detected");
    // Still in the editor, now editing the task that was created: saving again
    // updates it instead of creating a second one.
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const commands = calls().map(call => call.command);
    expect(commands).toContain("update_task");
    expect(commands).not.toContain("create_task");
  });

  it("offers no upstream candidates when this is the only task", async () => {
    const { container } = await renderView([task({ depCount: 1 })]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    const fieldset = dependencyFieldset(container)!;
    expect(fieldset.textContent).toContain("no other task to depend on");
    expect([...((labelled(fieldset, "Add an upstream task") as HTMLSelectElement).options)]).toHaveLength(1);
  });

  // The two trigger kinds are orthogonal in the store (a CLI task can have
  // both), so a task with a schedule *and* upstreams keeps its edges on screen:
  // an edge nobody can see is an edge nobody can remove.
  it("keeps a schedule's upstreams visible, and says the task runs for both", async () => {
    const { container } = await renderView([
      task({ depJoin: "all", depCount: 1 }),
      upstreamTask(),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    await setValue(triggerModeSelect(container), "daily");

    const fieldset = dependencyFieldset(container)!;
    expect(fieldset.textContent).toContain("also has its own schedule");
    expect(fieldset.textContent).toContain("upstream");
  });

  // Switching the trigger to a schedule leaves the edges in the draft (they are
  // written on save), so choosing a schedule does not silently delete them.
  it("does not drop the upstreams when the trigger changes to a schedule", async () => {
    const { container } = await renderView([task({ depCount: 1 }), upstreamTask()]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    await setValue(triggerModeSelect(container), "daily");
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));

    const sent = calls();
    expect(sent.find(call => call.command === "update_task")?.args).toMatchObject({
      input: { triggerKind: "schedule" },
    });
    // The stored edge is kept, and not rewritten.
    expect(sent.some(call => call.command === "remove_task_dep")).toBe(false);
    expect(sent.some(call => call.command === "set_task_dep")).toBe(false);
  });

  it("falls back to the defaults when a task pins neither model nor thinking level", async () => {
    const { container } = await renderView([task({ modelId: null, thinkingLevel: null })]);
    await click(rows(container)[0]);
    const text = container.textContent ?? "";
    expect(text).toContain("Default model");
    expect(text).toContain("Default");
  });

  it("says a chat task runs in its own workspace rather than an empty directory row", async () => {
    const { container } = await renderView([task({ conversationMode: "chat", cwd: "" })]);
    await click(rows(container)[0]);
    expect(container.textContent).toContain("own temporary workspace");
  });

  it("picks the working directory through the directory chooser", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    mocks.openDialog.mockResolvedValueOnce("/Users/me/finally-a-real-folder");
    await click(buttonByText(container, "Browse"));
    // The chooser is asked for a directory, and its answer lands in the field.
    expect(mocks.openDialog).toHaveBeenCalledWith(expect.objectContaining({ directory: true }));
    expect(field(container, "Working directory").value).toBe("/Users/me/finally-a-real-folder");
  });

  it("ignores a cancelled directory pick and reports one that cannot open", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    // Cancelling answers null; the path the user already has must survive.
    mocks.openDialog.mockResolvedValueOnce(null);
    await click(buttonByText(container, "Browse"));
    expect(field(container, "Working directory").value).toBe("/tmp/repo");

    mocks.openDialog.mockRejectedValueOnce(new Error("no picker here"));
    await click(buttonByText(container, "Browse"));
    expect((container.textContent ?? "")).toContain("no picker here");
  });

  it("re-reads the list and the open detail when the host announces a change", async () => {
    const { container } = await renderView([task({ queued: true })]);
    await click(rows(container)[0]);
    expect(container.textContent).toContain("queued");

    // The run started (or finished) elsewhere: the status and the ledger have
    // moved on, and the panel has to say so without being reopened.
    backend(
      [task({ queued: false, latestRun: run({ status: "completed", resultSummary: "landed" }) })],
      [run({ status: "completed", resultSummary: "landed" })],
    );
    mocks.invokeCommand.mockClear();
    await fireHostEvent("threads-updated");
    expect(mocks.invokeCommand).toHaveBeenCalledWith("list_tasks");
    expect(mocks.invokeCommand).toHaveBeenCalledWith("list_task_runs", { id: "tsk_1", limit: 20 });
    expect(rows(container)[0]?.textContent ?? "").toContain("done");
    expect(container.textContent).toContain("landed");
  });

  it("runs, toggles and deletes through the backend", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    mocks.invokeCommand.mockClear();

    await click(buttonByText(container, "Run now"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("run_task_now", { id: "tsk_1" });

    await click(buttonByText(container, "Disable"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("set_task_enabled", { id: "tsk_1", enabled: false });

    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    await click(container.querySelector<HTMLButtonElement>("button[aria-label='Delete task']")!);
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("daily report"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("delete_task", { id: "tsk_1" });

    // A declined confirmation must not delete anything.
    mocks.invokeCommand.mockClear();
    confirm.mockReturnValue(false);
    await click(container.querySelector<HTMLButtonElement>("button[aria-label='Delete task']")!);
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("delete_task", { id: "tsk_1" });
  });

  it("applies a stored prompt version", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    mocks.invokeCommand.mockClear();
    await click(buttonByText(container, "Apply"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("apply_task_revision", {
      id: "tsk_1",
      revisionId: "rev_2",
    });
  });

  // A suggestion is the one thing on this page the user has to act on, so it is
  // labelled as a suggestion (not a version), it shows the confidence the pass
  // reported, and the row offers a button that says what it does.
  it("shows a prompt suggestion with its confidence and how to accept it", async () => {
    const { container } = await renderView(
      [task({ pendingProposals: 1 })],
      vi.fn(),
      MODELS,
      [
        revision({
          id: "rev_suggestion",
          version: 0,
          status: "proposed",
          source: "reflection",
          reason: "the output path was not stated",
          confidence: 0.82,
        }),
        revision({ id: "rev_1", version: 1, status: "active", source: "user", confidence: null }),
      ],
    );

    // The list row carries a count, so a suggestion is visible without opening
    // the task.
    expect(rows(container)[0]?.textContent ?? "").toContain("1 suggestion");

    await click(rows(container)[0]);
    const text = container.textContent ?? "";
    expect(text).toContain("Suggestion");
    expect(text).toContain("confidence 82%");
    expect(text).toContain("the output path was not stated");
    expect(text).not.toContain("v0");

    mocks.invokeCommand.mockClear();
    await click(buttonByText(container, "Use suggestion"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("apply_task_revision", {
      id: "tsk_1",
      revisionId: "rev_suggestion",
    });
  });

  it("marks an accepted suggestion as applied", async () => {
    const { container } = await renderView(
      [task()],
      vi.fn(),
      MODELS,
      [revision({ id: "rev_suggestion", version: 0, status: "applied", confidence: 0.9 })],
    );
    await click(rows(container)[0]);
    const text = container.textContent ?? "";
    // It is no longer a version, and it is no longer pending: both facts are on
    // the row, so the history says what happened to it.
    expect(text).toContain("Applied");
    expect(buttonByText(container, "Use suggestion")).toBeUndefined();
  });

  it("creates a chat task that needs no directory", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    mocks.invokeCommand.mockClear();

    // Saving an incomplete form reports what is missing instead of calling out.
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("required");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    await setValue(field(container, "Name") as HTMLInputElement, "my task");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "do the thing");
    // No working directory is asked for: a chat conversation carries its own.
    expect(container.querySelector("input[aria-label='Working directory']")).toBeNull();
    await chooseModelAndThinking(container);
    await click(buttonByText(container, "Save"));

    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: {
        name: "my task",
        prompt: "do the thing",
        cwd: "",
        conversationMode: "chat",
        triggerKind: "manual",
        sessionPolicy: "new",
        reflection: "ask",
      },
    });
    expect(created?.[1]).toMatchObject({ input: { modelId: "future/gpt-5", thinkingLevel: "high" } });
  });

  it("refuses a save without a model or a thinking level", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "unconfigured");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    mocks.invokeCommand.mockClear();

    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("Choose a model first");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    // The pickers open on "not chosen yet" rather than an inherited default.
    expect((field(container, "Model") as HTMLSelectElement).value).toBe("");
    expect((field(container, "Thinking level") as HTMLSelectElement).value).toBe("");

    await chooseModelAndThinking(container, "future/gpt-5", "medium");
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: { modelId: "future/gpt-5", thinkingLevel: "medium" },
    });
  });

  it("asks for a directory only for a workspace conversation", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "filed");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(field(container, "Conversation type") as HTMLSelectElement, "workspace");

    mocks.invokeCommand.mockClear();
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("needs a working directory");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp/filed");
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: { conversationMode: "workspace", cwd: "/tmp/filed" },
    });
  });

  it("says so when no model is enabled", async () => {
    const { container } = await renderView([], vi.fn(), []);
    await click(buttonByText(container, "New task"));
    expect(container.textContent).toContain("No models are enabled");
    expect(container.querySelectorAll("select")).not.toHaveLength(0);
  });

  it("edits a task and reports a save failure", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    expect(container.textContent).toContain("summarize yesterday");
    mocks.invokeCommand.mockImplementation(async (command: string) => {
      if (command === "update_task")
        throw new Error("name already taken");
      if (command === "list_tasks")
        return [task()];
      return undefined;
    });
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("name already taken");
  });

  it("ignores a model the user has since disabled only as far as the list goes", async () => {
    // The task points at a model that is no longer enabled. It stays selected
    // and selectable, so editing another field cannot silently rewrite it.
    const { container } = await renderView([task({ modelId: "future/old-model" })]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    expect((field(container, "Model") as HTMLSelectElement).value).toBe("future/old-model");
    mocks.invokeCommand.mockClear();
    backend([task({ modelId: "future/old-model" })]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(call?.[1]).toMatchObject({ input: { modelId: "future/old-model" } });
  });

  it("only sends the fields that belong to the selected trigger", async () => {
    /** Fill the editor for one trigger shape and report what was submitted. */
    async function submit(
      mode: string,
      extra: (container: HTMLElement) => Promise<void> = async () => {},
    ) {
      const { container } = await renderView([]);
      await click(buttonByText(container, "New task"));
      await setValue(field(container, "Name") as HTMLInputElement, "scheduled");
      await setValue(field(container, "Prompt") as HTMLTextAreaElement, "prompt");
      await chooseModelAndThinking(container);
      await setValue(triggerModeSelect(container), mode);
      await extra(container);
      mocks.invokeCommand.mockClear();
      backend([task()]);
      await click(buttonByText(container, "Save"));
      const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
      return (call?.[1] as { input: { triggerKind: string; trigger: Record<string, unknown> } }).input;
    }

    expect(
      await submit("once", async (container) => {
        await setValue(field(container, "Date") as HTMLInputElement, "2026-12-24");
        await setValue(field(container, "Time") as HTMLInputElement, "18:00");
      }),
    ).toMatchObject({
      triggerKind: "schedule",
      trigger: { mode: "once", date: "2026-12-24", time: "18:00" },
    });

    expect(
      await submit("interval", async (container) => {
        await setValue(field(container, "Minutes between runs") as HTMLInputElement, "45");
      }),
    ).toMatchObject({
      triggerKind: "schedule",
      trigger: { mode: "interval", every_minutes: 45 },
    });

    expect(await submit("daily")).toMatchObject({
      triggerKind: "schedule",
      trigger: { mode: "daily", time: "09:00" },
    });

    expect(await submit("weekly")).toMatchObject({
      triggerKind: "schedule",
      trigger: { mode: "weekly", days: ["mon"], time: "09:00" },
    });

    expect(
      await submit("monthly", async (container) => {
        await setValue(field(container, "Day of month") as HTMLInputElement, "31");
      }),
    ).toMatchObject({
      triggerKind: "schedule",
      trigger: { mode: "monthly", day: 31, time: "09:00" },
    });

    expect(await submit("manual")).toMatchObject({ triggerKind: "manual", trigger: {} });
  });

  it("shows the short-month rule and the full-permission warning while editing", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(triggerModeSelect(container), "monthly");
    expect(container.textContent).toContain("last day of the month");
    expect(container.textContent).toContain("full permission");
  });

  it("adds and removes weekdays on a weekly trigger", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(triggerModeSelect(container), "weekly");

    const boxes = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    // The first checkbox is "Mon" (already selected); the second is "Tue".
    await act(async () => {
      boxes[1]!.click();
    });
    await act(async () => {
      boxes[0]!.click();
    });
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await setValue(field(container, "Name") as HTMLInputElement, "w");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect((created?.[1] as { input: { trigger: { days: string[] } } }).input.trigger.days).toEqual(["tue"]);
  });

  it("summarises every trigger shape in the list", async () => {
    const tasks = [
      task({ id: "t1", name: "manual one", triggerKind: "manual", trigger: {} }),
      task({ id: "t2", name: "once one", trigger: { mode: "once", date: "2026-12-24", time: "09:00" } }),
      task({ id: "t3", name: "interval one", trigger: { mode: "interval", every_minutes: 30 } }),
      task({ id: "t4", name: "weekly one", trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" } }),
      task({ id: "t5", name: "monthly one", trigger: { mode: "monthly", day: 15, time: "08:00" } }),
      task({ id: "t6", name: "unknown one", trigger: { mode: "nonsense" } }),
    ];
    const { container } = await renderView(tasks);
    const text = rows(container).map(row => row.textContent ?? "").join("\n");
    expect(text).toContain("Manual");
    // The stored ISO day is formatted for the active locale, not printed raw.
    expect(text).toContain("12/24/2026");
    expect(text).toContain("Every 30 min");
    // The weekday codes are localized, not printed raw.
    expect(text).toContain("Mon, Fri 10:00");
    expect(text).toContain("15");
    expect(text).toContain("Daily");
  });

  it("switches the session policy and says what reusing a conversation means", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Conversation") as HTMLSelectElement, "existing");
    // Each run continues the same conversation, and nothing is compacted for
    // the user — the point of reusing it is the accumulated context.
    expect(container.textContent).toContain("continues that same conversation");
    expect(container.textContent).toContain("Nothing is compacted");
  });

  it("cancels the editor without saving", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    mocks.invokeCommand.mockClear();
    await click(buttonByText(container, "Cancel"));
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());
    expect(container.textContent).toContain("Select a task");
  });

  it("edits a manual task back to the manual trigger", async () => {
    const { container } = await renderView([
      task({ triggerKind: "manual", trigger: {}, nextDueAt: null }),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    // A task with no schedule opens on "Manual" and has no mode-specific field.
    expect((triggerModeSelect(container) as HTMLSelectElement).value).toBe("manual");
    expect(container.querySelector("fieldset")!.querySelectorAll("input")).toHaveLength(0);
    mocks.invokeCommand.mockClear();
    // Re-install the canned backend rather than answering *every* command with
    // the task list: a blanket mock also feeds task objects to the dependency
    // list, whose rows are keyed by `upstreamTaskId` — undefined there, which
    // React reports as a missing key and which would drown out a real one.
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(call?.[1]).toMatchObject({ id: "tsk_1", input: { triggerKind: "manual", trigger: {} } });
  });

  it("does not offer a default model or thinking level", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    const model = field(container, "Model") as HTMLSelectElement;
    const thinking = field(container, "Thinking level") as HTMLSelectElement;
    // Neither picker offers "default": the run's model and thinking level are
    // the user's call, so the only empty option is "not chosen yet".
    expect([...model.options].filter(option => option.value === "")).toHaveLength(1);
    expect(model.options[0]!.textContent).toBe("Choose a model");
    expect(model.options[0]!.disabled).toBe(true);
    expect([...thinking.options].filter(option => option.value === "")).toHaveLength(1);
    expect(thinking.options[0]!.textContent).toBe("Choose a thinking level");
    // Exactly the enabled models are offered.
    expect([...model.options].slice(1).map(option => option.value)).toEqual(["future/gpt-5"]);
  });

  it("carries every editor field into the saved task", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "configured");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(field(container, "Conversation type") as HTMLSelectElement, "workspace");
    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
    await setValue(field(container, "Conversation") as HTMLSelectElement, "existing");
    await setValue(field(container, "Prompt suggestions") as HTMLSelectElement, "auto");
    await setValue(triggerModeSelect(container), "weekly");
    await setValue(field(container, "Time") as HTMLInputElement, "07:30");
    const enabled = [...container.querySelectorAll<HTMLInputElement>("input[type=checkbox]")]
      .find(box => (box.parentElement?.textContent ?? "").includes("Enabled"))!;
    await act(async () => {
      enabled.click();
    });

    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(call?.[1]).toMatchObject({
      input: {
        modelId: "future/gpt-5",
        thinkingLevel: "high",
        sessionPolicy: "existing",
        reflection: "auto",
        enabled: false,
        trigger: { mode: "weekly", time: "07:30" },
      },
    });
  });

  it("edits the time of a daily trigger", async () => {
    const { container } = await renderView([
      task({ trigger: { mode: "daily", time: "09:00" } }),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    await setValue(field(container, "Time") as HTMLInputElement, "23:15");
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(call?.[1]).toMatchObject({ input: { trigger: { mode: "daily", time: "23:15" } } });
  });

  it("edits the day and time of a monthly trigger", async () => {
    const { container } = await renderView([
      task({ trigger: { mode: "monthly", day: 31, time: "09:00" } }),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    // The stored short-month day is what the editor opens on, with the rule shown.
    expect((field(container, "Day of month") as HTMLInputElement).value).toBe("31");
    expect((field(container, "Time") as HTMLInputElement).value).toBe("09:00");
    expect(container.textContent).toContain("last day of the month");
    await setValue(field(container, "Day of month") as HTMLInputElement, "15");
    await setValue(field(container, "Time") as HTMLInputElement, "06:45");
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(call?.[1]).toMatchObject({
      input: { trigger: { mode: "monthly", day: 15, time: "06:45" } },
    });
  });

  it("promotes exact intervals to hours and days", async () => {
    const { container } = await renderView([
      task({ id: "t1", name: "hourly", trigger: { mode: "interval", every_minutes: 60 } }),
      task({ id: "t2", name: "daily", trigger: { mode: "interval", every_minutes: 1440 } }),
      task({ id: "t3", name: "odd", trigger: { mode: "interval", every_minutes: 90 } }),
    ]);
    const text = rows(container).map(row => row.textContent ?? "").join("\n");
    expect(text).toContain("Every 1 h");
    expect(text).toContain("Every 1 d");
    expect(text).toContain("Every 90 min");
  });

  it("says so when a run recorded no summary", async () => {
    const { container } = await renderView([task()]);
    mocks.invokeCommand.mockImplementation(async (command: string) => {
      if (command === "list_tasks")
        return [task()];
      if (command === "list_task_runs")
        return [run({ resultSummary: null, errorMessage: null })];
      return [];
    });
    await click(rows(container)[0]);
    expect(container.textContent).toContain("No summary recorded");
  });

  it("opens a new task on a chat conversation, and switches to workspace on request", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    // Chat is the default: it needs no directory, so none is asked for.
    expect((field(container, "Conversation type") as HTMLSelectElement).value).toBe("chat");
    expect(container.querySelector("input[aria-label='Working directory']")).toBeNull();
    expect(container.textContent).toContain("brings its own temporary working directory");
    // Switching to a workspace conversation is what asks for the directory.
    await setValue(field(container, "Conversation type") as HTMLSelectElement, "workspace");
    expect(container.querySelector("input[aria-label='Working directory']")).not.toBeNull();
  });

  it("opens a stored task on its own conversation type", async () => {
    const { container } = await renderView([task({ conversationMode: "chat" })]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    expect((field(container, "Conversation type") as HTMLSelectElement).value).toBe("chat");
    expect(container.querySelector("input[aria-label='Working directory']")).toBeNull();
  });
});
