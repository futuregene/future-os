/**
 * Minimal hook harness for jsdom tests: mounts a probe component with
 * `react-dom/client` inside `act` so effects, state updates and timers behave
 * like the real app. Each test file using this must opt into jsdom via
 * `// @vitest-environment jsdom` at the top.
 */
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

export interface HookHarness<R> {
  /** Hook return value from the latest committed render. */
  readonly current: R;
  /** Re-render the probe (e.g. after changing closed-over values). */
  rerender: () => void;
  unmount: () => void;
}

export function renderHook<R>(useHook: () => R): HookHarness<R> {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  let current!: R;
  function Probe() {
    current = useHook();
    return null;
  }
  act(() => {
    root.render(createElement(Probe));
  });
  return {
    get current() {
      return current;
    },
    rerender: () => {
      act(() => {
        root.render(createElement(Probe));
      });
    },
    unmount: () => {
      act(() => {
        root.unmount();
      });
      container.remove();
    },
  };
}

/** Flush pending promise continuations inside act (for async loaders). */
export async function flushAsync() {
  await act(async () => {
    await Promise.resolve();
  });
}
