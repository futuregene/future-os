import type { ReactTestRenderer } from "react-test-renderer";
import { createElement } from "react";
import { AccessibilityInfo, Alert, Animated, Image, Linking, Text } from "react-native";
import { act, create } from "react-test-renderer";
import { MarkdownText } from "../MarkdownText";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

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
      expect(streamed.toJSON()).toEqual(settled.toJSON());
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
