import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import type { DesktopSettings } from "../../../remote/types";
import { SettingsSection } from "../SettingsPrimitives";
import type { SettingsPageBack } from "../pageBack";
import { TasksSettingsPage } from "../TasksSettingsPage";
import { loadTaskDraft, saveTaskDraft, taskDraftKey } from "../taskDraftStorage";
import AsyncStorage from "@react-native-async-storage/async-storage";
import type { RemoteTaskDetail, RemoteTaskRow, RemoteTaskRun } from "../../../remote/taskTypes";

let rows: RemoteTaskRow[];
let detail: RemoteTaskDetail;
let runs: RemoteTaskRun[];
let models: { id: string; label?: string; provider?: string }[];
/** The desktop's own settings: the enabled-model list comes from them. */
let settings: DesktopSettings;
let mockLanguage = "en";

// Untyped jest mocks: the seam under test is which command the page sends with
// which payload, and a narrower fake would fight the real signatures.
const mockRemote = {
  // A paired desktop that nothing else is driving: reads are allowed.
  desktopReadable: true,
  // The page offers the paired desktop's workspaces as working-directory
  // candidates, so the fake control plane has to carry them.
  workspaces: [
    { id: "ws_1", name: "future-os", path: "/Users/me/future-os" },
    { id: "ws_2", name: "notes", path: "/Users/me/notes" },
  ],
  listTasks: jest.fn(async () => [...rows]),
  getTask: jest.fn(async () => ({ ...detail })),
  listTaskRuns: jest.fn(async () => [...runs]),
  listSettingsModels: jest.fn(async () => [...models]),
  listProviders: jest.fn(async () => ({
    builtin: [{ id: "future", name: "Future 平台", baseUrl: "", hasApiKey: true, modelCount: models.length, requiresBaseUrl: false }],
    custom: [],
  })),
  updateTask: jest.fn<Promise<unknown>, unknown[]>(async () => ({ ...detail })),
  createTask: jest.fn<Promise<unknown>, unknown[]>(async () => ({ ...detail })),
  deleteTask: jest.fn(async () => undefined),
  runTask: jest.fn(async () => ({ ...detail })),
  setTaskEnabled: jest.fn(async () => ({ ...detail })),
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
/**
 * The time picker's trigger, and the `HH:MM` it currently shows.
 *
 * The time is a picker now, not a text field: the value is read from the
 * accessibility value it announces, which is the same string the form sends.
 */
const timeTrigger = () => tree.root
  .findAll(node => node.props.accessibilityRole === "button" && node.props.accessibilityLabel === "tasks.form.time")
  .find(node => typeof node.props.onPress === "function")!;
const timeValue = (): string => timeTrigger().props.accessibilityValue.text;
/** Open the picker and choose a value, the way a user does. */
async function pickTime(hour: string, minute: string) {
  await act(async () => timeTrigger().props.onPress());
  const radio = (label: string) => tree.root
    .findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
    .find(node => node.props.accessibilityLabel === label)!;
  await act(async () => radio(`tasks.form.hourOption:{"hour":"${hour}"}`).props.onPress());
  await act(async () => radio(`tasks.form.minuteOption:{"minute":"${minute}"}`).props.onPress());
}

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
 * The button that opens the form, one level down from the task. Located by the
 * label it carries rather than by index, so a control added to the row cannot
 * silently retarget what "open the editor" means.
 */
const settingsToggle = () => pressables()
  .find(node => node.findAllByType(Text).some(text => typeof text.props.children === "string" && text.props.children.includes("tasks.edit")))!;
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
  // Drafts live in the mock store, which outlives one test's mount.
  await AsyncStorage.clear();
  mockLanguage = "en";
  mockRemote.capabilities = new Set(["tasks_v1"]);
  rows = [taskRow()];
  detail = taskDetail();
  runs = [{ id: "trn_1", kind: "main", origin: "schedule", status: "completed", threadId: "thr_1", startedAt: 1, finishedAt: 2, resultSummary: "all good", errorMessage: null }];
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

test("opens a task on its runs, with the form a level down", async () => {
  await act(async () => firstTask().props.onPress());

  expect(mockRemote.getTask).toHaveBeenCalledWith("tsk_1");
  // The task's own page: what it did, and the way into its settings.
  expect(sections()).toContain("daily report");
  expect(texts().some(text => String(text).includes("all good"))).toBe(true);
  // The form is the only level down from here, and it is named after the act.
  expect(button("tasks.edit")).toBeDefined();

  // The form is its own page, carrying the prompt.
  await act(async () => settingsToggle().props.onPress());
  expect(input("tasks.form.prompt").props.value).toBe("summarize yesterday");
  // …and the history it covers is gone, not merely scrolled past.
  expect(sections()).not.toContain("tasks.runs");
});

test("runs, enables and edits through the desktop", async () => {
  await act(async () => firstTask().props.onPress());

  // The desktop dropped the run-now button: a task runs when its trigger
  // fires, so the phone carries the same actions only.
  await act(async () => button("tasks.disable").props.onPress());
  expect(mockRemote.setTaskEnabled).toHaveBeenCalledWith("tsk_1", false);

  // Saving the prompt sends the whole record back, prompt included.
  await act(async () => settingsToggle().props.onPress());
  await act(async () => input("tasks.form.prompt").props.onChangeText("a new prompt"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ prompt: "a new prompt", name: "daily report", cwd: "/tmp/repo" }));
});

