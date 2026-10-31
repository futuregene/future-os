import type { ReactTestInstance, ReactTestRenderer } from "react-test-renderer";
import { createElement } from "react";
import { AccessibilityInfo, Animated, FlatList, Image, Linking, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { lightCodeColors, lightColors } from "../../theme/tokens";
import { AppAlert as Alert } from "../appAlerts";
import { act, create } from "react-test-renderer";
import { MarkdownText } from "../MarkdownText";
import { SvgXml } from "react-native-svg";
import * as parser from "../../../../packages/markdown/src/parseFutureMarkdown";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, options?: { count?: number }) =>
    options?.count === undefined ? key : `${key}:${options.count}` }),
}));

/** Everything a code row paints, in order: colored token spans interleave with
 * the raw strings of the unchanged tokens. */
function paintedText(node: ReactTestInstance | string): string {
  return typeof node === "string"
    ? node
    : node.children.map(child => paintedText(child)).join("");
}

/** A virtual row's source, without the continuation marker the row prepends. */
function rowSource(node: ReactTestInstance): string {
  const text = paintedText(node);
  return text.startsWith("↪ ") ? text.slice(2) : text;
}

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

  test("a ten-row table paints its full body instead of a clipped eight-row viewport", () => {
    const text = "| # | project | ★ |\n|---:|---|---:|\n" +
      Array.from({ length: 10 }, (_, i) => `| ${i + 1} | repo-${i + 1} | ${100 - i} |\n`).join("");
    const root = render(text);
    expect(root.findAllByType(FlatList)).toHaveLength(0);
    // Under the cutoff a table is complete: nothing waits behind a control.
    expect(root.findAll(node => node.props.accessibilityRole === "button"
      && String(node.props.accessibilityLabel).startsWith("chat.tableRows"))).toHaveLength(0);
    const output = JSON.stringify(renderer.toJSON());
    expect(output).toContain("repo-9");
    expect(output).toContain("repo-10");
  });

  test("a table past the inline limit holds its tail back until the reader asks for it", () => {
    const text = "| A | B |\n|---|---|\n" + Array.from({ length: 25 }, (_, i) => `| ${i} | value |\n`).join("");
    const root = render(text);
    // No inner viewport: the list around the table takes the vertical pan, so a
    // nested one would hide rows the reader cannot reach.
    expect(root.findAllByType(FlatList)).toHaveLength(0);
    const painted = () => root.findAllByType(Text).map(node => paintedText(node));
    expect(painted()).not.toContain("24");
    const open = root.findAll(node => node.props.accessibilityLabel === "chat.tableRowsExpand:25" && typeof node.props.onPress === "function")[0]!;
    act(() => open.props.onPress());
    expect(painted()).toContain("24");
    const close = root.findAll(node => node.props.accessibilityLabel === "chat.tableRowsCollapse:25" && typeof node.props.onPress === "function")[0]!;
    act(() => close.props.onPress());
    expect(painted()).not.toContain("24");
  });

  test("a 5000-row table paints a bounded head and keeps its columns aligned", () => {
    const text = "| A | B |\n|---|---|\n" + Array.from({ length: 5000 }, (_, i) => `| ${i} | value |\n`).join("");
    const root = render(text);
    expect(root.findAllByType(Text).length).toBeLessThan(100);
    const headerCells = root.findByType(ScrollView).findAllByType(View)
      .filter(node => StyleSheet.flatten(node.props.style)?.paddingHorizontal === 8).slice(0, 2);
    expect(headerCells).toHaveLength(2);
    // The body rows are the table's own children, so they carry the header's
    // widths rather than a viewport's.
    const widths = root.findByType(ScrollView).findAllByType(View)
      .filter(node => typeof StyleSheet.flatten(node.props.style)?.width === "number")
      .map(node => StyleSheet.flatten(node.props.style).width);
    expect(widths.slice(2, 4)).toEqual(widths.slice(0, 2));
  });

  test("large code previews a bounded head and expands to the whole source", () => {
    const code = "line with real indentation\n".repeat(5000) + "x".repeat(100000);
    const root = render(`\`\`\`ts\n${code}\n\`\`\``);
    const rows = () => root.findAllByType(Text).filter(node => node.props.selectable);
    expect(rows()).toHaveLength(1);
    const head = rowSource(rows()[0]!);
    expect(rows()[0]!.props).toMatchObject({ numberOfLines: 16, ellipsizeMode: "tail" });
    expect(head.length).toBeLessThanOrEqual(2048);
    expect(code.startsWith(head)).toBe(true);
    const toggle = root.findAll(node => node.props.accessibilityLabel === "chat.expandCode" && typeof node.props.onPress === "function")[0]!;
    act(() => toggle.props.onPress());
    const expanded = rows();
    expect(expanded.length).toBeGreaterThan(40);
    expect(expanded.every(node => node.props.numberOfLines === undefined && node.props.ellipsizeMode === undefined)).toBe(true);
    // Every chunk is painted: the tail is reachable by scrolling the message, not an inner viewport.
    const painted = expanded.map(rowSource).join("");
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

  test("a bold URL keeps the punctuation and the code span after it out of the link", () => {
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    try {
      const root = render("**https://github.com/huichen/futureos-wechat-articles**（PRIVATE，`huichen` 账号下）");
      const links = root.findAll(node =>
        typeof node.props.onPress === "function" && StyleSheet.flatten(node.props.style)?.textDecorationLine === "underline");
      expect(links.length).toBeGreaterThan(0);
      // The composite and host node of one link: both paint exactly the URL.
      expect(links.every(node => paintedText(node) === "https://github.com/huichen/futureos-wechat-articles")).toBe(true);
      act(() => links[0]!.props.onPress());
      // Tapping opened `…articles**（PRIVATE，`huichen`` before: the target ran
      // on to the end of the sentence.
      expect(open).toHaveBeenCalledWith("https://github.com/huichen/futureos-wechat-articles");
      let bold = false;
      for (let parent = links[0]!.parent; parent; parent = parent.parent) {
        if (StyleSheet.flatten(parent.props?.style)?.fontWeight === "700") bold = true;
      }
      expect(bold).toBe(true);
      // Only the URL is a link and only the URL is blue: the sentence's own
      // punctuation, code span and text are outside both.
      const output = JSON.stringify(renderer.toJSON());
      expect(output).not.toContain("**");
      expect(output).toContain("（PRIVATE，");
      expect(output).toContain("账号下）");
    } finally { open.mockRestore(); }
  });

  test("a local image inside a list item renders as an openable file chip", () => {
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    try {
      const root = render("- ![chart](docs/chart.png)");
      // A path the phone cannot display inline becomes a chip that names the
      // file and opens it, rather than an alt-text word that does nothing.
      const chip = root.findAllByType(Text).find(
        node => paintedText(node) === "chart" && typeof node.props.onPress === "function",
      );
      expect(chip).toBeDefined();
      act(() => chip!.props.onPress());
      expect(open).not.toHaveBeenCalled();
    } finally { open.mockRestore(); }
  });

  test("an in-document anchor wrapped around an image never reaches the OS", () => {
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
    try {
      const root = render("[![chart](docs/chart.png)](#section)");
      const pressables = root.findAll(node => typeof node.props.onPress === "function");
      expect(pressables.length).toBeGreaterThan(0);
      // The image's own chip and its link wrapper both resolve to an anchor in
      // this document; handing it to Linking would open a browser on a fragment.
      act(() => { pressables.forEach(node => node.props.onPress()); });
      expect(open).not.toHaveBeenCalled();
    } finally { open.mockRestore(); }
  });

  test("headings have distinct scales and accessible heading roles", () => {
    const root = render("# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six");
    const headings = root.findAllByType(Text).filter(node => node.props.accessibilityRole === "header");
    expect(headings).toHaveLength(6);
    const sizes = headings.map(node => StyleSheet.flatten(node.props.style).fontSize);
    expect(new Set(sizes).size).toBe(6);
    expect(sizes).toEqual([...sizes].sort((a, b) => b - a));
  });

  test("a centered HTML block aligns its text and caps/centers a widthed image", () => {
    const root = render('<p align="center">hello</p>\n\n<div align="center">\n<img src="https://example.com/a.png" width="120">\n</div>');
    // The centered paragraph's text carries textAlign.
    const hello = root.findAllByType(Text).find(
      node => Array.isArray(node.props.children) && node.props.children[0] === "hello",
    );
    expect(StyleSheet.flatten(hello?.props.style)).toMatchObject({ textAlign: "center" });
    // The widthed image is capped at 120 and centered within its block.
    const image = root.findByType(Image);
    const style = StyleSheet.flatten(image.props.style);
    expect(style).toMatchObject({ width: "100%", maxWidth: 120, alignSelf: "center" });
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

  test("a list item's body keeps a content-based flex basis for a shrink-wrapped bubble", () => {
    // The user's own prompt bubble sizes itself to what it draws, and Yoga — the
    // engine the phone runs — measures a flex item from its *basis*, not from
    // its content. `flex: 1` is a zero basis, so a prompt that parsed as a list
    // measured as wide as its marker: the bubble painted its accent fill at the
    // width of "1." with the item text laid out at no width. A regression here
    // is invisible in the browser harness, because CSS sizes a flex container
    // from its items' max-content whatever their basis is.
    const root = render("1. PC 和手机上安装：只要包含安装内容即可。\n2. 科研技能使用：给出具体的例子。");
    const bodies = root.findAllByType(View).filter(node =>
      StyleSheet.flatten(node.props.style)?.minWidth === 0
      && StyleSheet.flatten(node.props.style)?.flexShrink === 1);
    expect(bodies).toHaveLength(2);
    for (const body of bodies) {
      const style = StyleSheet.flatten(body.props.style);
      expect(style.flexGrow).toBe(1);
      expect(style.flex).toBeUndefined();
      expect(style.flexBasis).toBeUndefined();
    }
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

  test("a code block never wraps: a long line keeps its line and pans sideways", () => {
    // Box-drawing art is what forced this: folded, the line after the fold loses
    // the frame it was drawn in, and every column to its right shifts.
    const code = "const frame = [\n" + `╭─ New Chat ─╮\n│ ${"x".repeat(200)} │\n╰──────────╯\n` + "  indented();";
    const root = render(`\`\`\`typescript\n${code}\n\`\`\``);
    // The block pans sideways in place; the surrounding message list keeps the
    // vertical pan, so no inner vertical viewport is created.
    const scroll = root.findByType(ScrollView);
    expect(scroll.props).toMatchObject({ horizontal: true, nestedScrollEnabled: true });
    const text = scroll.findAllByType(Text).find(node => node.props.selectable)!;
    // Nothing pins a width or asks the platform to fold a line.
    expect(text.props.numberOfLines).toBeUndefined();
    expect(text.props.ellipsizeMode).toBeUndefined();
    expect(StyleSheet.flatten(text.props.style).width).toBeUndefined();
    expect(paintedText(text)).toContain("╭─ New Chat ─╮");
    expect(text.findAllByType(Text).some(node => StyleSheet.flatten(node.props.style)?.color === lightCodeColors.keyword)).toBe(true);
    expect(text.props.selectable).toBe(true);
    expect(StyleSheet.flatten(text.props.style).fontFamily).toBe(Platform.OS === "ios" ? "Menlo" : "monospace");
    // The gutter is on the scrolling content and the rows stack (a horizontal
    // ScrollView's content is a row by default).
    expect(StyleSheet.flatten(scroll.props.contentContainerStyle)).toMatchObject({ flexDirection: "column", padding: 12 });
    expect(root.findAllByType(Text).some(node => node.props.children === "typescript")).toBe(true);
  });

  test("TSX fragments highlight strings and update correctly as a code block grows", () => {
    const root = render('```tsx\nunderlineColorAndroid="trans');
    act(() => renderer.update(createElement(MarkdownText, { text: '```tsx\nunderlineColorAndroid="transparent"\n```' })));
    expect(root.findAllByType(Text).some(node => node.props.children === '"transparent"'
      && StyleSheet.flatten(node.props.style)?.color === lightCodeColors.string)).toBe(true);
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
      expect(root.findAllByType(Text).some(node => StyleSheet.flatten(node.props.style)?.color === lightCodeColors.keyword)).toBe(true);
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
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
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
    // The display formula pans, and so does the code block beside it: neither
    // wraps, so each is its own sideways scroller.
    expect(root.findAllByType(ScrollView)).toHaveLength(2);
    const formulas = root.findAllByType(SvgXml);
    expect(formulas).toHaveLength(2);
    expect(formulas.every(node => node.props.xml.includes("<path"))).toBe(true);
    expect(formulas.map(node => node.props.accessibilityLabel)).toEqual(["x^2", "\\frac{a}{b}"]);
    const output = JSON.stringify(renderer.toJSON());
    expect(output).toContain("x^2");
    expect(output).toContain("frac{a}{b}");
    expect(output).toContain("graph TD; A-->B;");
  });

  test("a formula the TeX layout cannot render stays readable as its source", () => {
    // `\frac{a}` is an arity error: MathJax answers with an error node, so the
    // vector is refused and the reader keeps the TeX rather than a blank gap.
    const root = render("Broken $\\frac{a}$ here");
    expect(root.findAllByType(SvgXml)).toHaveLength(0);
    const source = root.findAllByType(Text).map(node => paintedText(node)).filter(Boolean);
    expect(source.join(" ")).toContain("\\frac{a}");
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

  test("an image whose source is a path on the desktop is a chip that opens it", () => {
    const onOpenFile = jest.fn();
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          text: "![shot](file:///Users/me/Downloads/shot.png)",
          onOpenFile,
        }),
      );
    });
    // The row names the file rather than drawing a broken image: a `file://`
    // source is a path on the machine that sent the message, so the phone offers
    // it to the desktop instead of trying to load it.
    const chip = renderer?.root.findAllByType(Text).find(node => node.props.onPress);
    expect(chip).toBeDefined();
    act(() => chip!.props.onPress());
    expect(onOpenFile).toHaveBeenCalledWith("/Users/me/Downloads/shot.png");
  });

  test("a local link inside a Markdown preview opens beside it, not on a desktop-only notice", () => {
    const onOpenFile = jest.fn();
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          imageBasePath: "/root/articles/silver/cover-notes.md",
          mode: "file-preview",
          onOpenFile,
          text: "[SOURCES](../../SOURCES.md) and [sub](images/diagram.png)",
        }),
      );
    });
    const links = renderer?.root.findAllByType(Text).filter(node => node.props.onPress) ?? [];
    act(() => links[0]?.props.onPress());
    act(() => links[1]?.props.onPress());
    // Relative to the document that carries the link, not to the workspace.
    expect(onOpenFile).toHaveBeenNthCalledWith(1, "/root/articles/silver/../../SOURCES.md");
    expect(onOpenFile).toHaveBeenNthCalledWith(2, "/root/articles/silver/images/diagram.png");
    expect(alert).not.toHaveBeenCalled();
    alert.mockRestore();
  });

  test("cannot follow a link in a document opened from this phone's own storage", () => {
    const onOpenFile = jest.fn();
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(
        createElement(MarkdownText, {
          imageBasePath: "file:///phone/cache/report.md",
          mode: "file-preview",
          onOpenFile,
          text: "[diagram](assets/pic.png)",
        }),
      );
    });
    const chip = renderer?.root.findAllByType(Text).find(node => node.props.onPress);
    act(() => chip?.props.onPress());
    expect(onOpenFile).not.toHaveBeenCalled();
    expect(alert).toHaveBeenCalledWith("attachment.title", "attachment.localLinkUnresolvable");
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
    const open = jest.spyOn(Linking, "openURL").mockResolvedValue(undefined);
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

