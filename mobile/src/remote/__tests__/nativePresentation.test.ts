import { AppState, type AppStateStatus } from "react-native";
import {
  withNativeHandoff,
  beginNativePresentation,
  endNativePresentation,
  nativePresentationInFlight,
  withNativePresentation,
} from "../nativePresentation";

afterEach(() => {
  // Keep the process-global depth clean for the next test.
  while (nativePresentationInFlight()) endNativePresentation();
});

test("reports a presentation for as long as the caller is waiting", async () => {
  expect(nativePresentationInFlight()).toBe(false);
  let release!: () => void;
  const pending = withNativePresentation(
    () =>
      new Promise<void>(resolve => {
        release = resolve;
      }),
  );
  expect(nativePresentationInFlight()).toBe(true);
  release();
  await pending;
  expect(nativePresentationInFlight()).toBe(false);
});

test("releases on failure so a cancelled picker cannot hold the connection", async () => {
  await expect(
    withNativePresentation(async () => {
      throw new Error("attachment_camera_permission");
    }),
  ).rejects.toThrow("attachment_camera_permission");
  expect(nativePresentationInFlight()).toBe(false);
});

describe("Android handoff without an activity result", () => {
  let onState: (state: AppStateStatus) => void;
  const remove = jest.fn();
  beforeEach(() => {
    jest.useFakeTimers();
    remove.mockClear();
    jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
      onState = listener;
      return { remove };
    });
  });
  afterEach(() => {
    jest.runOnlyPendingTimers();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  test("acknowledges launch immediately, releases grace on background/foreground", async () => {
    await withNativeHandoff(async () => {});
    expect(nativePresentationInFlight()).toBe(true);
    onState("active"); // A redundant active event is not a round trip.
    expect(nativePresentationInFlight()).toBe(true);
    onState("background");
    onState("active");
    expect(nativePresentationInFlight()).toBe(false);
    expect(remove).toHaveBeenCalledTimes(1);
    jest.runOnlyPendingTimers();
    expect(remove).toHaveBeenCalledTimes(1);
  });

  test("missing lifecycle callbacks cannot keep grace forever or block a second share", async () => {
    const launch = jest.fn(async () => {});
    await withNativeHandoff(launch);
    await withNativeHandoff(launch);
    expect(launch).toHaveBeenCalledTimes(2);
    expect(nativePresentationInFlight()).toBe(true);
    jest.advanceTimersByTime(60_000);
    expect(nativePresentationInFlight()).toBe(false);
    expect(remove).toHaveBeenCalledTimes(2);
  });

  test("launch failure clears grace immediately", async () => {
    await expect(withNativeHandoff(async () => { throw new Error("No activity"); })).rejects.toThrow("No activity");
    expect(nativePresentationInFlight()).toBe(false);
    expect(remove).toHaveBeenCalledTimes(1);
  });
});

test("a second round trip after the grace was released cannot release twice", async () => {
  const onState = jest.fn();
  const remove = jest.fn();
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
    onState.mockImplementation(listener);
    return { remove };
  });
  jest.useFakeTimers();
  try {
    await withNativeHandoff(async () => {});
    onState("background");
    onState("active");
    expect(nativePresentationInFlight()).toBe(false);
    // The activity returns a second time (a chooser reopened and dismissed).
    onState("background");
    onState("active");
    // The handle was already released: the grace must not be released twice,
    // which would drive the process-global depth negative.
    expect(nativePresentationInFlight()).toBe(false);
    expect(remove).toHaveBeenCalledTimes(1);
  } finally {
    jest.useRealTimers();
    jest.restoreAllMocks();
  }
});

test("nested presentations unwind one at a time", async () => {
  beginNativePresentation();
  beginNativePresentation();
  endNativePresentation();
  expect(nativePresentationInFlight()).toBe(true);
  endNativePresentation();
  expect(nativePresentationInFlight()).toBe(false);
  // An unbalanced release can never drive the counter negative.
  endNativePresentation();
  expect(nativePresentationInFlight()).toBe(false);
});
