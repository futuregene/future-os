import { createElement } from "react";
import { Platform } from "react-native";
import * as Clipboard from "expo-clipboard";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { MarkdownText } from "../../../../components/MarkdownText";
import { PromptReader } from "../PromptReader";
import { previewSurfaceInsets } from "../previewInsets";

jest.mock("../../../../components/MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("lucide-react-native", () => ({ Check: "Check", Copy: "Copy", X: "X" }));
jest.mock("expo-clipboard", () => ({ setStringAsync: jest.fn(async () => true) }));
const mockCopy = jest.fn();
const mockCopyState = { copied: false };
jest.mock("../../messages/useCopyState", () => ({ useCopyState: () => ({ copied: mockCopyState.copied, copy: () => mockCopy() }) }));
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

let tree: ReactTestRenderer;
afterEach(() => {
  if (tree) act(() => tree.unmount());
  mockCopyState.copied = false;
  jest.clearAllMocks();
});

function render(element: Parameters<typeof create>[0]) {
  act(() => { tree = create(element); });
}

describe("the prompt reader", () => {
  test("draws the whole prompt through the preview's own markdown mode", () => {
    // `file-preview` is what makes this safe: it paints one list row per block,
    // so no single view is taller than a paragraph — the shape that left the
    // transcript's bubble blank.
    const text = Array.from({ length: 60 }, (_, i) => `Paragraph ${i}: long prompt prose.`).join("\n\n");
    render(createElement(PromptReader, { text, visible: true, onClose: jest.fn() }));
    const body = tree.root.findByType(MarkdownText);
    expect(body.props).toMatchObject({ text, mode: "file-preview" });
    expect(body.props.tone).toBeUndefined();
    // The whole prompt, not the transcript's preview head.
    expect(body.props.text).toBe(text);
  });

  test("the header names the prompt and closes the surface", () => {
    const onClose = jest.fn();
    render(createElement(PromptReader, { text: "a prompt", visible: true, onClose }));
    expect(tree.root.findAll(node => node.props.accessibilityRole === "header")).not.toHaveLength(0);
    const close = tree.root.findAll(node =>
      typeof node.props.onPress === "function" && node.props.accessibilityLabel === "common.close")[0]!;
    act(() => close.props.onPress());
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test("copy hands the whole prompt to the clipboard", () => {
    // The reader exists so the prompt can be read; taking it away with you is
    // the other half, and it must be the whole text rather than the bubble's
    // preview.
    const text = "the entire prompt, not the head";
    render(createElement(PromptReader, { text, visible: true, onClose: jest.fn() }));
    const button = tree.root.findAll(node =>
      typeof node.props.onPress === "function" && node.props.accessibilityLabel === "chat.copy")[0]!;
    act(() => button.props.onPress());
    expect(Clipboard.setStringAsync).toHaveBeenCalledWith(text);
    return Promise.resolve().then(() => expect(mockCopy).toHaveBeenCalled());
  });

  test("a copied prompt says so in place of the copy glyph", () => {
    // The confirmation is the only feedback this surface has; asserting the
    // icon swap keeps it from silently disappearing.
    mockCopyState.copied = true;
    render(createElement(PromptReader, { text: "a prompt", visible: true, onClose: jest.fn() }));
    const glyphs = (name: string) => tree.root.findAll(node => String(node.type) === name);
    expect(glyphs("Check")).toHaveLength(1);
    expect(glyphs("Copy")).toHaveLength(0);
  });

  test("the text is on the app's own surface, not the bubble's accent fill", () => {
    // A document to read, not the reader's own bubble: the accent fill would
    // also drag the on-accent tone and its palette with it.
    render(createElement(PromptReader, { text: "a prompt", visible: true, onClose: jest.fn() }));
    const body = tree.root.findByType(MarkdownText);
    expect(body.props.tone).toBeUndefined();
  });
});

describe("a reader's insets", () => {
  test("Android pushes the surface out of the status bar", () => {
    // The reader is a Modal, which is its own native window; a SafeAreaView
    // inside one resolves nothing, so the top inset has to come from the
    // context and land as padding here.
    jest.replaceProperty(Platform, "OS", "android");
    expect(previewSurfaceInsets({ top: 24, bottom: 34, left: 0, right: 0 })).toEqual({ paddingTop: 24 });
  });

  test("iOS takes nothing, because its pageSheet already sits below the bar", () => {
    jest.replaceProperty(Platform, "OS", "ios");
    expect(previewSurfaceInsets({ top: 24, bottom: 34, left: 0, right: 0 })).toBeNull();
  });
});
