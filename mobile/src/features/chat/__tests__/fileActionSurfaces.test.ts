import { createElement } from "react";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { NativeFileActionSheet } from "../components/NativeFileActionSheet";
import { PreviewModal, previewLayerKey } from "../components/PreviewModal";
import { MarkdownText } from "../../../components/MarkdownText";
import { colors } from "../../../theme/tokens";
import type { ActiveDownload, FileAction, FileOperation } from "../utils";
import type { PreviewState } from "../useFileDownload";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("lucide-react-native", () => ({ ChevronLeft: "ChevronLeft", Download: "Download", Ellipsis: "Ellipsis", ExternalLink: "ExternalLink", Share2: "Share2", X: "X" }));
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
    previews: [preview], activeDownload: null, closePreview: jest.fn(), popPreview: jest.fn(),
    dismissPreviewThen, downloadOriginal, flushPendingPreviewAction: jest.fn(),
    openLinkedFile: jest.fn(async () => {}), t,
  })); });
  try {
    expect(tree.root.findAll(node => node.props.accessibilityLabel === `attachment.${operation}`)).toHaveLength(0);
    const more = tree.root.findAll(node => node.props.accessibilityLabel === "common.more" && typeof node.props.onPress === "function")[0]!;
    act(() => more.props.onPress());
    expect(tree.root.findAllByType(Modal)).toHaveLength(1);
    const button = tree.root.findAll(node => node.props.accessibilityLabel === `attachment.${operation}` && typeof node.props.onPress === "function")[0]!;
    act(() => button.props.onPress());
    expect(dismissPreviewThen).toHaveBeenCalledTimes(1);
    expect(tree.root.findAll(node => node.props.accessibilityLabel === `attachment.${operation}`)).toHaveLength(0);
    expect(downloadOriginal).not.toHaveBeenCalled();
    act(() => pending());
    if (operation === "save") expect(downloadOriginal).toHaveBeenCalledWith(preview.attachment);
    else expect(downloadOriginal).toHaveBeenCalledWith(preview.attachment, operation);
  } finally { act(() => tree.unmount()); }
});

