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
function advance(ms: number) { act(() => { jest.advanceTimersByTime(ms); }); }

beforeEach(() => {
  jest.useFakeTimers();
  jest.clearAllMocks();
  jest.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(false);
  jest.spyOn(AccessibilityInfo, "addEventListener");
  // The RN API is overloaded; this hook only subscribes to the boolean event.
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

test("initial streaming history is immediately readable; only appended text is revealed", async () => {
  await mount();
  expect(result.text).toBe("cached");
  const target = "cached " + "new content ".repeat(10);
  update(target);
  expect(result.text).toBe("cached");
  advance(32);
  expect(result.text.length).toBeGreaterThan(6);
  expect(result.text.length).toBeLessThan(target.length);
  expect(target.startsWith(result.text)).toBe(true);
  advance(192);
  expect(result.text).toBe(target);
});

test("frequent deltas and the final chunk finish within one bounded reveal window", async () => {
  await mount();
  update("cached " + "a".repeat(60));
  advance(32);
  const first = result.text;
  const final = "cached " + "a".repeat(60) + "b".repeat(60);
  update(final, false);
  advance(32);
  expect(result.text.startsWith(first)).toBe(true);
  advance(192);
  expect(result.text).toBe(final);
  expect(jest.getTimerCount()).toBe(0);
});

test("authoritative replacements cancel the old reveal instead of mixing transcripts", async () => {
  await mount();
  update("cached " + "a".repeat(100));
  advance(32);
  update("corrected content", false);
  expect(result.text).toBe("corrected content");
  advance(500);
  expect(result.text).toBe("corrected content");
});

test("reduced motion displays new content immediately and interrupts an active reveal", async () => {
  await mount();
  update("cached " + "a".repeat(100));
  advance(32);
  act(() => changeMotion(true));
  expect(result.text).toBe("cached " + "a".repeat(100));
  update("cached " + "a".repeat(100) + " done");
  expect(result.text.endsWith(" done")).toBe(true);
  expect(jest.getTimerCount()).toBe(0);
});

test("settled history does not animate or subscribe to accessibility changes", async () => {
  await mount("first", false);
  update("first plus history", false);
  expect(result.text).toBe("first plus history");
  expect(AccessibilityInfo.isReduceMotionEnabled).not.toHaveBeenCalled();
  expect(AccessibilityInfo.addEventListener).not.toHaveBeenCalled();
});

test("revealed boundaries do not split surrogate pairs", async () => {
  await mount("", true);
  const target = "😀".repeat(15);
  update(target);
  for (let index = 0; index < 6; index++) {
    advance(32);
    expect(result.text.length % 2).toBe(0);
  }
  expect(result.text).toBe(target);
});

test("unmount cancels animation work and removes the system preference listener", async () => {
  await mount();
  update("cached " + "a".repeat(100));
  advance(0); // Drain React's scheduled microtasks, not the 32ms reveal frame.
  expect(jest.getTimerCount()).toBe(1);
  act(() => tree.unmount());
  advance(0);
  expect(jest.getTimerCount()).toBe(0);
  expect(remove).toHaveBeenCalledTimes(1);
});
