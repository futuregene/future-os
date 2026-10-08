import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import type { DesktopSettings } from "../../../remote/types";
import { SettingsSection } from "../SettingsPrimitives";
import { TasksSettingsPage } from "../TasksSettingsPage";
import type { RemoteTaskDep, RemoteTaskDetail, RemoteTaskRevision, RemoteTaskRow, RemoteTaskRun } from "../../../remote/taskTypes";

let rows: RemoteTaskRow[];
let detail: RemoteTaskDetail;
let runs: RemoteTaskRun[];
let deps: RemoteTaskDep[];
let revisions: RemoteTaskRevision[];
let models: { id: string; label?: string; provider?: string }[];
/** The desktop's own settings: the enabled-model list comes from them. */
let settings: DesktopSettings;
let mockLanguage = "en";

// Untyped jest mocks: the seam under test is which command the page sends with
// which payload, and a narrower fake would fight the real signatures.
const mockRemote = {
  // The page offers the paired desktop's workspaces as working-directory
  // candidates, so the fake control plane has to carry them.
  workspaces: [
    { id: "ws_1", name: "future-os", path: "/Users/me/future-os" },
    { id: "ws_2", name: "notes", path: "/Users/me/notes" },
  ],
  listTasks: jest.fn(async () => [...rows]),
  getTask: jest.fn(async () => ({ ...detail })),
  listTaskRuns: jest.fn(async () => [...runs]),
  listTaskDeps: jest.fn(async () => [...deps]),
  listTaskRevisions: jest.fn(async () => [...revisions]),
  listSettingsModels: jest.fn(async () => [...models]),
  updateTask: jest.fn<Promise<unknown>, unknown[]>(async () => ({ ...detail })),
  createTask: jest.fn<Promise<unknown>, unknown[]>(async () => ({ ...detail })),
  deleteTask: jest.fn(async () => undefined),
  runTask: jest.fn(async () => ({ ...detail })),
  setTaskEnabled: jest.fn(async () => ({ ...detail })),
  applyTaskRevision: jest.fn(async () => ({ ...detail })),
  setTaskDep: jest.fn(async () => undefined),
  removeTaskDep: jest.fn(async () => undefined),
  // The dependency editor is gated on the desktop declaring it. Tests that want
  // the editor replace this set; the rest exercise the older-desktop path.
  capabilities: new Set<string>(["tasks_v1"]),
};
jest.mock("../../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("lucide-react-native", () => ({ ChevronRight: "ChevronRight", ChevronDown: "ChevronDown" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => (options ? `${key}:${JSON.stringify(options)}` : key),
    i18n: { get language() { return mockLanguage; } },
  }),
}));

let tree: ReactTestRenderer;

function taskRow(overrides: Partial<RemoteTaskRow> = {}): RemoteTaskRow {
  return {
    id: "tsk_1",
    name: "daily report",
    enabled: true,
    triggerKind: "schedule",
    trigger: { mode: "daily", time: "09:00" },
    nextDueAt: 1_700_000_000_000,
    queued: false,
    latestRun: null,
    ...overrides,
  };
}

function taskDetail(overrides: Partial<RemoteTaskDetail> = {}): RemoteTaskDetail {
  return {
    ...taskRow(),
    prompt: "summarize yesterday",
    promptVersion: 3,
    cwd: "/tmp/repo",
    conversationMode: "workspace",
    // A task the editor can save unchanged: the model and the thinking level
    // are the user's explicit choice, so a draft without them cannot be saved.
    modelId: "future/deepseek-v4-pro",
    thinkingLevel: "high",
    sessionPolicy: "new",
    depJoin: "all",
    ...overrides,
  };
}

const sections = () => tree.root.findAllByType(SettingsSection).map(node => node.props.title);
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children).flat().filter(child => typeof child === "string");
/**
 * An input by the label it carries. Located by label rather than by position:
 * the form has several inputs and adding one above another must not silently
 * retarget a test (the name field did exactly that to the prompt).
 */
const input = (label: string) => tree.root
  .findAllByType(TextInput)
  .find(node => node.props.accessibilityLabel === label)!;
/**
 * A chip by its label. Chips are located by text, not by index: the page has
 * two chip groups (conversation type, trigger mode) and adding one must not
 * silently retarget the other.
 */
const chip = (label: string) => tree.root
  .findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
  .find(node => node.findAllByType(Text).some(text => text.props.children === label))!;

/** Pressable is a wrapper type, so rows are located by their press behaviour. */
const pressables = () => tree.root.findAll(node => typeof node.props.onPress === "function" && node.props.accessibilityRole === "button");
/**
 * The row for a named task. Located by the name it shows rather than by index,
 * so a control added above the list cannot silently retarget what "open a
 * task" means — the "new task" button did exactly that.
 */
const rowFor = (name: string) => pressables()
  .find(node => node.findAllByType(Text).some(text => text.props.children === name))!;
const firstTask = () => rowFor("daily report");
/**
 * The fold that hides the form on an existing task. The page opens on the run
 * history, so a test that edits a field has to unfold it first — which is also
 * what pins the default (`the detail opens on the run history`).
 */
const settingsToggle = () => pressables()
  .find(node => node.findAllByType(Text).some(text => typeof text.props.children === "string" && text.props.children.includes("tasks.settings")))!;
/** Open a task and its form: the shape almost every editing test wants. */
async function openTask() {
  await act(async () => firstTask().props.onPress());
  await act(async () => settingsToggle().props.onPress());
}

/**
 * The form's session-retention chips, or `undefined` when the choice is not
 * offered (which is the point of the test that uses this). `chip()` below
 * asserts presence instead.
 */
const retentionChip = (label: string) => tree.root
  .findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
  .find(node => node.findAllByType(Text).map(text => text.props.children).flat().includes(label));

