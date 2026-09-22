import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Text } from "react-native";
import { SkillDetailsDialog } from "../components/SkillDetailsDialog";
import type { RemoteSkill } from "../../../remote/types";

let mockLanguage = "zh";
jest.mock("lucide-react-native", () => ({ X: "X" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: mockLanguage } }),
}));
jest.mock("react-native-safe-area-context", () => ({ useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) }));

const localized: RemoteSkill = {
  name: "future-web",
  description: "Search the public web.",
  nameZh: "网页检索",
  descriptionZh: "搜索公开网页并核实信息。",
};
const plain: RemoteSkill = { name: "lark-doc", description: "Feishu docs." };
const onClose = jest.fn();
let tree: ReactTestRenderer | null = null;

const mount = (skill: RemoteSkill | null) => {
  act(() => { tree = create(createElement(SkillDetailsDialog, { skill, onClose })); });
};
const texts = () => tree!.root.findAllByType(Text).map(node => node.props.children);
const close = () => tree!.root.findAll(node => node.props.accessibilityLabel === "common.close" && node.props.onPress)[0]!;

beforeEach(() => { jest.clearAllMocks(); mockLanguage = "zh"; });
afterEach(() => { if (tree) act(() => tree!.unmount()); tree = null; });

test("shows the localized name, its command and the whole description", () => {
  mount(localized);
  expect(tree!.root.findAllByType(Modal)).toHaveLength(1);
  expect(texts()).toEqual(expect.arrayContaining(["网页检索", "/future-web", "搜索公开网页并核实信息。"]));
  // The command is only spelled out because the row shows the localized name.
  expect(texts()).not.toContain("Search the public web.");
  // Nothing is clamped: this surface gets the whole viewport, unlike the row.
  expect(texts()).not.toContain(undefined);
  act(() => close().props.onPress());
  expect(onClose).toHaveBeenCalledTimes(1);
});

test("an unlocalized skill is titled by its command, so the command is not repeated", () => {
  mount(plain);
  expect(texts()).toEqual(expect.arrayContaining(["lark-doc", "Feishu docs."]));
  expect(texts()).not.toContain("/lark-doc");
});

test("reads the English text on an English UI", () => {
  mockLanguage = "en";
  mount(localized);
  expect(texts()).toEqual(expect.arrayContaining(["future-web", "Search the public web."]));
  expect(texts()).not.toContain("网页检索");
  // English shows the command as the name, so there is no command line.
  expect(texts()).not.toContain("/future-web");
});

test("renders nothing while no skill is open", () => {
  mount(null);
  expect(tree!.root.findAllByType(Modal)).toHaveLength(0);
});
