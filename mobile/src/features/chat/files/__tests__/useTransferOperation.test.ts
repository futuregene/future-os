import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useTransferOperation } from "../useTransferOperation";

/**
 * Starting a transfer, and what the progress surface knows about it at that
 * moment.
 *
 * A transfer that is meant to be seen announces itself the instant it starts —
 * the reader is owed an answer to their tap before the desktop has been asked
 * anything. One that must stay quiet until its size is known (the cached-file
 * path) says nothing until something is known.
 */
describe("useTransferOperation", () => {
  let api: ReturnType<typeof useTransferOperation>;
  let renderer: ReactTestRenderer | null = null;

  function Harness(): null {
    api = useTransferOperation(((key: string) => key) as never);
    return null;
  }

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
  });

  test("a visible transfer is announced as preparing, a quiet one is not", () => {
    act(() => { renderer = create(createElement(Harness)); });

    let visible: ReturnType<typeof api.beginDownload> = null;
    act(() => { visible = api.beginDownload("k1", "photo.jpg"); });
    expect(visible).not.toBeNull();
    expect(api.activeDownload).toEqual({
      id: visible!.id,
      fileName: "photo.jpg",
      phase: "preparing",
      completedBytes: 0,
      totalBytes: 0,
    });

    // The transfer owns the lane: a second one cannot start on top of it.
    act(() => { expect(api.beginDownload("k2", "other.jpg")).toBeNull(); });

    act(() => { api.cancelActiveDownload(); });
    expect(api.activeDownload).toBeNull();

    // A quiet transfer is the cached-file path: nothing is on screen until its
    // size is known, so a cached file never flashes a dialog.
    let quiet: ReturnType<typeof api.beginDownload> = null;
    act(() => { quiet = api.beginDownload("k3", "cached.bin", 0, false); });
    expect(quiet).not.toBeNull();
    expect(api.activeDownload).toBeNull();
  });
});
