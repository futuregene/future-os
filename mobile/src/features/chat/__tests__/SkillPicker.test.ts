import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, StyleSheet, Text } from "react-native";
import { SKILL_ROW_HEIGHT, SkillPicker, VISIBLE_SKILL_ROWS, skillPickerHeight } from "../components/SkillPicker";
import type { RemoteSkill } from "../../../remote/types";

jest.mock("lucide-react-native", () => ({ Info: () => null, X: () => null }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key, i18n: { language: "zh" } }) }));

const skills: RemoteSkill[] = [{ name: "future-web", description: "Search pages", nameZh: "网页搜索", descriptionZh: "读取网页" }];
const load = jest.fn(async () => skills);
const onSelect = jest.fn();
const onClose = jest.fn();
const onShowDetails = jest.fn();
const props = { query: "", supported: true, load, onSelect, onClose, onShowDetails, maxHeight: 220 };
let tree: ReactTestRenderer;
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children);
const details = () => tree.root.findAll(node => node.props.accessibilityLabel === "skills.details" && node.props.onPress)[0]!;
beforeEach(() => { jest.clearAllMocks(); load.mockReset().mockResolvedValue(skills); });
afterEach(() => { if (tree) act(() => tree.unmount()); });

test("loads once per opening, searches locally and selects without dismissing keyboard", async () => {
  await act(async () => { tree = create(createElement(SkillPicker, props)); });
  expect(load).toHaveBeenCalledTimes(1);
  expect(texts()).toEqual(expect.arrayContaining(["/future-web", "读取网页"]));
  expect(tree.root.findByType(ScrollView).props.keyboardShouldPersistTaps).toBe("always");
  await act(async () => { tree.update(createElement(SkillPicker, { ...props, query: "读取" })); });
  expect(load).toHaveBeenCalledTimes(1);
  const option = tree.root.findAll(node => node.props.accessibilityLabel === "/future-web · 网页搜索" && node.props.onPress)[0]!;
  act(() => option.props.onPress());
  expect(onSelect).toHaveBeenCalledWith("future-web");
  const close = tree.root.findAll(node => node.props.accessibilityLabel === "skills.close" && node.props.onPress)[0]!;
  act(() => close.props.onPress());
  expect(onClose).toHaveBeenCalledTimes(1);
  await act(async () => { tree.update(createElement(SkillPicker, { ...props, query: "不存在" })); });
  expect(texts()).toContain("skills.noResults");
});

test("each row is the English command over one localized description line", async () => {
  await act(async () => { tree = create(createElement(SkillPicker, props)); });
  const option = tree.root.findAll(node => node.props.accessibilityLabel === "/future-web · 网页搜索" && node.props.onPress)[0]!;
  // Stacked, never side by side: the command is wide enough to own a line.
  expect(StyleSheet.flatten(option.props.style({ pressed: false }))).toMatchObject({ flexDirection: "column", minHeight: 56, minWidth: 0 });
  expect(option.findAllByType(Text).map(node => node.props.children)).toEqual(["/future-web", "读取网页"]);
  expect(option.findAllByType(Text).every(node => node.props.numberOfLines === 1)).toBe(true);
  // English name over the localized description; an English UI swaps the line.
  expect(texts()).not.toContain("网页搜索");
  expect(texts()).not.toContain("Search pages");
  expect(details().props.accessibilityState.expanded).toBe(false);
  act(() => details().props.onPress());
  expect(onShowDetails).toHaveBeenCalledWith(skills[0]);
  expect(onSelect).not.toHaveBeenCalled();
  // The open description is reported back so the row can show it is the one.
  await act(async () => tree.update(createElement(SkillPicker, { ...props, detailsName: "future-web" })));
  expect(details().props.accessibilityState.expanded).toBe(true);
  await act(async () => tree.update(createElement(SkillPicker, { ...props, detailsName: "other-skill" })));
  expect(details().props.accessibilityState.expanded).toBe(false);
  expect(load).toHaveBeenCalledTimes(1);
});

