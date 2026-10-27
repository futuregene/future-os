import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Image, Text } from "react-native";
import { MarkdownText } from "../MarkdownText";
import { MarkdownImageLoaderContext, resolveMarkdownPath, type MarkdownImageLoader } from "../MarkdownImage";
import { ImageViewerContext } from "../imageViewer";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
afterEach(() => { act(() => tree?.unmount()); });
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
/** The tappable picture carrying a screen-reader label. */
const pressLabel = (label: string) => {
  const node = tree.root.findAll(candidate => candidate.props.accessibilityLabel === label && typeof candidate.props.onPress === "function").at(-1)!;
  act(() => node.props.onPress());
};
function render(loader: MarkdownImageLoader, text = "![chart](assets/a.png)", imageBasePath?: string) {
  return createElement(MarkdownImageLoaderContext, { value: loader }, createElement(MarkdownText, { text, imageBasePath }));
}

test.each([
  ["assets/a.png", undefined, "assets/a.png"],
  ["../images/a.png", "/docs/sub/report.md", "/docs/sub/../images/a.png"],
  ["a.png", "docs/report.md", "docs/a.png"],
  ["a.png", "C:\\Docs\\report.md", "C:\\Docs\\a.png"],
  ["D:/a.png", "C:/Docs/report.md", "D:/a.png"],
  ["a.png", "file:///phone/cache/report.md", null],
  ["https://example.com/a.png", "/docs/a.md", null],
  ["javascript:alert(1)", "/docs/a.md", null],
])("image path %s relative to %s", (src, base, expected) => {
  expect(resolveMarkdownPath(src!, base)).toBe(expected);
});

test("uncached images in a reply body load themselves", async () => {
  const loader = { scope: "desktop:session", cached: jest.fn(() => null), load: jest.fn(async () => "file:///verified.png") };
  await act(async () => { tree = create(render(loader, "![chart](assets/a.png)", "/docs/report.md")); });
  // No tap: the image the agent put in its answer is part of the reply.
  expect(loader.load).toHaveBeenCalledWith("/docs/assets/a.png", expect.any(AbortSignal));
  expect(tree.root.findByType(Image).props.source).toEqual({ uri: "file:///verified.png" });
  for (let node = tree.root.findByType(Image).parent; node; node = node.parent) expect(node.type).not.toBe(Text);
});

// A file preview is the same story: an image written into the document is part
// of it, so opening a document shows its images without a per-image tap.
test("images in a file preview load themselves too", async () => {
  const loader = { scope: "one", cached: jest.fn(() => null), load: jest.fn(async () => "file:///x.png") };
  await act(async () => { tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
    createElement(MarkdownText, { text: "![chart](./a.png)", mode: "file-preview" }))); });
  expect(loader.load).toHaveBeenCalledWith("a.png", expect.any(AbortSignal));
  expect(tree.root.findByType(Image).props.source.uri).toBe("file:///x.png");
});

test("already verified cached images display without downloading", () => {
  const loader = { scope: "one", cached: jest.fn(() => "file:///cached.png"), load: jest.fn() };
  act(() => { tree = create(render(loader)); });
  expect(tree.root.findByType(Image).props.source.uri).toBe("file:///cached.png");
  expect(loader.load).not.toHaveBeenCalled();
});

test("a failed auto-load offers retry, duplicate taps do not start duplicate work", async () => {
  let reject!: (error: Error) => void;
  const loader = { scope: "one", cached: () => null, load: jest.fn(() => new Promise<string>((_, r) => { reject = r; })) };
  await act(async () => { tree = create(render(loader)); });
  expect(loader.load).toHaveBeenCalledTimes(1);
  await act(async () => reject(new Error("offline")));
  expect(button("attachment.retryImage")).toBeDefined();
  loader.load.mockResolvedValue("file:///retry.png");
  await act(async () => button("attachment.retryImage").props.onPress());
  expect(tree.root.findByType(Image).props.source.uri).toBe("file:///retry.png");
  act(() => tree.root.findByType(Image).props.onError());
  expect(button("attachment.retryImage")).toBeDefined();
});

test("a double tap on the load button starts only one transfer", async () => {
  let resolveLoad!: (uri: string) => void;
  const loader = {
    scope: "one",
    cached: () => null,
    load: jest.fn(() => new Promise<string>(resolve => { resolveLoad = resolve; })),
  };
  await act(async () => { tree = create(render(loader)); });
  // The auto-start already owns a transfer; the taps land in the same frame,
  // before the disabled state has been rendered.
  expect(loader.load).toHaveBeenCalledTimes(1);
  await act(async () => {
    button("attachment.loadImage").props.onPress();
    button("attachment.loadImage").props.onPress();
  });
  // A second controller would race the first one's abort and could leave the
  // placeholder spinning forever with both transfers cancelled.
  expect(loader.load).toHaveBeenCalledTimes(1);
  await act(async () => resolveLoad("file:///a.png"));
  expect(tree.root.findByType(Image).props.source.uri).toBe("file:///a.png");
});

// A reply re-projects on every streaming delta; the image must not be
// re-requested on each one.
test("a streaming re-projection does not re-request the image", async () => {
  const loader = { scope: "one", cached: () => null, load: jest.fn(async () => "file:///a.png") };
  await act(async () => { tree = create(render(loader, "![chart](a.png)")); });
  expect(loader.load).toHaveBeenCalledTimes(1);
  await act(async () => { tree.update(render(loader, "![chart](a.png)\n\nmore text")); });
  expect(loader.load).toHaveBeenCalledTimes(1);
});