describe("preview overflow menu", () => {
  const preview: PreviewState = {
    attachment: { path: "/notes.txt", name: "notes.txt" },
    info: { ...action.info, name: "A very long filename for a narrow phone.txt", previewKind: "text", variant: "preview" },
    uri: "file:///preview.txt", text: "Preview content",
  };
  const activeDownload: ActiveDownload = {
    id: "download", fileName: "notes.txt", phase: "sharing", completedBytes: 3, totalBytes: 3,
  };
  const props = {
    previews: [preview], activeDownload: null, closePreview: jest.fn(), popPreview: jest.fn(),
    dismissPreviewThen: jest.fn(), downloadOriginal: jest.fn(),
    flushPendingPreviewAction: jest.fn(), openLinkedFile: jest.fn(async () => {}), t,
  };
  let tree: ReactTestRenderer;
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
  const openMenu = () => act(() => button("common.more").props.onPress());
  const menus = () => tree.root.findAllByType(View).filter(node => node.props.accessibilityViewIsModal);
  beforeEach(() => {
    jest.clearAllMocks();
    act(() => { tree = create(createElement(PreviewModal, props)); });
  });
  afterEach(() => act(() => tree.unmount()));

  test("keeps the title and only two 44-point controls in a single header row", () => {
    const title = tree.root.findAllByType(Text).find(node => node.props.children === preview.info.name)!;
    expect(title.props.numberOfLines).toBe(1);
    const header = tree.root.findAllByType(View).find(node => typeof node.props.onLayout === "function")!;
    expect(StyleSheet.flatten(header.props.style)).toMatchObject({ flexDirection: "row", minHeight: 60 });
    const controls = header.findAll(node => node.props.accessibilityRole === "button" && typeof node.props.style === "function");
    expect(controls.map(node => node.props.accessibilityLabel)).toEqual(["common.more", "common.close"]);
    for (const control of controls) {
      expect(StyleSheet.flatten(control.props.style({ pressed: false }))).toMatchObject({ width: 44, height: 44 });
    }
    expect(menus()).toHaveLength(0);
  });

  test.each(["backdrop", "back", "escape"])("%s dismisses only the menu and leaves the preview open", method => {
    openMenu();
    expect(menus()).toHaveLength(1);
    expect(tree.root.findAllByType(Modal)).toHaveLength(1);
    const hiddenContent = tree.root.findAllByType(View).find(node => node.props.accessibilityElementsHidden)!;
    expect(hiddenContent.props.importantForAccessibility).toBe("no-hide-descendants");
    act(() => {
      if (method === "backdrop") tree.root.findAll(node => node.props.testID === "preview-menu-backdrop" && node.props.onPress)[0]!.props.onPress();
      else if (method === "escape") menus()[0]!.props.onAccessibilityEscape();
      else tree.root.findByType(Modal).props.onRequestClose();
    });
    expect(menus()).toHaveLength(0);
    expect(props.closePreview).not.toHaveBeenCalled();
    expect(props.dismissPreviewThen).not.toHaveBeenCalled();
    act(() => tree.root.findByType(Modal).props.onRequestClose());
    expect(props.closePreview).toHaveBeenCalledTimes(1);
  });

  test("closing and changing documents do not retain the expanded menu", () => {
    openMenu();
    act(() => tree.update(createElement(PreviewModal, { ...props, previews: [] })));
    act(() => tree.update(createElement(PreviewModal, props)));
    expect(menus()).toHaveLength(0);
    openMenu();
    act(() => tree.update(createElement(PreviewModal, { ...props, previews: [{ ...preview, attachment: { path: "/other.txt", name: "other.txt" } }] })));
    expect(menus()).toHaveLength(0);
    act(() => button("common.close").props.onPress());
    expect(props.closePreview).toHaveBeenCalledTimes(1);
  });

  test("active downloads close the menu and disable and dim its trigger, but not close", () => {
    openMenu();
    act(() => tree.update(createElement(PreviewModal, { ...props, activeDownload })));
    expect(menus()).toHaveLength(0);
    const more = button("common.more");
    expect(more.props.disabled).toBe(true);
    expect(more.props.accessibilityState).toMatchObject({ disabled: true, busy: true });
    expect(StyleSheet.flatten(more.props.style({ pressed: false })).opacity).toBe(0.4);
    act(() => button("common.close").props.onPress());
    expect(props.closePreview).toHaveBeenCalledTimes(1);
    act(() => tree.update(createElement(PreviewModal, props)));
    expect(menus()).toHaveLength(0);
    expect(button("common.more").props.disabled).toBe(false);
  });

  test("markdown previews put their gutter on the scrolling list, not on a static parent", () => {
    const markdown: PreviewState = {
      ...preview,
      info: { ...preview.info, previewKind: "markdown" },
      markdown: "# Title\n\nBody",
      truncated: true,
    };
    act(() => tree.update(createElement(PreviewModal, { ...props, previews: [markdown] })));
    // No static parent of the document may apply a uniform gutter: padding there
    // stays put while the list scrolls, leaving a blank strip under the header.
    const padded = tree.root.findAllByType(View)
      .filter(node => StyleSheet.flatten(node.props.style)?.padding !== undefined);
    expect(padded).toHaveLength(0);
    // The truncation notice stays outside the scrolling surface, so it keeps its own gutter.
    const notice = tree.root.findAllByType(Text).find(node => node.props.children === "attachment.markdownTruncated")!;
    expect(StyleSheet.flatten(notice.parent!.props.style)).toMatchObject({ paddingTop: 16, paddingHorizontal: 16 });
  });

  test("positions the floating menu below the measured header without reflowing content", () => {
    const header = tree.root.findAllByType(View).find(node => typeof node.props.onLayout === "function")!;
    act(() => header.props.onLayout({ nativeEvent: { layout: { height: 72 } } }));
    openMenu();
    const overlay = tree.root.findAllByType(View).find(node => StyleSheet.flatten(node.props.style)?.paddingTop === 76)!;
    expect(StyleSheet.flatten(overlay.props.style)).toMatchObject({ position: "absolute", top: 0, bottom: 0 });
    expect(StyleSheet.flatten(menus()[0]!.props.style)).toMatchObject({ maxWidth: 240, flexShrink: 1 });
  });

  test("native dismissal still flushes the pending preview action", () => {
    openMenu();
    act(() => tree.root.findByType(Modal).props.onDismiss());
    expect(props.flushPendingPreviewAction).toHaveBeenCalledTimes(1);
    expect(menus()).toHaveLength(0);
  });
});

