// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { startWindowDrag } from "./windowDrag";

const mocks = vi.hoisted(() => ({ startDragging: vi.fn(), getCurrentWindow: vi.fn() }));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: (...args: unknown[]) => mocks.getCurrentWindow(...args),
}));

/** A mouse event whose `target` is forced, since real targets are read-only. */
function mouseDown(target: EventTarget | null, button = 0) {
  const event = new MouseEvent("mousedown", { bubbles: true, button });
  Object.defineProperty(event, "target", { value: target });
  const preventDefault = vi.spyOn(event, "preventDefault");
  return { event, preventDefault };
}

let removeAllRanges: ReturnType<typeof vi.fn>;

beforeEach(() => {
  removeAllRanges = vi.fn();
  vi.spyOn(document, "getSelection").mockReturnValue({ removeAllRanges } as never);
  mocks.startDragging.mockReset();
  mocks.startDragging.mockResolvedValue(undefined);
  mocks.getCurrentWindow.mockReset();
  mocks.getCurrentWindow.mockReturnValue({ startDragging: mocks.startDragging });
});

afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("startWindowDrag", () => {
  it("starts a window drag from a plain area, clearing the text selection first", () => {
    const target = document.createElement("div");
    const { event, preventDefault } = mouseDown(target);

    startWindowDrag(event as never);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(mocks.getCurrentWindow).toHaveBeenCalledTimes(1);
    expect(mocks.startDragging).toHaveBeenCalledTimes(1);
    expect(removeAllRanges).toHaveBeenCalledTimes(1);
  });

  it.each([1, 2])("ignores the non-primary button %d", (button) => {
    const target = document.createElement("div");
    const { event, preventDefault } = mouseDown(target, button);

    startWindowDrag(event as never);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(mocks.getCurrentWindow).not.toHaveBeenCalled();
    expect(mocks.startDragging).not.toHaveBeenCalled();
  });

  it.each(["button", "input", "textarea", "select", "a"])(
    "leaves a drag starting on an interactive <%s> to that control",
    (tag) => {
      const control = document.createElement(tag);
      const target = document.createElement("span");
      control.append(target);
      const { event, preventDefault } = mouseDown(target);

      startWindowDrag(event as never);

      expect(preventDefault).not.toHaveBeenCalled();
      expect(mocks.startDragging).not.toHaveBeenCalled();
    },
  );

  it("treats a click on the interactive element itself as interactive too", () => {
    const target = document.createElement("button");
    const { event } = mouseDown(target);

    startWindowDrag(event as never);

    expect(mocks.startDragging).not.toHaveBeenCalled();
  });

  it("still drags from a non-Element target (e.g. the window itself)", () => {
    const { event } = mouseDown(null);

    startWindowDrag(event as never);

    expect(mocks.startDragging).toHaveBeenCalledTimes(1);
  });

  it("tolerates a document without a selection API", () => {
    vi.spyOn(document, "getSelection").mockReturnValue(null);
    const target = document.createElement("div");
    const { event, preventDefault } = mouseDown(target);

    expect(() => startWindowDrag(event as never)).not.toThrow();
    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(mocks.startDragging).toHaveBeenCalledTimes(1);
  });

  it("swallows a rejected startDragging (no unhandled rejection, no throw)", async () => {
    mocks.startDragging.mockRejectedValue(new Error("no window"));
    const target = document.createElement("div");
    const { event } = mouseDown(target);

    startWindowDrag(event as never);
    await Promise.resolve();

    expect(mocks.startDragging).toHaveBeenCalledTimes(1);
  });
});
