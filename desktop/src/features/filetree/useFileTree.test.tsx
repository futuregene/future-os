// @vitest-environment jsdom
import type { DirEntry } from "../../integrations/storage/files";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { useFileTree } from "./useFileTree";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Props-aware probe: the shared helper cannot re-render with new props. */
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

const listDirectory = vi.fn<(path: string) => Promise<DirEntry[]>>();

vi.mock("../../integrations/storage/files", () => ({
  listDirectory: (path: string) => listDirectory(path),
}));

function entry(name: string, isDir = false, root = "/r"): DirEntry {
  return { isDir, modified: 1, name, path: `${root}/${name}`, size: isDir ? 0 : 10 };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, reject, resolve };
}

/** Unique root per test: the hook keeps a module-level cache keyed by root. */
let rootSeq = 0;
function freshRoot(label: string) {
  rootSeq += 1;
  return `/ws/${label}-${rootSeq}`;
}

async function flush(times = 4) {
  await act(async () => {
    for (let index = 0; index < times; index += 1)
      await Promise.resolve();
  });
}

beforeEach(() => {
  // Each test owns its own call history and implementations: a leftover
  // `mockReturnValueOnce` would otherwise be consumed by the next test's load.
  listDirectory.mockReset();
});

describe("useFileTree loading", () => {
  it("is inert without a root", () => {
    const harness = renderHook(() => useFileTree(null));
    expect(harness.current.rootEntries).toBeNull();
    expect(harness.current.rootLoading).toBe(false);
    expect(harness.current.rootErrored).toBe(false);
    act(() => harness.current.toggle("/x"));
    act(() => harness.current.reload("/x"));
    // `refresh` is async: keep the thenable out of `act`'s return value so the
    // call is a synchronous act scope.
    act(() => {
      void harness.current.refresh();
    });
    expect(listDirectory).not.toHaveBeenCalled();
    expect(harness.current.isExpanded("/x")).toBe(false);
    harness.unmount();
  });

  it("loads the root on mount and reports the loading state", async () => {
    const root = freshRoot("basic");
    const pending = deferred<DirEntry[]>();
    listDirectory.mockReturnValueOnce(pending.promise).mockResolvedValue([]);

    const harness = renderHook(() => useFileTree(root));
    expect(harness.current.rootEntries).toBeNull();
    expect(harness.current.rootLoading).toBe(true);
    expect(listDirectory).toHaveBeenCalledWith(root);

    await act(async () => {
      pending.resolve([entry("src", true, root), entry("readme.md", false, root)]);
    });
    await flush();
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["src", "readme.md"]);
    expect(harness.current.rootLoading).toBe(false);
    expect(harness.current.rootErrored).toBe(false);
    harness.unmount();
  });

  it("hides dotfiles by default and reveals them without refetching", async () => {
    const root = freshRoot("hidden");
    listDirectory.mockResolvedValue([
      entry(".env", false, root),
      entry("src", true, root),
      entry(".gitignore", false, root),
    ]);

    const harness = mountHook(({ showHidden }: { showHidden: boolean }) => useFileTree(root, showHidden), { showHidden: false });
    await flush();
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["src"]);
    expect(listDirectory).toHaveBeenCalledTimes(1);

    harness.setProps({ showHidden: true });
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual([".env", "src", ".gitignore"]);
    // Filtering is a display concern — the cache already holds every entry.
    expect(listDirectory).toHaveBeenCalledTimes(1);
    harness.unmount();
  });
});

