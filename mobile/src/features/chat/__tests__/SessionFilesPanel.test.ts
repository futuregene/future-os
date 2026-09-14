import { createElement, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { BackHandler, FlatList } from "react-native";
import { SessionFilesPanel } from "../components/SessionFilesPanel";
import type { SessionFileListing } from "../../../remote/types";

jest.mock("lucide-react-native", () => ({
  ArrowUp: "ArrowUp", ChevronRight: "ChevronRight", Eye: "Eye", EyeOff: "EyeOff",
  File: "File", Folder: "Folder", RefreshCw: "RefreshCw",
}));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string, options?: { name: string }) => options ? `${key}:${options.name}` : key }),
}));

const root: SessionFileListing = {
  rootPath: "C:\\work", path: "C:\\work",
  entries: [
    { name: "reports", path: "C:\\work\\reports", isDir: true, size: 0 },
    { name: "notes.md", path: "C:\\work\\notes.md", isDir: false, size: 12 },
    { name: ".hidden", path: "C:\\work\\.hidden", isDir: false, size: 2 },
  ],
};
let tree: ReactTestRenderer;
let props: ComponentProps<typeof SessionFilesPanel>;
const listFiles = jest.fn();
const onOpenFile = jest.fn();
const onClose = jest.fn();
let systemBack: (() => boolean | null | undefined) | null;
const removeBack = jest.fn();
const entries = () => tree.root.findByType(FlatList).props.data;
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
async function mount(patch: Partial<typeof props> = {}) {
  props = { online: true, supported: true, isWorkspace: false, listFiles, onOpenFile, onClose, ...patch };
  await act(async () => { tree = create(createElement(SessionFilesPanel, props)); });
}
async function press(label: string) {
  await act(async () => { button(label).props.onPress(); });
}
beforeEach(() => {
  jest.clearAllMocks();
  systemBack = null;
  jest.spyOn(BackHandler, "addEventListener").mockImplementation((_event, handler) => {
    systemBack = () => handler({ type: "hardwareBackPress", timeStamp: Date.now() });
    return { remove: removeBack };
  });
  listFiles.mockResolvedValue(root);
  onOpenFile.mockResolvedValue(undefined);
});
afterEach(() => {
  act(() => tree?.unmount());
  jest.restoreAllMocks();
});

test("loads the session root, toggles dotfiles, and opens the exact desktop file path", async () => {
  await mount();
  expect(listFiles).toHaveBeenCalledWith("");
  expect(entries().map((item: { name: string }) => item.name)).toEqual(["reports", "notes.md"]);
  await press("files.showHidden");
  expect(entries()).toHaveLength(3);
  await press("files.openFile:notes.md");
  expect(onOpenFile).toHaveBeenCalledWith("C:\\work\\notes.md");
  expect(listFiles).toHaveBeenCalledTimes(1);
});

test("enters a directory, refreshes it, and returns to the session root", async () => {
  await mount({ isWorkspace: true });
  expect(entries()).toHaveLength(3);
  const child = { ...root, path: "C:\\work\\reports", entries: [] };
  listFiles.mockResolvedValue(child);
  await press("files.openFolder:reports");
  expect(listFiles).toHaveBeenLastCalledWith(child.path);
  expect(entries()).toHaveLength(0);
  await press("files.refresh");
  expect(listFiles).toHaveBeenLastCalledWith(child.path);
  listFiles.mockResolvedValue(root);
  await press("files.up");
  expect(listFiles).toHaveBeenLastCalledWith("");
});

test("system back pops one directory at a time and only closes at the root", async () => {
  const child = {
    ...root, path: "C:\\work\\reports",
    entries: [{ name: "drafts", path: "C:\\work\\reports\\drafts", isDir: true, size: 0 }],
  };
  listFiles.mockImplementation(async (path: string) => path === child.path ? child
    : path.endsWith("drafts") ? { ...root, path, entries: [] } : root);
  await mount();
  await press("files.openFolder:reports");
  await press("files.openFolder:drafts");
  await act(async () => { expect(systemBack?.()).toBe(true); });
  expect(listFiles).toHaveBeenLastCalledWith(child.path);
  expect(onClose).not.toHaveBeenCalled();
  await act(async () => { expect(systemBack?.()).toBe(true); });
  expect(listFiles).toHaveBeenLastCalledWith("");
  expect(onClose).not.toHaveBeenCalled();
  await act(async () => { expect(systemBack?.()).toBe(true); });
  expect(onClose).toHaveBeenCalledTimes(1);
  act(() => tree.unmount());
  expect(removeBack).toHaveBeenCalled();
});

test("system back works during loading and ignores a late child-directory response", async () => {
  await mount();
  let resolve!: (listing: SessionFileListing) => void;
  listFiles.mockReturnValueOnce(new Promise<SessionFileListing>(yes => { resolve = yes; }));
  await press("files.openFolder:reports");
  await act(async () => { systemBack?.(); });
  expect(listFiles).toHaveBeenLastCalledWith("");
  await act(async () => { resolve({ ...root, path: "C:\\work\\reports", entries: [] }); });
  expect(entries()).toHaveLength(2);
  expect(onClose).not.toHaveBeenCalled();
});

test("system back navigates up even when the desktop is offline", async () => {
  await mount();
  await press("files.openFolder:reports");
  await act(async () => { tree.update(createElement(SessionFilesPanel, { ...props, online: false })); });
  await act(async () => { systemBack?.(); });
  expect(button("files.up").props.disabled).toBe(true);
  expect(onClose).not.toHaveBeenCalled();
});

test.each([{ online: false }, { supported: false }])("does not request files when unavailable: %j", async patch => {
  await mount(patch);
  expect(listFiles).not.toHaveBeenCalled();
  expect(button("files.refresh").props.disabled).toBe(true);
});

test("shows a retryable error and recovers on refresh", async () => {
  listFiles.mockRejectedValueOnce(new Error("directory removed"));
  await mount();
  expect(tree.root.findAll(node => node.props.children === "files.loadFailed").length).toBeGreaterThan(0);
  listFiles.mockResolvedValue(root);
  await press("files.refresh");
  expect(entries()).toHaveLength(2);
});

test("late requests cannot replace a refreshed directory or repopulate an offline panel", async () => {
  let resolve!: (value: SessionFileListing) => void;
  listFiles.mockReturnValueOnce(new Promise<SessionFileListing>(yes => { resolve = yes; }));
  await mount();
  await act(async () => { tree.update(createElement(SessionFilesPanel, { ...props, online: false })); });
  await act(async () => { resolve(root); });
  expect(entries()).toEqual([]);
  expect(tree.root.findAll(node => node.props.children === "files.offline").length).toBeGreaterThan(0);
  await act(async () => { tree.update(createElement(SessionFilesPanel, props)); });
  expect(entries()).toHaveLength(2);
});
