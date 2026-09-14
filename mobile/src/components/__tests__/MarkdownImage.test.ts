import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Image, Text } from "react-native";
import { MarkdownText } from "../MarkdownText";
import { MarkdownImageLoaderContext, markdownImagePath, type MarkdownImageLoader } from "../MarkdownImage";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
afterEach(() => { act(() => tree?.unmount()); });
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
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
  expect(markdownImagePath(src!, base)).toBe(expected);
});

test("uncached local images only download after a click and render inline", async () => {
  const loader = { scope: "desktop:session", cached: jest.fn(() => null), load: jest.fn(async () => "file:///verified.png") };
  act(() => { tree = create(render(loader, "![chart](assets/a.png)", "/docs/report.md")); });
  expect(loader.load).not.toHaveBeenCalled();
  expect(tree.root.findAllByType(Image)).toHaveLength(0);
  await act(async () => button("attachment.loadImage").props.onPress());
  expect(loader.load).toHaveBeenCalledWith("/docs/assets/a.png", expect.any(AbortSignal));
  expect(tree.root.findByType(Image).props.source).toEqual({ uri: "file:///verified.png" });
  for (let node = tree.root.findByType(Image).parent; node; node = node.parent) expect(node.type).not.toBe(Text);
});

test("already verified cached images display without downloading", () => {
  const loader = { scope: "one", cached: jest.fn(() => "file:///cached.png"), load: jest.fn() };
  act(() => { tree = create(render(loader)); });
  expect(tree.root.findByType(Image).props.source.uri).toBe("file:///cached.png");
  expect(loader.load).not.toHaveBeenCalled();
});

test("failed loads offer retry, duplicate taps do not start duplicate work", async () => {
  let reject!: (error: Error) => void;
  const loader = { scope: "one", cached: () => null, load: jest.fn(() => new Promise<string>((_, r) => { reject = r; })) };
  act(() => { tree = create(render(loader)); });
  act(() => { button("attachment.loadImage").props.onPress(); button("attachment.loadImage").props.onPress(); });
  expect(loader.load).toHaveBeenCalledTimes(1);
  await act(async () => reject(new Error("offline")));
  expect(button("attachment.retryImage")).toBeDefined();
  loader.load.mockResolvedValue("file:///retry.png");
  await act(async () => button("attachment.retryImage").props.onPress());
  expect(tree.root.findByType(Image).props.source.uri).toBe("file:///retry.png");
  act(() => tree.root.findByType(Image).props.onError());
  expect(button("attachment.retryImage")).toBeDefined();
});

test("session/desktop changes cancel loads and never display a previous session's bytes", async () => {
  let resolve!: (uri: string) => void;
  let signal!: AbortSignal;
  const loader: MarkdownImageLoader = { scope: "one", cached: () => null, load: (_, s) => { signal = s; return new Promise(r => { resolve = r; }); } };
  act(() => { tree = create(render(loader)); });
  act(() => button("attachment.loadImage").props.onPress());
  act(() => tree.update(render({ ...loader, scope: "two" })));
  expect(signal.aborted).toBe(true);
  await act(async () => resolve("file:///old-session.png"));
  expect(tree.root.findAllByType(Image)).toHaveLength(0);
});

test("unsupported inline images retain the existing open-file fallback", async () => {
  const onOpenFile = jest.fn();
  const loader = { scope: "one", cached: () => null, load: jest.fn(async () => { throw new Error("unsupported"); }) };
  act(() => { tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
    createElement(MarkdownText, { text: "![chart](./a.svg)", onOpenFile }))); });
  await act(async () => button("attachment.loadImage").props.onPress());
  const fallback = tree.root.findAllByType(Text).find(node => node.props.children === "attachment.open")!;
  act(() => fallback.props.onPress());
  expect(onOpenFile).toHaveBeenCalledWith("a.svg");
});

test("local-link-wrapped images and formatted labels are not flattened into text", () => {
  const loader = { scope: "one", cached: () => "file:///cached.png", load: jest.fn() };
  act(() => { tree = create(render(loader, "[![chart](a.png)](./report.md) [**bold**](./report.md)")); });
  expect(tree.root.findAllByType(Image)).toHaveLength(1);
  expect(JSON.stringify(tree.toJSON())).toContain('"fontWeight":"700"');
});
