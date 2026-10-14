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
const preview = vi.fn<(input: Record<string, unknown>) => Promise<{ name: string; path: string }>>();
vi.mock("./remotePeerClient", () => ({
  listRemoteSessionFiles: (...args: Parameters<typeof list>) => list(...args),
  downloadRemoteFile: (...args: Parameters<typeof download>) => download(...args),
  previewRemoteFile: (...args: Parameters<typeof preview>) => preview(...args),
}));

/**
 * The overlay is stubbed, as elsewhere: what belongs to *this* panel is that it
 * shows the local preview component for a fetched file, not what that component
 * paints. Its own behaviour — kinds, readers, errors — is its own test file's.
 */
const previews: Array<{ kind: string; name: string; path: string }> = [];
vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; name: string; onClose: () => void; path: string }) => {
    previews.push({ kind: props.kind, name: props.name, path: props.path });
    return (
      <div data-kind={props.kind} data-preview="">
        <button data-close-preview="" onClick={props.onClose} type="button" />
      </div>
    );
  },
}));

const save = vi.fn<(options: Record<string, unknown>) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...args: Parameters<typeof save>) => save(...args),
}));

const peer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  stopped: false,
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
  previews.length = 0;
  list.mockReset().mockResolvedValue(listing([]));
  download.mockReset().mockResolvedValue("report.txt");
  preview.mockReset().mockResolvedValue({ name: "forest.png", path: "/home/me/.future/app/remote-previews/abc__forest.png" });
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

/**
 * A previewable file is fetched and shown *here*, without a save dialog.
 *
 * The local panel's rule, kept: a click on a file the overlay can render shows
 * it, and the bytes come from that machine. The destination is this app's own
 * cache — looking at a file is not copying it, and asking where to put a copy
 * every time someone clicks a file in a panel makes the quickest thing on the
 * screen the slowest.
 */
it("fetches a previewable file and shows it in the local overlay", async () => {
  list.mockResolvedValue(listing([
    { isDir: false, name: "forest.png", path: "/home/studio/forest.png", size: 4_096 },
  ]));
  const view = await mount();

  await act(async () => {
    [...view.container.querySelectorAll("button")].find(node => node.textContent === "forest.png")!.click();
  });

  expect(preview).toHaveBeenCalledWith({
    desktopId: "desktop_a",
    name: "forest.png",
    path: "/home/studio/forest.png",
    sessionId: "sess_1",
  });
  expect(save).not.toHaveBeenCalled();
  expect(download).not.toHaveBeenCalled();
  // The local overlay, over the local cache entry, with the kind the *name*
  // selects — the same renderers and the same supported types as the local panel.
  expect(previews).toEqual([{
    kind: "image",
    name: "forest.png",
    path: "/home/me/.future/app/remote-previews/abc__forest.png",
  }]);
  await view.unmount();
});

/** The file types are the local overlay's own list, read off the name. */
it("previews every kind the local overlay can render, and nothing else", async () => {
  list.mockResolvedValue(listing([
    { isDir: false, name: "notes.md", path: "/home/studio/notes.md", size: 10 },
    { isDir: false, name: "data.json", path: "/home/studio/data.json", size: 10 },
    { isDir: false, name: "effect-size.R", path: "/home/studio/effect-size.R", size: 10 },
    { isDir: false, name: "archive.zip", path: "/home/studio/archive.zip", size: 10 },
  ]));
  const view = await mount();
  const nameButton = (label: string) => [...view.container.querySelectorAll<HTMLButtonElement>("button")]
    .find(node => node.textContent === label)!;

  // Markdown, JSON and a code file are previewable; a zip is not.
  expect(nameButton("notes.md").disabled).toBe(false);
  expect(nameButton("data.json").disabled).toBe(false);
  expect(nameButton("effect-size.R").disabled).toBe(false);
  expect(nameButton("archive.zip").disabled).toBe(true);

  preview.mockResolvedValue({ name: "notes.md", path: "/cache/abc__notes.md" });
  await act(async () => nameButton("notes.md").click());
  expect(previews[0]!.kind).toBe("markdown");
  await view.unmount();
});

