import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform } from "react-native";
import type { TFunction } from "i18next";
import { useAttachmentPicker } from "../useAttachmentPicker";
import { ActionMenu } from "../../../components/ActionMenu";
import { pickAttachments, pickFromAlbum, takePhoto } from "../../../remote/files";

jest.mock("../../../remote/files", () => ({ pickAttachments: jest.fn(async () => []), pickFromAlbum: jest.fn(async () => []), takePhoto: jest.fn(async () => []) }));
jest.mock("../utils", () => ({ showToast: jest.fn() }));
jest.mock("lucide-react-native", () => ({ Camera: () => null, Images: () => null, File: () => null, X: () => null }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
const t = ((key: string) => key) as TFunction;
let api: ReturnType<typeof useAttachmentPicker>;
const setAttachments = jest.fn();
function Harness() { const value = useAttachmentPicker([], setAttachments, t); useEffect(() => { api = value; }); return value.attachmentMenu; }

test.each([0, 1, 2])("source %i waits for the shared sheet to close before opening the native picker", async index => {
  Platform.OS = "ios";
  jest.clearAllMocks();
  let tree: ReactTestRenderer;
  act(() => { tree = create(createElement(Harness)); });
  try {
    act(() => api.openAttachmentMenu());
    const menu = tree!.root.findByType(ActionMenu);
    const modal = menu.findByType(Modal);
    const label = menu.props.actions[index].label;
    const button = menu.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
    act(() => button.props.onPress());
    for (const picker of [takePhoto, pickFromAlbum, pickAttachments]) expect(picker).not.toHaveBeenCalled();
    await act(async () => { modal.props.onDismiss(); modal.props.onDismiss(); });
    expect([takePhoto, pickFromAlbum, pickAttachments][index]).toHaveBeenCalledTimes(1);
    expect(setAttachments).toHaveBeenCalledTimes(1);
  } finally { act(() => tree!.unmount()); }
});