test("edits every trigger shape and sends only its own fields", async () => {
  await openTask();

  const save = async () => {
    mockRemote.updateTask.mockClear();
    await act(async () => button("tasks.form.save").props.onPress());
    const payload = mockRemote.updateTask.mock.calls[0]![1] as {
      triggerKind: string;
      trigger: Record<string, unknown>;
    };
    // A landed save leaves the form for the task, so the next shape is edited
    // by opening it again — the draft is still the task's own.
    await act(async () => settingsToggle().props.onPress());
    return payload;
  };

  await act(async () => chip("tasks.triggerMode.once").props.onPress());
  await act(async () => input("tasks.form.date").props.onChangeText("2026-12-24"));
  // The picker, not a keystroke: this is the path a phone user takes.
  await pickTime("18", "00");
  expect(timeValue()).toBe("18:00");
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
  // A save that did not land keeps the form (and the draft) on screen.
  expect(sections()).toContain("tasks.form.details");
  expect(tree.root.findAllByType(ScrollView).length).toBeGreaterThan(0);
  mockRemote.getTask.mockResolvedValueOnce({ ...detail });
  await act(async () => button("common.retry").props.onPress());
  expect(mockRemote.getTask).toHaveBeenCalledTimes(2);
});

// Back is one level at a time: the form leaves for the task, and the task for
// the list. The page registers the first step with the settings stack, which is
// what the header arrow and the system back gesture call.
test("the level back leaves the form first, then the task", async () => {
  let goBackLevel: () => boolean = () => false;
  await remount({
    onBackLevel: (handle: SettingsPageBack | null) => {
      goBackLevel = handle ? () => handle.goBack() : () => false;
    },
  });
  await act(async () => firstTask().props.onPress());
  await act(async () => settingsToggle().props.onPress());
  expect(sections()).toContain("tasks.editTitle");

  // Back from the form is the task it changes…
  await act(async () => { expect(goBackLevel()).toBe(true); });
  expect(sections()).toContain("daily report");
  expect(sections()).toContain("tasks.runs");

  // …and back from the task is the list.
  await act(async () => { expect(goBackLevel()).toBe(true); });
  expect(sections()).toContain("tasks.title");

  // At the list there is nothing left for this page to step back through: it
  // says so rather than answering "handled" and swallowing the gesture, which
  // would leave the user unable to leave the Settings page at all.
  await act(async () => { expect(goBackLevel()).toBe(false); });
});

