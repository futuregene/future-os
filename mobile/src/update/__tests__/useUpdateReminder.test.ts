import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState, type AppStateStatus } from "react-native";
import { useUpdateReminder } from "../useUpdateReminder";
import { checkForUpdate } from "../update";
import { promptUpgrade } from "../prompt";

const mockT = (key: string) => key;
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: mockT }) }));
jest.mock("../update", () => ({ checkForUpdate: jest.fn() }));
jest.mock("../prompt", () => ({ promptUpgrade: jest.fn() }));
const check = jest.mocked(checkForUpdate);
const status = { currentVersion: "1.0.0", latestVersion: "1.1.0", hasUpdate: true, appStoreUrl: null, downloadUrl: "https://dl.future-os.cn/update.apk", canInstallInApp: true };
let tree: ReactTestRenderer;
let listener: (state: AppStateStatus) => void;
const remove = jest.fn();
function Harness() { useUpdateReminder(); return null; }
async function render() { await act(async () => { tree = create(createElement(Harness)); }); }
async function resume() {
  await act(async () => { AppState.currentState = "active"; listener("active"); });
}
beforeEach(() => {
  jest.useFakeTimers();
  jest.clearAllMocks();
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
