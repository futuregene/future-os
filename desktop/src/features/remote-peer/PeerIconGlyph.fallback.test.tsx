// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";

/**
 * The glyph component with an incomplete icon table.
 *
 * `PEER_GLYPHS` is typed as a complete record, so in the app `PEER_GLYPHS[id]` is
 * never undefined — but a screen test that replaces the icon module with a handful of
 * named stubs leaves the rest of the table empty, and such a test is about the screen,
 * not the icon set. The component's documented answer for that case is an empty span:
 * no crash, no invented drawing, and the same box so the row's layout does not shift.
 */
vi.mock("./peerGlyphs", () => ({ PEER_GLYPHS: {} }));

const { PeerIconGlyph } = await import("./PeerIconGlyph");

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

describe("peerIconGlyph without a glyph for the id", () => {
  it("draws an empty box instead of crashing on a stubbed table", () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    act(() => root.render(<PeerIconGlyph className="size-4" id="linux" tone="#123456" />));

    expect(container.querySelector("svg")).toBeNull();
    const box = container.querySelector("span");
    expect(box).not.toBeNull();
    // The tone it was given still paints, so a chip that asked for its own colour
    // gets it even in the fallback.
    expect(box!.getAttribute("aria-hidden")).toBe("true");
    expect(box!.style.color).toBe("rgb(18, 52, 86)");
    expect(box!.className).toContain("size-4");

    act(() => root.unmount());
    container.remove();
  });
});
