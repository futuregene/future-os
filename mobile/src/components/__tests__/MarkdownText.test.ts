import type { ReactTestRenderer } from "react-test-renderer";
import React from "react";
import { Alert, Image, Linking, Text } from "react-native";
import TestRenderer, { act } from "react-test-renderer";
import { MarkdownText } from "../MarkdownText";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe("MarkdownText", () => {
  test("renders a bold local-file link without exposing markdown syntax", () => {
    let renderer: ReactTestRenderer | undefined;
    const onOpenFile = jest.fn();
    act(() => {
      renderer = TestRenderer.create(
        React.createElement(MarkdownText, {
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
    act(() => linkedText?.props.onPress());
    expect(onOpenFile).toHaveBeenCalledWith("gomoku.html");
  });

  test("uses the shared GFM parser for tables, tasks and nested formatting", () => {
    let renderer: ReactTestRenderer | undefined;
    act(() => {
      renderer = TestRenderer.create(
        React.createElement(MarkdownText, {
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
      renderer = TestRenderer.create(
        React.createElement(MarkdownText, {
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
      renderer = TestRenderer.create(
        React.createElement(MarkdownText, {
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
      renderer = TestRenderer.create(
        React.createElement(MarkdownText, {
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
      renderer = TestRenderer.create(
        React.createElement(MarkdownText, { text: "[bad](javascript:alert(1))" }),
      );
    });
    const pressable = renderer?.root.findAllByType(Text).find(node => node.props.onPress);
    expect(pressable).toBeUndefined();
    expect(open).not.toHaveBeenCalled();
    open.mockRestore();
  });
});
