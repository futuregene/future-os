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
  loadAgentModelOptions: vi.fn(),
  openDialog: vi.fn(),
}));

vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: mocks.invokeCommand }));
// The working-directory picker opens the OS directory chooser; the fake stands
// in for the native tree so the field's contract can be asserted.
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.openDialog }));
vi.mock("../../integrations/agent/agentClient", () => ({
  loadAgentModelOptions: mocks.loadAgentModelOptions,
}));

function task(overrides: Partial<TaskView> = {}): TaskView {
  return {
    id: "tsk_1",
    name: "daily report",
    enabled: true,
    prompt: "summarize yesterday",
    promptVersion: 3,
    cwd: "/tmp/repo",
    modelId: null,
    thinkingLevel: null,
    sessionPolicy: "new",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "daily", time: "09:00" },
    depJoin: "all",
    nextDueAt: 1_700_000_000_000,
    queued: false,
    reflection: "ask",
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
  mocks.loadAgentModelOptions.mockReset();
  mocks.openDialog.mockReset();
  mocks.loadAgentModelOptions.mockResolvedValue([
    { id: "gpt-5", label: "GPT-5", provider: "future" },
  ]);
});

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
function backend(tasks: TaskView[]) {
  mocks.invokeCommand.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case "list_tasks":
        return tasks;
      case "list_task_runs":
        return args?.id === "tsk_1" ? [run()] : [];
      case "list_task_deps":
        return args?.id === "tsk_1"
          ? [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: false }]
          : [];
      case "list_task_revisions":
        return args?.id === "tsk_1"
          ? [
              {
                id: "rev_2",
                version: 2,
                prompt: "newer prompt",
                source: "reflection",
                status: "active",
                reason: "shorter",
                confidence: 0.8,
                createdAt: 2,
              },
            ]
          : [];
      default:
        return undefined;
    }
  });
}

async function renderView(tasks: TaskView[] = [task()], onOpenThread = vi.fn()) {
  backend(tasks);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(
      <TasksView
        leftPanelExpanded
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
      root.render(<TasksView leftPanelExpanded onToggleLeftPanel={() => {}} onOpenThread={vi.fn()} />);
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

  it("falls back to the defaults when a task pins neither model nor thinking level", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    const text = container.textContent ?? "";
    expect(text).toContain("Default model");
    expect(text).toContain("Default");
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

  it("creates a task from the editor", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    mocks.invokeCommand.mockClear();

    // Saving an incomplete form reports what is missing instead of calling out.
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("required");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    await setValue(field(container, "Name") as HTMLInputElement, "my task");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "do the thing");
    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
    await click(buttonByText(container, "Save"));

    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: {
        name: "my task",
        prompt: "do the thing",
        cwd: "/tmp",
        triggerKind: "manual",
        sessionPolicy: "new",
        reflection: "ask",
      },
    });
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
      await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
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
    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
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

  it("switches the session policy and explains the compaction cost", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Conversation") as HTMLSelectElement, "existing");
    expect(container.textContent).toContain("compacted before every run");
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

  it("survives a model list that cannot be loaded", async () => {
    mocks.loadAgentModelOptions.mockRejectedValue(new Error("agent offline"));
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    // The picker falls back to "default model" instead of breaking the form.
    expect(field(container, "Model")).toBeDefined();
    expect(container.querySelectorAll("select")).not.toHaveLength(0);
  });

  it("carries every editor field into the saved task", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "configured");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
    await setValue(field(container, "Model") as HTMLSelectElement, "future/gpt-5");
    await setValue(field(container, "Thinking level") as HTMLSelectElement, "high");
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

  it("creates a chat conversation when the task asks for one", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New task"));
    await setValue(field(container, "Name") as HTMLInputElement, "chatty");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
    await setValue(field(container, "Conversation type") as HTMLSelectElement, "chat");
    expect(container.textContent).toContain("appears under Chat");
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(call?.[1]).toMatchObject({ input: { conversationMode: "chat" } });
  });

  it("opens a stored task on its own conversation type", async () => {
    const { container } = await renderView([task({ conversationMode: "chat" })]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    expect((field(container, "Conversation type") as HTMLSelectElement).value).toBe("chat");
  });
});
