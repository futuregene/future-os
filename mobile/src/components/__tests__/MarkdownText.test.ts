import type { ReactTestRenderer } from "react-test-renderer";
import { createElement } from "react";
import { AccessibilityInfo, Animated, FlatList, Image, Linking, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { codeColors } from "../../theme/tokens";
import { AppAlert as Alert } from "../appAlerts";
import { act, create } from "react-test-renderer";
import { MarkdownText } from "../MarkdownText";
import { SvgXml } from "react-native-svg";
import * as parser from "../../../../packages/markdown/src/parseFutureMarkdown";

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

  test("large file previews virtualize blocks and bypass the shared message parse cache", () => {
    const parse = jest.spyOn(parser, "parseFutureMarkdown");
    const text = Array.from({ length: 2000 }, (_, i) => `Paragraph ${i}: ${"readable words ".repeat(12)}\n\n`).join("");
    try {
      act(() => { renderer = create(createElement(MarkdownText, { text, mode: "file-preview" })); });
      const list = renderer.root.findByType(FlatList);
      expect(list.props.data).toHaveLength(2000);
      expect(list.props).toMatchObject({ initialNumToRender: 8, maxToRenderPerBatch: 8, windowSize: 5 });
      expect(parse).toHaveBeenCalledWith(text, undefined, false);
      expect(renderer.root.findAllByType(Text)).toHaveLength(8);
    } finally { parse.mockRestore(); }
  });

  test("a file preview carries its gutter on the scrolling content, not a static parent", () => {
    const text = "# Title\n\n" + "Body paragraph. ".repeat(400);
    act(() => { renderer = create(createElement(MarkdownText, { text, mode: "file-preview" })); });
    const list = renderer.root.findByType(FlatList);
    // Padding on a parent of the list would stay put while the text scrolls,
    // showing a blank strip under the header and clipping the first line.
    expect(StyleSheet.flatten(list.props.contentContainerStyle)).toMatchObject({ padding: 16 });
    expect(StyleSheet.flatten(list.props.style)).toMatchObject({ flex: 1 });
  });

  test("a 5000-row table mounts a bounded internal viewport", () => {
    const text = "| A | B |\n|---|---|\n" + Array.from({ length: 5000 }, (_, i) => `| ${i} | value |\n`).join("");
    const root = render(text);
    const list = root.findByType(FlatList);
    expect(list.props.data).toHaveLength(5000);
    expect(root.findAllByType(Text).length).toBeLessThan(100);
    const headerCells = root.findByType(ScrollView).findAllByType(View)
      .filter(node => StyleSheet.flatten(node.props.style)?.paddingHorizontal === 8).slice(0, 2);
    expect(headerCells).toHaveLength(2);
    expect(StyleSheet.flatten(list.props.style).width).toBe(headerCells.reduce((sum, cell) =>
      sum + StyleSheet.flatten(cell.props.style).width, 0));
  });

  test("large code previews a bounded wrapped head and expands to the whole source", () => {
    const code = "line with real indentation\n".repeat(5000) + "x".repeat(100000);
    const root = render(`\`\`\`ts\n${code}\n\`\`\``);
    const rows = () => root.findAllByType(Text).filter(node => node.props.selectable);
    expect(rows()).toHaveLength(1);
    const head = rows()[0]!.props.children[1] as string;
    expect(rows()[0]!.props).toMatchObject({ numberOfLines: 16, ellipsizeMode: "tail" });
    expect(head.length).toBeLessThanOrEqual(2048);
    expect(code.startsWith(head)).toBe(true);
    const toggle = root.findAll(node => node.props.accessibilityLabel === "chat.expandCode" && typeof node.props.onPress === "function")[0]!;
    act(() => toggle.props.onPress());
    const expanded = rows();
    expect(expanded.length).toBeGreaterThan(40);
    expect(expanded.every(node => node.props.numberOfLines === undefined && node.props.ellipsizeMode === undefined)).toBe(true);
    // Every chunk is painted: the tail is reachable by scrolling the message, not an inner viewport.
    const painted = expanded.map(node => node.props.children[1] as string).join("");
    expect(painted.length).toBeGreaterThan(code.length - 500);
    expect(painted.endsWith(code.slice(-200))).toBe(true);
    expect(root.findAll(node => node.props.accessibilityLabel === "chat.collapseCode" && typeof node.props.onPress === "function").length).toBe(1);
  });

  test("CJK list labels ending in punctuation render as bold native Text", () => {
    const labels = ["迁移失败处理不合适：", "新旧端兼容未完善：", "失效工作区校验不足："];
    const root = render(labels.map(label => `- **${label}**正文继续。`).join("\n"));
    const bold = root.findAllByType(Text).filter(node => StyleSheet.flatten(node.props.style)?.fontWeight === "700");
    expect(bold.map(node => node.props.children.join(""))).toEqual(labels);
    expect(JSON.stringify(renderer.toJSON())).not.toContain("**");
  });

  test("CJK labels followed by a digit or a Latin name render bold without literal asterisks", () => {
    const root = render("**复现：**10 轮对话，每轮 assistant 内容 150 KB。\n\n**结论：**FutureOS 侧仍然超限。");
    const bold = root.findAllByType(Text).filter(node => StyleSheet.flatten(node.props.style)?.fontWeight === "700");
    expect(bold.map(node => node.props.children.join(""))).toEqual(["复现：", "结论："]);
    expect(JSON.stringify(renderer.toJSON())).not.toContain("**");
  });

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
    const root = render("| A long heading | B long heading | C long heading | D |\n|:---|:---:|---:|---|\n| one<br>two | **bold** | 3 | 4 |\n| missing |");
    const scroll = root.findByType(ScrollView);
    expect(scroll.props).toMatchObject({ horizontal: true, nestedScrollEnabled: true });
    const container = root.findAllByType(View).find(node => node.props.onLayout)!;
    act(() => container.props.onLayout({ nativeEvent: { layout: { width: 320 } } }));
    const cells = scroll.findAllByType(View).filter(node => typeof StyleSheet.flatten(node.props.style)?.width === "number");
    expect(cells).toHaveLength(12);
    const widths = cells.map(node => StyleSheet.flatten(node.props.style).width);
    expect(widths.slice(4, 8)).toEqual(widths.slice(0, 4));
    expect(widths.slice(8, 12)).toEqual(widths.slice(0, 4));
    expect(widths[3]).toBeLessThan(widths[0]);
    expect(widths.slice(0, 4).reduce((sum, width) => sum + width, 0)).toBeGreaterThan(320);
    expect(scroll.findAllByType(Text).filter(node => StyleSheet.flatten(node.props.style)?.textAlign === "right")).toHaveLength(2); // The padded empty cell needs no Text.
    expect(JSON.stringify(renderer.toJSON())).not.toContain("<br>");
    // A wider orientation updates all rows together.
    act(() => container.props.onLayout({ nativeEvent: { layout: { width: 1602 } } }));
    const wider = scroll.findAllByType(View).filter(node => typeof StyleSheet.flatten(node.props.style)?.width === "number")
      .map(node => StyleSheet.flatten(node.props.style).width);
    expect(wider.slice(4, 8)).toEqual(wider.slice(0, 4));
    expect(wider.slice(8, 12)).toEqual(wider.slice(0, 4));
    expect(wider[0]).toBeGreaterThan(widths[0]);
    expect(wider[3]).toBe(widths[3]);
  });

  test("code wraps to the block width, stays selectable and uses an iOS-safe monospace font", () => {
    const code = `const veryLongLine = '${"x".repeat(200)}';\n  indented();`;
    const root = render(`\`\`\`typescript\n${code}\n\`\`\``);
    // No scroll container of either axis: the surrounding message list scrolls.
    expect(root.findAllByType(ScrollView)).toHaveLength(0);
    const text = root.findAllByType(Text).find(node => node.props.selectable)!;
    const source = text.findAllByType(Text).filter(node => typeof node.props.children === "string")
      .map(node => node.props.children).join("");
    expect(source).toContain("const");
    expect(text.findAllByType(Text).some(node => StyleSheet.flatten(node.props.style)?.color === codeColors.keyword)).toBe(true);
    expect(text.props.selectable).toBe(true);
    // Wrapping is the native Text behaviour once nothing pins a fixed width.
    expect(text.props.numberOfLines).toBeUndefined();
    expect(StyleSheet.flatten(text.props.style).width).toBeUndefined();
    for (let parent = text.parent; parent; parent = parent.parent) expect(parent.type).not.toBe(ScrollView);
    expect(StyleSheet.flatten(text.props.style).fontFamily).toBe(Platform.OS === "ios" ? "Menlo" : "monospace");
    expect(root.findAllByType(Text).some(node => node.props.children === "typescript")).toBe(true);
  });

  test("TSX fragments highlight strings and update correctly as a code block grows", () => {
    const root = render('```tsx\nunderlineColorAndroid="trans');
    act(() => renderer.update(createElement(MarkdownText, { text: '```tsx\nunderlineColorAndroid="transparent"\n```' })));
    expect(root.findAllByType(Text).some(node => node.props.children === '"transparent"'
      && StyleSheet.flatten(node.props.style)?.color === codeColors.string)).toBe(true);
  });

  test("copying a large highlighted block yields the unmodified source", async () => {
    const code = '// first\n' + 'const value = "text";\n'.repeat(40);
    const copy = jest.spyOn(Clipboard, "setStringAsync").mockResolvedValue(true);
    try {
      const root = render(`\`\`\`ts\n${code}\n\`\`\``);
      const preview = root.findAllByType(Text).filter(node => node.props.selectable);
      expect(preview).toHaveLength(1);
      expect(preview[0]!.props.numberOfLines).toBe(16); // The painted preview is clipped; the copy is not.
      const button = root.findAll(node => node.props.accessibilityLabel === "chat.copy" && typeof node.props.onPress === "function")[0]!;
      await act(async () => { button.props.onPress(); });
      expect(copy).toHaveBeenCalledWith(code);
      const toggle = root.findAll(node => node.props.accessibilityLabel === "chat.expandCode" && typeof node.props.onPress === "function")[0]!;
      act(() => toggle.props.onPress());
      expect(root.findAllByType(Text).some(node => StyleSheet.flatten(node.props.style)?.color === codeColors.keyword)).toBe(true);
    } finally { copy.mockRestore(); }
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

  test("inline and display math render as native vectors while Mermaid retains its source fallback", () => {
    const root = render("Formula $x^2$\n\n$$\n\\frac{a}{b}\n$$\n\n```mermaid\ngraph TD; A-->B;\n```");
    // Only the math block scrolls; code wraps in place.
    expect(root.findAllByType(ScrollView)).toHaveLength(1);
    const formulas = root.findAllByType(SvgXml);
    expect(formulas).toHaveLength(2);
    expect(formulas.every(node => node.props.xml.includes("<path"))).toBe(true);
    expect(formulas.map(node => node.props.accessibilityLabel)).toEqual(["x^2", "\\frac{a}{b}"]);
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

  test("a file preview reflows the source's soft-wrapped lines while a message keeps them", () => {
    const text = "投影最小（1 706 tok）、回本最快\n基础指令，压缩只要 0.42 元。";
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(createElement(MarkdownText, { mode: "file-preview", text }));
    });
    // A document's own wrap column is a soft break: CommonMark renders it as a space.
    expect(renderer?.root.findByType(FlatList).props.data).toEqual([{
      type: "paragraph",
      children: [{ type: "text", text: "投影最小（1 706 tok）、回本最快 基础指令，压缩只要 0.42 元。" }],
    }]);
    // A chat bubble keeps the newline its author typed.
    act(() => renderer?.update(createElement(MarkdownText, { text })));
    expect(JSON.stringify(renderer?.toJSON())).toContain("回本最快\\n基础指令");
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