// Saving is the other way out of the form, and it lands on the task it just
// changed — the same level the header arrow would have returned to.
test("a landed save returns to the task", async () => {
  await openTask();
  expect(sections()).toContain("tasks.form.details");

  await act(async () => input("tasks.form.prompt").props.onChangeText("a new prompt"));
  await act(async () => button("tasks.form.save").props.onPress());

  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ prompt: "a new prompt" }));
  expect(sections()).toContain("daily report");
  expect(sections()).not.toContain("tasks.form.details");
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
  await remount();
  expect(texts().some(text => String(text).includes("tasks.trigger.manual"))).toBe(true);
  await openTask();
  expect(sections()).not.toContain("tasks.runs");
});

test("starts on a weekly trigger's selected weekdays", async () => {
  detail = taskDetail({ trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" } });
  rows = [{ ...rows[0]!, trigger: detail.trigger }];
  await remount();
  await openTask();
  expect(input("tasks.form.days").props.value).toBe("mon,fri");
  expect(timeValue()).toBe("10:00");
});

test("summarises every trigger shape on the list row", async () => {
  rows = [
    taskRow({ id: "t1", name: "once", trigger: { mode: "once", date: "2026-12-24", time: "09:00" } }),
    taskRow({ id: "t2", name: "interval", trigger: { mode: "interval", every_minutes: 30 } }),
    taskRow({ id: "t3", name: "weekly", trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" } }),
    taskRow({ id: "t4", name: "monthly", trigger: { mode: "monthly", day: 15, time: "08:00" } }),
    taskRow({ id: "t5", name: "yearly", trigger: { mode: "yearly", month: 12, day: 31, time: "22:00" } }),
    taskRow({ id: "t6", name: "unknown", trigger: { mode: "nonsense" } }),
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
  // A yearly trigger names its month: the default arm would render "Daily",
  // stating the wrong schedule with nothing to show it was guessed.
  expect(text).toContain("tasks.trigger.yearly");
});

// Round-tripping matters most here: opening a yearly task and saving it without
// noticing must not move the run to another date.
test("edits a yearly trigger's month, day and time", async () => {
  detail = taskDetail({ trigger: { mode: "yearly", month: 12, day: 31, time: "22:00" } });
  rows = [{ ...rows[0]!, trigger: detail.trigger }];
  await remount();
  await openTask();
  expect(chip("tasks.triggerMode.yearly").props.accessibilityState.selected).toBe(true);
  expect(input("tasks.form.month").props.value).toBe("12");
  expect(input("tasks.form.dayOfMonth").props.value).toBe("31");

  // The month is a field like the day: editing it is what the field is for, and
  // a form that rendered it but dropped its edits would move the run's month
  // back to the stored one on the next save.
  await act(async () => input("tasks.form.month").props.onChangeText("6"));
  await act(async () => input("tasks.form.dayOfMonth").props.onChangeText("24"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({
    triggerKind: "schedule",
    trigger: { mode: "yearly", month: 6, day: 24, time: "22:00" },
  }));
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

// The schedule's times are read on the desktop's clock, not the phone's: the
// editor shows the zone it got and sends back what the user picked.
test("carries the time zone the trigger times are read in", async () => {
  detail = taskDetail({ timezone: "Asia/Shanghai" });
  await openTask();

  // The stored zone is shown, and the "desktop's zone" chip is not selected.
  expect(input("tasks.form.timezone").props.value).toBe("Asia/Shanghai");
  expect(chip("tasks.form.timezoneDesktop").props.accessibilityState.selected).toBe(false);

  await act(async () => chip("Europe/Berlin").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({
    timezone: "Europe/Berlin",
  }));
});

test("a new task sends the desktop's zone as an empty value", async () => {
  await act(async () => button("tasks.new").props.onPress());
  expect(chip("tasks.form.timezoneDesktop").props.accessibilityState.selected).toBe(true);

  await act(async () => input("tasks.form.name").props.onChangeText("zoned"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("p"));
  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  await act(async () => chip("tasks.triggerMode.daily").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());

  // 空值 = "跟随桌面"——由桌面解析并落具体时区，手机不猜。
  expect(mockRemote.createTask).toHaveBeenCalledWith(expect.objectContaining({ timezone: "" }));
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
  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
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
  await act(async () => chip("gpt-5 · openai").props.onPress());
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
  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
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
  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
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
  // task stores. Each chip names its vendor (see the picker change).
  expect(chip("DeepSeek V4 Pro · Future 平台")).toBeTruthy();
  expect(chip("gpt-5 · openai")).toBeTruthy();
  await act(async () => chip("gpt-5 · openai").props.onPress());
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
  // it keeps it on screen (by its bare key, without a vendor suffix) — editing
  // another field must not rewrite the task.
  expect(chip("openai/gpt-5")).toBeTruthy();
  expect(chip("openai/gpt-5").props.accessibilityState.selected).toBe(true);
  await act(async () => input("tasks.form.name").props.onChangeText("kept"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ modelId: "openai/gpt-5" }));
  // A save that landed returns to the task it just changed.
  expect(sections()).toContain("daily report");

  // Re-open the form: a model the user has hidden is not offered as a new
  // choice, even though the task keeps the one it already points at.
  await act(async () => settingsToggle().props.onPress());
  expect(chip("DeepSeek V4 Pro · Future 平台")).toBeTruthy();
  expect(tree.root.findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
    .filter(node => node.findAllByType(Text).some(text => String(text.props.children).includes("openai/gpt-5")))).toHaveLength(1);
});

test("a task without a model cannot be saved until one is chosen", async () => {
  // Tasks made by the CLI may pin no model. The phone's form requires one, so
  // editing such a task is where the choice gets made.
  detail = taskDetail({ modelId: null, thinkingLevel: null });
  await remount();
  await openTask();
  expect(texts()).toContain("tasks.form.problem.model");
  expect(button("tasks.form.save").props.disabled).toBe(true);

  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
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
  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
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
  // The pinned chip is a choice like any other: picking it is how a task whose
  // model has been disabled keeps it, and the chip must be able to say so.
  await act(async () => chip("future/deepseek-v4-pro").props.onPress());
  expect(chip("future/deepseek-v4-pro").props.accessibilityState.selected).toBe(true);
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ modelId: "future/deepseek-v4-pro" }));
});

// ─── deleting a task ───────────────────────────────────────────────────────

test("deletes a task after asking, and returns to the list", async () => {
  await act(async () => firstTask().props.onPress());
  // The first press only asks; nothing is deleted yet.
  await act(async () => button("tasks.delete").props.onPress());
  expect(mockRemote.deleteTask).not.toHaveBeenCalled();
  expect(texts().some(text => String(text).includes("tasks.deleteConfirm"))).toBe(true);

  await act(async () => button("tasks.deleteConfirmAction").props.onPress());
  expect(mockRemote.deleteTask).toHaveBeenCalledWith("tsk_1");
  expect(sections()).toContain("tasks.title");
});

test("backs out of a delete", async () => {
  await act(async () => firstTask().props.onPress());
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

// A blanked numeric field falls back rather than sending NaN to the desktop,
// which would either be rejected or stored as a broken trigger. (The time is a
// picker, so it can no longer be blanked at all — it always writes HH:MM.)
test("falls back on a blanked trigger field instead of sending nonsense", async () => {
  await openTask();

  await act(async () => chip("tasks.triggerMode.daily").props.onPress());
  await pickTime("07", "03");
  expect(timeValue()).toBe("07:03");
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({ trigger: { mode: "daily", time: "07:03" } }));

  // The save landed, so the form closed; open it again for the next shape.
  await act(async () => settingsToggle().props.onPress());
  await act(async () => chip("tasks.triggerMode.interval").props.onPress());
  await act(async () => input("tasks.form.everyMinutes").props.onChangeText(""));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({ trigger: { mode: "interval", every_minutes: 1 } }));

  await act(async () => settingsToggle().props.onPress());
  await act(async () => chip("tasks.triggerMode.monthly").props.onPress());
  await act(async () => input("tasks.form.day").props.onChangeText(""));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenLastCalledWith("tsk_1", expect.objectContaining({ trigger: { mode: "monthly", day: 1, time: "07:03" } }));
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
  expect(timeValue()).toBe("09:00");
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
// ─── the form's unfinished work across a cold start ─────────────────────────

const createDraftKey = () => taskDraftKey("unknown", null);
const editDraftKey = () => taskDraftKey("unknown", "tsk_1");
/** The form's own shape, as the draft store keeps it. */
const storedDraft = (overrides: Record<string, unknown> = {}) => ({
  name: "half typed",
  prompt: "still writing it",
  cwd: "",
  modelId: "",
  thinkingLevel: "",
  timezone: "",
  sessionPolicy: "new",
  sessionRetention: "keep",
  conversationMode: "chat",
  enabled: true,
  trigger: {
    mode: "daily",
    date: "",
    time: "07:30",
    everyMinutes: "60",
    days: "mon",
    day: "1",
    month: "12",
  },
  ...overrides,
});
/** Past the draft write's debounce, so the store has what was typed. */
const settleDraftWrite = async () => {
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 320)); });
};

// The reported bug: leaving the app while filling the new-task form came back
// to the same page with nothing in it.
test("restores a new task's draft after the app was killed", async () => {
  await saveTaskDraft(createDraftKey(), storedDraft(), null);
  await act(async () => button("tasks.new").props.onPress());
  await flush();

  expect(input("tasks.form.name").props.value).toBe("half typed");
  expect(input("tasks.form.prompt").props.value).toBe("still writing it");
  expect(timeValue()).toBe("07:30");
});

test("restores an existing task's draft, including the picker's time", async () => {
  detail = taskDetail({ promptVersion: 3 });
  await saveTaskDraft(editDraftKey(), storedDraft({ prompt: "edited locally" }), 3);
  await remount();
  await openTask();
  await flush();

  expect(input("tasks.form.prompt").props.value).toBe("edited locally");
  expect(timeValue()).toBe("07:30");
});

// A prompt that moved underneath the draft (edited from the desktop) is not
// what the draft was written against: the stored task wins and the slot is
// dropped rather than offered back as a page that would overwrite it.
test("drops a draft written against a prompt that has since changed", async () => {
  detail = taskDetail({ promptVersion: 3 });
  await saveTaskDraft(editDraftKey(), storedDraft({ prompt: "edited locally" }), 2);
  await remount();
  await openTask();
  await flush();

  expect(input("tasks.form.prompt").props.value).toBe("summarize yesterday");
  expect(await loadTaskDraft(editDraftKey())).toBeNull();
});

// A stored value is data from disk, not from this component: a wrong shape must
// be ignored field by field rather than rendered.
test("ignores a stored draft whose fields have the wrong shape", async () => {
  await saveTaskDraft(createDraftKey(), { name: 42, prompt: { text: "x" }, trigger: "daily" }, null);
  await act(async () => button("tasks.new").props.onPress());
  await flush();

  expect(input("tasks.form.name").props.value).toBe("");
  expect(input("tasks.form.prompt").props.value).toBe("");
  // The unreadable trigger falls back to the new-task default (manual, so no
  // time is asked for at all) rather than to a half-read schedule.
  expect(chip("tasks.triggerMode.manual").props.accessibilityState.selected).toBe(true);
});

// Merely opening a task is not work in progress: storing the task's own values
// would later be restored over a task somebody edited elsewhere.
test("stores no draft for a task nobody edited", async () => {
  await remount();
  await openTask();
  await flush();
  await settleDraftWrite();
  expect(await loadTaskDraft(editDraftKey())).toBeNull();
});

test("writes what is typed and forgets it once the task is saved", async () => {
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("nightly"));
  await act(async () => input("tasks.form.prompt").props.onChangeText("summarise"));
  await settleDraftWrite();
  expect(await loadTaskDraft(createDraftKey())).toMatchObject({
    draft: expect.objectContaining({ name: "nightly", prompt: "summarise" }),
  });

  await act(async () => chip("DeepSeek V4 Pro · Future 平台").props.onPress());
  await act(async () => chip("tasks.thinkingLabels.high").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());
  expect(await loadTaskDraft(createDraftKey())).toBeNull();
});

// Backing out of the form is leaving the work behind: the next open must show
// the task as the desktop has it, not a page of edits nobody kept.
test("forgets the draft when the user backs out of the form", async () => {
  await saveTaskDraft(editDraftKey(), storedDraft({ prompt: "edited locally" }), detail.promptVersion);
  let goBackLevel: () => boolean = () => false;
  await remount({
    onBackLevel: (handle: SettingsPageBack | null) => {
      goBackLevel = handle ? () => handle.goBack() : () => false;
    },
  });
  await openTask();
  await flush();
  expect(input("tasks.form.prompt").props.value).toBe("edited locally");

  await act(async () => { expect(goBackLevel()).toBe(true); });
  expect(await loadTaskDraft(editDraftKey())).toBeNull();
});

test("forgets a new task's draft when it is cancelled", async () => {
  await act(async () => button("tasks.new").props.onPress());
  await act(async () => input("tasks.form.name").props.onChangeText("nightly"));
  await settleDraftWrite();
  expect(await loadTaskDraft(createDraftKey())).not.toBeNull();

  await act(async () => button("tasks.form.cancel").props.onPress());
  expect(await loadTaskDraft(createDraftKey())).toBeNull();
});

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

// A catalogue model with no provider is keyed by its bare id.
test("keys a model without a provider by its bare id", async () => {
  models = [{ id: "bare-model" }];
  await remount();

  await act(async () => firstTask().props.onPress());
  await act(async () => settingsToggle().props.onPress());
  // No provider means no vendor suffix: the label is the bare id.
  await act(async () => chip("bare-model").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ modelId: "bare-model" }));
});

