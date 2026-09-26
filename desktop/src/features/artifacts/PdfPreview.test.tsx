// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PdfPreview } from "./PdfPreview";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const convertFileSrc = vi.fn<(path: string) => string>();
const getDocument = vi.fn<(options: { url: string; wasmUrl: string }) => FakeLoadingTask>();
const destroy = vi.fn<() => Promise<void>>();
const getPage = vi.fn<(page: number) => Promise<FakePage>>();
const cleanup = vi.fn();
const cancel = vi.fn();
const render = vi.fn<(args: { canvas: HTMLCanvasElement; viewport: { height: number; width: number } }) => { cancel: () => void; promise: Promise<void> }>();

interface FakePage {
  cleanup: () => void;
  getViewport: (options: { scale: number }) => { height: number; width: number };
  render: typeof render;
}

interface FakeLoadingTask {
  destroy: typeof destroy;
  promise: Promise<FakeDocument>;
}

interface FakeDocument {
  getPage: typeof getPage;
  numPages: number;
}

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => convertFileSrc(path),
}));

vi.mock("pdfjs-dist", () => ({
  GlobalWorkerOptions: { workerSrc: "" },
  getDocument: (options: { url: string; wasmUrl: string }) => getDocument(options),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, reject, resolve };
}

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let renderDeferreds: Array<ReturnType<typeof deferred<void>>>;

function page(): FakePage {
  return {
    cleanup,
    getViewport: ({ scale }) => ({ height: Math.round(792 * scale), width: Math.round(612 * scale) }),
    render,
  };
}

function pdfDocument(numPages: number): FakeDocument {
  return { getPage, numPages };
}

function loadingTask(promise: Promise<FakeDocument>): FakeLoadingTask {
  return { destroy, promise };
}

async function mount(path = "/ws/doc.pdf") {
  await act(async () => {
    root.render(createElement(PdfPreview, { path }));
  });
}

async function flush() {
  await act(async () => {
    await Promise.resolve();
  });
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}
const PREV = "\u2190 Prev";
const NEXT = "Next \u2192";

beforeEach(() => {
  convertFileSrc.mockReset();
  getDocument.mockReset();
  destroy.mockReset();
  getPage.mockReset();
  cleanup.mockReset();
  cancel.mockReset();
  render.mockReset();

  convertFileSrc.mockImplementation(path => `asset:${path}`);
  destroy.mockResolvedValue(undefined);
  getPage.mockImplementation(async () => page());
  renderDeferreds = [];
  render.mockImplementation(() => {
    const pending = deferred<void>();
    renderDeferreds.push(pending);
    return { cancel, promise: pending.promise };
  });
  getDocument.mockImplementation(() => loadingTask(Promise.resolve(pdfDocument(3))));

  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("pdfPreview loading", () => {
  it("loads through the Tauri asset protocol and reports the page count", async () => {
    const cjkPath = "/ws/\u6587\u6863.pdf";
    const pending = deferred<FakeDocument>();
    getDocument.mockImplementation(() => loadingTask(pending.promise));
    await mount(cjkPath);
    expect(container.textContent).toContain("Loading PDF...");
    expect(convertFileSrc).toHaveBeenCalledWith(cjkPath);

    await act(async () => {
      pending.resolve(pdfDocument(3));
    });
    await flush();
    expect(getDocument).toHaveBeenCalledTimes(1);
    const options = getDocument.mock.calls[0]![0];
    expect(options.url).toBe(`asset:${cjkPath}`);
    // pdf.js resolves its wasm helpers against this base dir.
    expect(options.wasmUrl).toMatch(/pdfjs-wasm\/$/);

    expect(container.textContent).toContain("Page 1 / 3");
    expect(buttonByText(PREV)?.disabled).toBe(true);
    expect(buttonByText(NEXT)?.disabled).toBe(false);
    expect(container.textContent).not.toContain("Loading PDF...");
  });

  it("paints a canvas at the requested scale and releases the page", async () => {
    await mount();
    await flush();

    expect(getPage).toHaveBeenCalledWith(1);
    expect(render).toHaveBeenCalledTimes(1);
    const { canvas, viewport } = render.mock.calls[0]![0];
    expect(viewport).toEqual({ height: 1188, width: 918 });
    expect(canvas.width).toBe(918);
    expect(canvas.height).toBe(1188);
    expect(canvas.style.width).toBe("100%");
    expect(canvas.style.height).toBe("auto");
    expect(container.querySelector("canvas")).toBe(canvas);

    await act(async () => {
      renderDeferreds[0]!.resolve();
    });
    await flush();
    expect(cleanup).toHaveBeenCalledTimes(1);
  });

  it("reloads from the new path and resets to page 1", async () => {
    getDocument.mockImplementation(() => loadingTask(Promise.resolve(pdfDocument(5))));
    await mount("/ws/one.pdf");
    await flush();
    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    expect(container.textContent).toContain("Page 2 / 5");

    await act(async () => {
      root.render(createElement(PdfPreview, { path: "/ws/two.pdf" }));
    });
    await flush();
    expect(getDocument).toHaveBeenCalledTimes(2);
    expect(getDocument.mock.calls[1]![0].url).toBe("asset:/ws/two.pdf");
    expect(container.textContent).toContain("Page 1 / 5");
  });
});

describe("pdfPreview navigation", () => {
  it("moves forward and back one page at a time", async () => {
    await mount();
    await flush();

    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    expect(container.textContent).toContain("Page 2 / 3");
    expect(getPage).toHaveBeenLastCalledWith(2);
    expect(buttonByText(PREV)?.disabled).toBe(false);

    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    expect(container.textContent).toContain("Page 3 / 3");
    expect(buttonByText(NEXT)?.disabled).toBe(true);

    await act(async () => {
      buttonByText(PREV)?.click();
    });
    await flush();
    expect(container.textContent).toContain("Page 2 / 3");
    expect(getPage).toHaveBeenLastCalledWith(2);
  });

  it("clamps rapid paging at both ends", async () => {
    await mount();
    await flush();

    // Five Next clicks, one act apart, on a 3-page document.
    for (let index = 0; index < 5; index += 1) {
      await act(async () => {
        buttonByText(NEXT)?.click();
      });
      await flush();
    }
    expect(container.textContent).toContain("Page 3 / 3");
    expect(getPage).toHaveBeenLastCalledWith(3);

    for (let index = 0; index < 5; index += 1) {
      await act(async () => {
        buttonByText(PREV)?.click();
      });
      await flush();
    }
    expect(container.textContent).toContain("Page 1 / 3");
    expect(getPage).toHaveBeenLastCalledWith(1);
  });

  it("keeps the page index in range under programmatic activation of a disabled pager", async () => {
    // The pager buttons are disabled at the ends, which stops user activation —
    // but a synthetic/programmatic click still reaches the handler, so the
    // clamp in the updater is the invariant that actually holds the range.
    await mount();
    await flush();
    const next = () => buttonByText(NEXT)!;
    const prev = () => buttonByText(PREV)!;

    await act(async () => {
      next().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      next().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      next().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      next().dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flush();
    expect(container.textContent).toContain("Page 3 / 3");
    expect(getPage).toHaveBeenLastCalledWith(3);

    await act(async () => {
      prev().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      prev().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      prev().dispatchEvent(new MouseEvent("click", { bubbles: true }));
      prev().dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    await flush();
    expect(container.textContent).toContain("Page 1 / 3");
    expect(getPage).toHaveBeenLastCalledWith(1);
  });

  it("keeps both buttons disabled for a single-page document", async () => {
    getDocument.mockImplementation(() => loadingTask(Promise.resolve(pdfDocument(1))));
    await mount();
    await flush();
    expect(container.textContent).toContain("Page 1 / 1");
    expect(buttonByText(PREV)?.disabled).toBe(true);
    expect(buttonByText(NEXT)?.disabled).toBe(true);
  });

  it("abandons a page whose fetch lands after the view moved on", async () => {
    // The page lookup itself is slow: the effect is cleaned up (page flipped)
    // while `getPage` is still pending, so the result must be dropped and
    // nothing may be painted.
    const pendingPage = deferred<FakePage>();
    getPage.mockReturnValueOnce(pendingPage.promise);
    await mount();
    await flush();
    expect(render).not.toHaveBeenCalled();

    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    expect(render).toHaveBeenCalledTimes(1);
    expect(container.querySelectorAll("canvas")).toHaveLength(1);

    await act(async () => {
      pendingPage.resolve(page());
    });
    await flush();
    // The stale page-1 lookup did not take over the page-2 canvas.
    expect(container.textContent).toContain("Page 2 / 3");
    expect(container.querySelectorAll("canvas")).toHaveLength(1);
    expect(render).toHaveBeenCalledTimes(1);
  });

  it("cancels the superseded render when the page flips again", async () => {
    await mount();
    await flush();
    expect(render).toHaveBeenCalledTimes(1);

    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    // The first render was still pending, so switching pages cancels it instead
    // of letting it paint a stale page onto the canvas.
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(render).toHaveBeenCalledTimes(2);
  });
});

describe("pdfPreview failures", () => {
  it("reports a document that cannot be loaded", async () => {
    getDocument.mockImplementation(() => loadingTask(Promise.reject(new Error("Invalid PDF structure"))));
    await mount();
    await flush();
    expect(container.textContent).toContain("PDF Preview Error");
    expect(container.textContent).toContain("Invalid PDF structure");
    expect(container.querySelector("canvas")).toBeNull();
  });

  it("falls back to its own wording when the loader rejects with a non-Error", async () => {
    // eslint-disable-next-line prefer-promise-reject-errors -- the rejection is deliberately a non-Error: this test pins the fallback wording.
    getDocument.mockImplementation(() => loadingTask(Promise.reject("nope")));
    await mount();
    await flush();
    expect(container.textContent).toContain("Failed to load PDF");
  });

  it("reports a page that cannot be rendered", async () => {
    getPage.mockRejectedValue(new Error("page 1 is damaged"));
    await mount();
    await flush();
    expect(container.textContent).toContain("PDF Preview Error");
    expect(container.textContent).toContain("page 1 is damaged");
  });

  it("falls back to its own wording when the render rejects with a non-Error", async () => {
    getPage.mockRejectedValue({ reason: "unknown" });
    await mount();
    await flush();
    expect(container.textContent).toContain("Failed to render page");
  });

  it("keeps its own wording when the page render was cancelled by a superseded page", async () => {
    await mount();
    await flush();

    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    // The superseded render rejects once its task is cancelled; the `cancelled`
    // guard keeps that rejection out of the error banner.
    await act(async () => {
      renderDeferreds[0]!.reject(new Error("RenderingCancelledException"));
    });
    await flush();
    expect(container.textContent).not.toContain("RenderingCancelledException");
    expect(container.textContent).toContain("Page 2 / 3");
  });
});

describe("pdfPreview teardown", () => {
  it("destroys the loading task when the preview unmounts mid-load", async () => {
    const pending = deferred<FakeDocument>();
    getDocument.mockImplementation(() => loadingTask(pending.promise));
    await mount();
    expect(container.textContent).toContain("Loading PDF...");

    act(() => root.unmount());
    root = createRoot(container);
    expect(destroy).toHaveBeenCalledTimes(1);

    // The late resolution must not paint anything either.
    await act(async () => {
      pending.resolve(pdfDocument(2));
    });
    await flush();
    expect(destroy).toHaveBeenCalledTimes(2);
    expect(container.querySelector("canvas")).toBeNull();
  });

  it("destroys the previous task when the path changes", async () => {
    await mount("/ws/one.pdf");
    await flush();
    await act(async () => {
      root.render(createElement(PdfPreview, { path: "/ws/two.pdf" }));
    });
    await flush();
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(getDocument).toHaveBeenCalledTimes(2);
  });

  it("cancels an in-flight render on unmount", async () => {
    await mount();
    await flush();
    expect(cancel).not.toHaveBeenCalled();

    act(() => root.unmount());
    root = createRoot(container);
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("replaces the canvas rather than stacking pages", async () => {
    await mount();
    await flush();
    expect(container.querySelectorAll("canvas")).toHaveLength(1);

    await act(async () => {
      buttonByText(NEXT)?.click();
    });
    await flush();
    expect(container.querySelectorAll("canvas")).toHaveLength(1);
  });
});
