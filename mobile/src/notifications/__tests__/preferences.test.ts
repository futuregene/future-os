import {
  DEFAULT_TASK_FINISHED_REMINDERS,
  getTaskFinishedReminders,
  initializeTaskFinishedReminders,
  parseTaskFinishedReminders,
  setTaskFinishedReminders,
  subscribeTaskFinishedReminders,
} from "../preferences";

const mockGetItem = jest.fn(async (_key: string): Promise<string | null> => null);
const mockSetItem = jest.fn(async (_key: string, _value: string) => {});
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: (key: string) => mockGetItem(key),
  setItem: (key: string, value: string) => mockSetItem(key, value),
}));

/** A module instance of its own, whose one-shot restore has not run yet. */
function freshModule(): typeof import("../preferences") {
  let fresh!: typeof import("../preferences");
  jest.isolateModules(() => {
    // The restore is one-shot per module instance, so reading it again has to
    // run the module again. `import()` needs --experimental-vm-modules here.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    fresh = require("../preferences");
  });
  return fresh;
}

beforeEach(async () => {
  mockGetItem.mockReset().mockResolvedValue(null);
  mockSetItem.mockReset().mockResolvedValue(undefined);
  // Back to the delivered default, which is what a phone that never touched the
  // switch reports.
  await initializeTaskFinishedReminders();
  await setTaskFinishedReminders(DEFAULT_TASK_FINISHED_REMINDERS);
  mockGetItem.mockClear();
  mockSetItem.mockClear();
});

test("reminders are on for a phone that has never chosen", () => {
  expect(DEFAULT_TASK_FINISHED_REMINDERS).toBe(true);
  expect(freshModule().getTaskFinishedReminders()).toBe(true);
});

describe("reading a stored choice", () => {
  test("an absent record, foreign JSON and a non-boolean all mean the default", () => {
    expect(parseTaskFinishedReminders(null)).toBe(true);
    expect(parseTaskFinishedReminders("true")).toBe(true);
    expect(parseTaskFinishedReminders("false")).toBe(false);
    // Not our JSON, not a boolean, or a value this app never wrote.
    expect(parseTaskFinishedReminders("{")).toBe(true);
    expect(parseTaskFinishedReminders("null")).toBe(true);
    expect(parseTaskFinishedReminders('"off"')).toBe(true);
    expect(parseTaskFinishedReminders('{"enabled":false}')).toBe(true);
  });

  test("the stored value is restored on launch, once", async () => {
    const fresh = freshModule();
    mockGetItem.mockResolvedValue("false");
    await fresh.initializeTaskFinishedReminders();
    await fresh.initializeTaskFinishedReminders();
    expect(mockGetItem).toHaveBeenCalledTimes(1);
    expect(mockGetItem).toHaveBeenCalledWith("future.mobile.taskFinishedReminders");
    expect(fresh.getTaskFinishedReminders()).toBe(false);
  });

  test("unavailable storage keeps the default instead of failing launch", async () => {
    const fresh = freshModule();
    mockGetItem.mockRejectedValue(new Error("no storage"));
    await expect(fresh.initializeTaskFinishedReminders()).resolves.toBeUndefined();
    expect(fresh.getTaskFinishedReminders()).toBe(true);
  });
});

describe("saving a choice", () => {
  test("writes the flag and applies it, telling every reader", async () => {
    const listener = jest.fn();
    const unsubscribe = subscribeTaskFinishedReminders(listener);
    await setTaskFinishedReminders(false);
    expect(mockSetItem).toHaveBeenCalledWith("future.mobile.taskFinishedReminders", "false");
    expect(getTaskFinishedReminders()).toBe(false);
    expect(listener).toHaveBeenCalledTimes(1);
    // Writing the value that is already in effect is not a change: a switch
    // pressed twice must not look like two decisions.
    await setTaskFinishedReminders(false);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    await setTaskFinishedReminders(true);
    expect(getTaskFinishedReminders()).toBe(true);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  test("keeps the previous value when the write is refused", async () => {
    await setTaskFinishedReminders(false);
    mockSetItem.mockRejectedValueOnce(new Error("full"));
    await expect(setTaskFinishedReminders(true)).rejects.toThrow("full");
    expect(getTaskFinishedReminders()).toBe(false);
  });
});
