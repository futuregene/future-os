import { Keyboard, Platform } from "react-native";
import { useCallback, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { Camera, Images, File } from "lucide-react-native";
import type { TFunction } from "i18next";
import { ActionMenu } from "../../components/ActionMenu";
import {
  albumSource,
  loadAlbumImages,
  pickAttachments,
  pickFromAlbum,
  prepareAlbumImages,
  remainingImageSlots,
  takePhoto,
} from "../../remote/files";
import type { AlbumImage } from "future-file-handler";
import type { AlbumSource } from "../../remote/files";
import type { MobileAttachment } from "../../remote/types";
import { colors } from "../../theme/tokens";
import { AlbumPickerModal } from "./components/AlbumPickerModal";
import { showToast } from "./utils";

export interface AttachmentPickerApi {
  chooseFiles: () => Promise<void>;
  capturePhoto: () => Promise<void>;
  chooseFromAlbum: () => Promise<void>;
  openAttachmentMenu: () => void;
  attachmentMenu: ReactNode;
  /** The app's own album grid, for phones with no system picker to show. */
  albumPicker: ReactNode;
}

export function useAttachmentPicker(
  attachments: MobileAttachment[],
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>,
  t: TFunction,
): AttachmentPickerApi {
  const [menuOpen, setMenuOpen] = useState(false);
  const [album, setAlbum] = useState<{ images: AlbumImage[] | null; error: string | null } | null>(
    null,
  );
  // Reopening the grid must not restore the previous round's ticks, and the
  // modal stays mounted between openings, so each listing gets a new instance.
  const [albumSession, setAlbumSession] = useState(0);
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
  // The album is the one source the phone may not be able to present: a system
  // picker opens as a native overlay, while a device with neither a gallery nor
  // a photo picker needs the grid the app draws for itself.
  const chooseFromAlbum = useCallback(async () => {
    if (picking.current) return;
    let source: AlbumSource;
    try {
      source = await albumSource();
    } catch {
      source = "unavailable";
    }
    if (Platform.OS !== "android" || source === "system") {
      await pick(pickFromAlbum);
      return;
    }
    if (source === "unavailable") {
      showToast(t("attachment.errors.attachment_album_unavailable"));
      return;
    }
    // The grid owns its failures: an empty or unreadable album has to say so
    // where the choice was made, not vanish into a toast behind a closed sheet.
    setAlbum({ images: null, error: null });
    setAlbumSession(session => session + 1);
    try {
      const images = await loadAlbumImages();
      setAlbum(current => (current ? { images, error: null } : current));
    } catch (error) {
      const key = error instanceof Error && error.message.startsWith("attachment_")
        ? error.message
        : "attachment_album_unavailable";
      setAlbum(current =>
        current ? { images: [], error: t(`attachment.errors.${key}`) } : current,
      );
    }
  }, [pick, t]);
  const openAttachmentMenu = useCallback(() => {
    if (picking.current) return;
    Keyboard.dismiss();
    setMenuOpen(true);
  }, []);
  const confirmAlbum = useCallback(
    (images: AlbumImage[]) => {
      setAlbum(null);
      void pick(() => prepareAlbumImages(attachments, images));
    },
    [attachments, pick],
  );

  const limit = Math.max(remainingImageSlots(attachments), 1);
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
  const albumPicker = (
    <AlbumPickerModal
      error={album?.error ?? null}
      images={album?.images ?? null}
      key={albumSession}
      limit={limit}
      onCancel={() => setAlbum(null)}
      onConfirm={confirmAlbum}
      t={t}
      visible={album !== null}
    />
  );
  return {
    chooseFiles,
    capturePhoto,
    chooseFromAlbum,
    openAttachmentMenu,
    attachmentMenu,
    albumPicker,
  };
}
