import type { ImageViewerTarget } from "../../../../components/imageViewer";
import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ImageViewer } from "../ImageViewer";
import { ZoomableImage } from "../ZoomableImage";
import { MarkdownImageLoaderContext } from "../../../../components/MarkdownImage";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("lucide-react-native", () => ({ ChevronLeft: "ChevronLeft", ChevronRight: "ChevronRight", X: "X" }));
// The viewer reads the window's insets for its header; the test tree has no
// provider above it, so the hook answers with zeros.
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ bottom: 0, left: 0, right: 0, top: 0 }),
}));

let tree: ReactTestRenderer;
afterEach(() => {
  if (tree) act(() => tree.unmount());
  jest.restoreAllMocks();
});

function render(target: ImageViewerTarget | null, onClose = jest.fn()) {
  act(() => { tree = create(createElement(ImageViewer, { onClose, target })); });
  return onClose;
}

describe("the conversation's image viewer", () => {
  test("draws the picture full screen with its name, and a way out", () => {
    const onClose = render({ name: "forest-plot.png", uri: "file:///cache/forest.png" });
    expect(tree.root.findByType(ZoomableImage).props.uri).toBe("file:///cache/forest.png");
    expect(tree.root.findAll(node => node.props.children === "forest-plot.png")).not.toHaveLength(0);
    // The header's close control is the only labelled button: the picture itself
    // is panned with fingers, not a button.
    const close = tree.root.findAll(node => node.props.accessibilityLabel === "common.close" && node.props.onPress).at(-1)!;
    act(() => close.props.onPress());
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  test("renders nothing while no picture is open", () => {
    render(null);
    expect(tree.root.findAllByType(ZoomableImage)).toHaveLength(0);
  });

  test("a picture with no name still opens (an inline reply image may have only alt text)", () => {
    render({ uri: "https://example.com/a.png" });
    expect(tree.root.findByType(ZoomableImage).props.uri).toBe("https://example.com/a.png");
  });

  describe("a drawing with several pictures", () => {
    const entries = [
      { id: "one", name: "one.png", src: "https://example.com/one.png", uri: "https://example.com/one.png" },
      { id: "two", name: "two.png", src: "https://example.com/two.png", uri: "https://example.com/two.png" },
      { id: "three", name: "three.png", src: "https://example.com/three.png", uri: "https://example.com/three.png" },
    ];
    const opened = (index: number) => ({ uri: entries[index]!.uri, name: entries[index]!.name, gallery: { entries, index } });
    const press = (label: string) => {
      const node = tree.root.findAll(candidate => candidate.props.accessibilityLabel === label && typeof candidate.props.onPress === "function").at(-1)!;
      act(() => node.props.onPress());
    };
    const shown = () => tree.root.findByType(ZoomableImage).props.uri;

    test("opens on the tapped picture and pages both ways with wrap-around", () => {
      render(opened(0));
      expect(shown()).toBe("https://example.com/one.png");
      press("common.nextImage");
      expect(shown()).toBe("https://example.com/two.png");
      expect(tree.root.findAll(node => node.props.children === "two.png").length).toBeGreaterThan(0);
      press("common.previousImage");
      expect(shown()).toBe("https://example.com/one.png");
      // Left from the first lands on the last.
      press("common.previousImage");
      expect(shown()).toBe("https://example.com/three.png");
      // And right from the last returns to the first.
      press("common.nextImage");
      expect(shown()).toBe("https://example.com/one.png");
    });

    test("a sideways swipe asks the same step the arrows do", () => {
      render(opened(0));
      act(() => tree.root.findByType(ZoomableImage).props.onPage(1));
      expect(shown()).toBe("https://example.com/two.png");
      act(() => tree.root.findByType(ZoomableImage).props.onPage(-1));
      expect(shown()).toBe("https://example.com/one.png");
      act(() => tree.root.findByType(ZoomableImage).props.onPage(-1));
      expect(shown()).toBe("https://example.com/three.png");
    });

    test("shows where the reader is in the sequence", () => {
      render(opened(1));
      const position = tree.root.findAll(node => node.props.accessibilityLabel === "common.imagePosition")[0]!;
      expect(position.props.children).toEqual([2, " / ", 3]);
    });

    test("leaves a lone picture without arrows", () => {
      render({ uri: "https://example.com/only.png", name: "only.png" });
      expect(tree.root.findAll(node => node.props.accessibilityLabel === "common.nextImage")).toHaveLength(0);
      expect(tree.root.findAll(node => node.props.accessibilityLabel === "common.previousImage")).toHaveLength(0);
    });

    test("loads a picture whose bytes had not arrived when the viewer opened", async () => {
      const load = jest.fn(async () => "file:///loaded.png");
      const loader = { scope: "one", cached: () => null, load };
      const target = {
        uri: "file:///first.png",
        name: "first.png",
        gallery: {
          entries: [
            { id: "first", name: "first.png", src: "file:///first.png", uri: "file:///first.png" },
            { id: "second", name: "second.png", src: "assets/second.png", uri: null },
          ],
          index: 0,
        },
      };
      await act(async () => {
        tree = create(createElement(MarkdownImageLoaderContext, { value: loader },
          createElement(ImageViewer, { onClose: jest.fn(), target })));
      });
      act(() => tree.root.findByType(ZoomableImage).props.onPage(1));
      await act(async () => { await Promise.resolve(); });
      expect(load).toHaveBeenCalledWith("assets/second.png", expect.any(AbortSignal));
      expect(tree.root.findByType(ZoomableImage).props.uri).toBe("file:///loaded.png");
    });
  });
});
