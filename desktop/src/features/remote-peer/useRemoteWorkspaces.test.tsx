// @vitest-environment jsdom
import type { RemoteWorkspaceRow } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { useRemoteWorkspaceRows } from "./useRemoteWorkspaces";

/**
 * The hook that answers "which folders does that machine have".
 *
 * Its contract is about *which* machine answered: the ids a draft sends are
 * resolved against the host on screen, so a list from the machine the reader
 * already left is not merely stale — it would offer folders that do not exist
 * there. The read is also the chooser's only source of names, so it has to be
 * repeatable (a workspace created from the draft's own menu is not in the list
 * it read) and it must clear itself when the draft is this machine's.
 */

const list = vi.fn<(desktopId: string) => Promise<RemoteWorkspaceRow[]>>();
vi.mock("./remotePeerClient", () => ({
  listRemoteWorkspaces: (desktopId: string) => list(desktopId),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function rows(...names: string[]): RemoteWorkspaceRow[] {
  return names.map((name, index) => ({
    id: `ws_${index + 1}`,
    name,
    path: `/Users/me/${name}`,
    pinned: false,
  }));
}

type Hook = ReturnType<typeof useRemoteWorkspaceRows>;

async function mount(desktopId: string | null) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let latest!: Hook;
  function Probe({ host }: { host: string | null }) {
    latest = useRemoteWorkspaceRows(host);
    return null;
  }
  await act(async () => {
    root.render(<Probe host={desktopId} />);
  });
  return {
    get: () => latest,
    /** Let the outstanding read settle, then one more render pass. */
    settle: async () => {
      await act(async () => {
        for (let i = 0; i < 6; i += 1) await Promise.resolve();
      });
    },
    switchTo: async (host: string | null) => {
      await act(async () => {
        root.render(<Probe host={host} />);
      });
    },
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

/** A promise whose answer this test decides, so an order can be forced. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

beforeEach(() => {
  list.mockReset();
  list.mockResolvedValue([]);
});

it("reads the workspace list of the host it is asked about", async () => {
  list.mockResolvedValue(rows("future-os", "notes"));
  const view = await mount("desktop_a");
  await view.settle();

  expect(list).toHaveBeenCalledWith("desktop_a");
  expect(view.get().rows.map(row => row.name)).toEqual(["future-os", "notes"]);
  expect(view.get().error).toBeNull();
  await view.unmount();
});

/** A draft on this machine has no host to ask, and nothing to show. */
it("asks nobody for a draft with no host, and forgets the last machine's folders", async () => {
  list.mockResolvedValue(rows("future-os"));
  const view = await mount("desktop_a");
  await view.settle();
  expect(view.get().rows).toHaveLength(1);

  await view.switchTo(null);
  await view.settle();

  expect(view.get().rows).toEqual([]);
  expect(view.get().error).toBeNull();
  await view.unmount();
});

/**
 * A slow answer from the machine the reader left must not land.
 *
 * Two reads can overlap — the reader opens a draft on one machine and then on
 * another — and the older one can finish last. Its ids would be offered as the
 * new host's, which is a conversation created in a folder that is not there.
 */
it("drops an answer that lost the race with a newer read", async () => {
  const first = deferred<RemoteWorkspaceRow[]>();
  const second = deferred<RemoteWorkspaceRow[]>();
  list.mockImplementation(desktopId => (desktopId === "desktop_a" ? first.promise : second.promise));

  const view = await mount("desktop_a");
  await view.switchTo("desktop_b");

  await act(async () => {
    second.resolve(rows("on-b"));
    await Promise.resolve();
  });
  await view.settle();
  expect(view.get().rows.map(row => row.name)).toEqual(["on-b"]);

  await act(async () => {
    first.resolve(rows("on-a"));
    await Promise.resolve();
  });
  await view.settle();
  expect(view.get().rows.map(row => row.name)).toEqual(["on-b"]);
  await view.unmount();
});

/** A failure is reported, not turned into an empty disk. */
it("reports a read that failed", async () => {
  list.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount("desktop_a");
  await view.settle();

  expect(view.get().error).toBe("peer_not_connected");
  expect(view.get().loading).toBe(false);
  await view.unmount();
});

/** And a caller can ask again after it changed something on that host. */
it("re-reads on request", async () => {
  list.mockResolvedValueOnce(rows("future-os"));
  const view = await mount("desktop_a");
  await view.settle();
  expect(view.get().rows).toHaveLength(1);

  list.mockResolvedValueOnce(rows("future-os", "notes"));
  await act(async () => {
    await view.get().reload();
  });
  expect(view.get().rows).toHaveLength(2);
  expect(list).toHaveBeenCalledTimes(2);
  await view.unmount();
});