describe("useFileTree expansion", () => {
  it("lazy-loads a directory on first expand and caches it", async () => {
    const root = freshRoot("expand");
    const rootEntries = [entry("src", true, root)];
    listDirectory.mockImplementation(async (path) => {
      if (path === root)
        return rootEntries;
      return [entry("main.rs", false, path)];
    });

    const harness = renderHook(() => useFileTree(root));
    await flush();
    expect(listDirectory).toHaveBeenCalledTimes(1);
    expect(harness.current.childrenOf(`${root}/src`)).toBeNull();

    act(() => harness.current.toggle(`${root}/src`));
    await flush();
    expect(harness.current.isExpanded(`${root}/src`)).toBe(true);
    expect(harness.current.childrenOf(`${root}/src`)?.map(item => item.name)).toEqual(["main.rs"]);
    expect(listDirectory).toHaveBeenCalledTimes(2);

    // Collapse and expand again: the cache serves it, no third read.
    act(() => harness.current.toggle(`${root}/src`));
    expect(harness.current.isExpanded(`${root}/src`)).toBe(false);
    expect(harness.current.childrenOf(`${root}/src`)?.map(item => item.name)).toEqual(["main.rs"]);
    act(() => harness.current.toggle(`${root}/src`));
    await flush();
    expect(listDirectory).toHaveBeenCalledTimes(2);
    harness.unmount();
  });

  it("does not start a second read for a directory already loading", async () => {
    const root = freshRoot("inflight");
    const pending = deferred<DirEntry[]>();
    listDirectory.mockImplementation(async (path) => {
      if (path === root)
        return [entry("src", true, root)];
      return pending.promise;
    });

    const harness = renderHook(() => useFileTree(root));
    await flush();
    act(() => harness.current.toggle(`${root}/src`));
    act(() => harness.current.toggle(`${root}/src`)); // collapse
    act(() => harness.current.toggle(`${root}/src`)); // expand again while in flight
    await flush();
    expect(listDirectory.mock.calls.filter(([path]) => path === `${root}/src`)).toHaveLength(1);
    expect(harness.current.isLoading(`${root}/src`)).toBe(true);

    await act(async () => {
      pending.resolve([entry("main.rs", false, `${root}/src`)]);
    });
    await flush();
    expect(harness.current.isLoading(`${root}/src`)).toBe(false);
    expect(harness.current.childrenOf(`${root}/src`)).toHaveLength(1);
    harness.unmount();
  });

  it("marks a failed directory and lets reload clear it", async () => {
    const root = freshRoot("error");
    listDirectory.mockImplementation(async (path) => {
      if (path === root)
        return [entry("src", true, root)];
      if (listDirectory.mock.calls.filter(([candidate]) => candidate === path).length === 1)
        throw new Error("EACCES: permission denied");
      return [entry("main.rs", false, path)];
    });

    const harness = renderHook(() => useFileTree(root));
    await flush();
    act(() => harness.current.toggle(`${root}/src`));
    await flush();
    expect(harness.current.isErrored(`${root}/src`)).toBe(true);
    expect(harness.current.isLoading(`${root}/src`)).toBe(false);
    expect(harness.current.childrenOf(`${root}/src`)).toBeNull();

    act(() => harness.current.reload(`${root}/src`));
    await flush();
    expect(harness.current.isErrored(`${root}/src`)).toBe(false);
    expect(harness.current.childrenOf(`${root}/src`)?.map(item => item.name)).toEqual(["main.rs"]);
    harness.unmount();
  });
});

describe("useFileTree refresh and staleness", () => {
  it("re-reads the root and every expanded directory, keeping expansion", async () => {
    const root = freshRoot("refresh");
    listDirectory.mockImplementation(async (path) => {
      if (path === root)
        return [entry("src", true, root), entry("docs", true, root)];
      return [entry("child.txt", false, path)];
    });

    const harness = renderHook(() => useFileTree(root));
    await flush();
    act(() => harness.current.toggle(`${root}/src`));
    await flush();
    listDirectory.mockClear();

    await act(async () => {
      await harness.current.refresh();
    });
    expect(listDirectory.mock.calls.map(([path]) => path).sort()).toEqual([root, `${root}/src`].sort());
    expect(harness.current.isExpanded(`${root}/src`)).toBe(true);
    expect(harness.current.childrenOf(`${root}/src`)).toHaveLength(1);
    harness.unmount();
  });

  it("drops a response that a newer refresh superseded", async () => {
    const root = freshRoot("stale");
    const slow = deferred<DirEntry[]>();
    const fast = deferred<DirEntry[]>();
    listDirectory.mockReturnValueOnce(slow.promise).mockReturnValueOnce(fast.promise);

    // Props-aware probe: after the stale response lands the test forces an
    // unrelated re-render, which is what makes the guard observable. Without
    // it the stale write is invisible here (the load's own `bump()` is also
    // guarded), so the assertion would pass even if the stale bytes had been
    // written into the cache — and any *other* cause of a re-render would then
    // surface them as the tree contents.
    const harness = mountHook(
      (props: { showHidden: boolean }) => useFileTree(root, props.showHidden),
      { showHidden: false },
    );
    expect(listDirectory).toHaveBeenCalledTimes(1);

    // A second load of the same directory (manual refresh) starts while the
    // first is still in flight.
    act(() => harness.current.reload(root));
    await flush();
    expect(listDirectory).toHaveBeenCalledTimes(2);

    await act(async () => {
      fast.resolve([entry("fresh.txt", false, root)]);
    });
    await flush();
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["fresh.txt"]);

    // The stale first response lands afterwards and must not reach the cache.
    await act(async () => {
      slow.resolve([entry("stale.txt", false, root)]);
    });
    await flush();
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["fresh.txt"]);
    expect(harness.current.rootLoading).toBe(false);

    // Force a render from the cache: it must still hold the winning response.
    harness.setProps({ showHidden: true });
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["fresh.txt"]);
    harness.setProps({ showHidden: false });
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["fresh.txt"]);
    harness.unmount();
  });

  it("ignores a failure that a newer load superseded", async () => {
    const root = freshRoot("stale-error");
    const slow = deferred<DirEntry[]>();
    const fast = deferred<DirEntry[]>();
    listDirectory.mockReturnValueOnce(slow.promise).mockReturnValueOnce(fast.promise);

    const harness = mountHook(
      (props: { showHidden: boolean }) => useFileTree(root, props.showHidden),
      { showHidden: false },
    );
    act(() => harness.current.reload(root));
    await flush();
    await act(async () => {
      fast.resolve([entry("fresh.txt", false, root)]);
    });
    await flush();
    await act(async () => {
      slow.reject(new Error("stale failure"));
    });
    await flush();
    expect(harness.current.rootErrored).toBe(false);
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["fresh.txt"]);

    // Force a render from the cache: the superseded failure must not have been
    // recorded, or this render would show the error state over good data.
    harness.setProps({ showHidden: true });
    expect(harness.current.rootErrored).toBe(false);
    expect(harness.current.rootEntries?.map(item => item.name)).toEqual(["fresh.txt"]);
    harness.unmount();
  });
});