/** A tree of its own for the cases below: they assert error surfaces and block
 * kinds the layout suite does not need, and each mounts and unmounts its own. */
describe("MarkdownText error surfaces and block kinds", () => {
  const mounted: ReactTestRenderer[] = [];
  const render = (text: string) => {
    let tree!: ReactTestRenderer;
    act(() => { tree = create(createElement(MarkdownText, { text })); });
    mounted.push(tree);
    return tree.root;
  };
  afterEach(() => {
    for (const tree of mounted.splice(0)) act(() => tree.unmount());
  });

  test("a copy whose clipboard write fails says so instead of claiming success", async () => {
    const copy = jest.spyOn(Clipboard, "setStringAsync").mockRejectedValue(new Error("clipboard busy"));
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    try {
      const root = render("```ts\n" + 'const value = "text";\n'.repeat(40) + "```");
      const button = root.findAll(node =>
        node.props.accessibilityLabel === "chat.copy" && typeof node.props.onPress === "function")[0]!;
      await act(async () => { button.props.onPress(); });
      expect(copy).toHaveBeenCalled();
      expect(alert).toHaveBeenCalledWith("common.error", "clipboard busy");
    } finally {
      copy.mockRestore();
      alert.mockRestore();
    }
  });

  test("a link the phone cannot open reports itself rather than failing silently", async () => {
    const open = jest.spyOn(Linking, "openURL").mockRejectedValue(new Error("no handler"));
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    try {
      const root = render("[site](https://example.com)");
      const link = root.findAllByType(Text).find(node => node.props.onPress)!;
      await act(async () => { link.props.onPress(); });
      expect(open).toHaveBeenCalledWith("https://example.com");
      expect(alert).toHaveBeenCalledWith("attachment.title", "attachment.linkOpenFailed");
    } finally {
      open.mockRestore();
      alert.mockRestore();
    }
  });

  test("blockquotes, rules, struck text and file embeds all paint as themselves", async () => {
    // Each of these is a distinct block/inline kind. A fall-through would still
    // show the words, so the assertion is that the *structure* is gone: the
    // quote keeps its bar, the rule paints no literal dashes, the embed becomes
    // a pressable chip with its title, and `~~` never reaches the screen.
    const root = render([
      "> quoted **line** and *slanted*…",
      "",
      "---",
      "",
      "```futureos-file",
      "id: docs/report.md",
      "title: The report",
      "```",
      "",
      "~~struck~~ and ![shot](assets/pic.png)",
      "",
      "a **bold ![inline-chip](docs/chart.png) tail**",
    ].join("\n"));
    const screen = paintedText(root.findByType(View));
    expect(screen).toContain("quoted line");
    expect(screen).toContain("slanted");
    expect(screen).not.toContain("*slanted*");
    expect(screen).toContain("struck");
    expect(screen).not.toContain("~~");
    // The rule is a drawn line, not three characters of prose.
    expect(screen).not.toContain("---");
    // The embed keeps the title the directive carries, not the file path.
    expect(screen).toContain("The report");
    expect(screen).not.toContain("futureos-file");
    // Pressing the embed chip opens the file the directive names.
    const embedChip = root.findAllByType(Text).find(node =>
      node.props.onPress && paintedText(node) === "The report");
    expect(embedChip).toBeDefined();
    act(() => embedChip!.props.onPress());
    // ...and the local image nested in bold text is a chip for its own path,
    // not a bare file name the reader cannot open.
    const inlineChip = root.findAllByType(Text).find(node =>
      node.props.onPress && paintedText(node) === "inline-chip");
    expect(inlineChip).toBeDefined();
    act(() => inlineChip!.props.onPress());
  });

  test("a file embed opens its target, and a quote keeps its own bar", () => {
    const root = render([
      ">> nested **quote**",
      "",
      "```futureos-file",
      "id: docs/report.md",
      "```",
    ].join("\n"));
    // Nested quoting exercises the recursive block renderer too.
    expect(paintedText(root.findByType(View))).toContain("nested quote");
    // With no title, the chip falls back to the file's base name.
    const chip = root.findAllByType(Text).find(node =>
      node.props.onPress && paintedText(node) === "report.md");
    expect(chip).toBeDefined();
  });
});

