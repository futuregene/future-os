import { createElement } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { FlatList, Platform, StyleSheet, Text } from "react-native";
import type { TFunction } from "i18next";
import { PreviewModal } from "../components/PreviewModal";
import type { PreviewState } from "../useFileDownload";
import { codeRowText } from "../../../components/codePreviewRows";
import { codeColors } from "../../../theme/tokens";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("lucide-react-native", () => ({ ChevronLeft: "ChevronLeft", Download: "Download", Ellipsis: "Ellipsis", ExternalLink: "ExternalLink", Share2: "Share2", X: "X" }));
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
      previews: [preview], activeDownload: null, closePreview: jest.fn(), popPreview: jest.fn(),
      dismissPreviewThen: jest.fn(), downloadOriginal: jest.fn(),
      flushPendingPreviewAction: jest.fn(), openLinkedFile: jest.fn(async () => {}), t,
    }));
  });
  return tree;
}

/** Everything a node paints, in order — colored spans interleave with the raw
 * strings of the tokens Prism left uncolored. */
function painted(node: ReactTestInstance | string): string {
  return typeof node === "string"
    ? node
    : node.children.map(child => painted(child)).join("");
}

/** The body's virtualized rows: every chunk the list knows, and which of them
 * are actually mounted. */
function body(tree: ReactTestRenderer) {
  const list = tree.root.findByType(FlatList);
  const chunks = (list.props.data as { text: string }[]).map(row => row.text);
  const mounted = tree.root.findAllByType(Text).filter(node => node.props.selectable);
  const first = mounted[0]!;
  const spans = mounted.flatMap(node => node.findAllByType(Text).filter(inner => inner !== node));
  return {
    list,
    chunks,
    mounted,
    spans,
    // What the reader can select and copy out of the mounted rows.
    rendered: mounted.map(painted).join(""),
    // The same rows as the component paints them (chunk-ending newlines are the
    // layout's job, not the text's).
    paintedChunks: chunks.map(codeRowText),
    // Characters handed to native text layout. This is the cost that does not
    // scale: the file preview used to put the whole document (up to 2 MiB) in a
    // single `<Text>`.
    mountedChars: mounted.reduce((sum, node) => sum + painted(node).length, 0),
    style: StyleSheet.flatten(first.props.style),
  };
}

test("a code preview is colored and monospace without altering the source", () => {
  const tree = renderTextPreview("greet.py", python);
  try {
    const { chunks, rendered, paintedChunks, spans, style } = body(tree);
    expect(chunks.join("")).toBe(python);
    expect(rendered).toBe(paintedChunks.join(""));
    const colors = spans.map(node => StyleSheet.flatten(node.props.style)?.color);
    expect(colors).toContain(codeColors.keyword);
    expect(colors).toContain(codeColors.comment);
    expect(colors).toContain(codeColors.string);
    expect(style.fontFamily).toBe(monospace);
  } finally { act(() => tree.unmount()); }
});

test("prose previews keep the proportional font and stay uncolored", () => {
  const log = "2026-09-20 boot ok\n2026-09-20 ready\n";
  const tree = renderTextPreview("server.log", log);
  try {
    const { chunks, rendered, paintedChunks, spans, style } = body(tree);
    expect(chunks.join("")).toBe(log);
    expect(rendered).toBe(paintedChunks.join(""));
    expect(spans).toHaveLength(0);
    expect(style.fontFamily).toBeUndefined();
  } finally { act(() => tree.unmount()); }
});

test("a code file with no grammar still gets the monospace metrics", () => {
  // `Makefile`, `Cargo.lock` and `.csv` are code, config or tabular data — the
  // columns only line up monospace, and Prism ships no grammar for some of them.
  for (const [name, source] of [
    ["Cargo.lock", "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n"],
    ["rows.csv", "sample,value\na,1\n"],
    ["meson.build", "project('demo', 'c')\n"],
  ] as const) {
    const tree = renderTextPreview(name, source);
    try {
      const { chunks, spans, style } = body(tree);
      expect(chunks.join("")).toBe(source);
      expect(spans).toHaveLength(0);
      expect([name, style.fontFamily]).toEqual([name, monospace]);
    } finally { act(() => tree.unmount()); }
  }
});

test("a Makefile is highlighted with the grammar for its name", () => {
  const makefile = "# build\nall:\n\tcc -o app main.c\n";
  const tree = renderTextPreview("Makefile", makefile);
  try {
    const { chunks, rendered, paintedChunks, spans, style } = body(tree);
    expect(chunks.join("")).toBe(makefile);
    expect(rendered).toBe(paintedChunks.join(""));
    expect(spans.length).toBeGreaterThan(0);
    expect(spans.some(node => StyleSheet.flatten(node.props.style)?.color === codeColors.comment)).toBe(true);
    expect(style.fontFamily).toBe(monospace);
  } finally { act(() => tree.unmount()); }
});

test("a long code file mounts only its visible rows, not every token", () => {
  // 28 KB of dense source is what highlighting turns into thousands of spans;
  // mounting them all in one Text is what made opening a code file slow.
  const code = "export function handler(request: Request) {\n  const value = compute(request.body);\n  return { ok: true, value };\n}\n";
  const long = code.repeat(220);
  expect(long.length).toBeGreaterThan(20_000);
  const tree = renderTextPreview("big.ts", long);
  try {
    const { chunks, mounted, rendered, paintedChunks } = body(tree);
    // The whole file is reachable and byte-identical, but paged.
    expect(chunks.length).toBeGreaterThan(10);
    expect(chunks.join("")).toBe(long);
    expect(mounted.length).toBeLessThan(chunks.length);
    expect(rendered).toBe(paintedChunks.slice(0, mounted.length).join(""));
  } finally { act(() => tree.unmount()); }
});

test("code too large to tokenize keeps monospace and the untouched source", () => {
  const huge = "def long_line():\n" + "    value = 1  # comment\n".repeat(2000);
  const tree = renderTextPreview("huge.py", huge);
  try {
    const { chunks, spans, style } = body(tree);
    expect(huge.length).toBeGreaterThan(30_000);
    expect(chunks.join("")).toBe(huge);
    expect(spans).toHaveLength(0);
    expect(style.fontFamily).toBe(monospace);
  } finally { act(() => tree.unmount()); }
});

test("a very large file mounts a bounded slice, not the whole document", () => {
  // The route serves up to 2 MiB (MARKDOWN_RENDER_BYTES). Whatever the size,
  // native text layout only ever sees the rows on screen — that is what keeps
  // the preview from opening as a blank sheet while it lays out megabytes.
  const huge = "const value = compute(input); // a line of ordinary source\n".repeat(40_000);
  const tree = renderTextPreview("huge.ts", huge);
  try {
    const { chunks, mounted, mountedChars } = body(tree);
    expect(huge.length).toBeGreaterThan(2_000_000);
    expect(chunks.join("")).toBe(huge);
    expect(mounted.length).toBeLessThan(20);
    expect(mountedChars).toBeLessThan(32_768);
  } finally { act(() => tree.unmount()); }
});
