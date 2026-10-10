// @vitest-environment node
import { renderToString } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { useNow } from "./useNow";

/**
 * The shared clock is a client-side subscription, but `useSyncExternalStore`
 * also has a *server* snapshot: the value React uses when the tree is rendered
 * (or hydrated) outside a browser. This file runs in the node environment on
 * purpose — that is the only place the third argument of the hook is reachable,
 * and the desktop templates rely on it being a stable, bucket-aligned 0 so a
 * hydrated tree does not mismatch its server markup.
 */
function Probe({ intervalMs, enabled }: { intervalMs: number; enabled: boolean }) {
  return <span>{useNow(intervalMs, enabled)}</span>;
}

describe("useNow server snapshot", () => {
  it("renders the bucket-aligned zero outside a browser", () => {
    const html = renderToString(<Probe intervalMs={1000} enabled />);

    expect(html).toBe("<span>0</span>");
  });

  it("renders zero for a disabled clock too", () => {
    expect(renderToString(<Probe intervalMs={60_000} enabled={false} />)).toBe("<span>0</span>");
  });

  it("never touches a timer while server-rendering", () => {
    const setInterval = vi.fn();
    const clearInterval = vi.fn();
    // No DOM in this environment; the hooks must not reach for one.
    expect(typeof globalThis.window).toBe("undefined");

    renderToString(<Probe intervalMs={1000} enabled />);

    expect(setInterval).not.toHaveBeenCalled();
    expect(clearInterval).not.toHaveBeenCalled();
  });
});
