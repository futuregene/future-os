import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AccessibilityInfo } from "react-native";
import { useStreamingText } from "../useStreamingText";

let tree: ReactTestRenderer;
let result: ReturnType<typeof useStreamingText>;
let changeMotion: (enabled: boolean) => void;
const remove = jest.fn();
function Harness({ text, streaming }: { text: string; streaming: boolean }) {
  const value = useStreamingText(text, streaming);
  useEffect(() => { result = value; });
  return null;
}
async function mount(text = "cached", streaming = true) {
  await act(async () => { tree = create(createElement(Harness, { text, streaming })); });
}
function update(text: string, streaming = true) {
  act(() => { tree.update(createElement(Harness, { text, streaming })); });
}
beforeEach(() => {
  jest.useFakeTimers();
  jest.clearAllMocks();
  jest.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(false);
  jest.spyOn(AccessibilityInfo, "addEventListener");
  (AccessibilityInfo.addEventListener as jest.Mock).mockImplementation((_event: string, listener: (enabled: boolean) => void) => {
    changeMotion = listener;
    return { remove };
  });
});
afterEach(() => {
  act(() => tree?.unmount());
  jest.restoreAllMocks();
  jest.useRealTimers();
});

test("coalesced commits display immediately without a second typewriter timer", async () => {
  await mount();
  expect(result.text).toBe("cached");
  for (let i = 1; i <= 100; i++) {
    const text = "cached " + "中文😀".repeat(i);
    update(text);
    expect(result.text).toBe(text);
  }
  act(() => jest.advanceTimersByTime(0));
  expect(jest.getTimerCount()).toBe(0);
});

test("terminal content and replacements never wait for a reveal animation", async () => {
  await mount();
  update("cached " + "a".repeat(100));
  update("cached " + "a".repeat(100) + " done", false);
  expect(result.text.endsWith(" done")).toBe(true);
  update("corrected content", false);
  expect(result.text).toBe("corrected content");
  act(() => jest.advanceTimersByTime(500));
  expect(jest.getTimerCount()).toBe(0);
});

test("reduced motion still controls new-block fades without changing content", async () => {
  await mount();
  expect(result.reduceMotion).toBe(false);
  act(() => changeMotion(true));
  expect(result.reduceMotion).toBe(true);
  expect(result.text).toBe("cached");
});

test("settled history does not subscribe to accessibility changes", async () => {
  await mount("first", false);
  update("first plus history", false);
  expect(result.text).toBe("first plus history");
  expect(AccessibilityInfo.isReduceMotionEnabled).not.toHaveBeenCalled();
  expect(AccessibilityInfo.addEventListener).not.toHaveBeenCalled();
});

test("unmount removes the system preference listener", async () => {
  await mount();
  act(() => tree.unmount());
  expect(remove).toHaveBeenCalledTimes(1);
});
