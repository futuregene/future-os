import { createElement } from "react";
import { StyleSheet, Text } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import "../../i18n";
import { AppearanceSettings } from "../AppearanceSettings";
import { DEFAULT_APPEARANCE, getAppearance, setAppearance } from "../preferences";
import { ThemeProvider } from "../ThemeContext";
import { FONT_SIZE_SCALE } from "../tokens";

const mockSetItem = jest.fn(async (_key: string, _value: string) => {});
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: jest.fn(async () => null),
  setItem: (key: string, value: string) => mockSetItem(key, value),
}));

let tree: ReactTestRenderer;
function Harness() {
  return createElement(ThemeProvider, null, createElement(AppearanceSettings));
}
const radios = () => tree.root.findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function");
const radio = (label: string) => radios().find(node => node.props.accessibilityLabel === label)!;
const alerts = () => tree.root.findAllByType(Text).filter(node => node.props.accessibilityRole === "alert");
async function select(label: string) {
  await act(async () => { radio(label).props.onPress(); });
}

beforeEach(async () => {
  mockSetItem.mockReset().mockResolvedValue(undefined);
  await setAppearance(DEFAULT_APPEARANCE);
  // The reset above is itself a write; only the reader's choices count below.
  mockSetItem.mockClear();
  await act(async () => { tree = create(createElement(Harness)); });
});
afterEach(() => { act(() => tree.unmount()); });

it("offers all three schemes, all four text sizes, and the two reply levels", () => {
  expect(radios().map(node => node.props.accessibilityLabel)).toEqual([
    "Follow system", "Light", "Dark",
    "Small", "Medium", "Large", "Extra large",
    "Default", "Detailed",
  ]);
  expect(radio("Follow system").props.accessibilityState.checked).toBe(true);
  expect(radio("Medium").props.accessibilityState.checked).toBe(true);
  // A settled reply ships folded onto its answer.
  expect(radio("Default").props.accessibilityState.checked).toBe(true);
  expect(radio("Detailed").props.accessibilityState.checked).toBe(false);
});

it("stores a scheme or a text size as soon as it is picked", async () => {
  await select("Dark");
  expect(radio("Dark").props.accessibilityState.checked).toBe(true);
  expect(radio("Follow system").props.accessibilityState.checked).toBe(false);
  expect(mockSetItem).toHaveBeenLastCalledWith("future.mobile.appearance", JSON.stringify({ themeMode: "dark", fontSize: "medium", replyDetail: "default" }));
  await select("Extra large");
  expect(getAppearance()).toEqual({ themeMode: "dark", fontSize: "xlarge", replyDetail: "default" });
  expect(radio("Extra large").props.accessibilityState.checked).toBe(true);
});

it("re-selecting the active choice writes nothing", async () => {
  await select("Follow system");
  await select("Medium");
  expect(mockSetItem).not.toHaveBeenCalled();
  expect(getAppearance()).toEqual(DEFAULT_APPEARANCE);
});

it("sets each size chip at the size it picks", () => {
  const sizeOf = (label: string) => StyleSheet.flatten(
    radio(label).findAllByType(Text)[0]!.props.style,
  ).fontSize as number;
  expect(sizeOf("Small")).toBe(Math.round(14 * FONT_SIZE_SCALE.small));
  expect(sizeOf("Medium")).toBe(14);
  expect(sizeOf("Large")).toBe(Math.round(14 * FONT_SIZE_SCALE.large));
  expect(sizeOf("Extra large")).toBe(Math.round(14 * FONT_SIZE_SCALE.xlarge));
  expect(sizeOf("Extra large")).toBeGreaterThan(sizeOf("Large"));
});

it("reports a refused write and keeps the stored choice selected", async () => {
  mockSetItem.mockRejectedValueOnce(new Error("full"));
  await select("Dark");
  expect(radio("Follow system").props.accessibilityState.checked).toBe(true);
  expect(alerts().map(node => node.props.children)).toEqual(["Could not save the appearance setting. Please try again."]);
  await select("Dark");
  expect(radio("Dark").props.accessibilityState.checked).toBe(true);
  expect(alerts()).toEqual([]);
});

it("disables every choice while a write is in flight", async () => {
  let finish!: () => void;
  mockSetItem.mockReturnValueOnce(new Promise<void>(resolve => { finish = resolve; }));
  await select("Large");
  expect(radios()).toHaveLength(9);
  expect(radios().every(node => node.props.disabled)).toBe(true);
  await act(async () => { finish(); });
  expect(radio("Large").props.disabled).toBe(false);
  expect(radios().every(node => node.props.accessibilityState.disabled)).toBe(false);
});

describe("the reply reading level", () => {
  it("offers default and detailed, on `default`, and says what the level does", () => {
    expect(radio("Default").props.accessibilityState.checked).toBe(true);
    expect(radio("Detailed").props.accessibilityState.checked).toBe(false);
    expect(tree.root.findAll(node => String(node.type) === "Text"
      && String(node.props.children).startsWith("A finished reply shows its final answer")).length).toBe(1);
  });

  it("stores the level and applies it", async () => {
    await select("Detailed");
    expect(mockSetItem).toHaveBeenLastCalledWith(
      "future.mobile.appearance",
      JSON.stringify({ themeMode: "system", fontSize: "medium", replyDetail: "detailed" }),
    );
    expect(getAppearance().replyDetail).toBe("detailed");
    expect(radio("Detailed").props.accessibilityState.checked).toBe(true);
    // The row's own words follow the level it is on.
    expect(tree.root.findAll(node => String(node.type) === "Text"
      && String(node.props.children).startsWith("A finished reply keeps everything")).length).toBe(1);

    await select("Default");
    expect(getAppearance().replyDetail).toBe("default");
  });

  it("keeps the scheme and text size when the reply level changes", async () => {
    await act(async () => { radio("Dark").props.onPress(); });
    await select("Detailed");
    expect(getAppearance()).toEqual({ themeMode: "dark", fontSize: "medium", replyDetail: "detailed" });
  });
});