test("session/desktop changes cancel loads and never display a previous session's bytes", async () => {
  let resolveFirst!: (uri: string) => void;
  const signals: AbortSignal[] = [];
  const loader: MarkdownImageLoader = {
    scope: "one",
    cached: () => null,
    // The first session's transfer is the one that must be discarded; the second
    // (different scope) never settles here.
    load: (_path, signal) => {
      signals.push(signal);
      return signals.length === 1
        ? new Promise<string>(r => { resolveFirst = r; })
        : new Promise<string>(() => {});
    },
  };
  await act(async () => { tree = create(render(loader)); });
  expect(signals).toHaveLength(1);
  act(() => { tree.update(render({ ...loader, scope: "two" })); });
  // The previous session's transfer is cancelled, not merely ignored — and the
  // stale bytes never reach the screen even though its promise still resolves.
  expect(signals[0]!.aborted).toBe(true);
  await act(async () => resolveFirst("file:///old-session.png"));
  expect(tree.root.findAllByType(Image)).toHaveLength(0);
});

test("a link's picture opens its link, not the viewer", async () => {
  // A picture inside a link is the link's target: tapping it must go where the
  // author pointed, the same as tapping the text beside it.
  const onOpenFile = jest.fn();
  const openImageViewer = jest.fn();
  const loader = { scope: "one", cached: () => "file:///cached.png", load: jest.fn() };
  await act(async () => {
    tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
      createElement(ImageViewerContext, { value: openImageViewer },
        createElement(MarkdownText, { text: "[![chart](a.png)](./b.md)", onOpenFile }))));
  });
  act(() => button("chart").props.onPress());
  expect(openImageViewer).not.toHaveBeenCalled();
  expect(onOpenFile).toHaveBeenCalledWith("b.md");
});

test("a picture the reply drew full screen on a tap, without another transfer", async () => {
  const openImageViewer = jest.fn();
  const onOpenFile = jest.fn();
  const loader = { scope: "one", cached: () => null, load: jest.fn(async () => "file:///verified.png") };
  await act(async () => {
    tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
      createElement(ImageViewerContext, { value: openImageViewer },
        createElement(MarkdownText, { text: "![chart](assets/a.png)", onOpenFile }))));
  });
  pressLabel("chart");
  // The bytes are already here: the viewer opens on them, it does not route
  // through the file-preview download flow.
  expect(openImageViewer).toHaveBeenCalledWith({ name: "chart", uri: "file:///verified.png" });
  expect(onOpenFile).not.toHaveBeenCalled();
});

test("an image linked from elsewhere is still tappable", async () => {
  // No workspace file to read and no href: the only reason this one was dead to
  // touch is that nobody had anywhere to put it.
  const openImageViewer = jest.fn();
  const loader = { scope: "one", cached: () => null, load: jest.fn() };
  await act(async () => {
    tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
      createElement(ImageViewerContext, { value: openImageViewer },
        createElement(MarkdownText, { text: "![forest](https://example.com/forest.png)" }))));
  });
  pressLabel("forest");
  expect(openImageViewer).toHaveBeenCalledWith({ name: "forest", uri: "https://example.com/forest.png" });
});

test("without a viewer the picture keeps its old open-file target", async () => {
  // Every surface that is not the conversation (and any host with no viewer
  // mounted) must not grow a button with nowhere to go: the tap still means
  // "open this file", exactly as before.
  const onOpenFile = jest.fn();
  const loader = { scope: "one", cached: () => "file:///cached.png", load: jest.fn() };
  await act(async () => {
    tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
      createElement(MarkdownText, { onOpenFile, text: "![chart](assets/a.png)" })));
  });
  pressLabel("chart");
  expect(onOpenFile).toHaveBeenCalledWith("assets/a.png");
});

test("unsupported inline images retain the existing open-file fallback", async () => {
  const onOpenFile = jest.fn();
  const loader = { scope: "one", cached: () => null, load: jest.fn(async () => { throw new Error("unsupported"); }) };
  await act(async () => { tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
    createElement(MarkdownText, { text: "![chart](./a.svg)", onOpenFile }))); });
  const fallback = tree.root.findAllByType(Text).find(node => node.props.children === "attachment.open")!;
  act(() => fallback.props.onPress());
  expect(onOpenFile).toHaveBeenCalledWith("a.svg");
});

test("an image's intrinsic size sets the layout, and a renderer that omits it does not throw", () => {
  const loader = { scope: "one", cached: () => "file:///cached.png", load: jest.fn() };
  act(() => { tree = create(render(loader)); });
  const ratio = () => tree.root.findByType(Image).props.style[1].aspectRatio;
  expect(ratio()).toBe(1.5);
  act(() => tree.root.findByType(Image).props.onLoad({ nativeEvent: { source: { width: 400, height: 200 } } }));
  expect(ratio()).toBe(2);
  // react-native-web reports no `source` at all; the load callback must survive it
  // (this threw "Cannot read properties of undefined (reading 'width')").
  act(() => tree.root.findByType(Image).props.onLoad({ nativeEvent: {} }));
  act(() => tree.root.findByType(Image).props.onLoad({}));
  act(() => tree.root.findByType(Image).props.onLoad({ nativeEvent: { source: { width: 0, height: 0 } } }));
  expect(ratio()).toBe(2);
});

test("local-link-wrapped images and formatted labels are not flattened into text", () => {
  const loader = { scope: "one", cached: () => "file:///cached.png", load: jest.fn() };
  act(() => { tree = create(render(loader, "[![chart](a.png)](./report.md) [**bold**](./report.md)")); });
  expect(tree.root.findAllByType(Image)).toHaveLength(1);
  expect(JSON.stringify(tree.toJSON())).toContain('"fontWeight":"700"');
});
