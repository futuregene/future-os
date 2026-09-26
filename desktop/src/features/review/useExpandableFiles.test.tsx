// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";

import { useExpandableFiles } from "./useExpandableFiles";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The shared `src/test/renderHook` probe takes no props, and this hook's
 * contract includes "the caller changed the key function / the list" — so mount
 * a probe that can be re-rendered with new props.
 */
function mountHook<P, R>(useHook: (props: P) => R, initialProps: P) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let current!: R;
  function Probe({ props }: { props: P }) {
    current = useHook(props);
    return null;
  }
  act(() => {
    root.render(createElement(Probe, { props: initialProps }));
  });
  return {
    get current() {
      return current;
    },
    setProps: (props: P) => {
      act(() => {
        root.render(createElement(Probe, { props }));
      });
    },
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

interface File {
  id: string;
  path: string;
}

const a: File = { id: "f1", path: "src/a.ts" };
const b: File = { id: "f2", path: "src/b.ts" };
const c: File = { id: "f3", path: "src/c.ts" };

const byId = (file: File) => file.id;
const byPath = (file: File) => file.path;

describe("useExpandableFiles", () => {
  it("defaults every file to collapsed", () => {
    const harness = mountHook(() => useExpandableFiles([a, b], byId), {});
    expect(harness.current.hasOpen).toBe(false);
    expect(harness.current.isOpen(a)).toBe(false);
    expect(harness.current.isOpen(b)).toBe(false);
    // A file that is not in the list is collapsed too (no undefined leak).
    expect(harness.current.isOpen(c)).toBe(false);
    harness.unmount();
  });

  it("toggles one file without disturbing the others", () => {
    const harness = mountHook(() => useExpandableFiles([a, b, c], byId), {});
    act(() => harness.current.toggle(b));
    expect(harness.current.isOpen(b)).toBe(true);
    expect(harness.current.isOpen(a)).toBe(false);
    expect(harness.current.isOpen(c)).toBe(false);
    expect(harness.current.hasOpen).toBe(true);

    // A second toggle returns exactly to the previous state (involution).
    act(() => harness.current.toggle(b));
    expect(harness.current.isOpen(b)).toBe(false);
    expect(harness.current.hasOpen).toBe(false);
    harness.unmount();
  });

  it("expands all only when everything is collapsed, and collapses all otherwise", () => {
    const harness = mountHook(() => useExpandableFiles([a, b, c], byId), {});

    act(() => harness.current.toggleAll());
    expect([a, b, c].map(file => harness.current.isOpen(file))).toEqual([true, true, true]);
    expect(harness.current.hasOpen).toBe(true);

    act(() => harness.current.toggleAll());
    expect([a, b, c].map(file => harness.current.isOpen(file))).toEqual([false, false, false]);
    expect(harness.current.hasOpen).toBe(false);

    // One open file is enough for the next toggleAll to collapse everything.
    act(() => harness.current.toggle(c));
    act(() => harness.current.toggleAll());
    expect(harness.current.hasOpen).toBe(false);
    harness.unmount();
  });

  it.each([
    ["an empty list", [] as File[]],
    ["a single file", [a]],
    ["a long list", Array.from({ length: 500 }, (_, index) => ({ id: `f${index}`, path: `src/f${index}.ts` }))],
  ])("keeps toggleAll an involution over %s", (_label, files) => {
    const harness = mountHook(() => useExpandableFiles(files, byId), {});
    act(() => harness.current.toggleAll());
    expect(harness.current.hasOpen).toBe(files.length > 0);
    expect(files.every(file => harness.current.isOpen(file))).toBe(true);
    act(() => harness.current.toggleAll());
    expect(harness.current.hasOpen).toBe(false);
    expect(files.some(file => harness.current.isOpen(file))).toBe(false);
    harness.unmount();
  });

  it("keys state by the caller's key, so a re-keyed list starts collapsed", () => {
    // Last-run review keys files by change id; the working tree keys by path.
    // Same file objects, different key function: the second view must not
    // inherit the first view's open state.
    const harness = mountHook(
      ({ mode }: { mode: "id" | "path" }) =>
        useExpandableFiles([a, b], mode === "id" ? byId : byPath),
      { mode: "id" as "id" | "path" },
    );
    act(() => harness.current.toggleAll());
    expect(harness.current.isOpen(a)).toBe(true);

    harness.setProps({ mode: "path" });
    expect(harness.current.hasOpen).toBe(false);
    expect(harness.current.isOpen(a)).toBe(false);

    // …and going back restores the id-keyed state, which was never dropped.
    harness.setProps({ mode: "id" });
    expect(harness.current.isOpen(a)).toBe(true);
    harness.unmount();
  });

  it("remembers open state for files that leave and re-enter the list", () => {
    // A repaint of the same list identity is common (poll ticks); open state is
    // keyed, not positional, so it must survive a shorter list.
    const harness = mountHook(
      ({ files }: { files: File[] }) => useExpandableFiles(files, byId),
      { files: [a, b] },
    );
    act(() => harness.current.toggle(a));
    harness.setProps({ files: [b] });
    expect(harness.current.hasOpen).toBe(false);
    harness.setProps({ files: [a, b] });
    expect(harness.current.isOpen(a)).toBe(true);
    harness.unmount();
  });
});
