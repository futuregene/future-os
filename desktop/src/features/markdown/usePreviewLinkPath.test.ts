// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

import { usePreviewLinkPath } from "./usePreviewLinkPath";

const resolveMock = vi.fn<(base: string, target: string) => Promise<{ path: string; name: string }>>();

vi.mock("../../integrations/storage/files", () => ({
  resolvePreviewLinkPath: (base: string, target: string) => resolveMock(base, target),
}));

describe("usePreviewLinkPath", () => {
  it("returns null while loading, then the resolved path", async () => {
    resolveMock.mockResolvedValue({ path: "/w/dir/pic.png", name: "pic.png" });
    const h = renderHook(() => usePreviewLinkPath("/w/doc.md", "dir/pic.png"));
    expect(h.current).toBeNull();
    await flushAsync();
    expect(h.current).toEqual({ path: "/w/dir/pic.png", name: "pic.png" });
    expect(resolveMock).toHaveBeenCalledWith("/w/doc.md", "dir/pic.png");
    h.unmount();
  });

  it("returns null when the resolve fails", async () => {
    resolveMock.mockRejectedValue(new Error("bad path"));
    const h = renderHook(() => usePreviewLinkPath("/w/doc.md", "../x"));
    await flushAsync();
    expect(h.current).toBeNull();
    h.unmount();
  });
});
