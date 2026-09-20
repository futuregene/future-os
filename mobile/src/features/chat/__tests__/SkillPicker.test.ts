import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { SkillPicker } from "../components/SkillPicker";
import type { RemoteSkill } from "../../../remote/types";

jest.mock("lucide-react-native", () => ({ Info: () => null, X: () => null }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key, i18n: { language: "zh" } }) }));

const skills: RemoteSkill[] = [{ name: "future-web", description: "Search pages", nameZh: "网页搜索", descriptionZh: "读取网页" }];
const load = jest.fn(async () => skills);
const onSelect = jest.fn();
const onClose = jest.fn();
const props = { query: "", supported: true, load, onSelect, onClose, maxHeight: 220 };
let tree: ReactTestRenderer;
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children);
beforeEach(() => { jest.clearAllMocks(); load.mockReset().mockResolvedValue(skills); });
afterEach(() => { if (tree) act(() => tree.unmount()); });

test("loads once per opening, searches locally and selects without dismissing keyboard", async () => {
  await act(async () => { tree = create(createElement(SkillPicker, props)); });
  expect(load).toHaveBeenCalledTimes(1);
  expect(texts()).toEqual(expect.arrayContaining(["网页搜索", "读取网页"]));
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

test("skill choices keep name and description on one row, with full details only on demand", async () => {
  await act(async () => { tree = create(createElement(SkillPicker, props)); });
  const option = tree.root.findAll(node => node.props.accessibilityLabel === "/future-web · 网页搜索" && node.props.onPress)[0]!;
  expect(StyleSheet.flatten(option.props.style({ pressed: false }))).toMatchObject({ flexDirection: "row", minHeight: 44, minWidth: 0 });
  expect(option.findAllByType(Text)).toHaveLength(2);
  expect(option.findAllByType(Text).every(node => node.props.numberOfLines === 1)).toBe(true);
  expect(texts()).not.toContain("/future-web");
  const details = () => tree.root.findAll(node => node.props.accessibilityLabel === "skills.details" && node.props.onPress)[0]!;
  act(() => details().props.onPress());
  expect(details().props.accessibilityState.expanded).toBe(true);
  expect(texts()).toContain("/future-web");
  const expanded = tree.root.findAllByType(View).find(node => node.findAllByType(Text).some(text => text.props.children === "/future-web") && StyleSheet.flatten(node.props.style)?.gap === 4)!;
  expect(expanded.findAllByType(Text).every(node => node.props.numberOfLines === undefined)).toBe(true);
  expect(onSelect).not.toHaveBeenCalled();
  act(() => details().props.onPress());
  expect(texts()).not.toContain("/future-web");
  act(() => details().props.onPress());
  await act(async () => tree.update(createElement(SkillPicker, { ...props, query: "missing" })));
  await act(async () => tree.update(createElement(SkillPicker, props)));
  expect(texts()).not.toContain("/future-web");
  expect(load).toHaveBeenCalledTimes(1);
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

test("details do not repeat the command when the row already shows it as the name", async () => {
  load.mockResolvedValueOnce([{ name: "lark-doc", description: "Feishu docs" }]);
  await act(async () => { tree = create(createElement(SkillPicker, props)); });
  expect(texts()).toContain("lark-doc");
  const details = tree.root.findAll(node => node.props.accessibilityLabel === "skills.details" && node.props.onPress)[0]!;
  act(() => details.props.onPress());
  expect(texts()).toContain("Feishu docs");
  expect(texts()).not.toContain("/lark-doc");
});
