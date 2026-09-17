import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { DialogSurface } from "../../../components/DialogSurface";
import { DownloadProgressModal } from "../components/DownloadProgressModal";
import type { ActiveDownload } from "../utils";

jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));

const download: ActiveDownload = {
  id: "download", fileName: "A long filename to preview.txt", phase: "downloading", completedBytes: 1024, totalBytes: 2048,
};
const props = {
  activeDownload: download, activeDownloadFraction: 0.5,
  cancelActiveDownload: jest.fn(), flushPendingDownloadModal: jest.fn(), onDownloadModalShow: jest.fn(),
  t: ((key: string) => key) as TFunction,
};
let tree: ReactTestRenderer;
const texts = () => tree.root.findAllByType(Text);
const cancel = () => tree.root.findAll(node => node.props.accessibilityLabel === "chat.cancel" && node.props.onPress)[0]!;
beforeEach(() => {
  jest.clearAllMocks();
  act(() => { tree = create(createElement(DownloadProgressModal, props)); });
});
afterEach(() => act(() => tree.unmount()));

test("cancel shares the title row and related metadata forms compact groups", () => {
  const header = tree.root.findAllByType(View).find(node => StyleSheet.flatten(node.props.style)?.flexDirection === "row" && node.findAllByType(Text).some(text => text.props.children === "attachment.downloadProgressTitle"))!;
  expect(header.findAll(node => node.props.accessibilityLabel === "chat.cancel").length).toBeGreaterThan(0);
  expect(StyleSheet.flatten(cancel().props.style({ pressed: false }))).toMatchObject({ minWidth: 44, minHeight: 44 });
  expect(tree.root.findByType(DialogSurface).props.children).toHaveLength(3);
  const fileName = texts().find(node => node.props.children === download.fileName)!;
  expect(fileName.props.numberOfLines).toBe(1);
  expect(fileName.props.ellipsizeMode).toBe("middle");
  const bytes = texts().find(node => node.props.children === "1 KB / 2 KB")!;
  expect(bytes.props.numberOfLines).toBe(1);
  expect(StyleSheet.flatten(bytes.props.style)).toMatchObject({ flex: 1, minWidth: 0 });
  const percent = texts().find(node => node.props.children === "50%")!;
  expect(StyleSheet.flatten(percent.props.style).flexShrink).toBe(0);
});

test("unknown size and cancelling remain explicit without extra operation rows", () => {
  act(() => tree.update(createElement(DownloadProgressModal, { ...props, activeDownload: { ...download, totalBytes: 0, phase: "cancelling" } })));
  expect(texts().some(node => node.props.children === "attachment.calculatingSize")).toBe(true);
  expect(texts().some(node => node.props.children === "attachment.downloadPhases.cancelling")).toBe(true);
  expect(texts().some(node => node.props.children === "50%")).toBe(false);
  expect(cancel().props.disabled).toBe(true);
  expect(cancel().props.accessibilityState.disabled).toBe(true);
  expect(StyleSheet.flatten(cancel().props.style({ pressed: false })).opacity).toBe(0.5);
});

test("preserves cancel and native presentation callbacks", () => {
  act(() => cancel().props.onPress());
  expect(props.cancelActiveDownload).toHaveBeenCalledTimes(1);
  const modal = tree.root.findByType(Modal);
  act(() => { modal.props.onShow(); modal.props.onRequestClose(); modal.props.onDismiss(); });
  expect(props.onDownloadModalShow).toHaveBeenCalledTimes(1);
  expect(props.flushPendingDownloadModal).toHaveBeenCalledTimes(1);
  expect(props.cancelActiveDownload).toHaveBeenCalledTimes(2);
  act(() => tree.update(createElement(DownloadProgressModal, { ...props, activeDownload: null })));
  expect(tree.root.findByType(Modal).props.visible).toBe(false);
});