describe("preview stack", () => {
  const doc = (path: string, kind: PreviewState["info"]["previewKind"] = "markdown"): PreviewState => ({
    attachment: { path, name: path.slice(path.lastIndexOf("/") + 1) },
    info: { ...action.info, name: path, previewKind: kind, variant: "preview" },
    uri: `file:///cache${path}`,
    markdown: "# Title\n\nBody",
  });
  const props = {
    activeDownload: null, closePreview: jest.fn(), popPreview: jest.fn(), dismissPreviewThen: jest.fn(),
    downloadOriginal: jest.fn(), flushPendingPreviewAction: jest.fn(), openLinkedFile: jest.fn(async () => {}), t,
  };
  let tree!: ReactTestRenderer;
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
  const surface = () => tree.root.findAllByType(View).find(node => node.props.testID === "preview-surface")!;
  // Found *under* the surface on purpose: the padding only reaches the layers
  // while they hang off something the surface lays out normally.
  const stack = () => surface().findAllByType(View).find(node => node.props.testID === "preview-stack")!;
  // One layer per open document, bottom first. `findAllByType` counts the
  // element we wrote, not the host view it renders.
  const layers = () => tree.root.findAllByType(View).filter(node => node.props.testID === "preview-layer");
  // Test IDs from the nearest ancestor outwards.
  const ancestorsOf = (node: ReactTestInstance) => {
    const ids: string[] = [];
    for (let parent = node.parent; parent; parent = parent.parent) ids.push(parent.props?.testID as string);
    return ids;
  };
  beforeEach(() => jest.clearAllMocks());
  afterEach(() => act(() => tree.unmount()));

  test("the document that linked here stays open underneath, reachable by the back control", () => {
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/publishing.md"), doc("/root/SOURCES.md")] })); });
    expect(layers()).toHaveLength(2);
    // Only the top document is interactive; the one below keeps nothing but its place.
    expect(layers().map(layer => layer.props.pointerEvents)).toEqual(["none", "auto"]);
    expect(layers().map(layer => layer.props.importantForAccessibility)).toEqual(["no-hide-descendants", "auto"]);
    act(() => button("common.back").props.onPress());
    expect(props.popPreview).toHaveBeenCalledTimes(1);
    expect(props.closePreview).not.toHaveBeenCalled();
  });

  test("a covered document never shows through the one above it", () => {
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/publishing.md"), doc("/root/SOURCES.md")] })); });
    // The layers are absolutely positioned siblings, and the documents' text is
    // transparent, so an unpainted layer lets the covered document's header and
    // every line of its body draw over the top document's.
    expect(layers().map(layer => StyleSheet.flatten(layer.props.style).backgroundColor)).toEqual([colors.surface, colors.surface]);
  });

  test("the reader's surface clears the Android status bar", () => {
    // The reader draws under the status bar on purpose, so its header — and with
    // it the buttons' hit areas — has to be inset back out of the system bar's
    // reach. A SafeAreaView cannot do that inside a Modal, where there is no
    // provider to resolve insets against.
    Platform.OS = "android";
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/publishing.md")] })); });
    expect(tree.root.findByType(Modal).props.statusBarTranslucent).toBe(true);
    expect(StyleSheet.flatten(surface().props.style)).toMatchObject({ paddingTop: 24 });
  });

  test("the inset reaches the documents, which hang off their own flex stack", () => {
    // Yoga positions an `absoluteFill` child from its parent's border edge and
    // sizes it to the border box, so a parent's padding never moves one: padding
    // the surface directly (as the first attempt did) left the layers at the top
    // of the screen, still under the status bar. The padding has to land on a box
    // whose child lays out normally, and the layers hang off that instead.
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/a.md"), doc("/root/b.md")] })); });
    expect(layers().every(layer => ancestorsOf(layer).includes("preview-stack"))).toBe(true);
    expect(StyleSheet.flatten(stack().props.style)).toMatchObject({ flex: 1 });
  });

  test("an iOS page sheet takes no top inset: the system already clears the status bar for it", () => {
    Platform.OS = "ios";
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/publishing.md")] })); });
    expect(StyleSheet.flatten(surface().props.style).paddingTop).toBeUndefined();
  });

  test("the outermost document has no back control: there is nothing under it", () => {
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/publishing.md")] })); });
    expect(tree.root.findAll(node => node.props.accessibilityLabel === "common.back")).toHaveLength(0);
  });

  test("hardware back leaves one document at a time and closes only at the outermost", () => {
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/a.md"), doc("/root/b.md")] })); });
    act(() => tree.root.findByType(Modal).props.onRequestClose());
    expect(props.popPreview).toHaveBeenCalledTimes(1);
    expect(props.closePreview).not.toHaveBeenCalled();
    act(() => { tree.update(createElement(PreviewModal, { ...props, previews: [doc("/root/a.md")] })); });
    act(() => tree.root.findByType(Modal).props.onRequestClose());
    expect(props.closePreview).toHaveBeenCalledTimes(1);
  });

  test("a link inside a Markdown document opens through the stack, keeping the document", () => {
    act(() => { tree = create(createElement(PreviewModal, { ...props, previews: [doc("/root/articles/cover.md")] })); });
    const document = tree.root.findAllByType(MarkdownText)[0]!;
    expect(document.props.mode).toBe("file-preview");
    // The base is the document itself: the caller resolves the link against it.
    expect(document.props.imageBasePath).toBe("/root/articles/cover.md");
    act(() => document.props.onOpenFile("/root/SOURCES.md"));
    expect(props.openLinkedFile).toHaveBeenCalledWith("/root/SOURCES.md");
    expect(props.dismissPreviewThen).not.toHaveBeenCalled();
  });

  test.each([
    { expected: "/root/b.md#1", index: 1, name: "two documents", previews: [doc("/root/a.md"), doc("/root/b.md")] },
    // A → B → A must give the second visit its own layer, not B's rows.
    { expected: "/root/a.md#2", index: 1, name: "a second visit", previews: [doc("/root/a.md"), doc("/root/a.md")] },
  ])("keys a layer by document per visit: $name", ({ previews, index, expected }) => {
    expect(previewLayerKey(previews, index)).toBe(expected);
  });

  test("going back keeps the key of the document that stayed mounted", () => {
    const stack = [doc("/root/a.md"), doc("/root/b.md")];
    expect(previewLayerKey([stack[0]!], 0)).toBe(previewLayerKey(stack, 0));
  });
});