test("the menu is tall enough for the skills it shows, and never taller than the room", () => {
  // A portrait phone with the keyboard up: three skills plus the context action
  // that leads them, measured with the separator and the panel's own border.
  const rows = (count: number) => count * (SKILL_ROW_HEIGHT + 1) + 44 + 2;
  expect(skillPickerHeight(844, 300, 1)).toBe(rows(VISIBLE_SKILL_ROWS + 1));
  expect(skillPickerHeight(844, 300)).toBe(rows(VISIBLE_SKILL_ROWS));
  // A short screen with a tall keyboard cannot give that much: the list scrolls.
  expect(skillPickerHeight(568, 260, 1)).toBe(208);
  // Never negative or absurd on a nonsense measurement.
  expect(skillPickerHeight(300, 400, 1)).toBe(120);
  // The keyboard only ever takes room away, never adds.
  expect(skillPickerHeight(844, 0, 1)).toBe(skillPickerHeight(844, 300, 1));
});

test("context actions lead the menu, filter like skills, and run instead of inserting text", async () => {
  const onCompact = jest.fn();
  const action = {
    id: "compact",
    label: "压缩上下文",
    description: "压缩此对话的上下文",
    searchText: "compact 压缩 上下文",
  };
  const withAction = { ...props, actions: [action], onActionSelect: onCompact };
  await act(async () => { tree = create(createElement(SkillPicker, withAction)); });
  const row = () => tree.root.findAll(node => node.props.accessibilityLabel === "压缩上下文" && node.props.onPress)[0]!;
  expect(row()).toBeDefined();
  // The action must be reachable by the English command word too, and must not
  // be mistaken for a skill insertion.
  await act(async () => { tree.update(createElement(SkillPicker, { ...withAction, query: "compact" })); });
  act(() => row().props.onPress());
  expect(onCompact).toHaveBeenCalledWith(action);
  expect(onSelect).not.toHaveBeenCalled();
  // A query that matches no skill still shows the action instead of "no results".
  await act(async () => { tree.update(createElement(SkillPicker, { ...withAction, query: "压缩 上下文" })); });
  expect(row()).toBeDefined();
  expect(texts()).not.toContain("skills.noResults");
  await act(async () => { tree.update(createElement(SkillPicker, { ...withAction, query: "不存在" })); });
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "压缩上下文" && node.props.onPress)).toHaveLength(0);
  expect(texts()).toContain("skills.noResults");
});

test("distinguishes loading, failure with retry, and an empty installed catalogue", async () => {
  let reject!: (reason: Error) => void;
  load.mockImplementationOnce(() => new Promise((_, no) => { reject = no; }));
  await act(async () => { tree = create(createElement(SkillPicker, props)); });
  expect(texts()).toContain("skills.loading");
  await act(async () => { reject(new Error("offline")); });
  expect(texts()).toContain("skills.loadFailed");
  load.mockResolvedValueOnce([]);
  const retry = tree.root.findAll(node => node.props.accessibilityLabel === "common.retry" && node.props.onPress)[0]!;
  await act(async () => { retry.props.onPress(); });
  expect(load).toHaveBeenCalledTimes(2);
  expect(texts()).toContain("skills.empty");
});

test("old desktops show an upgrade hint without making an unsupported request", async () => {
  await act(async () => { tree = create(createElement(SkillPicker, { ...props, supported: false })); });
  expect(load).not.toHaveBeenCalled();
  expect(texts()).toContain("skills.updateDesktop");
});

test("late replies from a previous desktop cannot populate the new picker", async () => {
  let resolve!: (value: typeof skills) => void;
  load.mockImplementationOnce(() => new Promise(yes => { resolve = yes; }));
  await act(async () => { tree = create(createElement(SkillPicker, { ...props, key: "desktop-a" })); });
  load.mockResolvedValueOnce([]);
  await act(async () => { tree.update(createElement(SkillPicker, { ...props, key: "desktop-b" })); });
  await act(async () => { resolve(skills); });
  expect(texts()).toContain("skills.empty");
  expect(texts()).not.toContain("网页搜索");
});
