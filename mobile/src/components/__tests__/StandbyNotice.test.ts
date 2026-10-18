import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import { StandbyNotice } from "../StandbyNotice";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      const words: Record<string, string> = {
        "chat.standbyCannotSend": "Taken over by {{name}} — messages will not send.",
        "chat.standbyCannotSendUnknown": "Taken over by another device — messages will not send.",
        "connection.takeControl": "Take over",
      };
      return (words[key] ?? key).replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(values?.[name] ?? ""));
    },
  }),
}));
jest.mock("../../theme/ThemeContext", () => ({
  useTheme: () => ({ colors: { warning: "#000", warningSoft: "#fff", ink: "#000" } }),
  createThemedStyles: (factory: (theme: unknown) => unknown) =>
    () => factory({ colors: { warning: "#000", warningSoft: "#fff", ink: "#000" }, font: (n: number) => n }),
}));

const painted = (tree: ReactTestRenderer) =>
  tree.root.findAllByType(Text).map(node => node.props.children).join("");

describe("the standby notice in the conversation", () => {
  it("says why nothing will send, and names the device that has it", () => {
    let tree!: ReactTestRenderer;
    act(() => {
      tree = create(createElement(StandbyNotice, {
        holder: { name: "Lab MacBook Air" },
        onTakeControl: () => undefined,
      }));
    });
    expect(painted(tree)).toContain("Taken over by Lab MacBook Air — messages will not send.");
  });

  it("does not invent a name when the holder did not give one", () => {
    let tree!: ReactTestRenderer;
    act(() => {
      tree = create(createElement(StandbyNotice, { holder: null, onTakeControl: () => undefined }));
    });
    expect(painted(tree)).toContain("Taken over by another device — messages will not send.");
  });

  it("offers the way out beside the reason, and only acts when pressed", async () => {
    const take = jest.fn().mockResolvedValue(undefined);
    let tree!: ReactTestRenderer;
    act(() => {
      tree = create(createElement(StandbyNotice, { holder: { name: "A" }, onTakeControl: take }));
    });
    expect(take).not.toHaveBeenCalled();

    const button = tree.root.findAll(node =>
      node.props.accessibilityRole === "button"
      && node.props.accessibilityLabel === "Take over")[0]!;
    expect(button).toBeDefined();
    await act(async () => {
      button.props.onPress();
    });
    expect(take).toHaveBeenCalledTimes(1);
  });
});
