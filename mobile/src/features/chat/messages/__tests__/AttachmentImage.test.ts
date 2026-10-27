import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Image } from "react-native";
import type { HistoryAttachment } from "../../../../remote/types";
import { MarkdownImageLoaderContext } from "../../../../components/MarkdownImage";
import { ImageViewerContext } from "../../../../components/imageViewer";
import { AttachmentImage } from "../AttachmentImage";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("lucide-react-native", () => Object.fromEntries(["FileText", "Paperclip", "X"].map(name => [name, name])));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let tree: ReactTestRenderer;
afterEach(() => {
  if (tree) act(() => tree.unmount());
  jest.restoreAllMocks();
});

const attachment = (fields: Partial<HistoryAttachment> & { name: string }): HistoryAttachment =>
  ({ path: `/files/${fields.name}`, ...fields });

function render(attachment: HistoryAttachment, providers: { loader?: { scope: string; cached(): string | null; load(): Promise<string | null> }; openViewer?: jest.Mock } = {}) {
  const inner = createElement(AttachmentImage, { attachment, onOpen: jest.fn() });
  const withViewer = providers.openViewer === undefined
    ? inner
    : createElement(ImageViewerContext, { value: providers.openViewer }, inner);
  const element = providers.loader === undefined
    ? withViewer
    : createElement(MarkdownImageLoaderContext, { value: providers.loader }, withViewer);
  act(() => { tree = create(element); });
}

/** The tappable picture carrying the attachment's name. */
const picture = (name: string) => tree.root.findAll(node => node.props.accessibilityLabel === name && typeof node.props.onPress === "function").at(-1);

describe("a sent image is drawn as itself", () => {
  test("a phone-local pick shows immediately and opens the viewer", () => {
    const openViewer = jest.fn();
    render(attachment({ kind: "image", name: "shot.png", path: "file:///phone/shot.png" }), { openViewer });
    expect(tree.root.findByType(Image).props.source).toEqual({ uri: "file:///phone/shot.png" });
    act(() => picture("shot.png")!.props.onPress());
    expect(openViewer).toHaveBeenCalledWith({ name: "shot.png", uri: "file:///phone/shot.png" });
  });

  test("a durable history entry reads the desktop copy, then opens the viewer on it", async () => {
    const openViewer = jest.fn();
    const load = jest.fn(async () => "file:///cache/shot.png");
    await act(async () => {
      render(attachment({ kind: "image", name: "shot.png" }), { loader: { cached: () => null, load, scope: "desktop:session" }, openViewer });
    });
    expect(load).toHaveBeenCalledWith("/files/shot.png", expect.any(AbortSignal));
    act(() => picture("shot.png")!.props.onPress());
    expect(openViewer).toHaveBeenCalledWith({ name: "shot.png", uri: "file:///cache/shot.png" });
  });

  test("a failed transfer leaves the chip, which is the flow that reports it", async () => {
    const onOpen = jest.fn();
    await act(async () => {
      tree = create(createElement(MarkdownImageLoaderContext, {
        value: { cached: () => null, load: async () => { throw new Error("refused"); }, scope: "s" },
      }, createElement(ImageViewerContext, { value: jest.fn() },
        createElement(AttachmentImage, { attachment: attachment({ kind: "image", name: "shot.png" }), onOpen }))));
    });
    expect(tree.root.findAllByType(Image)).toHaveLength(0);
    const chip = tree.root.findAll(node => node.props.accessibilityLabel === "shot.png" && typeof node.props.onPress === "function").at(-1)!;
    act(() => chip.props.onPress());
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  test("no loader (a surface that cannot read the desktop) keeps the chip", () => {
    render(attachment({ kind: "image", name: "shot.png" }), { openViewer: jest.fn() });
    expect(tree.root.findAllByType(Image)).toHaveLength(0);
  });

  test("a picture only loads once across streaming re-renders", async () => {
    const load = jest.fn(async () => "file:///cache/a.png");
    const loader = { cached: () => null, load, scope: "s" };
    const element = createElement(MarkdownImageLoaderContext, { value: loader },
      createElement(ImageViewerContext, { value: jest.fn() },
        createElement(AttachmentImage, { attachment: attachment({ kind: "image", name: "a.png" }) })));
    await act(async () => { tree = create(element); });
    await act(async () => { tree.update(element); });
    expect(load).toHaveBeenCalledTimes(1);
  });
});
