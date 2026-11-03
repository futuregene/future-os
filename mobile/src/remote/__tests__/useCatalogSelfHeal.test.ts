import { createElement, useState } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { CATALOG_SELF_HEAL_MS, useCatalogSelfHeal } from "../useCatalogSelfHeal";

/**
 * The slow self-heal read: a timer that must exist only while the app is in
 * front, fire on a fixed slow cadence, read once on returning to the front, and
 * stop the moment it is unmounted.
 */
describe("useCatalogSelfHeal", () => {
  let refresh: jest.Mock;
  let renderer: ReactTestRenderer | null;

  function Harness({ foreground, intervalMs }: { foreground: boolean; intervalMs?: number }): null {
    useCatalogSelfHeal(refresh, { foreground, intervalMs });
    return null;
  }

  const render = (foreground: boolean, intervalMs = CATALOG_SELF_HEAL_MS) => {
    act(() => {
      renderer = create(createElement(Harness, { foreground, intervalMs }));
    });
  };
  const rerender = (foreground: boolean, intervalMs = CATALOG_SELF_HEAL_MS) => {
    act(() => {
      renderer!.update(createElement(Harness, { foreground, intervalMs }));
    });
  };

  beforeEach(() => {
    jest.useFakeTimers();
    refresh = jest.fn();
    renderer = null;
  });

  afterEach(() => {
    if (renderer) {
      act(() => renderer!.unmount());
      renderer = null;
    }
    jest.useRealTimers();
  });

  test("does not read on mount, then reads on the slow clock", () => {
    render(true);
    // A runtime that has just appeared is connecting, and its own recovery reads
    // the catalogue on the way up; this adds nothing there.
    expect(refresh).not.toHaveBeenCalled();

    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS - 1));
    expect(refresh).not.toHaveBeenCalled();
    act(() => void jest.advanceTimersByTime(1));
    expect(refresh).toHaveBeenCalledTimes(1);
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS));
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  test("a backgrounded app runs no timer at all", () => {
    render(false);
    expect(refresh).not.toHaveBeenCalled();
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS * 3));
    expect(refresh).not.toHaveBeenCalled();
  });

  test("leaving the foreground stops the clock, returning reads at once", () => {
    render(true);
    expect(refresh).not.toHaveBeenCalled();

    rerender(false);
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS * 2));
    expect(refresh).not.toHaveBeenCalled();

    // The time spent in the background is exactly when a self-heal was skipped,
    // so coming back reads now rather than waiting out the cadence.
    rerender(true);
    expect(refresh).toHaveBeenCalledTimes(1);
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS));
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  test("unmounting cancels the timer", () => {
    render(true);
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS));
    const calls = refresh.mock.calls.length;
    expect(calls).toBe(1);
    act(() => renderer!.unmount());
    renderer = null;
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS * 4));
    expect(refresh).toHaveBeenCalledTimes(calls);
  });

  test("a new refresh callback is used without restarting the clock", () => {
    const second = jest.fn();
    let swap!: () => void;
    function Swapping({ foreground }: { foreground: boolean }): null {
      const [useSecond, setUseSecond] = useState(false);
      swap = () => setUseSecond(true);
      useCatalogSelfHeal(useSecond ? second : refresh, { foreground });
      return null;
    }
    act(() => {
      renderer = create(createElement(Swapping, { foreground: true }));
    });
    // Swap the callback without touching the interval: the clock was armed by the
    // first render and keeps running.
    act(() => swap());
    expect(refresh).not.toHaveBeenCalled();
    act(() => void jest.advanceTimersByTime(CATALOG_SELF_HEAL_MS));
    // The latest callback is the one the timer reaches, even though the interval
    // itself was armed by the first render.
    expect(refresh).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});