/** Let the resource's promise settle before asserting on the rendered rows. */
async function flush() { await act(async () => { await Promise.resolve(); }); }

/**
 * Remount with the current fixtures. `useDesktopResource` reads on mount and on
 * a revision bump, never on an unrelated re-render, so a changed backend is
 * expressed by a fresh mount (exactly what reopening the page does).
 */
async function remount(extra: Record<string, unknown> = {}) {
  await act(async () => { tree.unmount(); });
  await act(async () => { tree = create(createElement(TasksSettingsPage, { desktopOnline: true, settings, ...extra })); });
  await flush();
}

beforeEach(async () => {
  jest.clearAllMocks();
  mockLanguage = "en";
  mockRemote.capabilities = new Set(["tasks_v1"]);
  rows = [taskRow()];
  detail = taskDetail();
  runs = [{ id: "trn_1", kind: "main", origin: "schedule", status: "completed", threadId: "thr_1", startedAt: 1, finishedAt: 2, resultSummary: "all good", errorMessage: null }];
  deps = [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: false }];
  revisions = [{ id: "rev_2", version: 2, source: "reflection", status: "active", reason: "shorter", confidence: 0.8, createdAt: 2, promptPreview: "newer prompt" }];
  models = [
    { id: "deepseek-v4-pro", label: "DeepSeek V4 Pro", provider: "future" },
    { id: "gpt-5", provider: "openai" },
  ];
  settings = { autoUpgradeSkills: false, autoTitleFirstTurn: false, autoConnectRemote: false, hiddenModels: [] };
  await act(async () => { tree = create(createElement(TasksSettingsPage, { desktopOnline: true, settings })); });
  await flush();
});
afterEach(() => act(() => tree.unmount()));

test("lists the desktop's tasks with their trigger and state", async () => {
  expect(mockRemote.listTasks).toHaveBeenCalled();
  expect(pressables().length).toBeGreaterThan(0);
  expect(texts()).toContain("daily report");
  expect(texts().some(text => String(text).includes("tasks.trigger.daily"))).toBe(true);
});

test("says so when the desktop has no tasks, and when the list cannot be read", async () => {
  rows = [];
  await remount();
  expect(texts()).toContain("tasks.empty");

  mockRemote.listTasks.mockRejectedValueOnce(new Error("offline"));
  await remount();
  expect(texts()).toContain("desktopSettings.loadFailed");
  // Retry re-reads the desktop rather than leaving the error on screen.
  mockRemote.listTasks.mockResolvedValueOnce([...rows]);
  await act(async () => button("common.retry").props.onPress());
  expect(mockRemote.listTasks).toHaveBeenCalledTimes(4);
  // The banner is gone now that the desktop answered.
  expect(texts()).not.toContain("desktopSettings.loadFailed");
});

test("opens a task with its prompt, runs, dependencies and prompt versions", async () => {
  await openTask();

  expect(mockRemote.getTask).toHaveBeenCalledWith("tsk_1");
  expect(sections()).toContain("daily report");
  expect(input("tasks.form.prompt").props.value).toBe("summarize yesterday");
  // Dependencies report which upstream has landed.
  expect(texts().some(text => String(text).includes("tasks.depsWaiting"))).toBe(true);
  // The run ledger shows the result summary.
  expect(texts().some(text => String(text).includes("all good"))).toBe(true);
  // Prompt versions can be applied.
  expect(button("tasks.apply")).toBeDefined();
});

test("runs, enables and edits through the desktop", async () => {
  await openTask();

  await act(async () => button("tasks.runNow").props.onPress());
  expect(mockRemote.runTask).toHaveBeenCalledWith("tsk_1");

  await act(async () => button("tasks.disable").props.onPress());
  expect(mockRemote.setTaskEnabled).toHaveBeenCalledWith("tsk_1", false);

  await act(async () => button("tasks.apply").props.onPress());
  expect(mockRemote.applyTaskRevision).toHaveBeenCalledWith("tsk_1", "rev_2");

  // Saving the prompt sends the whole record back, prompt included.
  await act(async () => input("tasks.form.prompt").props.onChangeText("a new prompt"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ prompt: "a new prompt", name: "daily report", cwd: "/tmp/repo" }));
});

