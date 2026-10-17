// @vitest-environment jsdom
import type { File as NodeFile } from "node:buffer";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useComposerAttachments } from "./useComposerAttachments";

/**
 * The attachment lane's own fences, driven at the hook rather than through the
 * composer's DOM.
 *
 * Three of them guard a *state*, not a gesture: a disabled composer must not
 * stage anything, and an operation that was superseded (the user switched
 * conversation while a multi-file paste was being copied) must stop where it is
 * rather than keep writing into a draft the user has left. Those are the states
 * that are awkward to produce through the UI — a disabled editor does not receive
 * a real paste — and trivial to produce here, where the hook's own API is the
 * caller.
 */

const mocks = vi.hoisted(() => ({
  classify: vi.fn<(path: string) => Promise<{ kind: string; path: string; reason: null }>>(),
  deleteTemp: vi.fn<(path: string) => Promise<void>>(),
  openDialog: vi.fn<() => Promise<string | null>>(),
  readNativePaths: vi.fn<() => Promise<string[]>>(),
  savePastedFile: vi.fn<(input: { bytes: number[]; name: string }) => Promise<{ path: string }>>(),
  savePastedImage: vi.fn<(input: { bytes: number[]; extension: string }) => Promise<{ path: string }>>(),
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => {} }),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: () => mocks.openDialog(),
}));
vi.mock("../../../integrations/storage/threadStore", () => ({
  deleteTempAttachment: (path: string) => mocks.deleteTemp(path),
  readNativeClipboardFilePaths: () => mocks.readNativePaths(),
  savePastedFile: (input: { bytes: number[]; name: string }) => mocks.savePastedFile(input),
  savePastedImage: (input: { bytes: number[]; extension: string }) => mocks.savePastedImage(input),
}));
vi.mock("../attachments", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../attachments")>();
  return {
    ...actual,
    classifyAttachment: (path: string) => mocks.classify(path),
  };
});

beforeEach(() => {
  mocks.classify.mockClear().mockImplementation(async (path: string) => ({ kind: "file", path, reason: null }));
  mocks.deleteTemp.mockClear().mockResolvedValue(undefined);
  mocks.openDialog.mockReset().mockResolvedValue(null);
  mocks.readNativePaths.mockReset().mockResolvedValue([]);
  mocks.savePastedFile.mockReset().mockImplementation(async input => ({ path: `/tmp/${input.name}` }));
  mocks.savePastedImage.mockReset().mockResolvedValue({ path: "/tmp/pasted.png" });
});

/** A `File` of the given type, sized so the byte limits are not in play. */
function file(name: string, type: string): File {
  return new File([new Uint8Array(4)], name, { type }) as unknown as File;
}

/**
 * The hook with a lifetime the test owns: `captureOperation` returns the
 * predicate the hook asks, and `supersede()` is the draft being left (a switch,
 * a close) while an operation is mid-flight.
 */
function mount(disabled = false) {
  let current = true;
  const hook = renderHook(() => useComposerAttachments({
    captureOperation: () => () => current,
    disabled,
  }));
  return {
    hook,
    supersede: () => {
      current = false;
    },
    unmount: () => hook.unmount(),
  };
}

describe("useComposerAttachments fences", () => {
  it("stages nothing from a paste while the composer is disabled", async () => {
    const view = mount(true);
    await act(async () => {
      await view.hook.current.attachPastedFiles([file("shot.png", "image/png")]);
    });

    // Not even the pasteboard is read: the guard is the first thing the paste
    // meets, which is what keeps a disabled composer from writing files.
    expect(mocks.readNativePaths).not.toHaveBeenCalled();
    expect(mocks.savePastedImage).not.toHaveBeenCalled();
    expect(mocks.savePastedFile).not.toHaveBeenCalled();
    expect(view.hook.current.attachments).toEqual([]);
    view.unmount();
  });

  it("does not open the file dialog while the composer is disabled", async () => {
    const view = mount(true);
    await act(async () => {
      await view.hook.current.handleAttachFiles();
    });

    expect(mocks.openDialog).not.toHaveBeenCalled();
    expect(view.hook.current.attachments).toEqual([]);
    view.unmount();
  });

  it("stops copying a pasted batch when the draft is left mid-copy", async () => {
    // Two pasted images. The first is written, and *then* the reader switches
    // conversation: the loop must stop rather than copy the second into a draft
    // the user has left — and it must not attach what it did write.
    const view = mount();
    mocks.savePastedImage.mockImplementation(async () => {
      view.supersede();
      return { path: "/tmp/first.png" };
    });

    await act(async () => {
      await view.hook.current.attachPastedFiles([
        file("first.png", "image/png"),
        file("second.png", "image/png"),
      ]);
    });

    // Only the first file was copied: the guard stops the loop, so the *bytes*
    // of the rest are never read into this process at all.
    expect(mocks.savePastedImage).toHaveBeenCalledTimes(1);
    // And the copy the leaving draft did write is reclaimed rather than left as
    // an orphan in the temporary directory.
    expect(mocks.deleteTemp).toHaveBeenCalledWith("/tmp/first.png");
    expect(view.hook.current.attachments).toEqual([]);
    view.unmount();
  });

  it("stops copying a copied-file batch when the draft is left mid-copy", async () => {
    // The same rule for the non-image half of a paste (a Finder copy is bytes,
    // not a path, so each file is written in turn).
    const view = mount();
    mocks.savePastedFile.mockImplementation(async (input) => {
      view.supersede();
      return { path: `/tmp/${input.name}` };
    });

    await act(async () => {
      await view.hook.current.attachPastedFiles([
        file("first.txt", "text/plain"),
        file("second.txt", "text/plain"),
      ]);
    });

    expect(mocks.savePastedFile).toHaveBeenCalledTimes(1);
    expect(mocks.deleteTemp).toHaveBeenCalledWith("/tmp/first.txt");
    expect(view.hook.current.attachments).toEqual([]);
    view.unmount();
  });
});

// A `File` in this environment is Node's, which jsdom's `File` shadows; the
// import keeps the type honest for the helper above.
export type { NodeFile };
