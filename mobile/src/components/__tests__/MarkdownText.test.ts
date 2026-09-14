import type { ReactTestRenderer } from "react-test-renderer";
import { createElement } from "react";
import { AccessibilityInfo, Alert, Animated, Image, Linking, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { act, create } from "react-test-renderer";
import { MarkdownText } from "../MarkdownText";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe("MarkdownText layout and fidelity", () => {
  let renderer: ReactTestRenderer;
  function render(text: string) {
    act(() => { renderer = create(createElement(MarkdownText, { text })); });
    return renderer.root;
  }
  afterEach(() => { act(() => renderer?.unmount()); });

  test("headings have distinct scales and accessible heading roles", () => {
    const root = render("# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six");
    const headings = root.findAllByType(Text).filter(node => node.props.accessibilityRole === "header");
    expect(headings).toHaveLength(6);
    const sizes = headings.map(node => StyleSheet.flatten(node.props.style).fontSize);
    expect(new Set(sizes).size).toBe(6);
    expect(sizes).toEqual([...sizes].sort((a, b) => b - a));
  });

  test("ordered markers retain start values and do not have a fixed clipping width", () => {
    const root = render("99. First\n100. Second\n\n- [x] done\n- [ ] todo");
    const markers = root.findAllByType(Text).filter(node => ["99.", "100."].includes(node.props.children));
    expect(markers).toHaveLength(2);
    for (const marker of markers) {
      expect(StyleSheet.flatten(marker.props.style)).toMatchObject({ flexShrink: 0, minWidth: 22 });
      expect(StyleSheet.flatten(marker.props.style).width).toBeUndefined();
    }
    expect(root.findAllByType(View).filter(node => node.props.accessibilityRole === "checkbox")
      .map(node => node.props.accessibilityState.checked)).toEqual([true, false]);
  });

  test("wide tables scroll as a unit with aligned columns, including missing cells", () => {
    const root = render("| A | B | C | D |\n|:---|:---:|---:|---|\n| one<br>two | **bold** | 3 | 4 |\n| missing |");
    const scroll = root.findByType(ScrollView);
    expect(scroll.props).toMatchObject({ horizontal: true, nestedScrollEnabled: true });
    const container = root.findAllByType(View).find(node => node.props.onLayout)!;
    act(() => container.props.onLayout({ nativeEvent: { layout: { width: 320 } } }));
    const cells = scroll.findAllByType(View).filter(node => typeof StyleSheet.flatten(node.props.style)?.width === "number");
    expect(cells).toHaveLength(12);
    const widths = cells.map(node => StyleSheet.flatten(node.props.style).width);
    expect(new Set(widths).size).toBe(1);
    expect(widths[0]).toBeGreaterThanOrEqual(144);
    expect(scroll.findAllByType(Text).filter(node => StyleSheet.flatten(node.props.style)?.textAlign === "right")).toHaveLength(2); // The padded empty cell needs no Text.
    expect(JSON.stringify(renderer.toJSON())).not.toContain("<br>");
    // A wider orientation updates all rows together.
    act(() => container.props.onLayout({ nativeEvent: { layout: { width: 1602 } } }));
    expect(scroll.findAllByType(View).filter(node => StyleSheet.flatten(node.props.style)?.width === 400)).toHaveLength(12);
  });

  test("code is selectable, horizontally scrollable and uses an iOS-safe monospace font", () => {
    const code = `const veryLongLine = '${"x".repeat(200)}';\n  indented();`;
    const root = render(`\`\`\`typescript\n${code}\n\`\`\``);
    expect(root.findByType(ScrollView).props.horizontal).toBe(true);
    const text = root.findAllByType(Text).find(node => node.props.children === code)!;
    expect(text.props.selectable).toBe(true);
    expect(StyleSheet.flatten(text.props.style).fontFamily).toBe(Platform.OS === "ios" ? "Menlo" : "monospace");
    expect(root.findAllByType(Text).some(node => node.props.children === "typescript")).toBe(true);
  });

  test("remote images are outside selectable Text and retain natural aspect ratio", () => {
    const root = render("before **bold ![chart](https://example.com/chart.png) after** end");
    const image = root.findByType(Image);
    for (let parent = image.parent; parent; parent = parent.parent) expect(parent.type).not.toBe(Text);
    expect(StyleSheet.flatten(image.props.style)).toMatchObject({ width: "100%", aspectRatio: 1.5 });
    expect(StyleSheet.flatten(image.props.style).height).toBeUndefined();
    act(() => image.props.onLoad({ nativeEvent: { source: { width: 800, height: 200 } } }));
    expect(StyleSheet.flatten(image.props.style).aspectRatio).toBe(4);
    expect(root.findAllByType(Text).filter(node => StyleSheet.flatten(node.props.style)?.fontWeight === "700")).toHaveLength(2);
    act(() => image.props.onError());
    expect(root.findAllByType(Image)).toHaveLength(0);
    expect(JSON.stringify(renderer.toJSON())).toContain("chart");
    act(() => renderer.update(createElement(MarkdownText, { text: "![new](https://example.com/new.png)" })));
    expect(root.findAllByType(Image)).toHaveLength(1);
  });

  test("linked images retain safe navigation without allowing unsafe protocols", () => {
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(true);
    try {
      const root = render("[![chart](https://example.com/chart.png)](https://example.com/docs) [![bad](https://example.com/bad.png)](javascript:alert(1))");
      const links = root.findAll(node => node.props.accessibilityRole === "link" && typeof node.props.onPress === "function");
      expect(links.length).toBeGreaterThan(0);
      expect(links.every(node => node.props.accessibilityLabel === "chart")).toBe(true);
      expect(root.findAllByType(Image)).toHaveLength(2);
      act(() => links[0]!.props.onPress());
      expect(open).toHaveBeenCalledWith("https://example.com/docs");
    } finally { open.mockRestore(); }
  });

  test("math and Mermaid remain honest source fallbacks, not missing content", () => {
    const root = render("Formula $x^2$\n\n$$\n\\frac{a}{b}\n$$\n\n```mermaid\ngraph TD; A-->B;\n```");
    expect(root.findAllByType(ScrollView)).toHaveLength(2);
    const output = JSON.stringify(renderer.toJSON());
    expect(output).toContain("x^2");
    expect(output).toContain("frac{a}{b}");
    expect(output).toContain("graph TD; A-->B;");
  });
});

describe("MarkdownText", () => {
  beforeEach(() => {
    jest.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(true);
  });
  afterEach(() => jest.restoreAllMocks());

  test("only newly appended blocks fade, not every update to their text", async () => {
    jest.useFakeTimers();
    const preference = jest.spyOn(AccessibilityInfo, "isReduceMotionEnabled").mockResolvedValue(false);
    const timing = jest.spyOn(Animated, "timing").mockReturnValue({ start: jest.fn(), stop: jest.fn(), reset: jest.fn() });
    let renderer!: ReactTestRenderer;
    try {
      await act(async () => { renderer = create(createElement(MarkdownText, { text: "Cached paragraph.", streaming: true })); });
      expect(timing).not.toHaveBeenCalled();
      act(() => renderer.update(createElement(MarkdownText, { text: "Cached paragraph.\n\nNew paragraph", streaming: true })));
      act(() => jest.advanceTimersByTime(192));
      expect(timing).toHaveBeenCalledTimes(1);
      expect(timing).toHaveBeenCalledWith(expect.anything(), { toValue: 1, duration: 180, useNativeDriver: true, isInteraction: false });
      act(() => renderer.update(createElement(MarkdownText, { text: "Cached paragraph.\n\nNew paragraph grows", streaming: true })));
      act(() => jest.advanceTimersByTime(192));
      expect(timing).toHaveBeenCalledTimes(1);
    } finally {
      act(() => renderer?.unmount());
      timing.mockRestore();
      preference.mockRestore();
      jest.useRealTimers();
    }
  });

  test("incremental prose/table rendering and finalization match ordinary rendering", () => {
    const source = "# Report\n\nA **paragraph**.\n\n| A | B |\n|---|---|\n| one | two |\n| three | four |\n\nDone.";
    let streamed!: ReactTestRenderer;
    let settled!: ReactTestRenderer;
    act(() => { streamed = create(createElement(MarkdownText, { text: "", streaming: true })); });
    try {
      for (let length = 1; length <= source.length; length += 7) {
        act(() => streamed.update(createElement(MarkdownText, { text: source.slice(0, length), streaming: true })));
      }
      act(() => {
        streamed.update(createElement(MarkdownText, { text: source, streaming: false }));
        settled = create(createElement(MarkdownText, { text: source }));
      });
      // Compare output, not the identities of per-instance layout callbacks.
      expect(JSON.stringify(streamed.toJSON())).toEqual(JSON.stringify(settled.toJSON()));
    } finally {
      act(() => { streamed.unmount(); settled?.unmount(); });
    }
  });


  test("renders a bold local-file link without exposing markdown syntax", () => {
    let renderer: ReactTestRenderer | undefined;
    const onOpenFile = jest.fn();
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          text: "**[gomoku.html](<./gomoku.html>)**",
          onOpenFile,
        }),
      );
    });

    const output = JSON.stringify(renderer?.toJSON());
    expect(output).toContain("gomoku.html");
    expect(output).not.toContain("[gomoku.html](<./gomoku.html>)");

    const linkedText = renderer?.root
      .findAllByType(Text)
      .find(node => typeof node.props.onPress === "function");
    expect(linkedText).toBeDefined();
    expect(linkedText?.props.style).toMatchObject({ textDecorationLine: "underline" });
    expect(linkedText?.props.style).not.toHaveProperty("backgroundColor");
    act(() => linkedText?.props.onPress());
    expect(onOpenFile).toHaveBeenCalledWith("gomoku.html");
  });

  test("uses the shared GFM parser for tables, tasks and nested formatting", () => {
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          text: "| A | B |\n|---|---|\n| **bold** | ~~old~~ |\n\n- [x] done",
        }),
      );
    });

    const output = JSON.stringify(renderer?.toJSON());
    expect(output).toContain("bold");
    expect(output).toContain("old");
    expect(output).toContain("done");
  });

  test("renders a local Markdown image as a file chip in a message", () => {
    const onOpenFile = jest.fn();
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          text: "![diagram](assets/pic.png)",
          onOpenFile,
        }),
      );
    });
    const chip = renderer?.root.findAllByType(Text).find(node => node.props.onPress);
    act(() => chip?.props.onPress());
    expect(onOpenFile).toHaveBeenCalledWith("assets/pic.png");
  });

  test("prompts for a local image from a Markdown file preview", () => {
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          mode: "file-preview",
          text: "![diagram](assets/pic.png)",
        }),
      );
    });
    const chip = renderer?.root.findAllByType(Text).find(node => node.props.onPress);
    act(() => chip?.props.onPress());
    expect(alert).toHaveBeenCalledTimes(1);
    alert.mockRestore();
  });

  test("renders only http(s) Markdown images as remote images", () => {
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          text: "![remote](https://example.com/pic.png) ![blocked](data:image/png;base64,x)",
        }),
      );
    });
    const images = renderer?.root.findAllByType(Image) ?? [];
    expect(images).toHaveLength(1);
    expect(images[0]?.props.source).toEqual({ uri: "https://example.com/pic.png" });
  });

  test("does not send blocked link protocols to the OS", () => {
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(true);
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(createElement(MarkdownText, { text: "[bad](javascript:alert(1))" }));
    });
    const pressable = renderer?.root.findAllByType(Text).find(node => node.props.onPress);
    expect(pressable).toBeUndefined();
    expect(open).not.toHaveBeenCalled();
    open.mockRestore();
  });
});
