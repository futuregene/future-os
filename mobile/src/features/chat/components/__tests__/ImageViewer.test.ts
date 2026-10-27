import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ImageViewer } from "../ImageViewer";
import { ZoomableImage } from "../ZoomableImage";

jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("lucide-react-native", () => ({ X: "X" }));
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

function render(target: { uri: string; name?: string } | null, onClose = jest.fn()) {
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
});
