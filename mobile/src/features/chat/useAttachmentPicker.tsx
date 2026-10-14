import { Keyboard, Platform } from "react-native";
import { useCallback, useLayoutEffect, useRef, useState, type Dispatch, type ReactNode, type SetStateAction } from "react";
import { Camera, Images, File } from "lucide-react-native";
import type { TFunction } from "i18next";
import { ActionMenu } from "../../components/ActionMenu";
import {
  deleteTemporaryAttachment,
  validateAttachmentBatch,
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
import { useCommittedRef } from "../../lib/useCommittedRef";
import { useTheme } from "../../theme/ThemeContext";
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
  operation: { scope?: string | object; captureOperation?: () => () => boolean; } = {},
): AttachmentPickerApi {
  const { colors } = useTheme();
  const [menuOpen, setMenuOpen] = useState(false);
  const [album, setAlbum] = useState<{ images: AlbumImage[] | null; error: string | null; } | null>(
    null,
  );
  // Reopening the grid must not restore the previous round's ticks, and the
  // modal stays mounted between openings, so each listing gets a new instance.
  const [albumSession, setAlbumSession] = useState(0);
  const picking = useRef<object | null>(null);
  const albumRequestRef = useRef<object | null>(null);
  const lifetimeRef = useRef<object | null>(null);
  const attachmentsRef = useCommittedRef(attachments);
  const captureRef = useCommittedRef(operation.captureOperation);
  const scopeRef = useCommittedRef(operation.scope);
  useLayoutEffect(() => {
    lifetimeRef.current = {};
    // Navigation retracts UI owned by the outgoing picker.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMenuOpen(false);
    setAlbum(null);
    return () => {
      lifetimeRef.current = null;
      picking.current = null;
      albumRequestRef.current = null;
    };
  }, [operation.scope]);
  const capture = useCallback(() => {
    const lifetime = lifetimeRef.current;
    const navigationIsCurrent = captureRef.current?.();
    const expectedScope = operation.scope;
    return () => expectedScope === scopeRef.current && lifetime !== null && lifetimeRef.current === lifetime
      && (navigationIsCurrent?.() ?? true);
  }, [captureRef, operation.scope, scopeRef]);
  const pick = useCallback(async (launch: typeof pickAttachments) => {
    const isCurrent = capture();
    if (!isCurrent() || picking.current) return;
    const attempt = {};
    picking.current = attempt;
    const original = attachmentsRef.current;
    const existing = new Set(original.map(item => item.localUri));
    let added: MobileAttachment[] = [];
    let accepted = false;
    try {
      const result = await launch(original);
      added = result.filter(item => !existing.has(item.localUri));
      if (!isCurrent()) return;
      const next = [...attachmentsRef.current];
      for (const item of added) if (!next.some(current => current.localUri === item.localUri)) next.push(item);
      validateAttachmentBatch(next);
      attachmentsRef.current = next;
      setAttachments(next);
      accepted = true;
    } catch (error) {
      if (isCurrent()) {
        const key = error instanceof Error ? error.message : "attachment_failed";
        showToast(t(`attachment.errors.${key}`));
      }
    } finally {
      if (!accepted) for (const item of added) {
        if (item.temporary) try { deleteTemporaryAttachment(item); } catch { /* best-effort cleanup */ }
      }
      if (picking.current === attempt) picking.current = null;
    }
  }, [attachmentsRef, capture, setAttachments, t]);
  const chooseFiles = useCallback(() => pick(pickAttachments), [pick]);
  const capturePhoto = useCallback(() => pick(takePhoto), [pick]);
  const chooseFromAlbum = useCallback(async () => {
    const isCurrent = capture();
    if (!isCurrent() || picking.current) return;
    const attempt = {};
    picking.current = attempt;
    try {
      let source: AlbumSource;
      try { source = await albumSource(); } catch { source = "unavailable"; }
      if (!isCurrent()) return;
      if (Platform.OS !== "android" || source === "system") {
        picking.current = null;
        await pick(pickFromAlbum);
        return;
      }
      if (source === "unavailable") {
        showToast(t("attachment.errors.attachment_album_unavailable"));
        return;
      }
      const request = {};
      albumRequestRef.current = request;
      setAlbum({ images: null, error: null });
      setAlbumSession(session => session + 1);
      try {
        const images = await loadAlbumImages();
        if (isCurrent() && albumRequestRef.current === request) setAlbum({ images, error: null });
      } catch (error) {
        if (!isCurrent() || albumRequestRef.current !== request) return;
        const key = error instanceof Error && error.message.startsWith("attachment_")
          ? error.message : "attachment_album_unavailable";
        setAlbum({ images: [], error: t(`attachment.errors.${key}`) });
      }
    } finally {
      if (picking.current === attempt) picking.current = null;
    }
  }, [capture, pick, t]);
  const openAttachmentMenu = useCallback(() => {
    if (picking.current) return;
    Keyboard.dismiss();
    setMenuOpen(true);
  }, []);
  const confirmAlbum = useCallback(
    (images: AlbumImage[]) => {
      albumRequestRef.current = null;
      setAlbum(null);
      void pick(existing => prepareAlbumImages(existing, images));
    },
    [pick],
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
      onCancel={() => { albumRequestRef.current = null; picking.current = null; setAlbum(null); }}
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
