import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsField, SettingsSection } from "../SettingsPrimitives";
import { TasksSettingsPage } from "../TasksSettingsPage";
import type { RemoteTaskDep, RemoteTaskDetail, RemoteTaskRevision, RemoteTaskRow, RemoteTaskRun } from "../../../remote/taskTypes";

let rows: RemoteTaskRow[];
let detail: RemoteTaskDetail;
let runs: RemoteTaskRun[];
let deps: RemoteTaskDep[];
let revisions: RemoteTaskRevision[];
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
  updateTask: jest.fn<Promise<unknown>, unknown[]>(async () => ({ ...detail })),
  runTask: jest.fn(async () => ({ ...detail })),
  setTaskEnabled: jest.fn(async () => ({ ...detail })),
  applyTaskRevision: jest.fn(async () => ({ ...detail })),
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
    lastRunAt: null,
    reflection: "ask",
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
    modelId: null,
    thinkingLevel: null,
    sessionPolicy: "new",
    depJoin: "all",
    ...overrides,
  };
}

const sections = () => tree.root.findAllByType(SettingsSection).map(node => node.props.title);
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children).flat().filter(child => typeof child === "string");
const field = (label: string) => tree.root.findAllByType(SettingsField).find(node => node.props.label === label)!;
const inputUnder = (label: string) => {
  const labelled = field(label);
  return labelled.findAllByType(TextInput)[0]!;
};
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
const firstTask = () => pressables()[0]!;

/** Let the resource's promise settle before asserting on the rendered rows. */
async function flush() { await act(async () => { await Promise.resolve(); }); }

/**
 * Remount with the current fixtures. `useDesktopResource` reads on mount and on
 * a revision bump, never on an unrelated re-render, so a changed backend is
 * expressed by a fresh mount (exactly what reopening the page does).
 */
async function remount() {
  await act(async () => { tree.unmount(); });
  await act(async () => { tree = create(createElement(TasksSettingsPage, { desktopOnline: true })); });
  await flush();
}

beforeEach(async () => {
  jest.clearAllMocks();
  mockLanguage = "en";
  rows = [taskRow()];
  detail = taskDetail();
  runs = [{ id: "trn_1", kind: "main", origin: "schedule", status: "completed", threadId: "thr_1", startedAt: 1, finishedAt: 2, resultSummary: "all good", errorMessage: null }];
  deps = [{ upstreamTaskId: "tsk_up", upstreamName: "upstream", on: "success", satisfied: false }];
  revisions = [{ id: "rev_2", version: 2, source: "reflection", status: "active", reason: "shorter", confidence: 0.8, createdAt: 2, promptPreview: "newer prompt" }];
  await act(async () => { tree = create(createElement(TasksSettingsPage, { desktopOnline: true })); });
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
  await act(async () => firstTask().props.onPress());

  expect(mockRemote.getTask).toHaveBeenCalledWith("tsk_1");
  expect(sections()).toContain("daily report");
  expect(tree.root.findAllByType(TextInput)[0]!.props.value).toBe("summarize yesterday");
  // Dependencies report which upstream has landed.
  expect(texts().some(text => String(text).includes("tasks.depsWaiting"))).toBe(true);
  // The run ledger shows the result summary.
  expect(texts().some(text => String(text).includes("all good"))).toBe(true);
  // Prompt versions can be applied.
  expect(button("tasks.apply")).toBeDefined();
});

test("runs, enables and edits through the desktop", async () => {
  await act(async () => firstTask().props.onPress());

  await act(async () => button("tasks.runNow").props.onPress());
  expect(mockRemote.runTask).toHaveBeenCalledWith("tsk_1");

  await act(async () => button("tasks.disable").props.onPress());
  expect(mockRemote.setTaskEnabled).toHaveBeenCalledWith("tsk_1", false);

  await act(async () => button("tasks.apply").props.onPress());
  expect(mockRemote.applyTaskRevision).toHaveBeenCalledWith("tsk_1", "rev_2");

  // Saving the prompt sends the whole record back, prompt included.
  await act(async () => tree.root.findAllByType(TextInput)[0]!.props.onChangeText("a new prompt"));
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ prompt: "a new prompt", name: "daily report", cwd: "/tmp/repo" }));
});

