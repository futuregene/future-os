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
  providerNames: { future: "Future 平台", openai: "OpenAI" } as Record<string, string>,
}));

// The model picker names each option's vendor; the map is stubbed so the
// rendered option text is assertable without the providers IPC.
vi.mock("../../integrations/agent/useProviderNames", () => ({
  useProviderNames: () => mocks.providerNames,
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

/**
 * The enabled-model list the view is handed (Settings → Models decides it).
 * `thinkingLevels` is the agent's effective list: `default` first, then the
 * wire values the model accepts.
 */
const MODELS: AgentModelOption[] = [{
  id: "gpt-5",
  label: "GPT-5",
  provider: "future",
  thinkingLevels: ["default", "off", "low", "medium", "high", "xhigh"],
  defaultThinkingLevel: "default",
}];

/** Adds a model the agent catalogue lists with only the implicit default. */
const MODELS_WITHOUT_THINKING: AgentModelOption[] = [
  ...MODELS,
  { id: "jev", label: "Jev", provider: "future", thinkingLevels: ["default"], defaultThinkingLevel: "default" },
];

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
    timezone: "Asia/Shanghai",
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "workspace",
    triggerKind: "schedule",
    trigger: { mode: "daily", time: "09:00" },
    depCount: 0,
    nextDueAt: 1_700_000_000_000,
    queued: false,
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
    sessionDeleted: false,
    ...overrides,
  };
}

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];

