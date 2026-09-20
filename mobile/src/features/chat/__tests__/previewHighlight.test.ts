import { createElement } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { Platform, StyleSheet, Text } from "react-native";
import type { TFunction } from "i18next";
import { PreviewModal } from "../components/PreviewModal";
import type { PreviewState } from "../useFileDownload";
import { codeColors } from "../../../theme/tokens";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("lucide-react-native", () => ({ Download: "Download", Ellipsis: "Ellipsis", ExternalLink: "ExternalLink", Share2: "Share2", X: "X" }));
jest.mock("../../../components/MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("../../../components/JsonPreview", () => ({ JsonPreview: "JsonPreview" }));

const t = ((key: string) => key) as TFunction;
const monospace = Platform.OS === "ios" ? "Menlo" : "monospace";
const python = 'def greet(name):\n    # say hi\n    return "hi " + name\n';

function renderTextPreview(name: string, text: string): ReactTestRenderer {
  const preview: PreviewState = {
    attachment: { path: `/tmp/${name}`, name },
    info: {
      transferId: "t", name, mimeType: "text/plain", size: text.length,
      contentHash: "hash", previewKind: "text", variant: "preview", chunkBytes: 0,
    },
    uri: `file:///preview/${name}`,
    text,
  };
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(PreviewModal, {
      preview, activeDownload: null, closePreview: jest.fn(), dismissPreviewThen: jest.fn(),
      downloadOriginal: jest.fn(), flushPendingPreviewAction: jest.fn(), t,
    }));
  });
  return tree;
}

/** Everything the body actually paints, in order — colored spans interleave
 * with the raw strings of the tokens Prism left uncolored. */
function painted(node: ReactTestInstance | string): string {
  return typeof node === "string"
    ? node
    : node.children.map(child => painted(child)).join("");
}

/** The selectable body, plus the source it paints. */
function body(tree: ReactTestRenderer) {
  const text = tree.root.findAllByType(Text).find(node => node.props.selectable)!;
  // `findAllByType` includes the node itself; only nested spans carry token colors.
  const spans = text.findAllByType(Text).filter(node => node !== text);
  return {
    text,
    spans,
    colors: spans.map(node => StyleSheet.flatten(node.props.style)?.color),
    source: painted(text),
  };
}

test("a code preview is colored and monospace without altering the source", () => {
  const tree = renderTextPreview("greet.py", python);
  try {
    const { text, colors, source } = body(tree);
    expect(source).toBe(python);
    expect(colors).toContain(codeColors.keyword);
    expect(colors).toContain(codeColors.comment);
    expect(colors).toContain(codeColors.string);
    expect(StyleSheet.flatten(text.props.style).fontFamily).toBe(monospace);
  } finally { act(() => tree.unmount()); }
});

test("prose previews keep the proportional font and stay uncolored", () => {
  const log = "2026-09-20 boot ok\n2026-09-20 ready\n";
  const tree = renderTextPreview("server.log", log);
  try {
    const { text, spans, source } = body(tree);
    expect(source).toBe(log);
    expect(spans).toHaveLength(0);
    expect(StyleSheet.flatten(text.props.style).fontFamily).toBeUndefined();
  } finally { act(() => tree.unmount()); }
});

test("code too large to tokenize keeps monospace and the untouched source", () => {
  const huge = "def long_line():\n" + "    value = 1  # comment\n".repeat(2000);
  const tree = renderTextPreview("huge.py", huge);
  try {
    const { text, spans, source } = body(tree);
    expect(huge.length).toBeGreaterThan(30_000);
    expect(source).toBe(huge);
    expect(spans).toHaveLength(0);
    expect(StyleSheet.flatten(text.props.style).fontFamily).toBe(monospace);
  } finally { act(() => tree.unmount()); }
});
