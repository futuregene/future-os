import { createElement } from "react";
import { act, create } from "react-test-renderer";
import { TakeControlBanner } from "../TakeControlBanner";

jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => {
      const words: Record<string, string> = {
        "connection.controlHeldBy": "{{name}} is controlling this desktop",
        "connection.controlHeldByUnknown": "Another device is controlling this desktop",
        "connection.takeControl": "Take over",
        "connection.takeControlHint": "Taking over disconnects the device that has it now.",
        "connection.takeControlFailed": "Could not take over. Try again.",
      };
      return (words[key] ?? key).replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(values?.[name] ?? ""));
    },
  }),
}));

jest.mock("../../theme/ThemeContext", () => ({
  useTheme: () => ({ colors: { warning: "#000", warningSoft: "#fff", ink: "#000", inkMuted: "#666" } }),
  createThemedStyles: (factory: (theme: unknown) => unknown) =>
    () => factory({ colors: { warning: "#000", warningSoft: "#fff", ink: "#000", inkMuted: "#666" }, font: (n: number) => n }),
}));

function painted(node: ReturnType<typeof create>): string {
  const texts: string[] = [];
  const walk = (value: unknown): void => {
    if (value == null) return;
    if (typeof value === "string") {
      texts.push(value);
      return;
    }
    if (Array.isArray(value)) {
      value.forEach(walk);
      return;
    }
    if (typeof value === "object" && "children" in (value as Record<string, unknown>))
      walk((value as { children: unknown }).children);
  };
  walk(node.toJSON());
  return texts.join("");
}

describe("TakeControlBanner", () => {
  it("names the device that holds the desktop, so the user knows who they would disconnect", () => {
    let tree: ReturnType<typeof create>;
    act(() => {
      tree = create(createElement(TakeControlBanner, {
        holder: { name: "Lab MacBook Air" },
        onTakeControl: () => undefined,
      }));
    });
    const text = painted(tree!);
    expect(text).toContain("Lab MacBook Air is controlling this desktop");
    expect(text).toContain("Taking over disconnects the device that has it now.");
  });

  it("does not invent a name when the holder never gave one", () => {
    let tree: ReturnType<typeof create>;
    act(() => {
      tree = create(createElement(TakeControlBanner, { holder: null, onTakeControl: () => undefined }));
    });
    expect(painted(tree!)).toContain("Another device is controlling this desktop");
  });

  it("takes over only when pressed, and reports a refusal instead of pretending", async () => {
    const take = jest.fn().mockResolvedValueOnce(undefined);
    let tree: ReturnType<typeof create>;
    act(() => {
      tree = create(createElement(TakeControlBanner, { holder: { name: "A" }, onTakeControl: take }));
    });
    expect(take).not.toHaveBeenCalled();

    const button = tree!.root.findAll(node =>
      node.props?.accessibilityLabel === "Take over" || node.props?.label === "Take over");
    await act(async () => {
      button[0]!.props.onPress();
    });
    expect(take).toHaveBeenCalledTimes(1);

    // A refusal is shown, not swallowed: the banner is still there and says so.
    const failing = jest.fn().mockRejectedValueOnce(new Error("takeover_failed"));
    let second: ReturnType<typeof create>;
    act(() => {
      second = create(createElement(TakeControlBanner, { holder: { name: "A" }, onTakeControl: failing }));
    });
    const retry = second!.root.findAll(node =>
      node.props?.accessibilityLabel === "Take over" || node.props?.label === "Take over");
    await act(async () => {
      retry[0]!.props.onPress();
    });
    expect(painted(second!)).toContain("Could not take over. Try again.");
  });
});