/**
 * The user's own bubble is filled with the accent colour, so a body drawn on it
 * cannot use the sheet's ordinary palette (see `MarkdownTone`).
 */
describe("a markdown body on the accent-filled bubble", () => {
  const mounted: ReactTestRenderer[] = [];
  const render = (props: Parameters<typeof MarkdownText>[0]) => {
    let tree!: ReactTestRenderer;
    act(() => { tree = create(createElement(MarkdownText, props)); });
    mounted.push(tree);
    return tree.root;
  };
  afterEach(() => {
    for (const tree of mounted.splice(0)) act(() => tree.unmount());
  });

  /** Every colour the tree paints text with. */
  function textColors(root: ReactTestInstance) {
    return root.findAllByType(Text).map(node => StyleSheet.flatten(node.props.style)?.color);
  }

  test("the on-accent tone paints the body in the bubble's own contrast colour", () => {
    const root = render({ text: "plain **bold** and `code`", tone: "onAccent" });
    // `surface` is the colour the bubble's text already uses — white on the
    // light scheme's accent fill, the dark navy on the dark scheme's.
    expect(textColors(root)).toContain(lightColors.surface);
    expect(textColors(root)).not.toContain(lightColors.ink);
  });

  test("a recess on the accent fill is a wash of that same contrast colour", () => {
    const root = render({ text: "`inline`", tone: "onAccent" });
    const code = root.findAllByType(Text)
      .filter(node => paintedText(node) === "inline").at(-1)!;
    expect(StyleSheet.flatten(code.props.style)).toMatchObject({
      // Not `surfaceSubtle`, which is a recess on `surface` and vanishes here.
      backgroundColor: `${lightColors.surface}33`,
      color: lightColors.surface,
    });
  });

  test("a code block on the accent fill is painted in one colour, not a syntax palette", () => {
    const root = render({ text: "```typescript\nconst value = \"text\";\n```", tone: "onAccent" });
    const colors = textColors(root);
    // The grammar's colours are drawn for a light/dark *surface* and would drop
    // contrast haphazardly on the accent fill.
    expect(colors).not.toContain(lightCodeColors.keyword);
    expect(colors).not.toContain(lightCodeColors.string);
    expect(paintedText(root)).toContain('const value = "text";');
  });

  test("the default tone is untouched: ink on the app's own surface", () => {
    // boundary: the other arm of the tone — a reply's body, which is what every
    // other case in this suite renders.
    const root = render({ text: "plain text" });
    expect(textColors(root)).toContain(lightColors.ink);
    expect(textColors(root)).not.toContain(lightColors.surface);
  });
});