describe("useFileTree state lifetime", () => {
  it("serves a warm tree synchronously on remount, expansion intact", async () => {
    const root = freshRoot("warm");
    listDirectory.mockImplementation(async path => (path === root
      ? [entry("src", true, root)]
      : [entry("main.rs", false, path)]));

    const first = renderHook(() => useFileTree(root));
    await flush();
    act(() => first.current.toggle(`${root}/src`));
    await flush();
    first.unmount();
    listDirectory.mockClear();

    const second = renderHook(() => useFileTree(root));
    // Cached entries are rendered from the first render, before the background
    // revalidation resolves — that is what keeps a tab switch from flickering.
    expect(second.current.rootEntries?.map(item => item.name)).toEqual(["src"]);
    expect(second.current.isExpanded(`${root}/src`)).toBe(true);
    await flush();
    // The background revalidation still happens.
    expect(listDirectory).toHaveBeenCalled();
    second.unmount();
  });

  it("keeps per-root state separate", async () => {
    const a = freshRoot("root-a");
    const b = freshRoot("root-b");
    listDirectory.mockImplementation(async path => [entry(path === a ? "a.txt" : "b.txt", false, path)]);

    const harnessA = renderHook(() => useFileTree(a));
    await flush();
    const harnessB = renderHook(() => useFileTree(b));
    await flush();
    expect(harnessA.current.rootEntries?.map(item => item.name)).toEqual(["a.txt"]);
    expect(harnessB.current.rootEntries?.map(item => item.name)).toEqual(["b.txt"]);
    harnessA.unmount();
    harnessB.unmount();
  });

  it("evicts the least recently used root once the cache is full", async () => {
    // CACHE_MAX_ROOTS is 20: the 21st distinct root drops the oldest.
    listDirectory.mockImplementation(async path => [entry("f.txt", false, path)]);
    const roots = Array.from({ length: 21 }, (_, index) => freshRoot(`lru-${index}`));

    const warm = renderHook(() => useFileTree(roots[0]!));
    await flush();
    expect(warm.current.rootEntries).toHaveLength(1);
    warm.unmount();

    for (const root of roots.slice(1)) {
      const harness = renderHook(() => useFileTree(root));
      await flush();
      harness.unmount();
    }

    // Revisiting the evicted root has to load again (no synchronous cache hit).
    const revisited = renderHook(() => useFileTree(roots[0]!));
    expect(revisited.current.rootEntries).toBeNull();
    await flush();
    expect(revisited.current.rootEntries).toHaveLength(1);
    revisited.unmount();
  });

  it("handles a large directory and Unicode names", async () => {
    const root = freshRoot("large");
    const entries = [
      ...Array.from({ length: 1000 }, (_, index) => entry(`file-${index}.ts`, false, root)),
      entry("工作区", true, root),
      entry("файл-Ω.md", false, root),
    ];
    listDirectory.mockResolvedValue(entries);

    const harness = renderHook(() => useFileTree(root));
    await flush();
    expect(harness.current.rootEntries).toHaveLength(1002);
    const rootEntries = harness.current.rootEntries ?? [];
    expect(rootEntries[rootEntries.length - 1]?.name).toBe("файл-Ω.md");
    harness.unmount();
  });
});
