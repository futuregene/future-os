import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState, type AppStateStatus } from "react-native";
import { useStopRequest } from "../useStopRequest";

function deferred() {
  let resolve!: () => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

let tree: ReactTestRenderer;
let current: ReturnType<typeof useStopRequest>;
let info: jest.SpyInstance;
function Harness({ streaming = true, sessionId = "s1", abort }: {
  streaming?: boolean; sessionId?: string; abort: () => Promise<void>;
}) {
  const value = useStopRequest(streaming, sessionId, abort);
  useEffect(() => { current = value; }, [value]);
  return null;
}
const render = (props: Parameters<typeof Harness>[0]) => act(() => {
  if (tree) tree.update(createElement(Harness, props));
  else tree = create(createElement(Harness, props));
});
const stages = () => info.mock.calls.map(([, data]) => data.stage);

beforeEach(() => {
  jest.useFakeTimers();
  info = jest.spyOn(console, "info").mockImplementation(() => {});
});
afterEach(() => {
  if (tree) act(() => tree.unmount());
  tree = undefined as never;
  jest.restoreAllMocks();
  jest.useRealTimers();
});

test("one click dispatches once, coalesces concurrent taps, and waits for authoritative idle after ACK", async () => {
  const pending = deferred();
  const abort = jest.fn(() => pending.promise);
  render({ abort });
  let request!: Promise<void>;
  act(() => {
    request = current.stop();
    void current.stop();
  });
  expect(abort).toHaveBeenCalledTimes(1);
  expect(current.status).toBe("requesting");
  act(() => jest.advanceTimersByTime(120));
  await act(async () => { pending.resolve(); await request; });
  expect(current.status).toBe("requested");
  expect(stages()).toEqual(["press_received", "request_acknowledged"]);
  expect(info.mock.calls[1]![1]).toMatchObject({ sessionId: "s1", elapsedMs: 120 });
  act(() => jest.advanceTimersByTime(380));
  render({ abort, streaming: false });
  expect(current.status).toBe("idle");
  expect(info.mock.calls[2]![1]).toMatchObject({ stage: "streaming_cleared", elapsedMs: 500 });
  expect(jest.getTimerCount()).toBe(0);
});

test("failure is handled, visible and retryable without an automatic second abort", async () => {
  const abort = jest.fn().mockRejectedValueOnce(new Error("timeout")).mockResolvedValue(undefined);
  render({ abort });
  await act(async () => current.stop());
  expect(current.status).toBe("failed");
  expect(abort).toHaveBeenCalledTimes(1);
  expect(stages()).toEqual(["press_received", "request_failed"]);
  await act(async () => current.stop());
  expect(current.status).toBe("requested");
  expect(abort).toHaveBeenCalledTimes(2);
});

test("ACK does not disable an explicit retry if the run has not stopped", async () => {
  const abort = jest.fn(async () => {});
  render({ abort });
  await act(async () => current.stop());
  await act(async () => current.stop());
  expect(abort).toHaveBeenCalledTimes(2);
  expect(current.status).toBe("requested");
});

test.each(["resolve", "reject"] as const)("idle before a late %s never restores the pending UI", async outcome => {
  const pending = deferred();
  const abort = jest.fn(() => pending.promise);
  render({ abort });
  let request!: Promise<void>;
  act(() => { request = current.stop(); });
  render({ abort, streaming: false });
  await act(async () => {
    if (outcome === "resolve") pending.resolve();
    else pending.reject(new Error("late timeout"));
    await request;
  });
  expect(current.status).toBe("idle");
  await act(async () => current.stop());
  expect(abort).toHaveBeenCalledTimes(1);
});

test("switching session discards old state and late replies; detaching is not a stop confirmation", async () => {
  const pending = deferred();
  const abort = jest.fn(() => pending.promise);
  render({ abort });
  let request!: Promise<void>;
  act(() => { request = current.stop(); });
  render({ abort, sessionId: "s2" });
  expect(current.status).toBe("idle");
  await act(async () => { pending.resolve(); await request; });
  expect(current.status).toBe("idle");
  expect(stages()).toEqual(["press_received", "view_detached", "request_acknowledged"]);
  expect(info.mock.calls.every(([, data]) => data.sessionId === "s1")).toBe(true);
});

test("streaming rerenders preserve a pending click and dispatch through the latest callback", async () => {
  const pending = deferred();
  const first = jest.fn(() => pending.promise);
  const latest = jest.fn(async () => {});
  render({ abort: first });
  let request!: Promise<void>;
  act(() => { request = current.stop(); });
  for (let i = 0; i < 100; i++) render({ abort: latest });
  expect(current.status).toBe("requesting");
  await act(async () => { pending.resolve(); await request; });
  await act(async () => current.stop());
  expect(first).toHaveBeenCalledTimes(1);
  expect(latest).toHaveBeenCalledTimes(1);
});

test("samples foreground JS scheduling lag without counting time in the background", async () => {
  let stateChanged!: (state: AppStateStatus) => void;
  jest.spyOn(AppState, "addEventListener").mockImplementation((_, handler) => {
    stateChanged = handler;
    return { remove: jest.fn() };
  });
  const now = jest.spyOn(performance, "now").mockReturnValue(0);
  render({ abort: async () => {} });
  act(() => stateChanged("active"));
  now.mockReturnValue(900); // The 250ms callback was delayed by 650ms.
  act(() => jest.advanceTimersByTime(250));
  await act(async () => current.stop());
  expect(info.mock.calls[0]![1].recentJsLagMs).toBe(650);
  const foregroundTimers = jest.getTimerCount();
  act(() => stateChanged("background"));
  expect(jest.getTimerCount()).toBe(foregroundTimers - 1);
  now.mockReturnValue(60_000);
  act(() => stateChanged("active"));
  await act(async () => current.stop());
  expect(info.mock.calls[2]![1].recentJsLagMs).toBe(0);
});

test("a lag sample older than the two-second window is dropped, not carried forward", async () => {
  let stateChanged!: (state: AppStateStatus) => void;
  jest.spyOn(AppState, "addEventListener").mockImplementation((_, handler) => {
    stateChanged = handler;
    return { remove: jest.fn() };
  });
  const now = jest.spyOn(performance, "now").mockReturnValue(0);
  render({ abort: async () => {} });
  act(() => stateChanged("active"));
  // First tick reports a 19.75 s stall (nextTick was armed at t=250).
  now.mockReturnValue(20_000);
  act(() => jest.advanceTimersByTime(250));
  // Second tick lands 10 s later, so the stale sample falls out of the window.
  now.mockReturnValue(30_000);
  act(() => jest.advanceTimersByTime(250));
  await act(async () => current.stop());
  // 9750 is the second sample alone: had the stale 19750 survived, the report
  // would still show it and every later stop would inherit a phantom stall.
  expect(info.mock.calls[0]![1].recentJsLagMs).toBe(9750);
});

