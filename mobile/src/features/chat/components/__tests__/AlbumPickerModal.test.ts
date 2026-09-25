import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Image, Modal, Text } from "react-native";
import type { TFunction } from "i18next";
import { AlbumPickerModal } from "../AlbumPickerModal";
import type { AlbumImage } from "future-file-handler";

jest.mock("lucide-react-native", () => ({ X: () => null }));
jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));

const t = ((key: string, values?: Record<string, unknown>) =>
  values ? `${key}:${JSON.stringify(values)}` : key) as TFunction;

function image(index: number): AlbumImage {
  return {
    uri: `content://media/external/images/media/${index}`,
    name: `IMG_${index}.jpg`,
    mimeType: "image/jpeg",
    size: 1_000 + index,
    modified: 1_700_000_000_000 + index,
  };
}

const IMAGES = [image(1), image(2), image(3), image(4)];

interface Options {
  images?: AlbumImage[] | null;
  error?: string | null;
  limit?: number;
}

// Every tree is unmounted after its test: a live FlatList keeps a batching
// timer that outlives the run and makes jest exit non-zero ("did not exit one
// second after the test run").
const mounted: ReactTestRenderer[] = [];

afterEach(() => {
  for (const tree of mounted.splice(0)) act(() => tree.unmount());
});

function modal({ images = IMAGES, error = null, limit = 4 }: Options = {}) {
  const onCancel = jest.fn();
  const onConfirm = jest.fn();
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(
      createElement(AlbumPickerModal, {
        error,
        images,
        limit,
        onCancel,
        onConfirm,
        t,
        visible: true,
      }),
    );
  });
  mounted.push(tree);
  return { tree, onCancel, onConfirm };
}

function cell(tree: ReactTestRenderer, name: string) {
  const node = tree.root.findAll(
    candidate => candidate.props.accessibilityLabel === name && typeof candidate.props.onPress === "function",
  )[0];
  if (!node) throw new Error(`no cell labelled ${name}`);
  return node;
}

function confirmButton(tree: ReactTestRenderer) {
  const node = tree.root.findAll(
    candidate =>
      typeof candidate.props.accessibilityLabel === "string" &&
      candidate.props.accessibilityLabel.startsWith("attachment.albumConfirm") &&
      typeof candidate.props.onPress === "function",
  )[0];
  if (!node) throw new Error("no confirm button");
  return node;
}

function showsText(tree: ReactTestRenderer, text: string): boolean {
  return tree.root
    .findAllByType(Text)
    .some(node => JSON.stringify(node.props.children).includes(text));
}

test("renders a thumbnail per image the phone reported", () => {
  const { tree } = modal();
  expect(tree.root.findAllByType(Image)).toHaveLength(IMAGES.length);
  expect(cell(tree, "IMG_3.jpg")).toBeTruthy();
});

test("confirms the images in the order they were listed, not the order they were tapped", () => {
  const { tree, onConfirm } = modal();
  act(() => cell(tree, "IMG_3.jpg").props.onPress());
  act(() => cell(tree, "IMG_1.jpg").props.onPress());
  act(() => confirmButton(tree).props.onPress());
  expect(onConfirm).toHaveBeenCalledWith([IMAGES[0], IMAGES[2]]);
});

test("deselecting an image removes it from the payload", () => {
  const { tree, onConfirm } = modal();
  act(() => cell(tree, "IMG_1.jpg").props.onPress());
  act(() => cell(tree, "IMG_1.jpg").props.onPress());
  expect(confirmButton(tree).props.disabled).toBe(true);
  act(() => cell(tree, "IMG_2.jpg").props.onPress());
  act(() => confirmButton(tree).props.onPress());
  expect(onConfirm).toHaveBeenCalledWith([IMAGES[1]]);
});
test("stops selecting at the remaining image quota", () => {
  const { tree, onConfirm } = modal({ limit: 2 });
  act(() => cell(tree, "IMG_1.jpg").props.onPress());
  act(() => cell(tree, "IMG_2.jpg").props.onPress());
  act(() => cell(tree, "IMG_3.jpg").props.onPress());
  act(() => confirmButton(tree).props.onPress());
  expect(onConfirm).toHaveBeenCalledWith([IMAGES[0], IMAGES[1]]);
});

test("surfaces the phone's answer instead of an empty grid", () => {
  const empty = modal({ images: [] });
  expect(showsText(empty.tree, "attachment.albumEmpty")).toBe(true);

  const denied = modal({ images: [], error: "需要相册权限" });
  expect(showsText(denied.tree, "需要相册权限")).toBe(true);
  expect(denied.tree.root.findAllByType(Image)).toHaveLength(0);
});

test("shows the loading state until the listing arrives", () => {
  const { tree } = modal({ images: null });
  expect(showsText(tree, "attachment.albumLoading")).toBe(true);
  expect(tree.root.findAllByType(Image)).toHaveLength(0);
});

test("the sheet stays modal and cancellable", () => {
  const { tree, onCancel, onConfirm } = modal();
  const sheet = tree.root.findByType(Modal);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.transparent).toBe(true);
  act(() => sheet.props.onRequestClose());
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(onConfirm).not.toHaveBeenCalled();
});

test("a reopened sheet starts with nothing selected", () => {
  const first = modal();
  act(() => cell(first.tree, "IMG_2.jpg").props.onPress());
  expect(confirmButton(first.tree).props.disabled).toBe(false);
  // The caller remounts the grid per listing (see useAttachmentPicker), so the
  // same element rendered again is a fresh sheet.
  const second = modal();
  expect(confirmButton(second.tree).props.disabled).toBe(true);
});
