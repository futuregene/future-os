import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import { useFileDownload } from "../useFileDownload";
import type { DownloadInfo } from "../../../remote/types";

jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("future-file-handler", () => ({ openFile: jest.fn() }));
jest.mock("../../../remote/files", () => ({ MAX_FILE_BYTES: 10 * 1024 * 1024 }));

const info: DownloadInfo = {
  transferId: "transfer", name: "notes.txt", mimeType: "text/plain", size: 3,
  contentHash: "new-content", previewKind: "text", variant: "preview", chunkBytes: 1024,
};
const file = { uri: "file:///cache/notes.txt", bytes: async () => new TextEncoder().encode("new") };

test("opening a directory file revalidates metadata instead of reusing a stale prepared preview", async () => {
  const remote = {
    cachedAttachment: jest.fn(() => ({ info, file })),
    prepareAttachment: jest.fn(async () => info),
    downloadAttachment: jest.fn(),
  } as unknown as Parameters<typeof useFileDownload>[0];
  let api!: ReturnType<typeof useFileDownload>;
  let tree!: ReactTestRenderer;
  function Harness() {
    api = useFileDownload(remote, ((key: string) => key) as TFunction, jest.fn());
    return null;
  }
  await act(async () => { tree = create(createElement(Harness)); });
  await act(async () => { await api.openFileLink("C:\\work\\notes.txt", true); });
  expect(remote.prepareAttachment).toHaveBeenCalledWith(
    { path: "C:\\work\\notes.txt", name: "notes.txt" }, "preview", expect.anything(), expect.any(Function),
  );
  expect(remote.cachedAttachment).toHaveBeenCalledTimes(1);
  expect(remote.downloadAttachment).not.toHaveBeenCalled();
  expect(api.preview).toMatchObject({ text: "new", info: { contentHash: "new-content" } });
  act(() => tree.unmount());
});
