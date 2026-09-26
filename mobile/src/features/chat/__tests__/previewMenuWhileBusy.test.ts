import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Pressable, Text } from "react-native";
import type { TFunction } from "i18next";
import { PreviewModal } from "../components/PreviewModal";
import type { PreviewState } from "../useFileDownload";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("lucide-react-native", () => ({
  ChevronLeft: "ChevronLeft", Download: "Download", Ellipsis: "Ellipsis",
  ExternalLink: "ExternalLink", Share2: "Share2", X: "X",
}));

const t = ((key: string) => key) as TFunction;

const preview: PreviewState = {
  attachment: { path: "/tmp/notes.txt", name: "notes.txt" },
  info: {
    transferId: "t", name: "notes.txt", mimeType: "text/plain", size: 5,
    contentHash: "hash", previewKind: "text", variant: "preview", chunkBytes: 0,
  },
  uri: "file:///preview/notes.txt",
  text: "hello",
};

function render(
  activeDownload: Parameters<typeof PreviewModal>[0]["activeDownload"],
  onDownload = jest.fn(),
) {
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(PreviewModal, {
      previews: [preview],
      activeDownload,
      closePreview: jest.fn(),
      popPreview: jest.fn(),
      dismissPreviewThen: (action: () => void) => action(),
      downloadOriginal: onDownload,
      flushPendingPreviewAction: jest.fn(),
      openLinkedFile: jest.fn(async () => {}),
      t,
    }));
  });
  rendered = tree;
  return { tree, onDownload };
}

// The modal's preview stack is a VirtualizedList, and every update schedules its
// 50 ms render batching timer. Unmounting runs the list's own cleanup, which
// clears that timer: without it the timer outlives the file and fires after Jest
// has frozen the test console, which is an error rather than a warning.
let rendered: ReactTestRenderer | null = null;
afterEach(() => {
  const tree = rendered;
  rendered = null;
  if (tree) act(() => tree.unmount());
});

const byLabel = (tree: ReactTestRenderer, label: string) =>
  tree.root.findAll(node => node.props?.accessibilityLabel === label
    && typeof node.props.onPress === "function")[0];

const menuActionLabels = (tree: ReactTestRenderer) =>
  tree.root.findAll(node => typeof node.props?.accessibilityLabel === "string")
    .map(node => String(node.props.accessibilityLabel))
    .filter(label => ["attachment.share", "attachment.open", "attachment.save"].includes(label));

test("an overflow menu cannot be opened while a transfer owns the lane", () => {
  // The overflow button is only *disabled* while busy, and a disable is not a
  // guarantee: a tap that began before it and lands after still calls its
  // handler. Opening the menu then would offer share/open/save for a document
  // whose bytes are still arriving, so the menu must not mount at all.
  const { tree } = render({
    id: "d1", fileName: "notes.txt", phase: "downloading", completedBytes: 1, totalBytes: 5,
  });
  const more = byLabel(tree, "common.more")!;
  expect(more.props.disabled).toBe(true);
  act(() => more.props.onPress());
  expect(menuActionLabels(tree)).toEqual([]);});

test("the menu opens normally when nothing is transferring, and its actions dispatch", () => {
  // The control for the test above: without it, "the menu is absent" could just
  // mean the menu never renders at all.
  const { tree, onDownload } = render(null);
  const more = byLabel(tree, "common.more")!;
  expect(more.props.disabled).toBe(false);
  act(() => more.props.onPress());
  expect(menuActionLabels(tree)).toEqual(
    expect.arrayContaining(["attachment.share", "attachment.open", "attachment.save"]));
  act(() => byLabel(tree, "attachment.open")!.props.onPress());
  // "open" carries the operation; "save" is the default and passes none.
  expect(onDownload).toHaveBeenCalledWith(
    expect.objectContaining({ name: "notes.txt" }), "open");});
