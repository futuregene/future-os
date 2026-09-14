import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform } from "react-native";
import type { TFunction } from "i18next";
import { NativeFileActionSheet } from "../components/NativeFileActionSheet";
import { PreviewModal } from "../components/PreviewModal";
import type { FileAction, FileOperation } from "../utils";
import type { PreviewState } from "../useFileDownload";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("lucide-react-native", () => ({ Download: "Download", ExternalLink: "ExternalLink", Share2: "Share2", X: "X" }));
jest.mock("../../../components/MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("../../../components/JsonPreview", () => ({ JsonPreview: "JsonPreview" }));

const action: FileAction = {
  info: {
    transferId: "t", name: "report.pdf", mimeType: "application/pdf", size: 3,
    contentHash: "hash", previewKind: "file", variant: "original", chunkBytes: 1024,
  },
  cachedFile: null,
};
const t = ((key: string) => key) as TFunction;
const platform = Platform.OS;
afterEach(() => { Platform.OS = platform; });

test.each<FileOperation>(["open", "save", "share"])("file menu dispatches %s only after dismissal", operation => {
  Platform.OS = "ios";
  const onSelect = jest.fn();
  const onClose = jest.fn();
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(NativeFileActionSheet, {
    action, openLabel: "open", saveLabel: "save", shareLabel: "share", onSelect, onClose,
  })); });
  try {
    const button = tree.root.findAll(node => node.props.accessibilityLabel === operation && typeof node.props.onPress === "function")[0]!;
    act(() => button.props.onPress());
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
    act(() => { tree.root.findByType(Modal).props.onDismiss(); });
    expect(onSelect).toHaveBeenCalledWith(action, operation);
    expect(onSelect).toHaveBeenCalledTimes(1);
  } finally { act(() => tree.unmount()); }
});

test.each<FileOperation>(["open", "save", "share"])("preview %s uses the original attachment after closing", operation => {
  const preview: PreviewState = {
    attachment: { path: "/notes.txt", name: "notes.txt" },
    info: { ...action.info, name: "notes.txt", previewKind: "text", variant: "preview" },
    uri: "file:///preview.txt", text: "truncated preview", truncated: true,
  };
  const downloadOriginal = jest.fn();
  let pending!: () => void;
  const dismissPreviewThen = jest.fn((next: () => void) => { pending = next; });
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(PreviewModal, {
    preview, activeDownload: null, closePreview: jest.fn(), dismissPreviewThen,
    downloadOriginal, flushPendingPreviewAction: jest.fn(), t,
  })); });
  try {
    const button = tree.root.findAll(node => node.props.accessibilityLabel === `attachment.${operation}` && typeof node.props.onPress === "function")[0]!;
    act(() => button.props.onPress());
    expect(dismissPreviewThen).toHaveBeenCalledTimes(1);
    expect(downloadOriginal).not.toHaveBeenCalled();
    act(() => pending());
    if (operation === "save") expect(downloadOriginal).toHaveBeenCalledWith(preview.attachment);
    else expect(downloadOriginal).toHaveBeenCalledWith(preview.attachment, operation);
  } finally { act(() => tree.unmount()); }
});
