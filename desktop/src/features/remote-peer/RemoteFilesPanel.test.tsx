// @vitest-environment jsdom
import type { RemoteFileListing } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteFilesPanel } from "./RemoteFilesPanel";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * The right panel for a remote conversation: that machine's files.
 *
 * Only files, and no tab strip — this app's other tabs describe *this* machine,
 * so offering them would mean one machine's facts under another's title.
 */
const list = vi.fn<(desktopId: string, sessionId: string, path?: string) => Promise<RemoteFileListing>>();
const download = vi.fn<(input: Record<string, unknown>) => Promise<string>>();
vi.mock("./remotePeerClient", () => ({
  listRemoteSessionFiles: (...args: Parameters<typeof list>) => list(...args),
  downloadRemoteFile: (...args: Parameters<typeof download>) => download(...args),
}));

const save = vi.fn<(options: Record<string, unknown>) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...args: Parameters<typeof save>) => save(...args),
}));

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

function listing(entries: RemoteFileListing["entries"], path = "/home/studio", root = "/home/studio"): RemoteFileListing {
  return { entries, path, rootPath: root } as RemoteFileListing;
}

beforeEach(() => {
  list.mockReset().mockResolvedValue(listing([]));
  download.mockReset().mockResolvedValue("report.txt");
  save.mockReset().mockResolvedValue("/Users/me/report.txt");
});

async function mount(props: { expanded?: boolean } = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onToggleExpanded = vi.fn();
  await act(async () => {
    root.render(
      <RemoteFilesPanel
        desktopId="desktop_a"
        expanded={props.expanded ?? true}
        onToggleExpanded={onToggleExpanded}
        peer={peer}
        sessionId="sess_1"
      />,
    );
  });
  return {
    container,
    onToggleExpanded,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

it("reads the conversation's files from the host on open", async () => {
  list.mockResolvedValue(listing([
    { isDir: true, name: "analysis", path: "/home/studio/analysis", size: 0 },
    { isDir: false, name: "effect-size.R", path: "/home/studio/effect-size.R", size: 2_048 },
  ]));
  const view = await mount();

  // The root is read with no path: the host names its own root.
  expect(list).toHaveBeenCalledWith("desktop_a", "sess_1");
  expect(view.container.textContent).toContain("analysis");
  expect(view.container.textContent).toContain("effect-size.R");
  // The heading names the machine, so a file list is never mistaken for this
  // machine's.
  expect(view.container.textContent).toContain("Studio iMac");
  await view.unmount();
});

it("opens a folder by the host's own path", async () => {
  list.mockResolvedValue(listing([{ isDir: true, name: "analysis", path: "/home/studio/analysis", size: 0 }]));
  const view = await mount();

  await act(async () => {
    [...view.container.querySelectorAll("button")].find(node => node.textContent === "analysis")!.click();
  });
  expect(list).toHaveBeenLastCalledWith("desktop_a", "sess_1", "/home/studio/analysis");
  await view.unmount();
});

it("goes up by the path the host reported, not by rebuilding it", async () => {
  list.mockResolvedValue(listing([], "/home/studio/analysis/plots", "/home/studio"));
  const view = await mount();

  const up = view.container.querySelector<HTMLButtonElement>("button[aria-label='Up']")!;
  expect(up.disabled).toBe(false);
  await act(async () => up.click());
  expect(list).toHaveBeenLastCalledWith("desktop_a", "sess_1", "/home/studio/analysis");
  await view.unmount();
});

it("offers no way up at the root", async () => {
  list.mockResolvedValue(listing([], "/home/studio", "/home/studio"));
  const view = await mount();
  expect(view.container.querySelector<HTMLButtonElement>("button[aria-label='Up']")!.disabled).toBe(true);
  await view.unmount();
});

/**
 * A download goes through a save dialog.
 *
 * Guessing a destination would write to a directory the user did not choose, on a
 * machine whose conventions we are guessing at.
 */
it("downloads a file to the chosen destination, and says so", async () => {
  list.mockResolvedValue(listing([{ isDir: false, name: "report.txt", path: "/home/studio/report.txt", size: 10 }]));
  const view = await mount();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("button[aria-label='Download report.txt']")!.click();
  });
  expect(save).toHaveBeenCalledWith({ defaultPath: "report.txt" });
  expect(download).toHaveBeenCalledWith({
    desktopId: "desktop_a",
    destination: "/Users/me/report.txt",
    name: "report.txt",
    path: "/home/studio/report.txt",
    sessionId: "sess_1",
  });
  // The name the *host* reported, which it may have changed (a preview variant).
  expect(view.container.textContent).toContain("report.txt");
  await view.unmount();
});

/** A cancelled dialog downloads nothing. */
it("downloads nothing when the destination is declined", async () => {
  list.mockResolvedValue(listing([{ isDir: false, name: "report.txt", path: "/home/studio/report.txt", size: 10 }]));
  save.mockResolvedValue(null);
  const view = await mount();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("button[aria-label='Download report.txt']")!.click();
  });
  expect(download).not.toHaveBeenCalled();
  await view.unmount();
});

it("surfaces a listing failure instead of showing an empty folder", async () => {
  list.mockRejectedValue(new Error("host is not connected"));
  const view = await mount();
  expect(view.container.textContent).toContain("host is not connected");
  await view.unmount();
});

it("surfaces a download failure", async () => {
  list.mockResolvedValue(listing([{ isDir: false, name: "report.txt", path: "/home/studio/report.txt", size: 10 }]));
  download.mockRejectedValue(new Error("transfer failed"));
  const view = await mount();

  await act(async () => {
    view.container.querySelector<HTMLButtonElement>("button[aria-label='Download report.txt']")!.click();
  });
  expect(view.container.textContent).toContain("transfer failed");
  await view.unmount();
});

it("re-reads the folder on demand", async () => {
  const view = await mount();
  await act(async () => view.container.querySelector<HTMLButtonElement>("button[aria-label='Re-read this folder']")!.click());
  expect(list).toHaveBeenCalledTimes(2);
  await view.unmount();
});

/** Collapsed, it is the floating toggle over the conversation header. */
it("shows only the toggle when collapsed, and reports the toggle", async () => {
  const view = await mount({ expanded: false });
  // Nothing of the panel is drawn...
  expect(view.container.querySelector("aside")).toBeNull();
  expect(list).not.toHaveBeenCalled();
  // ...and the affordance opens it.
  const toggle = view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Show this conversation's files\"]")!;
  await act(async () => toggle.click());
  expect(view.onToggleExpanded).toHaveBeenCalledTimes(1);
  await view.unmount();
});

/** Opening a folder that cannot be read says so, and keeps the listing it had. */
it("surfaces a folder that could not be opened", async () => {
  list.mockResolvedValue(listing([{ isDir: true, name: "analysis", path: "/home/studio/analysis", size: 0 }]));
  const view = await mount();

  list.mockRejectedValue(new Error("permission denied"));
  await act(async () => {
    [...view.container.querySelectorAll("button")].find(node => node.textContent === "analysis")!.click();
  });
  expect(view.container.textContent).toContain("permission denied");
  await view.unmount();
});