test("edits every trigger shape and sends only its own fields", async () => {
  await openTask();

  const save = async () => {
    mockRemote.updateTask.mockClear();
    await act(async () => button("tasks.form.save").props.onPress());
    return mockRemote.updateTask.mock.calls[0]![1] as {
      triggerKind: string;
      trigger: Record<string, unknown>;
    };
  };

  await act(async () => chip("tasks.triggerMode.once").props.onPress());
  await act(async () => input("tasks.form.date").props.onChangeText("2026-12-24"));
  await act(async () => input("tasks.form.time").props.onChangeText("18:00"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "once", date: "2026-12-24", time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.interval").props.onPress());
  await act(async () => input("tasks.form.everyMinutes").props.onChangeText("45"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "interval", every_minutes: 45 } });

  await act(async () => chip("tasks.triggerMode.daily").props.onPress());
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "daily", time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.weekly").props.onPress());
  await act(async () => input("tasks.form.days").props.onChangeText("mon, fri"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "weekly", days: ["mon", "fri"], time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.monthly").props.onPress());
  await act(async () => input("tasks.form.day").props.onChangeText("31"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "monthly", day: 31, time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.manual").props.onPress());
  expect(await save()).toMatchObject({ triggerKind: "manual", trigger: {} });
});

test("explains the short-month rule and the full-permission warning", async () => {
  await openTask();
  await act(async () => chip("tasks.triggerMode.monthly").props.onPress());
  expect(texts().some(text => String(text).includes("tasks.form.shortMonthHint"))).toBe(true);
  await act(async () => chip("tasks.triggerMode.weekly").props.onPress());
  expect(texts()).toContain("tasks.form.fullPermissionWarning");
});

test("reports a failed save instead of pretending it landed", async () => {
  await openTask();
  mockRemote.updateTask.mockRejectedValueOnce(new Error("desktop refused"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledTimes(1);
  // The banner says the save could not be confirmed; the page stays usable and
  // its retry re-reads the task.
  expect(texts()).toContain("desktopSettings.loadFailed");
  expect(tree.root.findAllByType(ScrollView).length).toBeGreaterThan(0);
  mockRemote.getTask.mockResolvedValueOnce({ ...detail });
  await act(async () => button("common.retry").props.onPress());
  expect(mockRemote.getTask).toHaveBeenCalledTimes(2);
});

test("returns to the list from a task", async () => {
  await openTask();
  await act(async () => button("common.back").props.onPress());
  expect(sections()).toContain("tasks.title");
});

// Pressing "run now" while the desktop is busy leaves the request queued for
// the tick. The row says so: otherwise the button looks like it did nothing
// until the in-flight run finishes.
test("shows a queued request instead of the last run's status", async () => {
  rows = [taskRow({ queued: true, latestRun: { id: "trn_1", kind: "main", origin: "schedule", status: "running", resultSummary: null, errorMessage: null } })];
  await remount();
  const shown = texts().map(String);
  expect(shown.some(text => text.includes("tasks.status.queued"))).toBe(true);
  expect(shown.some(text => text.includes("tasks.status.running"))).toBe(false);
});

test("shows the last run's status when nothing is waiting", async () => {
  rows = [taskRow({ queued: false, latestRun: { id: "trn_1", kind: "main", origin: "schedule", status: "completed", resultSummary: "ok", errorMessage: null } })];
  await remount();
  expect(texts().map(String).some(text => text.includes("tasks.status.completed"))).toBe(true);
});

test("summarises a manual task and a task with no runs", async () => {
  rows = [taskRow({ triggerKind: "manual", trigger: {}, latestRun: { id: "trn_9", kind: "manual", origin: "ui", status: "failed", resultSummary: null, errorMessage: "boom" } })];
  detail = taskDetail({ triggerKind: "manual", trigger: {}, latestRun: rows[0]!.latestRun });
  runs = [];
  deps = [];
  revisions = [];
  await remount();
  expect(texts().some(text => String(text).includes("tasks.trigger.manual"))).toBe(true);
  await openTask();
  expect(sections()).not.toContain("tasks.runs");
  expect(sections()).not.toContain("tasks.deps");
});

test("starts on a weekly trigger's selected weekdays", async () => {
  detail = taskDetail({ trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" } });
  rows = [{ ...rows[0]!, trigger: detail.trigger }];
  await remount();
  await openTask();
  expect(input("tasks.form.days").props.value).toBe("mon,fri");
  expect(input("tasks.form.time").props.value).toBe("10:00");
});

test("summarises every trigger shape on the list row", async () => {
  rows = [
    taskRow({ id: "t1", name: "once", trigger: { mode: "once", date: "2026-12-24", time: "09:00" } }),
    taskRow({ id: "t2", name: "interval", trigger: { mode: "interval", every_minutes: 30 } }),
    taskRow({ id: "t3", name: "weekly", trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" } }),
    taskRow({ id: "t4", name: "monthly", trigger: { mode: "monthly", day: 15, time: "08:00" } }),
    taskRow({ id: "t5", name: "unknown", trigger: { mode: "nonsense" } }),
  ];
  await remount();
  const text = texts().join(" ");
  // A stored ISO day is rendered for the active locale, not printed verbatim.
  expect(text).toContain("12/24/2026");
  expect(text).toContain("tasks.trigger.every");
  // The weekday codes are localized, not printed raw.
  expect(text).toContain("tasks.weekday.mon");
  expect(text).toContain("tasks.trigger.monthly");
  expect(text).toContain("tasks.trigger.daily");
});

// Intervals read in the largest unit that divides them exactly, and a one-off
// whose stored date cannot be turned into an instant is printed back as stored
// rather than as a blank or "Invalid Date".
test("reads intervals in hours and days, and falls back on an unusable date", async () => {
  rows = [
    taskRow({ id: "h", name: "hours", trigger: { mode: "interval", every_minutes: 120 } }),
    taskRow({ id: "d", name: "days", trigger: { mode: "interval", every_minutes: 1440 } }),
    // Not an ISO day at all: caught before parsing.
    taskRow({ id: "nodate", name: "weird date", trigger: { mode: "once", date: "next tuesday", time: "09:00" } }),
    // Iso-shaped but not a real date, so it reaches the parser and fails there.
    taskRow({ id: "impossible", name: "impossible", trigger: { mode: "once", date: "2026-13-45", time: "09:00" } }),
  ];
  await remount();
  const text = texts().join(" ");
  expect(text).toContain("tasks.trigger.everyHours");
  expect(text).toContain("tasks.trigger.everyDays");
  expect(text).toContain("next tuesday 09:00");
  // The unparseable one keeps its stored form too, instead of "Invalid Date".
  expect(text).toContain("2026-13-45T09:00:00");
  expect(text).not.toContain("Invalid Date");
});

test("offers the desktop's workspaces as a working directory for a workspace conversation", async () => {
  await remount();
  await openTask();
  // The task is filed under its directory, so the storage path is editable and
  // each known workspace is a one-tap candidate — a phone never has to type a
  // path from memory. (A chat task is not asked for one at all.)
  expect(input("tasks.form.cwdPath").props.value).toBe("/tmp/repo");
  expect(chip("future-os")).toBeTruthy();
  await act(async () => chip("notes").props.onPress());
  expect(input("tasks.form.cwdPath").props.value).toBe("/Users/me/notes");
});

test("sends the working directory picked from a workspace", async () => {
  await remount();
  await openTask();
  await act(async () => chip("notes").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ cwd: "/Users/me/notes" }));
});

test("shows the loading state while the task list is in flight", async () => {
  let release!: (value: RemoteTaskRow[]) => void;
  mockRemote.listTasks.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
  await act(async () => { tree.unmount(); });
  await act(async () => { tree = create(createElement(TasksSettingsPage, { desktopOnline: true, settings })); });
  // The spinner is up, and the empty-state text must not claim there are none.
  expect(tree.root.findAll(node => node.props.accessibilityRole === "alert")).toHaveLength(0);
  await act(async () => { release([]); await Promise.resolve(); });
  expect(texts()).toContain("tasks.empty");
});

test("reports a task that cannot be opened, and retries on request", async () => {
  await flush();
  mockRemote.getTask.mockRejectedValueOnce(new Error("gone"));
  await act(async () => firstTask().props.onPress());
  expect(texts()).toContain("desktopSettings.loadFailed");

  // A retry that fails again keeps the banner instead of opening an empty task.
  mockRemote.getTask.mockRejectedValueOnce(new Error("still gone"));
  await act(async () => button("common.retry").props.onPress());
  expect(mockRemote.getTask).toHaveBeenCalledTimes(2);
  expect(texts()).toContain("desktopSettings.loadFailed");

  mockRemote.getTask.mockResolvedValueOnce({ ...detail });
  await act(async () => button("common.retry").props.onPress());
  expect(mockRemote.getTask).toHaveBeenCalledTimes(3);
  await act(async () => settingsToggle().props.onPress());
  expect(input("tasks.form.prompt").props.value).toBe("summarize yesterday");
});

test("ignores a task read that lands after the page is gone", async () => {
  let release!: (value: RemoteTaskDetail) => void;
  mockRemote.getTask.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
  await act(async () => firstTask().props.onPress());
  await act(async () => tree.unmount());
  // Resolving after unmount must not set state on a dead component.
  await act(async () => { release({ ...detail }); await Promise.resolve(); });
  expect(mockRemote.getTask).toHaveBeenCalledTimes(1);
});

test("a save already in flight is not started twice", async () => {
  await openTask();
  let release!: () => void;
  mockRemote.updateTask.mockImplementationOnce(() => new Promise<void>((resolve) => { release = resolve; }));
  await act(async () => {
    void button("tasks.form.save").props.onPress();
    // A second press while the first write is pending is dropped, so the
    // desktop never sees two overlapping writes for one task.
    void button("tasks.form.save").props.onPress();
  });
  expect(mockRemote.updateTask).toHaveBeenCalledTimes(1);
  await act(async () => { release(); await Promise.resolve(); });
});

// ─── creating a task ────────────────────────────────────────────────────────

test("creates a chat task that needs no directory and returns to the list", async () => {
  await act(async () => button("tasks.new").props.onPress());
  // A new task opens the same editor with nothing behind it.
  expect(sections()).toContain("tasks.newTitle");
  expect(input("tasks.form.name").props.value).toBe("");
  expect(input("tasks.form.prompt").props.value).toBe("");
  // It starts on a chat conversation, which brings its own working directory:
  // the phone neither asks for one nor can browse the desktop's filesystem.
  expect(chip("tasks.form.conversationChat").props.accessibilityState.selected).toBe(true);
  expect(tree.root.findAllByType(TextInput).some(node => node.props.accessibilityLabel === "tasks.form.cwdPath")).toBe(false);

  await act(async () => input("tasks.form.name").props.onChangeText("weekly digest"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("summarise the week"));
  await act(async () => chip("DeepSeek V4 Pro").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  await act(async () => chip("tasks.triggerMode.daily").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.createTask).toHaveBeenCalledWith(expect.objectContaining({
    name: "weekly digest",
    prompt: "summarise the week",
    cwd: "",
    conversationMode: "chat",
    modelId: "future/deepseek-v4-pro",
    thinkingLevel: "high",
    triggerKind: "schedule",
    trigger: { mode: "daily", time: "09:00" },
  }));
  // And it does not also try to update a task that has no id yet.
  expect(mockRemote.updateTask).not.toHaveBeenCalled();
  // The list is re-read, and the editor closes.
  expect(mockRemote.listTasks).toHaveBeenCalledTimes(2);
  expect(sections()).toContain("tasks.title");
});

test("cancels a new task without writing anything", async () => {
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("half-typed"));
  await act(async () => button("tasks.form.cancel").props.onPress());
  expect(mockRemote.createTask).not.toHaveBeenCalled();
  expect(sections()).toContain("tasks.title");
});

test("refuses to save a task that cannot run, and says which field is missing", async () => {
  await act(async () => button("tasks.new").props.onPress());
  // Nothing filled in: the save is refused before it reaches the desktop, and
  // the first missing field is named.
  expect(button("tasks.form.save").props.disabled).toBe(true);
  expect(texts()).toContain("tasks.form.problem.name");

  await act(async () => input("tasks.form.name").props.onChangeText("named"));
  expect(texts()).toContain("tasks.form.problem.prompt");

  await act(async () => input("tasks.form.prompt").props.onChangeText("do the thing"));
  // The model and the thinking level are the user's explicit decisions: the
  // form asks for both rather than inheriting an app default.
  expect(texts()).toContain("tasks.form.problem.model");
  await act(async () => chip("gpt-5").props.onPress());
  expect(texts()).toContain("tasks.form.problem.thinking");
  await act(async () => chip("tasks.thinkingLabels.medium").props.onPress());

  // A one-off needs a real date, which is the last field that can be wrong.
  await act(async () => chip("tasks.triggerMode.once").props.onPress());
  expect(texts()).toContain("tasks.form.problem.date");
  expect(button("tasks.form.save").props.disabled).toBe(true);

  await act(async () => input("tasks.form.date").props.onChangeText("2026-12-24"));
  expect(texts()).not.toContain("tasks.form.problem.date");
  expect(button("tasks.form.save").props.disabled).toBe(false);
});

test("asks for a directory only for a workspace conversation, and is refused without one", async () => {
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("named"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("work"));
  await act(async () => chip("DeepSeek V4 Pro").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  expect(button("tasks.form.save").props.disabled).toBe(false);

  await act(async () => chip("tasks.form.conversationWorkspace").props.onPress());
  // The directory row appears with the choice, and starts empty.
  expect(input("tasks.form.cwdPath").props.value).toBe("");
  expect(texts()).toContain("tasks.form.problem.cwd");
  expect(button("tasks.form.save").props.disabled).toBe(true);

  // Whitespace is not a directory either.
  await act(async () => input("tasks.form.cwdPath").props.onChangeText("   "));
  expect(texts()).toContain("tasks.form.problem.cwd");

  await act(async () => chip("future-os").props.onPress());
  expect(button("tasks.form.save").props.disabled).toBe(false);
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.createTask).toHaveBeenCalledWith(expect.objectContaining({
    conversationMode: "workspace",
    cwd: "/Users/me/future-os",
  }));
});

test("clears the working directory and is refused for it", async () => {
  await remount();
  await openTask();
  // Whitespace is not a directory: a workspace conversation would have nowhere
  // to run, so it is refused before the write leaves the phone.
  await act(async () => input("tasks.form.cwdPath").props.onChangeText("   "));
  expect(texts()).toContain("tasks.form.problem.cwd");
  expect(button("tasks.form.save").props.disabled).toBe(true);
});

test("reports a create that the desktop refused, and keeps the draft", async () => {
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("doomed"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("work"));
  await act(async () => chip("DeepSeek V4 Pro").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  mockRemote.createTask.mockRejectedValueOnce(new Error("desktop refused"));
  await act(async () => button("tasks.form.save").props.onPress());

  // The failure is reported, and the editor stays open with the draft intact so
  // the work is not lost — Save is the retry.
  expect(texts()).toContain("desktopSettings.loadFailed");
  expect(input("tasks.form.name").props.value).toBe("doomed");
  expect(button("tasks.form.save").props.disabled).toBe(false);
});

// ─── editing the fields the phone used to pass through ──────────────────────

test("edits the name, the model and the thinking level", async () => {
  await openTask();
  await act(async () => input("tasks.form.name").props.onChangeText("renamed"));

  // The picker offers the enabled models — the same list the composer uses.
  // A catalogue id is only unique within its provider, so the pair is what the
  // task stores.
  expect(chip("DeepSeek V4 Pro")).toBeTruthy();
  expect(chip("gpt-5")).toBeTruthy();
  await act(async () => chip("gpt-5").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.low").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({
    name: "renamed",
    modelId: "openai/gpt-5",
    thinkingLevel: "low",
  }));
});

test("offers only the enabled models, and keeps a disabled one selectable", async () => {
  settings = { ...settings, hiddenModels: ["openai/gpt-5"] };
  detail = taskDetail({ modelId: "openai/gpt-5" });
  await remount();
  await openTask();

  // A disabled model is not in the enabled list, but a task already pointing at
  // it keeps it on screen — editing another field must not rewrite the task.
  expect(chip("openai/gpt-5")).toBeTruthy();
  expect(chip("openai/gpt-5").props.accessibilityState.selected).toBe(true);
  await act(async () => input("tasks.form.name").props.onChangeText("kept"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ modelId: "openai/gpt-5" }));

  // A model the user has hidden is not offered as a new choice.
  expect(chip("DeepSeek V4 Pro")).toBeTruthy();
  expect(tree.root.findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
    .filter(node => node.findAllByType(Text).some(text => text.props.children === "openai/gpt-5"))).toHaveLength(1);
});

test("a task without a model cannot be saved until one is chosen", async () => {
  // Tasks made by the CLI may pin no model. The phone's form requires one, so
  // editing such a task is where the choice gets made.
  detail = taskDetail({ modelId: null, thinkingLevel: null });
  await remount();
  await openTask();
  expect(texts()).toContain("tasks.form.problem.model");
  expect(button("tasks.form.save").props.disabled).toBe(true);

  await act(async () => chip("DeepSeek V4 Pro").props.onPress());
  expect(texts()).toContain("tasks.form.problem.thinking");
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  expect(button("tasks.form.save").props.disabled).toBe(false);
});

test("switches the conversation policy and the conversation type", async () => {
  await openTask();
  await act(async () => chip("tasks.form.sessionExisting").props.onPress());
  // Reusing a conversation costs a compaction before every run, so the cost is
  // stated where the choice is made.
  expect(texts()).toContain("tasks.form.sessionExistingHint");
  await act(async () => chip("tasks.form.conversationChat").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({
    sessionPolicy: "existing",
    conversationMode: "chat",
  }));
});

// The desktop's form can create a task that starts paused, so the phone's can
// too; without it a task made on a phone always ran from the moment it existed.
test("creates a task that starts disabled", async () => {
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("later"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("work"));
  await act(async () => chip("DeepSeek V4 Pro").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  await act(async () => chip("tasks.form.disabled").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.createTask).toHaveBeenCalledWith(expect.objectContaining({ enabled: false }));
});

test("sends the stored enabled state back when nothing about it changed", async () => {
  detail = taskDetail({ enabled: false });
  await remount();
  await openTask();
  await act(async () => input("tasks.form.prompt").props.onChangeText("a new prompt"));
  await act(async () => button("tasks.form.save").props.onPress());

  // Round-tripping it is what keeps a paused task paused across an edit that
  // had nothing to do with its state.
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ enabled: false }));
});

test("reports a model list that cannot be read, and still lets the task keep its model", async () => {
  mockRemote.listSettingsModels.mockRejectedValueOnce(new Error("offline"));
  await remount();
  await openTask();
  expect(texts()).toContain("tasks.form.modelsFailed");
  // The task's own model stays on screen, so an edit that has nothing to do
  // with the model does not silently rewrite it.
  expect(chip("future/deepseek-v4-pro")).toBeTruthy();
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ modelId: "future/deepseek-v4-pro" }));
});

// ─── deleting a task ───────────────────────────────────────────────────────

test("deletes a task after asking, and returns to the list", async () => {
  await openTask();
  // The first press only asks; nothing is deleted yet.
  await act(async () => button("tasks.delete").props.onPress());
  expect(mockRemote.deleteTask).not.toHaveBeenCalled();
  expect(texts().some(text => String(text).includes("tasks.deleteConfirm"))).toBe(true);

  await act(async () => button("tasks.deleteConfirmAction").props.onPress());
  expect(mockRemote.deleteTask).toHaveBeenCalledWith("tsk_1");
  expect(sections()).toContain("tasks.title");
});

test("backs out of a delete", async () => {
  await openTask();
  await act(async () => button("tasks.delete").props.onPress());
  await act(async () => button("chat.cancel").props.onPress());
  expect(mockRemote.deleteTask).not.toHaveBeenCalled();
  // Still on the task, and the delete control is back to its first step.
  expect(sections()).toContain("daily report");
  expect(texts()).not.toContain("tasks.deleteConfirm");
});

// Save is disabled while the draft cannot run, but the guard inside the handler
// is what actually holds: a press that slips through (a render behind the
// keystroke that invalidated the field) must not reach the desktop.
test("a press that slips past the disabled save still does not write", async () => {
  await openTask();
  await act(async () => input("tasks.form.name").props.onChangeText(""));
  expect(button("tasks.form.save").props.disabled).toBe(true);
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).not.toHaveBeenCalled();
});

// A blanked numeric or time field falls back rather than sending NaN to the
// desktop, which would either be rejected or stored as a broken trigger.
test("falls back on a blanked trigger field instead of sending nonsense", async () => {
  await openTask();

  await act(async () => chip("tasks.triggerMode.daily").props.onPress());
  await act(async () => input("tasks.form.time").props.onChangeText("  "));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({ trigger: { mode: "daily", time: "09:00" } }));

  await act(async () => chip("tasks.triggerMode.interval").props.onPress());
  await act(async () => input("tasks.form.everyMinutes").props.onChangeText(""));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({ trigger: { mode: "interval", every_minutes: 1 } }));

  await act(async () => chip("tasks.triggerMode.monthly").props.onPress());
  await act(async () => input("tasks.form.day").props.onChangeText(""));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({ trigger: { mode: "monthly", day: 1, time: "09:00" } }));
});

