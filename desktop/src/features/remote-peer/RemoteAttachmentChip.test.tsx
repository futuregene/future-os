// @vitest-environment jsdom
import type { MessageAttachment } from "@future-os/thread-projection";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";
import { AttachmentChip } from "../agent/messages/AttachmentChip";
import { RemoteAttachmentProvider } from "./RemoteAttachmentOrigin";

/**
 * A remote conversation's transcript draws this app's own attachment chip, but
 * its paths point at the *host*. A click has to fetch the bytes before opening
 * anything — running the local `openPath` on a path that exists only on the
 * other machine is what reported "the file was moved or deleted" about a file
 * the host still had.
 */
const mocks = vi.hoisted(() => ({
  convertFileSrc: vi.fn((path: string) => `asset://${path}`),
  openPath: vi.fn(async () => {}),
  previewRemoteFile: vi.fn(),
  previews: [] as { kind: string; name: string; open: boolean; path: string }[],
}));

vi.mock("@tauri-apps/api/core", () => ({ convertFileSrc: mocks.convertFileSrc }));
vi.mock("../../integrations/storage/threadStore", () => ({ openPath: mocks.openPath }));
vi.mock("./remotePeerClient", () => ({
  previewRemoteFile: (...args: unknown[]) => mocks.previewRemoteFile(...args),
  // `AttachmentChip` imports only the one function; the rest of the module is
  // here because mocking a module replaces it wholesale.
}));
vi.mock("../filepreview/FilePreviewOverlay", () => ({
  FilePreviewOverlay: (props: { kind: string; name: string; onClose: () => void; open: boolean; path: string }) => {
    mocks.previews.push({ kind: props.kind, name: props.name, open: props.open, path: props.path });
    return <div data-kind={props.kind} data-open={String(props.open)} data-preview="" />;
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLElement;

function render(node: ReactNode) {
  act(() => {
    root.render(node);
  });
}

const attachment: MessageAttachment = {
  kind: "image",
  name: "pasted-ab12cd34ef56.png",
  // The host's path: it exists there, and not on this machine.
  path: "/Users/someone/.future/app/images/thread-1/origin/upload_x_pasted-ab12cd34ef56.png",
  thumbnail: "/Users/someone/.future/app/images/thread-1/thumb/1.png",
};

const toasts: { message: string; tone?: string }[] = [];
let dispose: (() => void) | null = null;

beforeEach(() => {
  mocks.previewRemoteFile.mockReset();
  mocks.openPath.mockReset().mockResolvedValue(undefined);
  mocks.previews.length = 0;
  toasts.length = 0;
  dispose = onFutureEvent("toast", (payload) => {
    toasts.push(payload as { message: string; tone?: string });
  });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  dispose?.();
});

it("fetches a remote attachment before opening it, and previews the local copy", async () => {
  mocks.previewRemoteFile.mockResolvedValue({
    name: "pasted-ab12cd34ef56.png",
    path: "/Users/me/.future/app/remote-previews/0123456789abcdef__pasted-ab12cd34ef56.png",
  });

  render(
    <RemoteAttachmentProvider value={{ desktopId: "desktop_a", sessionId: "sess_1" }}>
      <AttachmentChip attachment={attachment} />
    </RemoteAttachmentProvider>,
  );

  // No thumbnail for a remote image: the host path would only 404 here.
  expect(host.querySelector("img")).toBeNull();

  await act(async () => {
    host.querySelector("button")!.click();
  });

  expect(mocks.previewRemoteFile).toHaveBeenCalledWith({
    desktopId: "desktop_a",
    sessionId: "sess_1",
    path: attachment.path,
    name: attachment.name,
  });
  // The overlay reads this machine's cache entry, named as the host names it.
  expect(mocks.previews).toEqual([{
    kind: "image",
    name: "pasted-ab12cd34ef56.png",
    open: true,
    path: "/Users/me/.future/app/remote-previews/0123456789abcdef__pasted-ab12cd34ef56.png",
  }]);
  // The host path is never handed to the local opener.
  expect(mocks.openPath).not.toHaveBeenCalledWith(attachment.path);
  expect(toasts).toEqual([]);
});

it("does not run the local opener on a remote path when the fetch fails", async () => {
  mocks.previewRemoteFile.mockRejectedValue(new Error("peer_not_connected"));

  render(
    <RemoteAttachmentProvider value={{ desktopId: "desktop_a", sessionId: "sess_1" }}>
      <AttachmentChip attachment={attachment} />
    </RemoteAttachmentProvider>,
  );

  await act(async () => {
    host.querySelector("button")!.click();
  });

  expect(mocks.openPath).not.toHaveBeenCalled();
  expect(mocks.previews).toEqual([]);
  // The failure names the other machine, rather than claiming this file was
  // moved or deleted. (`not.toContain` keeps the assertion about *which* story
  // is told, so it does not depend on the language the test suite runs in.)
  expect(toasts).toHaveLength(1);
  expect(toasts[0]?.tone).toBe("error");
  expect(toasts[0]?.message).toContain("pasted-ab12cd34ef56.png");
  expect(toasts[0]?.message).not.toContain("moved or deleted");
});

it("opens the cached copy on a second look instead of transferring again", async () => {
  // A second look at the same file is the cache's job: the entry it landed on is
  // keyed by machine, session and path, and re-fetching would be another round trip
  // for bytes this machine already holds.
  mocks.previewRemoteFile.mockResolvedValue({
    name: "pasted-ab12cd34ef56.png",
    path: "/Users/me/.future/app/remote-previews/0123456789abcdef__pasted-ab12cd34ef56.png",
  });
  render(
    <RemoteAttachmentProvider value={{ desktopId: "desktop_a", sessionId: "sess_1" }}>
      <AttachmentChip attachment={attachment} />
    </RemoteAttachmentProvider>,
  );

  await act(async () => {
    host.querySelector("button")!.click();
  });
  await act(async () => {
    host.querySelector("button")!.click();
  });

  expect(mocks.previewRemoteFile).toHaveBeenCalledTimes(1);
  // Still showing the same cached copy — the second look re-opened it rather than
  // re-rendering a new one from a fresh transfer.
  expect(mocks.previews.filter(item => item.open)).toEqual([{
    kind: "image",
    name: "pasted-ab12cd34ef56.png",
    open: true,
    path: "/Users/me/.future/app/remote-previews/0123456789abcdef__pasted-ab12cd34ef56.png",
  }]);
  expect(mocks.openPath).not.toHaveBeenCalled();
});

it("hands a fetched file with nothing to draw to the OS, from the cached copy too", async () => {
  // A zip has no preview: the bytes are here after the transfer, so the gesture
  // becomes the local one — and the second click finds the cache and does the same
  // thing without the transfer.
  const archive: MessageAttachment = { ...attachment, kind: "file", name: "bundle.zip" };
  mocks.previewRemoteFile.mockResolvedValue({
    name: "bundle.zip",
    path: "/Users/me/.future/app/remote-previews/feed__bundle.zip",
  });
  render(
    <RemoteAttachmentProvider value={{ desktopId: "desktop_a", sessionId: "sess_1" }}>
      <AttachmentChip attachment={archive} />
    </RemoteAttachmentProvider>,
  );

  await act(async () => {
    host.querySelector("button")!.click();
  });
  expect(mocks.openPath).toHaveBeenLastCalledWith("/Users/me/.future/app/remote-previews/feed__bundle.zip");
  expect(mocks.previews.filter(item => item.open)).toEqual([]);

  await act(async () => {
    host.querySelector("button")!.click();
  });
  expect(mocks.previewRemoteFile).toHaveBeenCalledTimes(1);
  expect(mocks.openPath).toHaveBeenCalledTimes(2);
});

it("stays a local chip outside a remote transcript", async () => {
  render(<AttachmentChip attachment={{ ...attachment, thumbnail: null }} />);

  await act(async () => {
    host.querySelector("button")!.click();
  });

  expect(mocks.previewRemoteFile).not.toHaveBeenCalled();
  // The overlay reads the local path directly; no transfer, no cache entry.
  // (The mock records every render, so only the open one is compared.)
  expect(mocks.previews.filter(item => item.open)).toEqual([{
    kind: "image",
    name: attachment.name,
    open: true,
    path: attachment.path,
  }]);
});
