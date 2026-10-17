// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ImagePreview } from "./ImagePreview";

/**
 * The image overlay's interaction layer: measure the container, zoom, pan, and
 * fall back to the OS handler when the bytes cannot be read.
 *
 * jsdom supplies neither a layout engine nor `ResizeObserver`, so the two seams
 * this component reads the world through are stubbed here: `getBoundingClientRect`
 * (the container's box, and hence every zoom anchor) and `ResizeObserver` (which
 * is what notices the box change). Both are mutable so a test can resize the
 * "window" the way a real one would.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const mocks = vi.hoisted(() => ({
  prepare: vi.fn<(path: string) => Promise<string | null>>(),
}));

vi.mock("../../integrations/storage/files", () => ({
  prepareImagePreviewUrl: (path: string) => mocks.prepare(path),
}));

/** The stubbed container box. Tests mutate it and then fire the resize seams. */
let box = { height: 300, width: 400 };
const observers: Array<() => void> = [];
let disconnects = 0;

class ResizeObserverStub {
  constructor(callback: () => void) {
    observers.push(callback);
  }

  disconnect(): void {
    disconnects += 1;
  }

  observe(): void {}

  unobserve(): void {}
}

function rectOf(): DOMRect {
  return {
    bottom: box.height,
    height: box.height,
    left: 0,
    right: box.width,
    top: 0,
    width: box.width,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  } as DOMRect;
}

let roots: Array<{ container: HTMLElement; root: ReturnType<typeof createRoot> }> = [];

beforeEach(() => {
  roots = [];
  observers.length = 0;
  disconnects = 0;
  box = { height: 300, width: 400 };
  mocks.prepare.mockReset().mockResolvedValue("asset://localhost/image.png");
  vi.stubGlobal("ResizeObserver", ResizeObserverStub);
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => rectOf());
});

