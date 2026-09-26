// @vitest-environment jsdom
import type { ReactElement } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../lib/futureEvents";
import { ToastHost } from "./ToastHost";

const DISMISS_MS = 3200;

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

const items = (container: HTMLElement) => [...container.querySelectorAll(".pointer-events-auto")];

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("toast host", () => {
  it("renders nothing until a toast arrives", () => {
    const view = mount(<ToastHost />);
    expect(view.container.textContent).toBe("");
    view.unmount();
  });

  it("stacks info and error toasts with their tone styling", () => {
    const view = mount(<ToastHost />);
    act(() => {
      emitFutureEvent("toast", { message: "Saved", tone: "info" });
    });
    act(() => {
      emitFutureEvent("toast", { message: "Could not save", tone: "error" });
    });

    const [info, error] = items(view.container);
    expect(info!.textContent).toBe("Saved");
    expect(info!.className).toContain("border-line-soft");
    expect(info!.className).not.toContain("border-danger-line");
    expect(error!.textContent).toBe("Could not save");
    expect(error!.className).toContain("border-danger-line");
    expect(error!.className).toContain("text-danger");
    view.unmount();
  });

  it("defaults a toast without a tone to info", () => {
    const view = mount(<ToastHost />);
    act(() => {
      emitFutureEvent("toast", { message: "Heads up" });
    });
    const [only] = items(view.container);
    expect(only!.textContent).toBe("Heads up");
    expect(only!.className).toContain("border-line-soft");
    view.unmount();
  });

  it("auto-dismisses each toast after its timeout, independently", () => {
    const view = mount(<ToastHost />);
    act(() => {
      emitFutureEvent("toast", { message: "first" });
    });
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    act(() => {
      emitFutureEvent("toast", { message: "second" });
    });

    act(() => {
      vi.advanceTimersByTime(DISMISS_MS - 1000);
    });
    // The first toast has expired; the newer one is still inside its window.
    expect(view.container.textContent).toBe("second");

    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(view.container.textContent).toBe("");
    view.unmount();
  });

  it("clears its dismiss timers on unmount", () => {
    const view = mount(<ToastHost />);
    act(() => {
      emitFutureEvent("toast", { message: "bye" });
    });
    expect(view.container.textContent).toBe("bye");
    view.unmount();
    expect(() => {
      act(() => {
        vi.advanceTimersByTime(DISMISS_MS * 2);
      });
    }).not.toThrow();
  });
});