test("edits every trigger shape and sends only its own fields", async () => {
  await act(async () => firstTask().props.onPress());

  const save = async () => {
    mockRemote.updateTask.mockClear();
    await act(async () => button("tasks.form.save").props.onPress());
    return mockRemote.updateTask.mock.calls[0]![1] as {
      triggerKind: string;
      trigger: Record<string, unknown>;
    };
  };

  await act(async () => chip("tasks.triggerMode.once").props.onPress());
  await act(async () => inputUnder("tasks.form.date").props.onChangeText("2026-12-24"));
  await act(async () => inputUnder("tasks.form.time").props.onChangeText("18:00"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "once", date: "2026-12-24", time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.interval").props.onPress());
  await act(async () => inputUnder("tasks.form.everyMinutes").props.onChangeText("45"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "interval", every_minutes: 45 } });

  await act(async () => chip("tasks.triggerMode.daily").props.onPress());
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "daily", time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.weekly").props.onPress());
  await act(async () => inputUnder("tasks.form.days").props.onChangeText("mon, fri"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "weekly", days: ["mon", "fri"], time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.monthly").props.onPress());
  await act(async () => inputUnder("tasks.form.day").props.onChangeText("31"));
  expect(await save()).toMatchObject({ triggerKind: "schedule", trigger: { mode: "monthly", day: 31, time: "18:00" } });

  await act(async () => chip("tasks.triggerMode.manual").props.onPress());
  expect(await save()).toMatchObject({ triggerKind: "manual", trigger: {} });
});

test("explains the short-month rule and the full-permission warning", async () => {
  await act(async () => firstTask().props.onPress());
  await act(async () => chip("tasks.triggerMode.monthly").props.onPress());
  expect(texts().some(text => String(text).includes("tasks.form.shortMonthHint"))).toBe(true);
  await act(async () => chip("tasks.triggerMode.weekly").props.onPress());
  expect(texts()).toContain("tasks.form.fullPermissionWarning");
});

test("reports a failed save instead of pretending it landed", async () => {
  await act(async () => firstTask().props.onPress());
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
  await act(async () => firstTask().props.onPress());
  await act(async () => button("common.back").props.onPress());
  expect(sections()).toContain("tasks.title");
});

test("summarises a manual task and a task with no runs", async () => {
  rows = [taskRow({ triggerKind: "manual", trigger: {}, latestRun: { id: "trn_9", kind: "manual", origin: "ui", status: "failed", resultSummary: null, errorMessage: "boom" } })];
  detail = taskDetail({ triggerKind: "manual", trigger: {}, latestRun: rows[0]!.latestRun });
  runs = [];
  deps = [];
  revisions = [];
  await remount();
  expect(texts().some(text => String(text).includes("tasks.trigger.manual"))).toBe(true);
  await act(async () => firstTask().props.onPress());
  expect(sections()).not.toContain("tasks.runs");
  expect(sections()).not.toContain("tasks.deps");
});

test("starts on a weekly trigger's selected weekdays", async () => {
  detail = taskDetail({ trigger: { mode: "weekly", days: ["mon", "fri"], time: "10:00" } });
  rows = [{ ...rows[0]!, trigger: detail.trigger }];
  await remount();
  await act(async () => firstTask().props.onPress());
  expect(inputUnder("tasks.form.days").props.value).toBe("mon,fri");
  expect(inputUnder("tasks.form.time").props.value).toBe("10:00");
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

test("shows what the task runs on, and offers the desktop's workspaces as its working directory", async () => {
  await remount();
  await act(async () => firstTask().props.onPress());
  const text = texts().join(" ");
  expect(text).toContain("tasks.colModel");
  expect(text).toContain("tasks.modelDefault");
  expect(text).toContain("tasks.colThinking");
  // The stored path is in the editable field, and each known workspace is a
  // one-tap candidate so a phone never has to type a path from memory.
  expect(inputUnder("tasks.form.cwdPath").props.value).toBe("/tmp/repo");
  expect(chip("future-os")).toBeTruthy();
  await act(async () => chip("notes").props.onPress());
  expect(inputUnder("tasks.form.cwdPath").props.value).toBe("/Users/me/notes");
});

test("sends the working directory picked from a workspace", async () => {
  await remount();
  await act(async () => firstTask().props.onPress());
  await act(async () => chip("notes").props.onPress());
  await act(async () => button("tasks.form.save").props.onPress());
  expect(mockRemote.updateTask).toHaveBeenCalledWith("tsk_1", expect.objectContaining({ cwd: "/Users/me/notes" }));
});

test("shows the loading state while the task list is in flight", async () => {
  let release!: (value: RemoteTaskRow[]) => void;
  mockRemote.listTasks.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
  await act(async () => { tree.unmount(); });
  await act(async () => { tree = create(createElement(TasksSettingsPage, { desktopOnline: true })); });
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
  expect(tree.root.findAllByType(TextInput)[0]!.props.value).toBe("summarize yesterday");
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
  await act(async () => firstTask().props.onPress());
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
