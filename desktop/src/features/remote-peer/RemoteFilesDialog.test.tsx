// @vitest-environment jsdom
import type { RemoteFileEntry, RemoteFileListing } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteFilesDialog } from "./RemoteFilesDialog";
import { formatSize, parentOf } from "./remotePeerFiles";

/**
 * The file browser: navigation is expressed in the host's own paths, a download
 * only ever fetches after the user has chosen a destination, and the name
 * reported as saved is the host's.
 */

const listFiles = vi.fn<(desktopId: string, sessionId: string, path?: string) => Promise<RemoteFileListing>>();
const downloadFile = vi.fn<(input: Record<string, unknown>) => Promise<string>>();
vi.mock("./remotePeerClient", () => ({
  downloadRemoteFile: (...args: Parameters<typeof downloadFile>) => downloadFile(...args),
  listRemoteSessionFiles: (...args: Parameters<typeof listFiles>) => listFiles(...args),
}));

const saveDialog = vi.fn<(options: Record<string, unknown>) => Promise<string | null>>();
vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: (...args: Parameters<typeof saveDialog>) => saveDialog(...args),
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function entry(overrides: Partial<RemoteFileEntry> = {}): RemoteFileEntry {
  return { name: "report.txt", path: "/root/report.txt", isDir: false, size: 2048, ...overrides };
}

function listing(overrides: Partial<RemoteFileListing> = {}): RemoteFileListing {
  return { rootPath: "/root", path: "/root", entries: [entry()], ...overrides };
}

async function mount() {
  const onClose = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <RemoteFilesDialog
        desktopId="desktop_a"
        onClose={onClose}
        peerName="Studio iMac"
        sessionId="sess_1"
      />,
    );
  });
  await settle();
  return {
    container,
    onClose,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent?.includes(label)),
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  listFiles.mockReset().mockResolvedValue(listing());
  downloadFile.mockReset().mockResolvedValue("report.txt");
  saveDialog.mockReset().mockResolvedValue("/local/saved.txt");
});

it("reads the session root on open and lists it", async () => {
  const view = await mount();

  // No path at all: the backend turns an absent one into the host's meaning for
  // "the session root", which is a different request from an empty string.
  expect(listFiles).toHaveBeenCalledWith("desktop_a", "sess_1");
  expect(view.text()).toContain("report.txt");
  expect(view.text()).toContain("2.0 KB");
  await view.unmount();
});

it("opens a directory using the host's own path", async () => {
  listFiles.mockResolvedValue(listing({
    entries: [entry({ name: "src", path: "/root/src", isDir: true, size: 0 })],
  }));
  const view = await mount();
  listFiles.mockResolvedValue(listing({ path: "/root/src", entries: [] }));

  await act(async () => view.button("src")!.click());
  await settle();

  expect(listFiles).toHaveBeenLastCalledWith("desktop_a", "sess_1", "/root/src");
  await view.unmount();
});

/** A download asks where to put it, and does nothing when that is cancelled. */
it("downloads only after a destination is chosen", async () => {
  const view = await mount();
  await act(async () => view.button("Download")!.click());
  await settle();

  expect(saveDialog).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "report.txt" }));
  expect(downloadFile).toHaveBeenCalledWith({
    desktopId: "desktop_a",
    sessionId: "sess_1",
    path: "/root/report.txt",
    name: "report.txt",
    destination: "/local/saved.txt",
  });
  await view.unmount();
});

it("fetches nothing when the save dialog is cancelled", async () => {
  saveDialog.mockResolvedValue(null);
  const view = await mount();
  await act(async () => view.button("Download")!.click());
  await settle();

  expect(downloadFile).not.toHaveBeenCalled();
  expect(view.text()).not.toContain("Saved as");
  await view.unmount();
});

/**
 * The host may rename the file it serves (a preview variant), so the name it
 * reports is the one to show: reporting the requested name would send the user
 * looking for a file that is not there.
 */
it("reports the name the host saved it as", async () => {
  downloadFile.mockResolvedValue("report-preview.png");
  const view = await mount();
  await act(async () => view.button("Download")!.click());
  await settle();

  expect(view.text()).toContain("report-preview.png");
  await view.unmount();
});

it("surfaces a failed listing and a failed download", async () => {
  listFiles.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();
  expect(view.text()).toContain("peer_not_connected");
  await view.unmount();

  listFiles.mockReset().mockResolvedValue(listing());
  downloadFile.mockRejectedValue(new Error("remote_download_hash_mismatch"));
  const second = await mount();
  await act(async () => second.button("Download")!.click());
  await settle();
  expect(second.text()).toContain("remote_download_hash_mismatch");
  await second.unmount();
});

