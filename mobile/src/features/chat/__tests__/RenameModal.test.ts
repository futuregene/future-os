import { createElement } from "react";
import { TextInput } from "react-native";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TFunction } from "i18next";
import { Button } from "../../../components/Button";
import { RenameModal } from "../components/RenameModal";

jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));

const t = ((key: string) => key) as TFunction;
let tree: ReactTestRenderer;
const setRenameValue = jest.fn();
const submitRename = jest.fn(async () => {});
const onClose = jest.fn();
const onGenerate = jest.fn<Promise<string>, []>();
const props = { renameOpen: true, renameValue: "Original", generationKey: "s1", setRenameValue, submitRename, onClose, onGenerate, t };
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;

beforeEach(() => {
  jest.clearAllMocks();
  act(() => { tree = create(createElement(RenameModal, props)); });
});
afterEach(() => act(() => tree.unmount()));

test("hides the native Android underline inside the rounded input border", () => {
  expect(tree.root.findByType(TextInput).props.underlineColorAndroid).toBe("transparent");
});

test("opening does not generate; click fills the draft without saving", async () => {
  expect(onGenerate).not.toHaveBeenCalled();
  onGenerate.mockResolvedValueOnce("Suggested title");
  await act(async () => button("chat.generateTitle").props.onPress());
  expect(onGenerate).toHaveBeenCalledTimes(1);
  expect(setRenameValue).toHaveBeenCalledWith("Suggested title");
  expect(submitRename).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
});

test("duplicate clicks do not spend twice, and closing discards late results", async () => {
  let resolve!: (title: string) => void;
  onGenerate.mockImplementationOnce(() => new Promise(r => { resolve = r; }));
  const generate = button("chat.generateTitle").props.onPress;
  act(() => { generate(); generate(); });
  expect(onGenerate).toHaveBeenCalledTimes(1);
  expect(button("chat.save").props.disabled).toBe(true);
  act(() => button("chat.cancel").props.onPress());
  await act(async () => resolve("Stale title"));
  expect(setRenameValue).not.toHaveBeenCalled();
});

test("switching the rename target discards the old suggestion", async () => {
  let resolve!: (title: string) => void;
  onGenerate.mockImplementationOnce(() => new Promise(r => { resolve = r; }));
  act(() => button("chat.generateTitle").props.onPress());
  act(() => tree.update(createElement(RenameModal, { ...props, generationKey: "s2" })));
  await act(async () => resolve("For s1"));
  expect(setRenameValue).not.toHaveBeenCalled();
});

test("errors keep the old draft and release the generate button", async () => {
  onGenerate.mockRejectedValueOnce(new Error("offline"));
  await act(async () => button("chat.generateTitle").props.onPress());
  expect(setRenameValue).not.toHaveBeenCalled();
  expect(button("chat.generateTitle").props.disabled).toBe(false);
  expect(tree.root.findAll(node => node.props.accessibilityRole === "alert")).not.toHaveLength(0);
});