// A write that lands after the page is gone must not set state on a dead
// component — the same rule the read path is held to.
test("a save that lands after the page is gone does not touch state", async () => {
  await openTask();
  let release!: () => void;
  mockRemote.updateTask.mockImplementationOnce(() => new Promise<void>((resolve) => { release = resolve; }));
  await act(async () => { void button("tasks.form.save").props.onPress(); });
  await act(async () => { tree.unmount(); });
  await act(async () => { release(); await Promise.resolve(); });
  expect(mockRemote.updateTask).toHaveBeenCalledTimes(1);
});

// ─── payloads that are missing pieces ───────────────────────────────────────

// A desktop one version behind can omit fields the phone expects. Each of these
// falls back to a working default instead of rendering nothing or crashing.
test("copes with a schedule that carries no trigger body", async () => {
  detail = taskDetail({ triggerKind: "schedule", trigger: {}, nextDueAt: null });
  rows = [{ ...rows[0]!, trigger: {}, nextDueAt: null }];
  await remount();
  // The list row still says something.
  expect(texts().some(text => String(text).includes("tasks.trigger.daily"))).toBe(true);
  await openTask();
  // And the form opens on a usable trigger rather than an empty one.
  expect(chip("tasks.triggerMode.daily").props.accessibilityState.selected).toBe(true);
  expect(input("tasks.form.time").props.value).toBe("09:00");
});