/** A failure while navigating is reported, not silently left showing the old directory. */
it("surfaces a failure while navigating into a directory", async () => {
  listFiles.mockResolvedValue(listing({
    entries: [entry({ name: "src", path: "/root/src", isDir: true, size: 0 })],
  }));
  const view = await mount();

  listFiles.mockRejectedValue(new Error("The directory is not readable."));
  await act(async () => view.button("src")!.click());
  await settle();

  expect(view.text()).toContain("The directory is not readable.");
  await view.unmount();
});

/**
 * A second read cannot be started while one is in flight, and the control that
 * could start it says so rather than swallowing the click.
 */
it("disables navigation while a directory read is in flight", async () => {
  listFiles.mockResolvedValue(listing({
    entries: [entry({ name: "src", path: "/root/src", isDir: true, size: 0 })],
  }));
  const view = await mount();
  expect(view.button("src")!.hasAttribute("disabled")).toBe(false);

  // The next read is left pending: the previous listing stays on screen, and its
  // control has to say it cannot be used yet.
  let release: ((value: RemoteFileListing) => void) | null = null;
  listFiles.mockImplementation(() => new Promise<RemoteFileListing>((resolve) => {
    release = resolve;
  }));
  await act(async () => view.button("src")!.click());
  expect(view.button("src")!.hasAttribute("disabled")).toBe(true);

  await act(async () => {
    release?.(listing({ path: "/root/src", entries: [] }));
  });
  await settle();
  await view.unmount();
});

/** Sizes are shown the way a person reads them; the units step at 1024. */
it("formats sizes in the unit that fits", () => {
  expect(formatSize(0)).toBe("0 B");
  expect(formatSize(1023)).toBe("1023 B");
  expect(formatSize(1024)).toBe("1.0 KB");
  expect(formatSize(2048)).toBe("2.0 KB");
  expect(formatSize(1024 * 1024)).toBe("1.0 MB");
  expect(formatSize(5 * 1024 * 1024)).toBe("5.0 MB");
  // Two steps of the same loop, and the ceiling: nothing above GB is invented.
  expect(formatSize(3 * 1024 * 1024 * 1024)).toBe("3.0 GB");
  expect(formatSize(5000 * 1024 * 1024 * 1024)).toBe("5000.0 GB");
});

it("offers no download for a directory, and no way up from the root", async () => {
  listFiles.mockResolvedValue(listing({
    entries: [entry({ name: "src", path: "/root/src", isDir: true, size: 0 })],
  }));
  const view = await mount();

  expect(view.button("Download")).toBeUndefined();
  // At the root there is nothing above it to offer.
  expect(view.button("Up")!.hasAttribute("disabled")).toBe(true);
  await view.unmount();
});

it("walks up towards the host's root, and no further", async () => {
  // Echoes the requested path, as a real host does: a mock that always returns
  // the same listing would make the second step invisible.
  listFiles.mockImplementation(async (_desktopId, _sessionId, path) =>
    listing({ path: path ?? "/root/nested/deep", entries: [] }));
  const view = await mount();
  expect(view.text()).toContain("/root/nested/deep");

  await act(async () => view.button("Up")!.click());
  await settle();
  expect(listFiles).toHaveBeenLastCalledWith("desktop_a", "sess_1", "/root/nested");

  await act(async () => view.button("Up")!.click());
  await settle();
  expect(listFiles).toHaveBeenLastCalledWith("desktop_a", "sess_1", "/root");

  // At the root there is nothing above it to offer.
  expect(view.button("Up")!.hasAttribute("disabled")).toBe(true);
  await view.unmount();
});

/**
 * The host's paths may be Windows-shaped, and the only boundary that matters is
 * the session root the host reported. Getting this wrong walks the user out of
 * the session, where every read is refused.
 */
it("trims the host's separators without rebuilding the path", () => {
  // A POSIX path.
  expect(parentOf("/root/nested/deep", "/root")).toBe("/root/nested");
  // A Windows path keeps its own separator.
  expect(parentOf("C:\\root\\nested", "C:\\root")).toBe("C:\\root");
  // A trailing separator does not produce an empty segment.
  expect(parentOf("/root/nested/", "/root")).toBe("/root");
  // The root itself has no parent to offer.
  expect(parentOf("/root", "/root")).toBeNull();
  expect(parentOf("", "/root")).toBeNull();
  // A path that would step *above* the root stops at it rather than walking the
  // user out of a directory the host will refuse to read.
  expect(parentOf("/root/a", "/root/a/b")).toBe("/root/a/b");
  // No separator at all: nothing to trim.
  expect(parentOf("bare-name", "/root")).toBeNull();
});