it("surfaces a preview the host could not serve", async () => {
  list.mockResolvedValue(listing([
    { isDir: false, name: "forest.png", path: "/home/studio/forest.png", size: 4_096 },
  ]));
  preview.mockRejectedValue(new Error("transfer failed"));
  const view = await mount();

  await act(async () => {
    [...view.container.querySelectorAll("button")].find(node => node.textContent === "forest.png")!.click();
  });
  expect(view.container.textContent).toContain("transfer failed");
  expect(previews).toEqual([]);
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

/**
 * A second file stays clickable while the first is still coming down.
 *
 * The panel used to disable every row's controls whenever *any* transfer was in
 * flight, so on a slow link choosing a second file did nothing at all — the
 * panel read as broken while it was working. One file's transfer says nothing
 * about whether the reader may open another, which the host serves one request at
 * a time and answers when it answers.
 */
it("lets another file be opened while one is still being fetched", async () => {
  list.mockResolvedValue(listing([
    { isDir: false, name: "forest.png", path: "/home/studio/forest.png", size: 4_096 },
    { isDir: false, name: "notes.md", path: "/home/studio/notes.md", size: 512 },
  ]));
  // Hold the first fetch open, so the second click happens during it.
  let releaseFirst: (value: { name: string; path: string }) => void = () => {};
  preview
    .mockImplementationOnce(() => new Promise((resolve) => { releaseFirst = resolve; }))
    .mockResolvedValue({ name: "notes.md", path: "/cache/abc__notes.md" });

  const view = await mount();
  const nameButton = (label: string) => [...view.container.querySelectorAll<HTMLButtonElement>("button")]
    .find(node => node.textContent === label)!;

  await act(async () => nameButton("forest.png").click());
  // The transfer is in flight: its own row says so, and the other row is still
  // something the reader can act on.
  expect(nameButton("forest.png").disabled).toBe(true);
  expect(nameButton("notes.md").disabled).toBe(false);

  await act(async () => nameButton("notes.md").click());
  expect(preview).toHaveBeenCalledTimes(2);
  // The file asked for last is the one on screen, even though the first answer
  // has not arrived yet.
  expect(previews[previews.length - 1]?.name).toBe("notes.md");

  // And when the earlier one finally lands, it does not replace it.
  await act(async () => {
    releaseFirst({ name: "forest.png", path: "/cache/abc__forest.png" });
  });
  expect(previews[previews.length - 1]?.name).toBe("notes.md");
  await view.unmount();
});

it("lets another file be downloaded while one is still downloading", async () => {
  list.mockResolvedValue(listing([
    { isDir: false, name: "report.txt", path: "/home/studio/report.txt", size: 10 },
    { isDir: false, name: "second.txt", path: "/home/studio/second.txt", size: 20 },
  ]));
  let releaseFirst: (value: string) => void = () => {};
  download
    .mockImplementationOnce(() => new Promise((resolve) => { releaseFirst = resolve; }))
    .mockResolvedValue("second.txt");

  const view = await mount();
  const downloadButton = (label: string) => view.container
    .querySelector<HTMLButtonElement>(`button[aria-label='Download ${label}']`)!;

  await act(async () => downloadButton("report.txt").click());
  // The first transfer is still running, and the second row is still usable.
  expect(downloadButton("report.txt").disabled).toBe(true);
  expect(downloadButton("second.txt").disabled).toBe(false);

  await act(async () => downloadButton("second.txt").click());
  expect(download).toHaveBeenCalledTimes(2);
  expect(download).toHaveBeenLastCalledWith(expect.objectContaining({ name: "second.txt" }));

  await act(async () => releaseFirst("/Users/me/report.txt"));
  await view.unmount();
});

/**
 * Dismissing the overlay gives up on the fetch still coming for it.
 *
 * Otherwise a file the reader already dismissed lands a moment later and opens
 * itself again over whatever they moved on to — the same stale-answer shape as
 * the wrong-file case, with the reader's own dismissal as the thing it undoes.
 */
it("does not re-open the overlay from a fetch that was dismissed", async () => {
  list.mockResolvedValue(listing([
    { isDir: false, name: "forest.png", path: "/home/studio/forest.png", size: 4_096 },
    { isDir: false, name: "notes.md", path: "/home/studio/notes.md", size: 512 },
  ]));
  let release: (value: { name: string; path: string }) => void = () => {};
  preview
    .mockResolvedValueOnce({ name: "forest.png", path: "/cache/abc__forest.png" })
    .mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));

  const view = await mount();
  const nameButton = (label: string) => [...view.container.querySelectorAll<HTMLButtonElement>("button")]
    .find(node => node.textContent === label)!;

  await act(async () => nameButton("forest.png").click());
  const overlay = () => view.container.querySelector("[data-preview]");
  expect(overlay()).not.toBeNull();

  // The second file is fetched while the first file's overlay is up...
  await act(async () => nameButton("notes.md").click());
  // ...and the reader dismisses it before that one lands.
  await act(async () => view.container.querySelector<HTMLButtonElement>("[data-close-preview]")!.click());
  expect(overlay()).toBeNull();

  await act(async () => release({ name: "notes.md", path: "/cache/abc__notes.md" }));
  expect(overlay()).toBeNull();
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