test("copes with a task that declares none of its policies", async () => {
  detail = {
    ...taskDetail(),
    sessionPolicy: undefined,
    conversationMode: undefined,
  } as unknown as RemoteTaskDetail;
  await remount();
  await openTask();
  expect(chip("tasks.form.sessionNew").props.accessibilityState.selected).toBe(true);
  expect(chip("tasks.form.conversationWorkspace").props.accessibilityState.selected).toBe(true);
});

// A task with no due time is a manual one: the row must not render a date.
test("a row with no due time omits the date", async () => {
  rows = [taskRow({ triggerKind: "manual", trigger: {}, nextDueAt: null })];
  await remount();
  const row = texts().join(" ");
  expect(row).toContain("tasks.trigger.manual");
  expect(row).not.toMatch(/\d{2}\/\d{2}\/\d{4}/);
});

// A workspace the desktop never named is still a one-tap candidate.
test("offers a workspace by its path when it has no name", async () => {
  mockRemote.workspaces = [{ id: "ws_anon", name: "", path: "/Users/me/anon" }];
  await remount();
  await openTask();
  expect(chip("/Users/me/anon")).toBeTruthy();
});

// A desktop that reports no workspaces still makes a perfectly runnable task:
// a chat conversation brings its own workspace. Switching it to a workspace
// conversation leaves the path empty rather than inventing one.
test("needs no desktop workspace for a chat task", async () => {
  mockRemote.workspaces = [];
  await remount();
  await act(async () => button("tasks.new").props.onPress());
  expect(tree.root.findAllByType(TextInput).some(node => node.props.accessibilityLabel === "tasks.form.cwdPath")).toBe(false);
  await act(async () => chip("tasks.form.conversationWorkspace").props.onPress());
  expect(input("tasks.form.cwdPath").props.value).toBe("");
  await act(async () => input("tasks.form.name").props.onChangeText("named"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("work"));
  expect(texts()).toContain("tasks.form.problem.cwd");
});

// A version row with no reason recorded falls back to the prompt it holds, and
// a catalogue model with no provider is keyed by its bare id.
test("falls back to a version's prompt, and keys a model without a provider", async () => {
  revisions = [{ id: "rev_1", version: 1, source: "user", status: "superseded", reason: null, confidence: null, createdAt: 1, promptPreview: "the original prompt" }];
  models = [{ id: "bare-model" }];
  await remount();
  await openTask();
  expect(texts()).toContain("the original prompt");
  await act(async () => chip("bare-model").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ modelId: "bare-model" }));
});

