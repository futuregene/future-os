import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState, type AppStateStatus } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useUpdateReminder } from "../useUpdateReminder";
import { DISMISSED_VERSION_KEY } from "../dismissedVersion";
import { checkForUpdate } from "../update";
import { promptUpgrade } from "../prompt";

const mockT = (key: string) => key;
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: mockT }) }));
jest.mock("../update", () => ({ checkForUpdate: jest.fn() }));
jest.mock("../prompt", () => ({ promptUpgrade: jest.fn() }));
jest.mock("@react-native-async-storage/async-storage", () => {
  const store = new Map<string, string>();
  return {
    __esModule: true,
    default: {
      getItem: jest.fn(async (key: string) => store.get(key) ?? null),
      setItem: jest.fn(async (key: string, value: string) => {
        store.set(key, value);
      }),
      removeItem: jest.fn(async (key: string) => {
        store.delete(key);
      }),
      __store: store,
    },
  };
});

const storage = AsyncStorage as unknown as {
  getItem: jest.Mock;
  setItem: jest.Mock;
  __store: Map<string, string>;
};
const check = jest.mocked(checkForUpdate);
const status = { currentVersion: "1.0.0", latestVersion: "1.1.0", hasUpdate: true, appStoreUrl: null, downloadUrl: "https://dl.future-os.cn/update.apk", canInstallInApp: true };
let tree: ReactTestRenderer;
let listener: (state: AppStateStatus) => void;
const remove = jest.fn();
function Harness() { useUpdateReminder(); return null; }
async function render() { await act(async () => { tree = create(createElement(Harness)); }); }
async function unmount() { await act(async () => { tree.unmount(); }); }
async function resume() {
  await act(async () => { AppState.currentState = "active"; listener("active"); });
}
/** The dismissal callback the reminder handed to the upgrade dialog. */
function onDismiss(call = 0): () => void {
  const callback = jest.mocked(promptUpgrade).mock.calls[call]?.[2] as (() => void) | undefined;
  if (!callback) throw new Error("promptUpgrade was called without a dismissal callback");
  return callback;
}
beforeEach(() => {
  jest.useFakeTimers();
  jest.clearAllMocks();
  storage.__store.clear();
  AppState.currentState = "active";
  check.mockResolvedValue(status);
  jest.spyOn(AppState, "addEventListener").mockImplementation((_type, fn) => {
    listener = fn;
    return { remove };
  });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.restoreAllMocks();
  jest.useRealTimers();
});

test("launch prompts once per version, while a later version is discovered during foreground use", async () => {
  await render();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
  await resume();
  expect(check).toHaveBeenCalledTimes(1);
  await act(async () => { jest.advanceTimersByTime(6 * 60 * 60 * 1000); });
  expect(check).toHaveBeenCalledTimes(2);
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
  check.mockResolvedValue({ ...status, latestVersion: "1.2.0" });
  await act(async () => { jest.advanceTimersByTime(6 * 60 * 60 * 1000); });
  expect(promptUpgrade).toHaveBeenCalledTimes(2);
});

test("offline launch retries and background checks wait until resume", async () => {
  check.mockRejectedValueOnce(new Error("offline"));
  await render();
  AppState.currentState = "background";
  await act(async () => { jest.advanceTimersByTime(60 * 1000); });
  expect(check).toHaveBeenCalledTimes(1);
  await resume();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
});

test("a check finishing in the background defers its dialog until the next resume", async () => {
  let resolve!: (value: typeof status) => void;
  check.mockReturnValueOnce(new Promise(done => { resolve = done; }));
  await render();
  await resume();
  expect(check).toHaveBeenCalledTimes(1);
  AppState.currentState = "background";
  await act(async () => { resolve(status); });
  expect(promptUpgrade).not.toHaveBeenCalled();
  await resume();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
});

test("unmount cancels listeners, timers and late dialog presentation", async () => {
  let resolve!: (value: typeof status) => void;
  check.mockReturnValueOnce(new Promise(done => { resolve = done; }));
  await render();
  act(() => tree.unmount());
  await act(async () => { resolve(status); jest.advanceTimersByTime(60 * 1000); });
  expect(remove).toHaveBeenCalledTimes(1);
  expect(check).toHaveBeenCalledTimes(1);
  expect(promptUpgrade).not.toHaveBeenCalled();
});

// "Later" answers about the build, not about this launch: the answer is
// written to the device and read back by the next cold start.
test("① a version answered with \"later\" is not asked again after a remount", async () => {
  await render();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
  await act(async () => { onDismiss()(); });
  expect(storage.__store.get(DISMISSED_VERSION_KEY)).toBe("1.1.0");
  await unmount();
  // A cold start: nothing survives but the storage the previous launch wrote.
  await render();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
  expect(storage.getItem).toHaveBeenCalled();
});

test("② a different stored version still prompts", async () => {
  storage.__store.set(DISMISSED_VERSION_KEY, "1.0.9");
  await render();
  expect(storage.getItem).toHaveBeenCalled();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
  expect(jest.mocked(promptUpgrade).mock.calls[0]?.[0]?.latestVersion).toBe("1.1.0");
});

test("③ unreadable storage falls back to prompting", async () => {
  storage.getItem.mockRejectedValueOnce(new Error("storage unavailable"));
  await render();
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
});

test("a failed write still closes the dialog and stays quiet for this session", async () => {
  storage.setItem.mockRejectedValueOnce(new Error("disk full"));
  await render();
  await act(async () => { onDismiss()(); });
  await act(async () => { jest.advanceTimersByTime(6 * 60 * 60 * 1000); });
  expect(check).toHaveBeenCalledTimes(2);
  expect(promptUpgrade).toHaveBeenCalledTimes(1);
});
