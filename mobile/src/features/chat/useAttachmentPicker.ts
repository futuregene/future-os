import { ActionSheetIOS, Platform } from "react-native";
import type { Dispatch, SetStateAction } from "react";
import { showActionSheet as showAndroidActionSheet } from "future-native-ui";
import type { TFunction } from "i18next";
import { pickAttachments, pickFromAlbum, takePhoto } from "../../remote/files";
import type { MobileAttachment } from "../../remote/types";
import { deferPresentation, showToast } from "./utils";

export interface AttachmentPickerApi {
  chooseFiles: () => Promise<void>;
  capturePhoto: () => Promise<void>;
  chooseFromAlbum: () => Promise<void>;
  openAttachmentMenu: () => void;
}

export function useAttachmentPicker(
  attachments: MobileAttachment[],
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>,
  t: TFunction,
): AttachmentPickerApi {
  const chooseFiles = async () => {
    try {
      setAttachments(await pickAttachments(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    }
  };

  const capturePhoto = async () => {
    try {
      setAttachments(await takePhoto(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    }
  };

  const chooseFromAlbum = async () => {
    try {
      setAttachments(await pickFromAlbum(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    }
  };

  const openAttachmentMenu = () => {
    if (Platform.OS === "ios") {
      ActionSheetIOS.showActionSheetWithOptions(
        {
          options: [
            t("attachment.takePhoto"),
            t("attachment.chooseFromAlbum"),
            t("attachment.chooseFiles"),
            t("chat.cancel"),
          ],
          cancelButtonIndex: 3,
        },
        index => {
          // Run after native presentation work has settled so the picker never
          // competes with the action sheet for the current view controller.
          if (index === 0) deferPresentation(() => void capturePhoto());
          if (index === 1) deferPresentation(() => void chooseFromAlbum());
          if (index === 2) deferPresentation(() => void chooseFiles());
        },
      );
      return;
    }
    void showAndroidActionSheet(
      [
        t("attachment.takePhoto"),
        t("attachment.chooseFromAlbum"),
        t("attachment.chooseFiles"),
        t("chat.cancel"),
      ],
      t("attachment.title"),
    )
      .then(index => {
        if (index === 0) deferPresentation(() => void capturePhoto());
        if (index === 1) deferPresentation(() => void chooseFromAlbum());
        if (index === 2) deferPresentation(() => void chooseFiles());
      })
      .catch(() => showToast(t("attachment.errors.attachment_failed")));
  };

  return { chooseFiles, capturePhoto, chooseFromAlbum, openAttachmentMenu };
}
