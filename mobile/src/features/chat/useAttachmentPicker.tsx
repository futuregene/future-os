import { Keyboard } from "react-native";
import { useCallback, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { Camera, Images, File } from "lucide-react-native";
import type { TFunction } from "i18next";
import { ActionMenu } from "../../components/ActionMenu";
import { pickAttachments, pickFromAlbum, takePhoto } from "../../remote/files";
import type { MobileAttachment } from "../../remote/types";
import { colors } from "../../theme/tokens";
import { showToast } from "./utils";

export interface AttachmentPickerApi {
  chooseFiles: () => Promise<void>;
  capturePhoto: () => Promise<void>;
  chooseFromAlbum: () => Promise<void>;
  openAttachmentMenu: () => void;
  attachmentMenu: ReactNode;
}

export function useAttachmentPicker(
  attachments: MobileAttachment[],
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>,
  t: TFunction,
): AttachmentPickerApi {
  const [menuOpen, setMenuOpen] = useState(false);
  const picking = useRef(false);
  const pick = useCallback(async (launch: typeof pickAttachments) => {
    if (picking.current) return;
    picking.current = true;
    try {
      setAttachments(await launch(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    } finally {
      picking.current = false;
    }
  }, [attachments, setAttachments, t]);
  const chooseFiles = useCallback(() => pick(pickAttachments), [pick]);
  const capturePhoto = useCallback(() => pick(takePhoto), [pick]);
  const chooseFromAlbum = useCallback(() => pick(pickFromAlbum), [pick]);
  const openAttachmentMenu = useCallback(() => {
    if (picking.current) return;
    Keyboard.dismiss();
    setMenuOpen(true);
  }, []);

  // Only the source menu is app-styled. ActionMenu waits for dismissal before
  // handing presentation to the real OS camera/photo/document picker.
  const attachmentMenu = (
    <ActionMenu
      title={t("attachment.title")}
      visible={menuOpen}
      onClose={() => setMenuOpen(false)}
      actions={[
        { label: t("attachment.takePhoto"), icon: <Camera size={18} color={colors.inkSoft} />, onPress: () => void capturePhoto() },
        { label: t("attachment.chooseFromAlbum"), icon: <Images size={18} color={colors.inkSoft} />, onPress: () => void chooseFromAlbum() },
        { label: t("attachment.chooseFiles"), icon: <File size={18} color={colors.inkSoft} />, onPress: () => void chooseFiles() },
      ]}
    />
  );
  return { chooseFiles, capturePhoto, chooseFromAlbum, openAttachmentMenu, attachmentMenu };
}