// A run's conversation is where its reasoning lives, so the phone opens it the
// same way the desktop panel does — in the chat this app already has.
// The page is opened to see what the task did, so the history leads and the form
// that changes it is a level of its own behind the Settings button (a new task
// has no history, so it still opens straight into the form — see the create
// tests).
test("the detail opens on the run history, with the form its own page", async () => {
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

// The way into the form is on the page that opens, not buried under the run
// history: with many runs behind it, a fold at the bottom would be a long
// scroll away (and the page opens on what the task did anyway).
test("the Settings button is above the run history", async () => {
  runs = [{ id: "trn_2", kind: "main", origin: "schedule", status: "completed", threadId: null, startedAt: 3, finishedAt: 4, resultSummary: "the newer run", errorMessage: null }];
  await remount();
  await act(async () => firstTask().props.onPress());

  // Render order, straight from the tree: the Settings button's label is a Text
  // inside a Pressable, the section heading is a Text of its own.
  const labels = tree.root.findAllByType(Text)
    .map(node => node.props.children)
    .flat()
    .filter(child => typeof child === "string");
  const toggle = labels.findIndex(text => (text as string).includes("tasks.edit"));
  const heading = labels.indexOf("tasks.runs");
  expect(toggle).toBeGreaterThanOrEqual(0);
  expect(heading).toBeGreaterThanOrEqual(0);
  expect(toggle).toBeLessThan(heading);
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

test("reports a run with no summary", async () => {
  runs = [{ id: "trn_2", kind: "main", origin: "schedule", status: "failed", threadId: null, startedAt: 1, finishedAt: 2, resultSummary: null, errorMessage: null }];
  await remount();
  await act(async () => firstTask().props.onPress());
  expect(texts().map(String)).toContain("tasks.runNoSummary");
});

test("reports a delete the desktop refused, and stays on the task", async () => {
  await act(async () => firstTask().props.onPress());
  mockRemote.deleteTask.mockRejectedValueOnce(new Error("desktop refused"));
  await act(async () => button("tasks.delete").props.onPress());
  await act(async () => button("tasks.deleteConfirmAction").props.onPress());
  expect(texts()).toContain("desktopSettings.loadFailed");
  expect(sections()).toContain("daily report");
});




