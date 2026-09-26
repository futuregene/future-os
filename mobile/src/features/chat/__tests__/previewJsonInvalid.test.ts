import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import type { TFunction } from "i18next";
import { PreviewModal } from "../components/PreviewModal";
import type { PreviewState } from "../useFileDownload";

// The JSON reader is NOT mocked here (unlike previewHighlight.test.ts): the
// claim under test is the message the preview composes for a document that is
// not JSON, which only exists once the real reader calls the callback.
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { detail?: string }) =>
      options?.detail ? `${key}: ${options.detail}` : key,
  }),
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("lucide-react-native", () => ({
  ChevronLeft: "ChevronLeft", Download: "Download", Ellipsis: "Ellipsis",
  ExternalLink: "ExternalLink", Share2: "Share2", X: "X",
}));
jest.mock("../../../components/MarkdownText", () => ({ MarkdownText: "MarkdownText" }));

const t = ((key: string, options?: { detail?: string }) =>
  options?.detail ? `${key}: ${options.detail}` : key) as TFunction;

function renderJsonPreview(text: string, sourceTruncated = false): ReactTestRenderer {
  const preview: PreviewState = {
    attachment: { path: "/tmp/notebook.ipynb", name: "notebook.ipynb" },
    info: {
      transferId: "t", name: "notebook.ipynb", mimeType: "application/json", size: text.length,
      contentHash: "hash", previewKind: "json", variant: "preview", chunkBytes: 0,
    },
    uri: "file:///preview/notebook.ipynb",
    text,
    truncated: sourceTruncated,
  };
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(PreviewModal, {
      previews: [preview], activeDownload: null, closePreview: jest.fn(), popPreview: jest.fn(),
      dismissPreviewThen: jest.fn(), downloadOriginal: jest.fn(),
      flushPendingPreviewAction: jest.fn(), openLinkedFile: jest.fn(async () => {}), t,
    }));
  });
  return tree;
}

const texts = (tree: ReactTestRenderer) =>
  tree.root.findAllByType(Text).map(node => node.props.children);

test("a document that is not JSON carries the parser's own detail, not a blank body", () => {
  // A .ipynb routed to the rich reader can still be a half-written file. The
  // reader must say what is wrong with it — a bare "invalid" would leave the
  // user with no idea whether the document or the app is broken.
  const tree = renderJsonPreview('{"cells": [{"source": ');
  const rendered = texts(tree).filter((child): child is string => typeof child === "string");
  expect(rendered.some(text => text.startsWith("attachment.jsonInvalid: "))).toBe(true);
  // The parser's message is threaded through, not replaced by the key alone.
  expect(rendered.find(text => text.startsWith("attachment.jsonInvalid: "))).not.toBe("attachment.jsonInvalid: ");
  act(() => tree.unmount());
});

test("a truncated document is reported as truncated instead of blamed for its syntax", () => {
  // A preview cut at the byte ceiling is almost never parseable; telling the
  // user their JSON is invalid would be actively misleading, so the reader
  // suppresses the parse error when the source itself was shortened.
  const tree = renderJsonPreview('{"cells": [{"source": ["print(1)"]', true);
  const rendered = texts(tree).filter((child): child is string => typeof child === "string");
  expect(rendered).toContain("attachment.jsonTruncated");
  expect(rendered.some(text => text.startsWith("attachment.jsonInvalid: "))).toBe(false);
  act(() => tree.unmount());
});