// A run's conversation is where its reasoning lives, so the phone opens it the
// same way the desktop panel does — in the chat this app already has.
// The page is opened to see what the task did, so the history leads and the
// form is folded behind one press (a new task has no history, so it still opens
// straight into the form — see the create tests).
test("the detail opens on the run history, with the form folded", async () => {
  runs = [{ id: "trn_2", kind: "main", origin: "schedule", status: "completed", threadId: null, startedAt: 3, finishedAt: 4, resultSummary: "the newer run", errorMessage: null }];
  await remount();
  await act(async () => firstTask().props.onPress());

  const shown = texts().map(String);
  expect(shown).toContain("tasks.runs");
  expect(shown).toContain("the newer run");
  // …and the form is not rendered at all, not merely scrolled past.
  expect(tree.root.findAllByType(TextInput)).toHaveLength(0);
  expect(shown).not.toContain("tasks.form.name");

  await act(async () => settingsToggle().props.onPress());
  expect(tree.root.findAllByType(TextInput).length).toBeGreaterThan(0);
  expect(texts()).toContain("tasks.form.name");
});

// The way into the settings sits above the history: with many runs behind it,
// a fold at the bottom would be a long scroll away (and the page opens on what
// the task did anyway).
test("the settings fold is above the run history", async () => {
  runs = [{ id: "trn_2", kind: "main", origin: "schedule", status: "completed", threadId: null, startedAt: 3, finishedAt: 4, resultSummary: "the newer run", errorMessage: null }];
  await remount();
  await act(async () => firstTask().props.onPress());

  // Render order, straight from the tree: the fold's label is a Text inside a
  // Pressable, the section heading is a Text of its own.
  const labels = tree.root.findAllByType(Text)
    .map(node => node.props.children)
    .flat()
    .filter(child => typeof child === "string");
  const fold = labels.findIndex(text => (text as string).includes("tasks.settings"));
  const heading = labels.indexOf("tasks.runs");
  expect(fold).toBeGreaterThanOrEqual(0);
  expect(heading).toBeGreaterThanOrEqual(0);
  // The fold is above the history, so a task with many runs never buries it.
  expect(`${fold} < ${heading}`).toBeTruthy();
  expect(fold).toBeLessThan(heading);
});

