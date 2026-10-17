import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useMinimumVisible } from "../useMinimumVisible";

/**
 * The notice's minimum visible time.
 *
 * A sync notice that appears and disappears within one status burst reads as a
 * flicker, so the hook holds it for a floor and cancels that hold the moment the
 * status comes back. These tests drive the transitions directly: the point is the
 * timer's lifecycle, not anything the screens draw.
 */
describe("useMinimumVisible", () => {
  let visible = false;
  let renderer: ReactTestRenderer | null = null;

  function Harness({ active, minimumMs, notice }: { active: boolean; minimumMs: number; notice: string }) {
    visible = useMinimumVisible(active, minimumMs, notice);
    return null;
  }

  function show(next: { active: boolean; minimumMs?: number; notice?: string }): void {
    const element = createElement(Harness, { minimumMs: 1_000, notice: "sync", ...next });
    act(() => {
      if (renderer) renderer.update(element);
      else renderer = create(element);
    });
  }

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
    jest.useRealTimers();
  });

  test("a status that comes back before the floor cancels the pending hide", () => {
    jest.useFakeTimers();
    show({ active: true });
    expect(visible).toBe(true);

    // The status goes away: the notice is held for the floor, so a hide timer
    // is armed rather than hiding it in the same frame.
    show({ active: false });
    expect(visible).toBe(true);
    expect(jest.getTimerCount()).toBe(1);

    // …and the status comes back before that timer fires. Re-running the effect
    // cancels the armed hide, so the late callback cannot hide a notice whose
    // status is current again.
    show({ active: true });
    expect(jest.getTimerCount()).toBe(0);
    act(() => { jest.advanceTimersByTime(5_000); });
    expect(visible).toBe(true);
  });

  test("the floor elapses into a hide, and a new notice starts its own window", () => {
    jest.useFakeTimers();
    show({ active: true });
    show({ active: false });
    act(() => { jest.advanceTimersByTime(1_000); });
    expect(visible).toBe(false);
    expect(jest.getTimerCount()).toBe(0);

    // A different notice is not the hidden one: it starts visible rather than
    // inheriting the presentation the previous status left behind.
    show({ active: true, notice: "upload" });
    expect(visible).toBe(true);
  });
});