beforeEach(() => {
  void i18n.changeLanguage("en");
  mocks.invokeCommand.mockReset();
  mocks.openDialog.mockReset();
  mocks.tauriEvents = {};
  mocks.providerNames = { future: "Future 平台", openai: "OpenAI" };
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
function backend(tasks: TaskView[], runs: TaskRunView[] = [run()]) {
  mocks.invokeCommand.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => backendFor(tasks, command, args, runs),
  );
}

/** The canned answers of the read commands, so a test can wrap them. */
function backendFor(
  tasks: TaskView[],
  command: string,
  args?: Record<string, unknown>,
  runs: TaskRunView[] = [run()],
) {
  switch (command) {
    case "list_tasks":
      return tasks;
    // A write answers with the stored task (the shape the backend returns).
    case "create_task":
    case "update_task":
      return tasks[0];
    case "list_task_runs":
      return args?.id === "tsk_1" ? runs : [];
    default:
      return undefined;
  }
}

async function renderView(
  tasks: TaskView[] = [task()],
  onOpenThread = vi.fn(),
  modelOptions: AgentModelOption[] = MODELS,
  runs: TaskRunView[] = [run()],
) {
  backend(tasks, runs);
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

/**
 * The detail's 设置 disclosure. The page opens on what the task did, so a test
 * that reads the definition (model, prompt, working directory…) has to unfold it —
 * which is also what pins the default (`opens on the run history`).
 */
function settingsToggle(container: HTMLElement) {
  return [...container.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")]
    .find(button => (button.textContent ?? "").trim() === "Settings");
}

/** Open a task's detail and its folded definition. */
async function openDetail(container: HTMLElement) {
  await click(rows(container)[0]);
  await click(settingsToggle(container));
}

/** The run card carrying `text` — one card per run, so what is inside it is grouped. */
function runCard(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLLIElement>("li")]
    .find(item => (item.textContent ?? "").includes(text));
}

/** The control under a labelled field, or null when the field is not rendered. */
function fieldOrNull(container: HTMLElement, label: string) {
  const span = [...container.querySelectorAll("span")]
    .find(node => (node.textContent ?? "").trim() === label);
  return span?.parentElement?.querySelector<HTMLSelectElement | HTMLInputElement | HTMLTextAreaElement>("input, textarea, select") ?? null;
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

function triggerModeSelect(container: HTMLElement) {
  const select = container.querySelector<HTMLSelectElement>("fieldset select");
  if (!select)
    throw new Error(`no trigger select; body=${container.innerHTML.slice(0, 400)}`);
  return select;
}

/**
 * The time-zone field: a trigger row that opens a searchable list.
 *
 * It is not a `<select>` — the list is four hundred zones named in the reader's
 * language with their offsets — so the tests address it by its own hooks.
 */
function timezoneTriggerOrNull(container: HTMLElement) {
  return container.querySelector<HTMLButtonElement>("[data-testid='timezone-trigger']");
}

function timezoneTrigger(container: HTMLElement) {
  const trigger = timezoneTriggerOrNull(container);
  if (!trigger)
    throw new Error("no time zone field");
  return trigger;
}

function timezoneSearch(container: HTMLElement) {
  const input = container.querySelector<HTMLInputElement>("[data-testid='timezone-search']");
  if (!input)
    throw new Error("the zone list is not open");
  return input;
}

/** The row offering `zone` (empty string is this machine's own zone). */
function timezoneOption(container: HTMLElement, zone: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("[data-testid='timezone-option']")]
    .find(node => node.dataset.zone === zone);
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
    expect(container.textContent).toContain("No scheduled tasks yet");
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

  it("opens a task's detail with its runs", async () => {
    const { container, onOpenThread } = await renderView();
    await openDetail(container);

    expect(container.textContent).toContain("summarize yesterday");
    // The run ledger shows the result summary and links to its conversation.
    expect(container.textContent).toContain("all good");
    await click(buttonByText(container, "Open"));
    expect(onOpenThread).toHaveBeenCalledWith("thr_1");
  });

  // The detail is opened to see what the task did, so the run ledger leads and
  // the definition (settings, prompt) is one press away.
  it("opens on the run history, with the definition folded", async () => {
    const { container } = await renderView(
      [task()],
      vi.fn(),
      MODELS,
      [run({ resultSummary: "all good" })],
    );
    await click(rows(container)[0]);

    const text = container.textContent ?? "";
    expect(text).toContain("Runs");
    expect(text).toContain("all good");
    // Folded, not just scrolled past: none of the definition is in the DOM.
    expect(text).not.toContain("summarize yesterday");
    expect(text).not.toContain("Default model");

    await click(settingsToggle(container));
    expect(container.textContent).toContain("summarize yesterday");
    expect(settingsToggle(container)?.getAttribute("aria-expanded")).toBe("true");
  });

  it("shows the model, thinking level and working directory of a task", async () => {
    const { container } = await renderView([task({ modelId: "future/gpt-5", thinkingLevel: "high" })]);
    await openDetail(container);
    const text = container.textContent ?? "";
    // A detail page that omits these cannot answer "what will this run on?".
    expect(text).toContain("Model");
    expect(text).toContain("future/gpt-5");
    expect(text).toContain("Thinking");
    expect(text).toContain("high");
    expect(text).toContain("Working directory");
    expect(text).toContain("/tmp/repo");
  });

  it("falls back to the defaults when a task pins neither model nor thinking level", async () => {
    const { container } = await renderView([task({ modelId: null, thinkingLevel: null })]);
    await openDetail(container);
    const text = container.textContent ?? "";
    expect(text).toContain("Default model");
    // An unset level means the implicit `default` the agent applies — the row
    // names that level, and never a stale "medium".
    expect(text).toContain("default");
    expect(text).not.toContain("medium");
  });

  it("says a chat task runs in its own workspace rather than an empty directory row", async () => {
    const { container } = await renderView([task({ conversationMode: "chat", cwd: "" })]);
    await openDetail(container);
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

  it("toggles and deletes through the backend", async () => {
    const { container } = await renderView();
    await click(rows(container)[0]);
    mocks.invokeCommand.mockClear();

    await click(buttonByText(container, "Disable"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("set_task_enabled", { id: "tsk_1", enabled: false });

    // The first press only opens the confirmation dialog; nothing is deleted yet.
    await click(container.querySelector<HTMLButtonElement>("button[aria-label='Delete scheduled task']")!);
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("delete_task", { id: "tsk_1" });
    expect(container.textContent).toContain("daily report");

    // A declined confirmation must not delete anything, and the task stays.
    await click(buttonByText(container, "Cancel"));
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("delete_task", { id: "tsk_1" });
    expect(container.textContent).toContain("daily report");

    // Confirming inside the dialog deletes, and the detail closes.
    await click(container.querySelector<HTMLButtonElement>("button[aria-label='Delete scheduled task']")!);
    await click(buttonByText(container, "Delete"));
    expect(mocks.invokeCommand).toHaveBeenCalledWith("delete_task", { id: "tsk_1" });
  });

  // Runs are the page's history: consecutive ones used to share a box split by a
  // hairline, so a run that ended in a paragraph ran into the next header.
  it("gives each run its own card", async () => {
    const { container } = await renderView([task()], vi.fn(), MODELS, [
      run({ id: "trn_2", resultSummary: "the newer run", startedAt: 1_700_000_120_000 }),
      run({ id: "trn_1", resultSummary: "the older run", startedAt: 1_700_000_000_000 }),
    ]);
    await click(rows(container)[0]);
    const first = runCard(container, "the older run");
    const second = runCard(container, "the newer run");
    expect(first).toBeDefined();
    expect(second).toBeDefined();
    expect(first).not.toBe(second);
    expect(first?.className).toContain("rounded-md");
  });

  it("creates a chat task that needs no directory", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
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
    // The trigger is required too: with the run-now button gone, a task the
    // user cannot fire needs a schedule that fires itself.
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("Choose a trigger first");
    await setValue(triggerModeSelect(container), "once");
    await setValue(field(container, "Date") as HTMLInputElement, "2026-12-24");
    await setValue(field(container, "Time") as HTMLInputElement, "18:00");
    await click(buttonByText(container, "Save"));

    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: {
        name: "my task",
        prompt: "do the thing",
        cwd: "",
        conversationMode: "chat",
        triggerKind: "schedule",
        trigger: { mode: "once", date: "2026-12-24", time: "18:00" },
        sessionPolicy: "new",
      },
    });
    expect(created?.[1]).toMatchObject({ input: { modelId: "future/gpt-5", thinkingLevel: "high" } });
  });

  // The trigger times are real clock times, so the form asks which clock they
  // belong to. A new task starts on "this machine's zone" — an empty value the
  // host resolves, Beijing when it cannot read one — and an existing task opens
  // on the zone it stores. Each row names the zone in the reader's language and
  // spells out the offset, so the value that gets saved is not what is read.
  it("carries the time zone the schedule's clock reads", async () => {
    const { container } = await renderView([task()]);
    await click(rows(container)[0]);
    await click(settingsToggle(container)!);
    // The detail page names the stored zone the same way the picker does.
    expect(container.textContent).toContain("UTC+8");

    await click(buttonByText(container, "Edit")!);
    // The field opens on the stored zone, under the clock the schedule reads.
    expect(timezoneTrigger(container).textContent).toContain("UTC+8");

    await click(timezoneTrigger(container));
    // The machine's own zone is still offered, and so is a concrete one.
    expect(timezoneOption(container, "")).toBeDefined();
    await click(timezoneOption(container, "Europe/Berlin"));
    await click(buttonByText(container, "Save")!);
    const updated = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(updated?.[1]).toMatchObject({ input: { timezone: "Europe/Berlin" } });
  });

  // The list is four hundred entries long in a database nobody reads by name, so
  // it is searchable by each thing a reader might type: the zone, its city, or
  // the number of hours it is away.
  it("narrows the zone list by name and by offset", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(triggerModeSelect(container), "daily");
    await click(timezoneTrigger(container));

    await setValue(timezoneSearch(container), "Berlin");
    const shown = [...container.querySelectorAll<HTMLElement>("[data-testid='timezone-option']")]
      .map(node => node.dataset.zone);
    expect(shown).toContain("Europe/Berlin");
    expect(shown).not.toContain("Asia/Tokyo");

    // `+8` is how the offset is spoken, and every zone east of UTC by eight
    // hours answers it.
    await setValue(timezoneSearch(container), "+8");
    const byOffset = [...container.querySelectorAll<HTMLElement>("[data-testid='timezone-option']")]
      .map(node => node.dataset.zone);
    expect(byOffset).toContain("Asia/Shanghai");
    expect(byOffset).not.toContain("Europe/Berlin");
  });

  it("leaves a new task on the machine's zone until one is chosen", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    mocks.invokeCommand.mockClear();

    await setValue(field(container, "Name") as HTMLInputElement, "zoned");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(triggerModeSelect(container), "daily");
    await setValue(field(container, "Time") as HTMLInputElement, "09:00");
    // The unset choice is what reaches the host: it resolves and stores a
    // concrete zone, so the record never carries a guess.
    expect(timezoneTrigger(container).textContent).toContain("This machine's zone");
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({ input: { timezone: "" } });
  });

  // The zone says which clock the trigger's times are read on, so it belongs to
  // the trigger: the form asks which trigger first and offers a zone only for a
  // mode that has times, inside the trigger's fieldset. "Every N minutes" counts
  // from the previous run and a manual task has no schedule, so neither has a
  // clock to name.
  it("asks for the time zone inside the trigger, only for a mode that has times", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));

    expect(timezoneTriggerOrNull(container)).toBeNull();

    await setValue(triggerModeSelect(container), "interval");
    expect(timezoneTriggerOrNull(container)).toBeNull();

    await setValue(triggerModeSelect(container), "daily");
    const trigger = timezoneTrigger(container);
    const fieldset = trigger.closest("fieldset");
    expect(fieldset, "the zone is asked for inside the trigger").not.toBeNull();
    const controls = [...fieldset!.querySelectorAll("select")];
    // The trigger mode is still the first question; the zone follows it.
    expect(controls.indexOf(triggerModeSelect(container))).toBe(0);
    expect(controls).toHaveLength(1);
  });

  it("refuses a save without a model or a thinking level", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
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
    // The trigger is now the last missing decision.
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("Choose a trigger first");
    await setValue(triggerModeSelect(container), "daily");
    await setValue(field(container, "Time") as HTMLInputElement, "09:00");
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: { modelId: "future/gpt-5", thinkingLevel: "medium", triggerKind: "schedule" },
    });
  });

  it("asks for a directory only for a workspace conversation", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(field(container, "Name") as HTMLInputElement, "filed");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(field(container, "Conversation type") as HTMLSelectElement, "workspace");

    mocks.invokeCommand.mockClear();
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("needs a working directory");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp/filed");
    await setValue(triggerModeSelect(container), "daily");
    await setValue(field(container, "Time") as HTMLInputElement, "09:00");
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: { conversationMode: "workspace", cwd: "/tmp/filed" },
    });
  });

  it("says so when no model is enabled", async () => {
    const { container } = await renderView([], vi.fn(), []);
    await click(buttonByText(container, "New scheduled task"));
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
      await click(buttonByText(container, "New scheduled task"));
      await setValue(field(container, "Name") as HTMLInputElement, "scheduled");
      await setValue(field(container, "Prompt") as HTMLTextAreaElement, "prompt");
      await chooseModelAndThinking(container);
      await setValue(triggerModeSelect(container), mode);
      await extra(container);
      // A scheduled mode with a time field needs that time explicitly: the
      // form no longer seeds one the user never picked.
      if (["daily", "weekly", "monthly", "yearly"].includes(mode)) {
        const timeField = fieldOrNull(container, "Time") as HTMLInputElement | null;
        if (timeField && !timeField.value)
          await setValue(timeField, "09:00");
      }
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

    // Manual is no longer offered on a fresh form: the run-now button is gone,
    // so a task the user cannot ever fire is not a choice worth making.
    const freshForm = (await (async () => {
      const { container: c } = await renderView([]);
      await click(buttonByText(c, "New scheduled task"));
      return c;
    })());
    expect((triggerModeSelect(freshForm) as HTMLSelectElement).querySelector("option[value='manual']")).toBeNull();
    expect(await submit("once", async (container) => {
      await setValue(field(container, "Date") as HTMLInputElement, "2026-01-01");
      await setValue(field(container, "Time") as HTMLInputElement, "08:00");
    })).toMatchObject({ triggerKind: "schedule" });
  });

  /** A stored manual task round-trips: its option stays selectable for edits. */
  it("keeps the manual option for a task that already is manual", async () => {
    const { container } = await renderView([
      task({ triggerKind: "manual", trigger: {}, nextDueAt: null }),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    expect((triggerModeSelect(container) as HTMLSelectElement).value).toBe("manual");
  });

  it("shows the short-month rule and the full-permission warning while editing", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(triggerModeSelect(container), "monthly");
    expect(container.textContent).toContain("last day of the month");
    expect(container.textContent).toContain("full permission");
  });

  /** The one-off trigger's date and time are required, each with its own message. */
  it("refuses a one-off save without a date or a time", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(field(container, "Name") as HTMLInputElement, "one-off");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(triggerModeSelect(container), "once");
    mocks.invokeCommand.mockClear();

    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("Pick the date");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    // A date without a time is refused on the time.
    await setValue(field(container, "Date") as HTMLInputElement, "2026-12-24");
    await click(buttonByText(container, "Save"));
    expect(container.textContent).toContain("Pick the time");
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());

    // Both set: the save goes out with exactly what the pickers wrote.
    await setValue(field(container, "Time") as HTMLInputElement, "07:15");
    await click(buttonByText(container, "Save"));
    const created = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(created?.[1]).toMatchObject({
      input: { trigger: { mode: "once", date: "2026-12-24", time: "07:15" } },
    });
  });

  /** A new one-off opens with no value in either picker. */
  it("opens the one-off pickers empty instead of pre-filling a date and time", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(triggerModeSelect(container), "once");
    expect((field(container, "Date") as HTMLInputElement).value).toBe("");
    expect((field(container, "Time") as HTMLInputElement).value).toBe("");
  });

  /** The run ledger renders the model's markdown, not its raw source. */
  it("renders a completed run's summary as markdown", async () => {
    const { container } = await renderView(
      [task()],
      vi.fn(),
      MODELS,
      [run({ resultSummary: "## Summary\n\n- **done** quickly" })],
    );
    await click(rows(container)[0]);
    const card = runCard(container, "done");
    expect(card?.querySelector("h2"));
    expect(card?.querySelector("li"));
    expect(card?.querySelector("strong"));
    expect(card?.textContent).not.toContain("## Summary");
  });

  /** An error stays plain text: it is the host's message, not the model's prose. */
  it("shows a failed run's error as plain text", async () => {
    const { container } = await renderView(
      [task()],
      vi.fn(),
      MODELS,
      [run({ status: "failed", resultSummary: null, errorMessage: "agent unreachable" })],
    );
    await click(rows(container)[0]);
    const card = runCard(container, "agent unreachable");
    expect(card?.querySelector("h2, li, strong")).toBeNull();
  });

  it("adds and removes weekdays on a weekly trigger", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
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
      task({ id: "t6", name: "yearly one", trigger: { mode: "yearly", month: 12, day: 31, time: "22:00" } }),
      task({ id: "t7", name: "unknown one", trigger: { mode: "nonsense" } }),
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
    // A yearly trigger has to name its month: rendering it as "Daily" (the
    // default arm) would state the wrong schedule with no sign anything is off.
    expect(text).toContain("12/31");
  });

  // A yearly task's editor has to round-trip its month/day, or opening and
  // saving it silently moves the run to the wrong date.
  it("edits a yearly trigger's month, day and time", async () => {
    const { container } = await renderView([
      task({ trigger: { mode: "yearly", month: 12, day: 31, time: "22:00" } }),
    ]);
    await click(rows(container)[0]);
    await click(buttonByText(container, "Edit"));
    expect((triggerModeSelect(container) as HTMLSelectElement).value).toBe("yearly");
    expect((field(container, "Month") as HTMLInputElement).value).toBe("12");
    // The yearly day field uses the neutral label: "Day of month" would
    // contradict a trigger whose month is chosen separately.
    expect((field(container, "Day") as HTMLInputElement).value).toBe("31");

    await setValue(field(container, "Day") as HTMLInputElement, "24");
    backend([]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(call?.[1]).toMatchObject({
      input: { triggerKind: "schedule", trigger: { mode: "yearly", month: 12, day: 24, time: "22:00" } },
    });
  });

  it("switches the session policy and says what reusing a conversation means", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(field(container, "Conversation") as HTMLSelectElement, "existing");
    // Each run continues the same conversation, and nothing is compacted for
    // the user — the point of reusing it is the accumulated context.
    expect(container.textContent).toContain("continues that same conversation");
    expect(container.textContent).toContain("Nothing is compacted");
  });

  it("cancels the editor without saving", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    mocks.invokeCommand.mockClear();
    await click(buttonByText(container, "Cancel"));
    expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());
    expect(container.textContent).toContain("Select a scheduled task");
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
    // the task list: a blanket mock would answer the other reads with a task
    // list too, which is not a shape any of them returns.
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
    expect(call?.[1]).toMatchObject({ id: "tsk_1", input: { triggerKind: "manual", trigger: {} } });
  });

  it("offers the model's own thinking levels and no implicit model default", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    const model = field(container, "Model") as HTMLSelectElement;
    const thinking = field(container, "Thinking level") as HTMLSelectElement;
    // The model picker has no "default" entry — the model is the user's call.
    // The thinking picker does: `default` is a real level the agent publishes,
    // listed first, while the empty option alone means "not chosen yet".
    expect([...model.options].filter(option => option.value === "")).toHaveLength(1);
    expect(model.options[0]!.textContent).toBe("Choose a model");
    expect(model.options[0]!.disabled).toBe(true);
    expect([...thinking.options].filter(option => option.value === "")).toHaveLength(1);
    expect(thinking.options[0]!.textContent).toBe("Choose a thinking level");
    await setValue(model, "future/gpt-5");
    expect([...thinking.options].slice(1).map(option => option.value)).toEqual([
      "default",
      "off",
      "low",
      "medium",
      "high",
      "xhigh",
    ]);
    // Exactly the enabled models are offered, each naming its vendor: two
    // providers often expose the same model name, so a name-only picker
    // cannot tell them apart.
    expect([...model.options].slice(1).map(option => option.value)).toEqual(["future/gpt-5"]);
    expect(model.options[1]!.textContent).toBe("GPT-5 · Future 平台");
  });

  /** A provider the name map does not know falls back to its raw id. */
  it("falls back to the provider id when no display name is known", async () => {
    mocks.providerNames = {};
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    const model = field(container, "Model") as HTMLSelectElement;
    expect(model.options[1]!.textContent).toBe("GPT-5 · future");
  });

  it("disables the thinking picker for a model with a single level", async () => {
    // A model whose list holds just the implicit default offers no choice, so
    // the picker is disabled rather than left offering one level — the same
    // rule the composer applies.
    const { container } = await renderView([], vi.fn(), MODELS_WITHOUT_THINKING);
    await click(buttonByText(container, "New scheduled task"));
    expect((field(container, "Thinking level") as HTMLSelectElement).disabled).toBe(false);

    await setValue(field(container, "Model") as HTMLSelectElement, "future/jev");
    expect((field(container, "Thinking level") as HTMLSelectElement).disabled).toBe(true);
  });

  it("normalises the thinking level when the model changes", async () => {
    const { container } = await renderView([], vi.fn(), MODELS_WITHOUT_THINKING);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(field(container, "Model") as HTMLSelectElement, "future/gpt-5");

    // A single-level model pins the implicit default: the stored level must not
    // keep claiming an effort the model never runs at.
    await setValue(field(container, "Thinking level") as HTMLSelectElement, "high");
    await setValue(field(container, "Model") as HTMLSelectElement, "future/jev");
    expect((field(container, "Thinking level") as HTMLSelectElement).value).toBe("default");

    // Switching back keeps it: `default` is a level the new model does accept,
    // and normalisation only replaces a level the model cannot run (or none).
    // Re-deciding here would silently overwrite a deliberate choice.
    await setValue(field(container, "Model") as HTMLSelectElement, "future/gpt-5");
    expect((field(container, "Thinking level") as HTMLSelectElement).value).toBe("default");
  });

  it("gives a model change the app default when no level was chosen yet", async () => {
    const { container } = await renderView([], vi.fn(), MODELS_WITHOUT_THINKING);
    await click(buttonByText(container, "New scheduled task"));
    // Untouched, the picker still says "not chosen yet"; picking a model fills in
    // the app default rather than leaving the required field empty.
    expect((field(container, "Thinking level") as HTMLSelectElement).value).toBe("");
    await setValue(field(container, "Model") as HTMLSelectElement, "future/gpt-5");
    expect((field(container, "Thinking level") as HTMLSelectElement).value).toBe("default");
  });

  it("carries every editor field into the saved task", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await setValue(field(container, "Name") as HTMLInputElement, "configured");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await chooseModelAndThinking(container);
    await setValue(field(container, "Conversation type") as HTMLSelectElement, "workspace");
    await setValue(field(container, "Working directory") as HTMLInputElement, "/tmp");
    await setValue(field(container, "Conversation") as HTMLSelectElement, "existing");
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
        enabled: false,
        trigger: { mode: "weekly", time: "07:30" },
      },
    });
  });

  // Deleting the run's conversation is offered only where it means something:
  // a conversation that is opened per run. A reused one is what the next run
  // continues, so the choice is not there — and a draft that somehow carries it
  // sends `keep` rather than storing a combination nothing honours.
  it("offers the delete-after-run choice only for a per-run conversation", async () => {
    const { container } = await renderView([]);
    await click(buttonByText(container, "New scheduled task"));
    await chooseModelAndThinking(container);

    const retention = () => fieldOrNull(container, "Conversation after the run");
    expect(retention(), "a new task opens a per-run conversation").not.toBeNull();

    await setValue(field(container, "Conversation") as HTMLSelectElement, "existing");
    expect(retention(), "reuse has no conversation to delete").toBeNull();

    await setValue(field(container, "Conversation") as HTMLSelectElement, "new");
    await setValue(retention()! as HTMLSelectElement, "delete");
    // The consequence is stated where the choice is made.
    expect(container.textContent).toContain("full output stay on the run");

    await setValue(field(container, "Name") as HTMLInputElement, "ephemeral");
    await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
    await setValue(triggerModeSelect(container), "daily");
    await setValue(field(container, "Time") as HTMLInputElement, "09:00");
    mocks.invokeCommand.mockClear();
    backend([task()]);
    await click(buttonByText(container, "Save"));
    const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "create_task");
    expect(call?.[1]).toMatchObject({
      input: { sessionPolicy: "new", sessionRetention: "delete" },
    });
  });

  // The run keeps its output; the conversation is the part that went away.
  it("says when a run's conversation was deleted, and hides its open link", async () => {
    const { container } = await renderView(
      [task()],
      vi.fn(),
      MODELS,
      [run({ threadId: null, sessionDeleted: true, resultSummary: "the saved answer" })],
    );
    await click(rows(container)[0]);

    const text = container.textContent ?? "";
    expect(text).toContain("the saved answer");
    expect(text).toContain("deleted as configured");
    expect(buttonByText(container, "Open")).toBeUndefined();
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
    await click(buttonByText(container, "New scheduled task"));
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

/**
 * A yearly task's month and time are editable, not just readable.
 *
 * The round-trip test above edits the *day*; month and time are separate
 * controls with their own writers, and a form that displays a value it cannot
 * change is worse than one that omits it — the user reads the schedule, believes
 * it, and cannot correct it. Both are asserted through what the save sends.
 */
it("edits a yearly trigger's month and time", async () => {
  const { container } = await renderView([
    task({ trigger: { mode: "yearly", month: 12, day: 31, time: "22:00" } }),
  ]);
  await click(rows(container)[0]);
  await click(buttonByText(container, "Edit"));

  await setValue(field(container, "Month") as HTMLInputElement, "3");
  await setValue(field(container, "Time") as HTMLInputElement, "07:05");
  backend([]);
  await click(buttonByText(container, "Save"));

  const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
  expect(call?.[1]).toMatchObject({
    input: { triggerKind: "schedule", trigger: { mode: "yearly", month: 3, day: 31, time: "07:05" } },
  });
});

/**
 * An emptied month field is read as January, not as month zero or `NaN`.
 *
 * Clearing a number input is the ordinary way a user starts typing a new one,
 * and the value React hands back for an empty box is the empty string — which
 * `Number()` turns into 0. Sending 0 (or `NaN`) would store a month the host
 * cannot schedule, so the field is clamped to a real month.
 */
it("reads an emptied month field as January rather than month zero", async () => {
  const { container } = await renderView([
    task({ trigger: { mode: "yearly", month: 12, day: 31, time: "22:00" } }),
  ]);
  await click(rows(container)[0]);
  await click(buttonByText(container, "Edit"));

  await setValue(field(container, "Month") as HTMLInputElement, "");
  backend([]);
  await click(buttonByText(container, "Save"));

  const call = mocks.invokeCommand.mock.calls.find(([command]) => command === "update_task");
  expect(call?.[1]).toMatchObject({ input: { trigger: { month: 1 } } });
});

/**
 * A model without a thinking level is refused on *that* message.
 *
 * The existing refusal test leaves both unchosen, so it stops at the model —
 * the thinking-level branch is only reached by a form that has a model and no
 * level, which is a state a user can produce by clearing the level after
 * choosing a model (the level select keeps its "not chosen yet" option, since
 * a model that can think may still be left unconfigured). Without this branch
 * the save would send `thinkingLevel: null` and the host would pick for them.
 */
it("refuses a save that has a model but no thinking level", async () => {
  const { container } = await renderView([]);
  await click(buttonByText(container, "New scheduled task"));
  await setValue(field(container, "Name") as HTMLInputElement, "half configured");
  await setValue(field(container, "Prompt") as HTMLTextAreaElement, "p");
  await chooseModelAndThinking(container, "future/gpt-5", "medium");
  // …and then the user clears the level.
  await setValue(field(container, "Thinking level") as HTMLSelectElement, "");
  mocks.invokeCommand.mockClear();

  await click(buttonByText(container, "Save"));

  expect(container.textContent).toContain("Choose a thinking level");
  expect(mocks.invokeCommand).not.toHaveBeenCalledWith("create_task", expect.anything());
});

/**
 * A stored date the summary cannot parse is shown as the host wrote it.
 *
 * An empty or malformed `date` is not a date to format — formatting it would
 * throw or invent a day. The reader gets the raw pair instead, which is at least
 * true, and the empty-clock case leaves no trailing space.
 */
it("shows an unparsable one-off date as the host wrote it", async () => {
  const tasks = [
    task({ id: "t_bad", name: "odd one", triggerKind: "schedule", trigger: { mode: "once", date: "someday", time: "09:00" } }),
    task({ id: "t_empty", name: "empty one", triggerKind: "schedule", trigger: { mode: "once", date: "", time: "" } }),
  ];
  const { container } = await renderView(tasks);
  const text = rows(container).map(row => row.textContent ?? "").join("\n");

  expect(text).toContain("someday 09:00");
  // Nothing to show at all: no stray separator either.
  expect(text).not.toContain("undefined");
  expect(text).not.toContain("  ");
});