// Deleting the run's conversation is offered only where it means something: a
// conversation opened per run. A reused one is what the next run continues.
test("offers the delete-after-run choice only for a per-run conversation", async () => {
  await openTask();
  // A per-run task (the default) offers it; a reused conversation has nothing
  // to delete, so the choice is not rendered at all.
  expect(retentionChip("tasks.form.sessionRetentionDelete")).toBeDefined();

  await act(async () => chip("tasks.form.sessionExisting").props.onPress());
  expect(retentionChip("tasks.form.sessionRetentionDelete")).toBeUndefined();

  await act(async () => chip("tasks.form.sessionNew").props.onPress());
  await act(async () => retentionChip("tasks.form.sessionRetentionDelete")!.props.onPress());
  // The consequence is stated where the choice is made.
  expect(texts()).toContain("tasks.form.sessionRetentionHint");

  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({
    sessionPolicy: "new",
    sessionRetention: "delete",
  }));
});

// The run keeps its output; the conversation is the part that went away.
test("says when a run's conversation was deleted, and hides its open link", async () => {
  runs = [{ id: "trn_2", kind: "main", origin: "schedule", status: "completed", threadId: null, sessionId: null, sessionDeleted: true, startedAt: 3, finishedAt: 4, resultSummary: "the saved answer", errorMessage: null }];
  await remount({ onOpenConversation: () => {} });
  await act(async () => firstTask().props.onPress());

  const shown = texts();
  expect(shown).toContain("the saved answer");
  expect(shown).toContain("tasks.runSessionDeleted");
  expect(shown).not.toContain("tasks.openConversation");
});

test("opens a run's conversation", async () => {
  const opened: string[] = [];
  runs = [
    { id: "trn_2", kind: "main", origin: "schedule", status: "completed", threadId: "thr_2", sessionId: "sess_2", startedAt: 3, finishedAt: 4, resultSummary: "the newer run", errorMessage: null },
    // A run that never reached an agent has no conversation to open.
    { id: "trn_1", kind: "main", origin: "schedule", status: "failed", threadId: null, startedAt: 1, finishedAt: 2, resultSummary: null, errorMessage: "no agent" },
  ];
  await remount({ onOpenConversation: (sessionId: string) => opened.push(sessionId) });
  await act(async () => firstTask().props.onPress());

  const links = pressables().filter(node => node.findAllByType(Text).some(text => text.props.children === "tasks.openConversation"));
  expect(links).toHaveLength(1);
  await act(async () => links[0]!.props.onPress());
  expect(opened).toEqual(["sess_2"]);
});

