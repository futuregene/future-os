// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useCollapsedWorkspaces } from "./useCollapsedWorkspaces";

beforeEach(() => {
  localStorage.clear();
});

describe("collapsed workspace groups", () => {
  it("persists collapses and restores them on the next launch", () => {
    const first = renderHook(() => useCollapsedWorkspaces());
    expect(first.current.collapsedWorkspaces.size).toBe(0);

    act(() => first.current.toggleWorkspaceCollapsed("ws-a"));
    act(() => first.current.toggleWorkspaceCollapsed("ws-b"));
    expect([...first.current.collapsedWorkspaces]).toEqual(["ws-a", "ws-b"]);
    // Expanding one workspace leaves the other collapsed.
    act(() => first.current.toggleWorkspaceCollapsed("ws-a"));
    expect([...first.current.collapsedWorkspaces]).toEqual(["ws-b"]);
    first.unmount();

    // A fresh mount (restart) reads the stored state back.
    const relaunched = renderHook(() => useCollapsedWorkspaces());
    expect([...relaunched.current.collapsedWorkspaces]).toEqual(["ws-b"]);
    relaunched.unmount();
  });

  it("tolerates unavailable and corrupt storage", () => {
    localStorage.setItem("future.collapsedWorkspaces", "{not json");
    const corrupt = renderHook(() => useCollapsedWorkspaces());
    expect(corrupt.current.collapsedWorkspaces.size).toBe(0);
    corrupt.unmount();

    localStorage.setItem("future.collapsedWorkspaces", JSON.stringify(["ws-a", 7, null]));
    const mixed = renderHook(() => useCollapsedWorkspaces());
    expect([...mixed.current.collapsedWorkspaces]).toEqual(["ws-a"]);
    act(() => mixed.current.toggleWorkspaceCollapsed("ws-c"));
    expect([...mixed.current.collapsedWorkspaces]).toEqual(["ws-a", "ws-c"]);
    mixed.unmount();

    // Storage that throws on both access paths: `vi.stubGlobal` replaces the
    // global itself, which works whether the environment provides a real
    // `Storage` (jsdom on Linux CI) or the plain in-memory object the macOS
    // test setup installs — an instance/prototype spy would only cover one.
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    });
    const unavailable = renderHook(() => useCollapsedWorkspaces());
    expect(unavailable.current.collapsedWorkspaces.size).toBe(0);
    act(() => unavailable.current.toggleWorkspaceCollapsed("ws-a"));
    expect([...unavailable.current.collapsedWorkspaces]).toEqual(["ws-a"]);
    unavailable.unmount();
    vi.unstubAllGlobals();
  });
});
