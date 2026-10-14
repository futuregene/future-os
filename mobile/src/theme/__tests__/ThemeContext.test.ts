import { createElement } from "react";
import { StyleSheet, Text, View } from "react-native";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { DEFAULT_APPEARANCE, setAppearance } from "../preferences";
import { createThemedStyles, ThemeProvider, useAppearance, useAppearanceReady, useTheme } from "../ThemeContext";
import { buildTheme, lightTheme } from "../theme";

jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: jest.fn(async () => null),
  setItem: jest.fn(async () => {}),
}));

const useProbeStyles = createThemedStyles(theme => StyleSheet.create({
  box: { backgroundColor: theme.colors.surface, fontSize: theme.font(14) },
}));

beforeEach(async () => {
  await setAppearance(DEFAULT_APPEARANCE);
});

describe("the styles a theme builds", () => {
  it("builds once per theme and hands out that same sheet", () => {
    const first = useProbeStyles.forTheme(lightTheme);
    const second = useProbeStyles.forTheme(lightTheme);
    expect(first).toBe(second);
    // A different theme is a different sheet, or switching the scheme in the
    // settings screen would leave every mounted component on the old colours.
    const dark = useProbeStyles.forTheme(buildTheme("dark", "medium"));
    expect(dark).not.toBe(first);
    expect(StyleSheet.flatten(dark.box)).toMatchObject({ backgroundColor: buildTheme("dark", "medium").colors.surface });
  });

  it("is the light theme outside a provider", () => {
    let seen!: ReturnType<typeof useTheme>;
    let styles!: ReturnType<typeof useProbeStyles>;
    function Probe() {
      seen = useTheme();
      styles = useProbeStyles();
      return null;
    }
    act(() => { create(createElement(Probe)); });
    expect(seen).toBe(lightTheme);
    expect(styles).toBe(useProbeStyles.forTheme(lightTheme));
  });
});

describe("the provider", () => {
  let tree: ReactTestRenderer;

  function Harness() {
    const ready = useAppearanceReady();
    const theme = useTheme();
    const appearance = useAppearance();
    const styles = useProbeStyles();
    return createElement(View, null,
      createElement(Text, { testID: "scheme" }, theme.scheme),
      createElement(Text, { testID: "body" }, String(theme.chat.fontSize)),
      createElement(Text, { testID: "ready" }, String(ready)),
      createElement(Text, { testID: "stored" }, `${appearance.themeMode}/${appearance.fontSize}`),
      createElement(View, { testID: "box", style: styles.box }),
    );
  }
  const text = (id: string) => tree.root.findAllByType(Text).find((node: ReactTestInstance) => node.props.testID === id)!.props.children;
  const box = (): Record<string, unknown> => StyleSheet.flatten(
    tree.root.findAllByType(View).find((node: ReactTestInstance) => node.props.testID === "box")!.props.style,
  ) as Record<string, unknown>;

  beforeEach(async () => {
    await act(async () => { tree = create(createElement(ThemeProvider, null, createElement(Harness))); });
  });
  afterEach(() => { act(() => tree.unmount()); });

  it("starts on the stored appearance and reports itself ready", async () => {
    await act(async () => { await Promise.resolve(); });
    expect(text("ready")).toBe("true");
    expect(text("scheme")).toBe("light");
    expect(text("body")).toBe("15");
  });

  it("repaints a mounted screen's styles when the appearance changes", async () => {
    expect(box()).toMatchObject({ backgroundColor: lightTheme.colors.surface, fontSize: 14 });
    await act(async () => { await setAppearance({ themeMode: "dark", fontSize: "xlarge" }); });
    expect(text("scheme")).toBe("dark");
    expect(text("stored")).toBe("dark/xlarge");
    expect(text("body")).toBe("20");
    // The sheet a mounted component holds is rebuilt, not just the tokens
    // beside it: a colour frozen at mount would keep a dark screen light.
    expect(box()).toMatchObject({ backgroundColor: buildTheme("dark", "xlarge").colors.surface, fontSize: 19 });
    await act(async () => { await setAppearance({ themeMode: "light", fontSize: "small" }); });
    expect(text("scheme")).toBe("light");
    expect(text("body")).toBe("13");
    expect(box()).toMatchObject({ backgroundColor: lightTheme.colors.surface, fontSize: 12 });
  });
});
