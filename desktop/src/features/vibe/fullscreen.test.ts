import { beforeEach, describe, expect, it, vi } from "vitest";
import { exitNativeFullscreen, toggleNativeFullscreen } from "./fullscreen";

/**
 * The window API, as `fullscreen.ts` uses it.
 *
 * One shared object rather than a fresh one per call: the module asks the window for its
 * state at the moment it needs it, so a test has to reach the same instance the code will.
 */
const win = vi.hoisted(() => ({
  fullscreen: false,
  isFullscreen: vi.fn(async () => win.fullscreen),
  setFullscreen: vi.fn(async (value: boolean) => {
    win.fullscreen = value;
  }),
}));

vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }));

beforeEach(() => {
  win.fullscreen = false;
  win.isFullscreen.mockClear();
  win.setFullscreen.mockClear();
});

describe("toggle native fullscreen", () => {
  it("enters fullscreen from windowed", async () => {
    await expect(toggleNativeFullscreen()).resolves.toEqual({ fullscreen: true, ok: true });
    expect(win.setFullscreen).toHaveBeenCalledWith(true);
  });

  it("leaves fullscreen from fullscreen", async () => {
    win.fullscreen = true;
    await expect(toggleNativeFullscreen()).resolves.toEqual({ fullscreen: false, ok: true });
    expect(win.setFullscreen).toHaveBeenCalledWith(false);
  });

  it("reports a refused toggle instead of swallowing it", async () => {
    const refusal = new Error("window.set_fullscreen not allowed");
    win.setFullscreen.mockRejectedValueOnce(refusal);
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(toggleNativeFullscreen()).resolves.toEqual({ error: refusal, ok: false });
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });
});

describe("exit native fullscreen", () => {
  it("leaves fullscreen", async () => {
    win.fullscreen = true;
    await exitNativeFullscreen();
    expect(win.setFullscreen).toHaveBeenCalledWith(false);
    expect(win.fullscreen).toBe(false);
  });

  it("asks even when the window reports windowed", async () => {
    // A window that has just been told to change state reports the old one for a
    // moment, and the report that prompted this is a panel closed in fullscreen that
    // left the reader fullscreen. `setFullscreen(false)` on a windowed window is a
    // no-op, so the request is unconditional and the *answer* is what decides whether
    // it is over.
    await exitNativeFullscreen();
    expect(win.setFullscreen).toHaveBeenCalledWith(false);
  });

  it("asks again when the first request is dropped", async () => {
    // Windows lost the request once during a re-layout; the reader was left in a
    // borderless window filling the screen with no panel to get out of it.
    win.fullscreen = true;
    win.setFullscreen.mockImplementationOnce(async () => {});
    await exitNativeFullscreen();
    expect(win.setFullscreen.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(win.fullscreen).toBe(false);
  });

  it("stays quiet when the window refuses", async () => {
    // Closing the panel happens either way: this is cleanup, not a request the reader
    // made, so a refusal is logged and no toast is raised.
    win.fullscreen = true;
    win.setFullscreen.mockRejectedValueOnce(new Error("window.set_fullscreen not allowed"));
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    await expect(exitNativeFullscreen()).resolves.toBeUndefined();
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });
});
