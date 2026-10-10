import { Fragment, createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform } from "react-native";
import type { TFunction } from "i18next";
import { useAttachmentPicker } from "../useAttachmentPicker";
import { ActionMenu } from "../../../components/ActionMenu";
import { AlbumPickerModal } from "../components/AlbumPickerModal";
import {
  albumSource,
  loadAlbumImages,
  pickAttachments,
  pickFromAlbum,
  prepareAlbumImages,
  takePhoto,
} from "../../../remote/files";

jest.mock("../../../remote/files", () => ({
  albumSource: jest.fn(async () => "system"),
  loadAlbumImages: jest.fn(async () => []),
  pickAttachments: jest.fn(async () => []),
  pickFromAlbum: jest.fn(async () => []),
  prepareAlbumImages: jest.fn(async () => []),
  remainingImageSlots: jest.fn(() => 4),
  takePhoto: jest.fn(async () => []),
}));
jest.mock("../utils", () => ({ showToast: jest.fn() }));
jest.mock("lucide-react-native", () => ({ Camera: () => null, Images: () => null, File: () => null, X: () => null }));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

const { showToast } = jest.requireMock("../utils") as { showToast: jest.Mock };
const t = ((key: string) => key) as TFunction;
let api: ReturnType<typeof useAttachmentPicker>;
const setAttachments = jest.fn();

function Harness() {
  const value = useAttachmentPicker([], setAttachments, t);
  useEffect(() => {
    api = value;
  });
  return createElement(Fragment, null, value.attachmentMenu, value.albumPicker);
}

function render(): ReactTestRenderer {
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(Harness));
  });
  return tree;
}

const album = [{ uri: "content://media/external/images/media/1", name: "IMG_1.jpg", mimeType: "image/jpeg", size: 10, modified: 1 }];

beforeEach(() => {
  jest.clearAllMocks();
  Platform.OS = "android";
  (albumSource as jest.Mock).mockResolvedValue("system");
  (loadAlbumImages as jest.Mock).mockResolvedValue(album);
  (prepareAlbumImages as jest.Mock).mockResolvedValue([{ name: "IMG_1.jpg" }]);
});

test("a system picker still opens as a native overlay", async () => {
  const tree = render();
  try {
    await act(async () => {
      await api.chooseFromAlbum();
    });
    expect(pickFromAlbum).toHaveBeenCalledTimes(1);
    expect(loadAlbumImages).not.toHaveBeenCalled();
    expect(tree.root.findAllByType(AlbumPickerModal)[0]!.props.visible).toBe(false);
  } finally {
    act(() => tree.unmount());
  }
});

test("a phone with no system picker gets the app's own grid", async () => {
  (albumSource as jest.Mock).mockResolvedValue("inApp");
  const tree = render();
  try {
    await act(async () => {
      await api.chooseFromAlbum();
    });
    const grid = tree.root.findAllByType(AlbumPickerModal)[0]!;
    expect(grid.props.visible).toBe(true);
    expect(grid.props.images).toEqual(album);
    expect(pickFromAlbum).not.toHaveBeenCalled();
  } finally {
    act(() => tree.unmount());
  }
});

test("confirming the grid adds the chosen images through the shared pipeline", async () => {
  (albumSource as jest.Mock).mockResolvedValue("inApp");
  const tree = render();
  try {
    await act(async () => {
      await api.chooseFromAlbum();
    });
    const grid = tree.root.findAllByType(AlbumPickerModal)[0]!;
    await act(async () => {
      grid.props.onConfirm(album);
    });
    expect(prepareAlbumImages).toHaveBeenCalledWith([], album);
    expect(setAttachments).toHaveBeenCalledWith([{ name: "IMG_1.jpg" }]);
    expect(tree.root.findAllByType(AlbumPickerModal)[0]!.props.visible).toBe(false);
  } finally {
    act(() => tree.unmount());
  }
});

test("cancelling the grid adds nothing and closes it", async () => {
  (albumSource as jest.Mock).mockResolvedValue("inApp");
  const tree = render();
  try {
    await act(async () => {
      await api.chooseFromAlbum();
    });
    act(() => tree.root.findAllByType(AlbumPickerModal)[0]!.props.onCancel());
    expect(tree.root.findAllByType(AlbumPickerModal)[0]!.props.visible).toBe(false);
    expect(prepareAlbumImages).not.toHaveBeenCalled();
    expect(setAttachments).not.toHaveBeenCalled();
  } finally {
    act(() => tree.unmount());
  }
});

test("a phone that can present nothing says so instead of opening a file browser", async () => {
  (albumSource as jest.Mock).mockResolvedValue("unavailable");
  const tree = render();
  try {
    await act(async () => {
      await api.chooseFromAlbum();
    });
    expect(showToast).toHaveBeenCalledWith("attachment.errors.attachment_album_unavailable");
    expect(pickFromAlbum).not.toHaveBeenCalled();
    expect(tree.root.findAllByType(AlbumPickerModal)[0]!.props.visible).toBe(false);
  } finally {
    act(() => tree.unmount());
  }
});

test("an unreadable grid keeps its reason inside the sheet", async () => {
  (albumSource as jest.Mock).mockResolvedValue("inApp");
  (loadAlbumImages as jest.Mock).mockRejectedValue(new Error("attachment_album_permission"));
  const tree = render();
  try {
    await act(async () => {
      await api.chooseFromAlbum();
    });
    const grid = tree.root.findAllByType(AlbumPickerModal)[0]!;
    expect(grid.props.visible).toBe(true);
    expect(grid.props.error).toBe("attachment.errors.attachment_album_permission");
    expect(grid.props.images).toEqual([]);
  } finally {
    act(() => tree.unmount());
  }
});

test("the album source menu still defers the camera and file pickers to the OS", async () => {
  const tree = render();
  try {
    act(() => api.openAttachmentMenu());
    const menu = tree.root.findByType(ActionMenu);
    const modal = menu.findByType(Modal);
    const label = menu.props.actions[0].label;
    const button = menu.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
    act(() => button.props.onPress());
    expect(takePhoto).not.toHaveBeenCalled();
    await act(async () => {
      modal.props.onDismiss();
    });
    expect(takePhoto).toHaveBeenCalledTimes(1);
    expect(pickAttachments).not.toHaveBeenCalled();
  } finally {
    act(() => tree.unmount());
  }
});