/** The `#` form the composer writes for another conversation (see
 * `@future-os/markdown`) reaches the renderer as an ordinary link. */
describe("a `#` conversation reference", () => {
  const mounted: ReactTestRenderer[] = [];
  const render = (props: Parameters<typeof MarkdownText>[0]) => {
    let tree!: ReactTestRenderer;
    act(() => { tree = create(createElement(MarkdownText, props)); });
    mounted.push(tree);
    return tree.root;
  };
  afterEach(() => {
    for (const tree of mounted.splice(0)) act(() => tree.unmount());
  });

  /** The innermost row painting the chip's own text. */
  function chip(root: ReactTestInstance, label: string) {
    return root.findAllByType(Text).filter(node => paintedText(node) === label).at(-1)!;
  }

  test("keeps the composer's `#` marker and opens the conversation it names", () => {
    const onOpenSession = jest.fn();
    const root = render({
      onOpenSession,
      text: "ask [Fix the flaky test](futureos://session/s-1) now",
    });

    const reference = chip(root, "#Fix the flaky test");
    expect(reference.props.accessibilityRole).toBe("link");
    act(() => reference.props.onPress());
    expect(onOpenSession).toHaveBeenCalledWith("s-1");
  });

  test("is a chip rather than a dead tap when the screen cannot switch conversation", () => {
    const root = render({ text: "[Fix CI](futureos://session/s-1)" });
    const reference = chip(root, "#Fix CI");
    expect(reference.props.onPress).toBeUndefined();
  });

  test("a link that only looks like one stays an ordinary blocked link", () => {
    const root = render({ onOpenSession: jest.fn(), text: "[x](futureos://run/1)" });
    expect(paintedText(root)).toContain("x");
    expect(root.findAllByType(Text).filter(node => paintedText(node) === "#x")).toHaveLength(0);
  });
});

describe("an image inside a sentence", () => {
  test("is a chip for the file it names, opened on the machine that holds it", () => {
    // A standalone picture becomes its own block (`MarkdownImage`); one written
    // in the middle of a run has to stay inline, and a `file://` source there is
    // still a path on the sending machine — a chip that names the file and asks
    // the desktop for it, never a broken image.
    const onOpenFile = jest.fn();
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = create(createElement(MarkdownText, {
        onOpenFile,
        text: "see ![shot](file:///Users/me/Downloads/shot.png) for the numbers",
      }));
    });
    const root = renderer!.root;
    expect(paintedText(root)).toContain("see");
    expect(paintedText(root)).toContain("for the numbers");
    const chip = root.findAllByType(Text).find(node => typeof node.props.onPress === "function");
    expect(chip).toBeDefined();
    // The chip carries the file's name only — the alt text here.
    expect(paintedText(chip!)).toBe("shot");
    act(() => chip!.props.onPress());
    expect(onOpenFile).toHaveBeenCalledWith("/Users/me/Downloads/shot.png");
    act(() => renderer!.unmount());
  });
});