afterEach(() => {
  for (const { container, root } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function flush(times = 4): Promise<void> {
  for (let i = 0; i < times; i += 1) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

async function mount(onError = vi.fn()) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(<ImagePreview name="shot.png" onError={onError} path="/tmp/shot.png" />);
  });
  await flush();
  return { container, onError, root };
}

/** Publish the decoded bytes' size, the way the browser does on `load`. */
async function loadImage(container: HTMLElement, natural = { height: 600, width: 800 }) {
  const image = container.querySelector("img")!;
  Object.defineProperty(image, "naturalHeight", { configurable: true, value: natural.height });
  Object.defineProperty(image, "naturalWidth", { configurable: true, value: natural.width });
  await act(async () => {
    image.dispatchEvent(new Event("load"));
  });
  await flush();
  return image;
}

/** The overlay is ready to interact with: bytes read, image decoded. */
async function mountReady(onError = vi.fn()) {
  const view = await mount(onError);
  const image = await loadImage(view.container);
  return { ...view, image };
}

function overlay(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>("div[role='presentation']")!;
}

/** The zoom percentage the controls display. */
function shownScale(container: HTMLElement): number {
  const text = [...container.querySelectorAll("span")].map(node => node.textContent ?? "").find(value => value.endsWith("%"));
  return Number.parseInt(text ?? "", 10);
}

function translation(image: HTMLElement): { x: number; y: number } {
  const match = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(image.style.transform);
  return { x: Number(match?.[1] ?? 0), y: Number(match?.[2] ?? 0) };
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(node => node.getAttribute("aria-label") === label)!;
}

describe("imagePreview zoom and pan", () => {
  it("waits for the bytes, then shows the image only once it has decoded", async () => {
    const view = await mount();
    // Bytes read, image not decoded: mounted (decoding is part of the wait) but
    // not painted.
    const image = view.container.querySelector("img")!;
    expect(image.getAttribute("src")).toBe("asset://localhost/image.png");
    expect(image.className).toContain("invisible");
    // No controls while nothing is on screen to control.
    expect(view.container.querySelector("button")).toBeNull();

    await loadImage(view.container);
    expect(image.className).toContain("visible");
    expect(image.className).not.toContain("invisible");
    // Fit, because the 800x600 image is larger than the 400x300 container.
    expect(shownScale(view.container)).toBe(100);
    expect(translation(image)).toEqual({ x: 0, y: 0 });
  });

  it("renders nothing while the first read is still pending", async () => {
    let settle!: (src: string | null) => void;
    mocks.prepare.mockReturnValue(new Promise<string | null>((resolve) => {
      settle = resolve;
    }));

    const view = await mount();
    expect(view.container.innerHTML).toBe("");

    await act(async () => {
      settle("asset://localhost/late.png");
    });
    await flush();
    expect(view.container.querySelector("img")).not.toBeNull();
  });

  it("zooms around the wheel's cursor, clamped to the zoom range", async () => {
    const view = await mountReady();
    const surface = overlay(view.container);

    // A wheel "click" upward zooms in; the anchor is where the pointer is, so
    // the translation moves to keep that pixel under it.
    await act(async () => {
      surface.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, clientX: 400, clientY: 0, deltaY: -200 }));
    });
    expect(shownScale(view.container)).toBeGreaterThan(100);
    const zoomed = translation(view.image);
    expect(zoomed.x).not.toBe(0);

    // Holding a wheel gesture cannot run past MAX_SCALE: the controls stop
    // reporting growth.
    for (let i = 0; i < 40; i += 1) {
      await act(async () => {
        surface.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, clientX: 200, clientY: 150, deltaY: -200 }));
      });
    }
    expect(shownScale(view.container)).toBe(600);

    // And scrolling down comes back, never below "fit".
    for (let i = 0; i < 80; i += 1) {
      await act(async () => {
        surface.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, clientX: 200, clientY: 150, deltaY: 200 }));
      });
    }
    expect(shownScale(view.container)).toBe(100);
    expect(translation(view.image)).toEqual({ x: 0, y: 0 });
  });

  it("toggles between fit and actual size on a double click", async () => {
    const view = await mountReady();
    const surface = overlay(view.container);

    await act(async () => {
      surface.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: 200, clientY: 150 }));
    });
    // "Actual size": the 800px-wide image is drawn at 2x the 400px container.
    expect(shownScale(view.container)).toBe(200);

    await act(async () => {
      surface.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: 200, clientY: 150 }));
    });
    expect(shownScale(view.container)).toBe(100);
  });

  it("pans on a left drag, clamped to the overhang", async () => {
    const view = await mountReady();
    const surface = overlay(view.container);
    await act(async () => {
      button(view.container, "Zoom in").click();
    });
    expect(shownScale(view.container)).toBe(125);

    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 200, clientY: 150 }));
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 700, clientY: 150 }));
    });
    // Effective image width is 800 * 0.5 = 400 px, scaled by 1.25 = 500 in a
    // 400 px container: 50 px of overhang per side, so the drag stops there.
    expect(translation(view.image).x).toBe(50);
    expect(view.image.className).toContain("cursor-grabbing");

    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    });
    expect(view.image.className).toContain("cursor-grab");
  });

  it("ignores a non-left press, and ends a drag that leaves the window", async () => {
    const view = await mountReady();
    const surface = overlay(view.container);
    await act(async () => {
      button(view.container, "Zoom in").click();
    });

    // A right-click (or one a child control already handled) must not arm a pan.
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 2, clientX: 200, clientY: 150 }));
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 900, clientY: 150 }));
    });
    expect(translation(view.image)).toEqual({ x: 0, y: 0 });
    expect(view.image.className).not.toContain("cursor-grabbing");

    // A drag that ends outside the overlay still ends: the wipe is global.
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 200, clientY: 150 }));
    });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 700, clientY: 150 }));
    });
    expect(translation(view.image)).toEqual({ x: 0, y: 0 });
  });

  it("re-measures its container on a resize and on the observer, re-clamping the pan", async () => {
    const view = await mountReady();
    const surface = overlay(view.container);
    await act(async () => {
      button(view.container, "Zoom in").click();
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 200, clientY: 150 }));
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 700, clientY: 150 }));
    });
    expect(translation(view.image).x).toBe(50);

    // The overlay grows past the zoomed image: the pan it allowed is no longer
    // a legal position, and the re-clamp is what keeps the picture centred.
    box = { height: 900, width: 1_200 };
    await act(async () => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(translation(view.image).x).toBe(0);

    // The observer is the other seam, for a box change no window resize
    // announces (a panel opening beside the overlay). Shrinking back is what
    // makes it observable: with room to pan again, the drag can only reach the
    // overhang if the *new* box was measured.
    box = { height: 300, width: 400 };
    await act(async () => {
      observers.forEach(callback => callback());
    });
    expect(observers).toHaveLength(1);
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0, clientX: 200, clientY: 150 }));
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("mousemove", { bubbles: true, clientX: 700, clientY: 150 }));
    });
    expect(translation(view.image).x).toBe(50);

    // Both seams are released with the component.
    const before = disconnects;
    act(() => view.root.unmount());
    roots = roots.filter(entry => entry.root !== view.root);
    expect(disconnects).toBeGreaterThan(before);
  });

  it("does nothing when the overlay is not in the document", async () => {
    // A wheel or a double click arriving on a container the host has already
    // detached has no box to anchor to; the guards must swallow it rather than
    // compute a zoom from `undefined`.
    const view = await mountReady();
    const surface = overlay(view.container);
    vi.mocked(Element.prototype.getBoundingClientRect).mockReturnValue(undefined as unknown as DOMRect);

    await act(async () => {
      surface.dispatchEvent(new WheelEvent("wheel", { bubbles: true, cancelable: true, clientX: 10, clientY: 10, deltaY: -200 }));
    });
    await act(async () => {
      surface.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, clientX: 10, clientY: 10 }));
    });

    expect(shownScale(view.container)).toBe(100);
  });

  it("hands an unreadable image back to the caller's fallback", async () => {
    const onError = vi.fn();
    const view = await mountReady(onError);
    expect(onError).not.toHaveBeenCalled();

    await act(async () => {
      view.image.dispatchEvent(new Event("error"));
    });
    await flush();

    expect(onError).toHaveBeenCalledTimes(1);
    // The overlay stops offering controls for a picture it could not paint.
    expect(view.container.querySelector("button")).toBeNull();
  });

  it("reports a failed read the same way, before an image ever exists", async () => {
    mocks.prepare.mockRejectedValue(new Error("too large"));
    const onError = vi.fn();
    await mount(onError);
    await flush();

    expect(onError).toHaveBeenCalledTimes(1);
  });

  it("zooms from the controls and resets back to fit", async () => {
    const view = await mountReady();

    await act(async () => {
      button(view.container, "Zoom in").click();
      button(view.container, "Zoom in").click();
    });
    expect(shownScale(view.container)).toBe(156);

    await act(async () => {
      button(view.container, "Zoom out").click();
    });
    expect(shownScale(view.container)).toBe(125);

    await act(async () => {
      button(view.container, "Reset zoom").click();
    });
    expect(shownScale(view.container)).toBe(100);
    expect(translation(view.image)).toEqual({ x: 0, y: 0 });
  });
});