test("reports a satisfied dependency and a run with no summary", async () => {
  deps = [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: true }];
  runs = [{ id: "trn_2", kind: "main", origin: "schedule", status: "failed", threadId: null, startedAt: 1, finishedAt: 2, resultSummary: null, errorMessage: null }];
  await remount();
  // The run's own summary is on the page that opens; the dependency rows are in
  // the folded form, so this reads both halves of the detail.
  await openTask();
  const shown = texts().map(String);
  expect(shown).toContain("tasks.depsReady");
  expect(shown).toContain("tasks.runNoSummary");
});

test("reports a delete the desktop refused, and stays on the task", async () => {
  await openTask();
  mockRemote.deleteTask.mockRejectedValueOnce(new Error("desktop refused"));
  await act(async () => button("tasks.delete").props.onPress());
  await act(async () => button("tasks.deleteConfirmAction").props.onPress());
  expect(texts()).toContain("desktopSettings.loadFailed");
  expect(sections()).toContain("daily report");
});

// ─── dependency triggers ────────────────────────────────────────────────────

// The editor writes the whole dependency set: kept edges are left alone, a
// changed condition is one call, a dropped edge removed, a new one added.
test("edits a task's dependencies and its join policy", async () => {
  mockRemote.capabilities = new Set(["tasks_v1", "task_deps_v1"]);
  rows = [
    // A task its upstreams start: no schedule of its own.
    taskRow({ triggerKind: "manual", trigger: {}, nextDueAt: null, depCount: 2 }),
    taskRow({ id: "tsk_keep", name: "keep", triggerKind: "manual", trigger: {} }),
    taskRow({ id: "tsk_change", name: "change", triggerKind: "manual", trigger: {} }),
    taskRow({ id: "tsk_three", name: "upstream three", triggerKind: "manual", trigger: {} }),
  ];
  deps = [
    { upstreamTaskId: "tsk_keep", upstreamName: "keep", on: "success", satisfied: false },
    { upstreamTaskId: "tsk_change", upstreamName: "change", on: "success", satisfied: false },
  ];
  detail = taskDetail({ depJoin: "all" });
  await remount();
  await openTask();

  expect(sections()).toContain("tasks.form.deps");
  // The stored edges are listed with their conditions…
  expect(texts().map(String).filter(text => text === "tasks.on.success").length).toBe(2);
  // …and the first one ("keep") is switched to "after failure". The chip list
  // is per edge, so the first failure chip belongs to the first edge.
  await act(async () => chip("tasks.on.failure").props.onPress());

  // A third task is added as an upstream: a new edge waits for success.
  await act(async () => chip("upstream three").props.onPress());

  // With several upstreams the join policy is offered; switch it to "any".
  expect(texts()).toContain("tasks.form.depJoin");
  await act(async () => chip("tasks.join.any").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.updateTask).toHaveBeenCalledWith(
    "tsk_1",
    expect.objectContaining({ depJoin: "any" }),
  );
  expect(mockRemote.setTaskDep).toHaveBeenCalledWith("tsk_1", "tsk_keep", "failure");
  expect(mockRemote.setTaskDep).toHaveBeenCalledWith("tsk_1", "tsk_three", "success");
  // The edge that was not touched is not rewritten.
  expect(mockRemote.setTaskDep).not.toHaveBeenCalledWith("tsk_1", "tsk_change", "success");
  expect(mockRemote.removeTaskDep).not.toHaveBeenCalled();
});

test("removes an edge the user dropped", async () => {
  mockRemote.capabilities = new Set(["tasks_v1", "task_deps_v1"]);
  deps = [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: false }];
  await remount();
  await openTask();
  await act(async () => button("tasks.form.depRemove:{\"name\":\"upstream\"}").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.removeTaskDep).toHaveBeenCalledWith("tsk_1", "tsk_up");
  expect(mockRemote.setTaskDep).not.toHaveBeenCalled();
});

test("does not offer the dependency editor on an older desktop", async () => {
  deps = [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: false }];
  await remount();
  await openTask();
  // The read-only list stays, the controls do not appear.
  expect(texts()).toContain("tasks.form.depsUnsupported");
  expect(texts()).not.toContain("tasks.form.depRemove:{\"name\":\"upstream\"}");
  expect(mockRemote.setTaskDep).not.toHaveBeenCalled();
});

test("creates a task with an upstream it waits for", async () => {
  mockRemote.capabilities = new Set(["tasks_v1", "task_deps_v1"]);
  rows = [taskRow(), taskRow({ id: "tsk_up2", name: "upstream two", triggerKind: "manual", trigger: {} })];
  await remount();
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("downstream"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("p"));
  await act(async () => chip("DeepSeek V4 Pro").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  // The dependency editor appears with the trigger choice…
  expect(chip("upstream two")).toBeUndefined();
  await act(async () => chip("tasks.triggerMode.dependency").props.onPress());
  // …and the candidate list offers the other task; adding it makes it an upstream.
  await act(async () => chip("upstream two").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.createTask).toHaveBeenCalledWith(expect.objectContaining({
    name: "downstream",
    depJoin: "all",
  }));
  // The edge is written against the created task's id.
  expect(mockRemote.setTaskDep).toHaveBeenCalledWith("tsk_1", "tsk_up2", "success");
});
