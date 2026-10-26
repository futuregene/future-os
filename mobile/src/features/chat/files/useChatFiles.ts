import { useCallback, useState } from "react";
import { Platform } from "react-native";
import { AppAlert as Alert } from "../../../components/appAlerts";
import { downloadWarning } from "../downloadPolicy";
import { readPreviewText } from "../readPreviewText";
import * as Sharing from "expo-sharing";
import { openFile as openNativeFile, saveFile, shareFile, supportsNativeFileActions } from "future-file-handler";
import { File } from "expo-file-system";
import * as LegacyFileSystem from "expo-file-system/legacy";
import type { TFunction } from "i18next";
import { useRemote } from "../../../remote/RemoteContext";
import { basename } from "../../../remote/localPath";
import { mobileFileType, mobilePreviewRoute } from "../../../remote/fileTypes";
import { supportedExternalMime } from "../../../remote/fileHandler";
import { withNativeHandoff, withNativePresentation } from "../../../remote/nativePresentation";
import { MAX_FILE_BYTES, mimeFor, namedExternalFile, TransferCancelledError } from "../../../remote/files";
import type { DownloadInfo, HistoryAttachment } from "../../../remote/types";
import { formatBytes, showToast, type ActiveDownload, type DownloadHandle, type FileAction, type FileOperation } from "../utils";
import { useTransferOperation } from "./useTransferOperation";
import { usePreviewStack } from "./usePreviewStack";

type Remote = ReturnType<typeof useRemote>;

/**
 * The alert a refused transfer maps to, read off the desktop's own message.
 *
 * The host names the limit that blocked the file, and the phone answers in its
 * own wording for the limits it knows. A size refusal has to be recognised
 * separately: it used to match only "view it on desktop", so a file that was
 * merely too large looked like a broken transfer.
 */
function refusalMessage(detail: string, t: TFunction): string {
  if (detail.includes("download limit"))
    return t("attachment.tooLarge");
  if (detail.includes("view it on desktop") || detail.includes("GIF preview"))
    return t("attachment.previewOnDesktop");
  return t("attachment.downloadFailed");
}

export interface PreviewState {
  attachment: HistoryAttachment;
  info: DownloadInfo;
  uri: string;
  markdown?: string;
  text?: string;
  truncated?: boolean;
}

export interface ChatFilesApi {
  activeDownload: ActiveDownload | null;
  activeDownloadFraction: number;
  /** Every open preview, oldest first. The last one is on screen. */
  previews: PreviewState[];
  /** The preview on screen, or null when the preview surface is closed. */
  preview: PreviewState | null;
  fileAction: FileAction | null;
  setFileAction: (action: FileAction | null) => void;
  openAttachment: (attachment: HistoryAttachment) => Promise<void>;
  openFileLink: (path: string, refresh?: boolean) => Promise<void>;
  /** A local-file link inside a previewed document: keeps it, opens the target. */
  openLinkedFile: (path: string) => Promise<void>;
  downloadOriginal: (attachment: HistoryAttachment, operation?: FileOperation) => Promise<void>;
  openOrShare: (
    info: DownloadInfo,
    cachedFile: File | null,
    operation: FileOperation,
    existingHandle?: DownloadHandle,
  ) => Promise<void>;
  /** Leave the whole preview surface, back to the conversation. */
  closePreview: () => void;
  /** Leave the document on top, back to the one that linked to it. */
  popPreview: () => void;
  dismissPreviewThen: (action: () => void) => void;
  cancelActiveDownload: () => void;
  flushPendingPreviewAction: () => void;
  flushPendingDownloadModal: () => void;
  onDownloadModalShow: () => void;
}

export function useChatFiles(remote: Remote, t: TFunction, setTransferProgress: (value: number | null) => void): ChatFilesApi {
  const { activeDownload, activeDownloadFraction, showDownload, beginDownload, updateDownload, finishDownload, flushPendingDownloadModal, confirmDownloadFor, handoffDownloadModal, handoffDownloadAlert, cancelActiveDownload, onDownloadModalShow } = useTransferOperation(t);
  const { previews, preview, showPreview, closePreview, popPreview, dismissPreviewThen, flushPendingPreviewAction } = usePreviewStack();
  const [fileAction, setFileAction] = useState<FileAction | null>(null);
  const openAttachment = useCallback(
    async (attachment: HistoryAttachment) => {
      let handle: DownloadHandle | null = null;
      try {
        const fileType = mobileFileType(attachment.name);
        if (!fileType) {
          Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
          return;
        }
        // The just-sent optimistic bubble still points at this phone's local
        // picker URI. Open it directly; durable history later replaces this
        // with the desktop path used by the NATS download flow.
        if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
          const local = new File(attachment.path);
          if (fileType.route === "image" && !attachment.mobilePreviewUnsupported) {
            showPreview({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: "image/*",
                size: local.size,
                contentHash: "",
                previewKind: "image",
                variant: "preview",
                chunkBytes: 0,
              },
              attachment,
              uri: local.uri,
            });
          } else if (fileType.route === "external") {
            setFileAction({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: fileType.mimeType,
                size: local.size,
                contentHash: "",
                previewKind: "file",
                variant: "original",
                chunkBytes: 0,
              },
              cachedFile: local,
            });
          } else {
            const content = await readPreviewText(local);
            if (!content) {
              Alert.alert(t("attachment.title"), t("attachment.previewOnDesktop"));
              return;
            }
            const { text: previewText, truncated } = content;
            const markdown = fileType.route === "markdown";
            const richJson = mobilePreviewRoute(attachment.name, local.size) === "json";
            showPreview({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: fileType.mimeType,
                size: local.size,
                contentHash: "",
                previewKind: richJson ? "json" : markdown ? "markdown" : "text",
                variant: "preview",
                chunkBytes: 0,
              },
              attachment,
              uri: local.uri,
              ...(markdown ? { markdown: previewText } : { text: previewText }),
              truncated,
            });
          }
          return;
        }
        handle = beginDownload(attachment.path, attachment.name, 0, false);
        if (!handle) {
          showToast(t("attachment.downloadInProgress"));
          return;
        }
        const variant = fileType.route === "external" ? "original" : "preview";
        const info = await remote.prepareAttachment(
          attachment,
          variant,
          handle.controller.signal,
          () => handle && updateDownload(handle, { phase: "waiting_network" }),
        );
        // prepareAttachment records the returned content identity before it
        // resolves. Re-read the index so a persistent disk-cache hit is used
        // immediately instead of showing a progress Modal and verifying the
        // same file a second time in downloadAttachment.
        const cachedPreview = remote.cachedAttachment(attachment, variant);
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        if (info.previewKind === "file") {
          handoffDownloadModal(handle, () =>
            setFileAction({ info, cachedFile: cachedPreview?.file ?? null }),
          );
          return;
        }
        let file = cachedPreview?.file ?? null;
        if (!file) {
          const warning = await downloadWarning(info.size);
          if (handle.controller.signal.aborted) throw new TransferCancelledError();
          if (warning) {
            const accepted = await confirmDownloadFor(
              handle,
              t("attachment.downloadTitle"),
              t(warning, { size: formatBytes(info.size) }),
              t("chat.cancel"),
              t("attachment.download"),
            );
            if (!accepted) return;
            if (handle.controller.signal.aborted) throw new TransferCancelledError();
          }
          // Metadata is intentionally silent. Once we know this is a cache
          // miss and have the real byte size, show 0 / total before the first
          // (possibly only) chunk arrives.
          showDownload(handle, {
            phase: "downloading",
            completedBytes: 0,
            totalBytes: info.size,
          });
        }
        if (!file) {
          file = await remote.downloadAttachment(
            info,
            (done, total) => {
              if (!handle) return;
              const patch = {
                phase: done >= total ? "verifying" : "downloading",
                completedBytes: done,
                totalBytes: total,
              } as const;
              if (handle.visible) updateDownload(handle, patch);
              else showDownload(handle, patch);
            },
            handle.controller.signal,
            () => {
              if (!handle) return;
              const patch = { phase: "waiting_network" } as const;
              if (handle.visible) updateDownload(handle, patch);
              else showDownload(handle, patch);
            },
          );
        }
        if (info.previewKind === "image") {
          handoffDownloadModal(handle, () => {
            showPreview({ attachment, info, uri: file.uri });
          });
        } else {
          const content = await readPreviewText(file, handle.controller.signal);
          if (!content) throw new Error("invalid_text_preview");
          const { text: previewText, truncated } = content;
          handoffDownloadModal(handle, () =>
            showPreview({
              attachment,
              info,
              uri: file.uri,
              ...(info.previewKind === "markdown"
                ? { markdown: previewText }
                : { text: previewText }),
              truncated,
            }),
          );
        }
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        const detail = error instanceof Error ? error.message : "";
        const message = refusalMessage(detail, t);
        if (handle) {
          handoffDownloadAlert(handle, message);
        } else {
          Alert.alert(t("attachment.title"), message);
        }
      } finally {
        if (handle) finishDownload(handle);
      }
    },
    [
      beginDownload,
      confirmDownloadFor,
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      remote,
      showDownload,
      showPreview,
      t,
      updateDownload,
    ],
  );

  // Download `info` to a cached File, confirming large metered/unknown-network
  // transfers. Returns null when the user declines.
  const fetchDownload = useCallback(
    async (
      info: DownloadInfo,
      cachedFile: File | null,
      handle?: DownloadHandle,
    ): Promise<File | null> => {
      if (cachedFile) return cachedFile;
      const warning = await downloadWarning(info.size);
      if (handle?.controller.signal.aborted) throw new TransferCancelledError();
      if (warning) {
        const accepted = await confirmDownloadFor(
          handle,
          t("attachment.downloadTitle"),
          t(warning, { size: formatBytes(info.size) }),
          t("chat.cancel"),
          t("attachment.download"),
        );
        if (!accepted) return null;
        if (handle?.controller.signal.aborted) throw new TransferCancelledError();
      }
      if (handle) {
        const patch = {
          phase: "downloading",
          completedBytes: 0,
          totalBytes: info.size,
        } as const;
        if (handle.visible) updateDownload(handle, patch);
        else showDownload(handle, patch);
      }
      return remote.downloadAttachment(
        info,
        (done, total) => {
          if (handle) {
            const patch = {
              phase: done >= total ? "verifying" : "downloading",
              completedBytes: done,
              totalBytes: total,
            } as const;
            if (handle.visible) updateDownload(handle, patch);
            else showDownload(handle, patch);
          } else {
            setTransferProgress(total > 0 ? done / total : null);
          }
        },
        handle?.controller.signal,
        () => {
          if (handle) {
            const patch = { phase: "waiting_network" } as const;
            if (handle.visible) updateDownload(handle, patch);
            else showDownload(handle, patch);
          }
        },
      );
    },
    [confirmDownloadFor, remote, setTransferProgress, showDownload, t, updateDownload],
  );

  // Distinct native open/save/share operations on both platforms. Older iOS
  // native builds retain the share-sheet fallback. Saving needs no reader.
  const openOrShare = useCallback(
    async (
      info: DownloadInfo,
      cachedFile: File | null,
      operation: FileOperation,
      existingHandle?: DownloadHandle,
    ) => {
      const handle =
        existingHandle ?? beginDownload(info.transferId || info.name, info.name, info.size, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      let errorKey = "attachment.downloadFailed";
      try {
        if (handle.controller.signal.aborted) throw new TransferCancelledError();
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        let openMimeType = info.mimeType;
        if (operation === "open") {
          errorKey = "attachment.openFailed";
          const supported = await supportedExternalMime(info.name);
          if (handle.controller.signal.aborted) throw new TransferCancelledError();
          if (!supported) {
            handoffDownloadAlert(handle, t("attachment.noHandler"));
            return;
          }
          openMimeType = supported;
        }
        const nativeIosAction = Platform.OS === "ios" && operation !== "share" && supportsNativeFileActions();
        const usesShareSheet = Platform.OS !== "android" && (operation === "share" || !nativeIosAction);
        if (usesShareSheet && !(await Sharing.isAvailableAsync())) {
          handoffDownloadAlert(handle, t("attachment.shareUnavailable"));
          return;
        }
        if (handle.controller.signal.aborted) throw new TransferCancelledError();
        errorKey = "attachment.downloadFailed";
        const file = await fetchDownload(info, cachedFile, handle);
        if (!file) return;
        if (handle.controller.signal.aborted) throw new TransferCancelledError();
        errorKey = `attachment.${operation}Failed`;
        if (operation === "save" && Platform.OS === "android") {
          updateDownload(handle, { phase: "saving" });
          const permission = await withNativePresentation(() =>
            LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync(),
          );
          if (!permission.granted) return;
          if (handle.controller.signal.aborted) throw new TransferCancelledError();
          const destination = await LegacyFileSystem.StorageAccessFramework.createFileAsync(
            permission.directoryUri,
            info.name,
            info.mimeType,
          );
          const base64 = await LegacyFileSystem.readAsStringAsync(file.uri, {
            encoding: LegacyFileSystem.EncodingType.Base64,
          });
          if (handle.controller.signal.aborted) throw new TransferCancelledError();
          await LegacyFileSystem.StorageAccessFramework.writeAsStringAsync(destination, base64, {
            encoding: LegacyFileSystem.EncodingType.Base64,
          });
          showToast(t("attachment.downloaded", { name: info.name }));
          return;
        }
        updateDownload(handle, {
          phase: operation === "share" ? "sharing" : operation === "save" ? "saving" : "opening",
        });
        const namedFile = await namedExternalFile(file, info.name);
        if (handle.controller.signal.aborted) throw new TransferCancelledError();
        if (Platform.OS === "android") {
          if (operation === "open") {
            await withNativePresentation(() => openNativeFile(namedFile.uri, openMimeType));
          } else {
            await withNativeHandoff(() => shareFile(namedFile.uri, info.mimeType, t("attachment.share")));
          }
          return;
        }
        const present = () => withNativePresentation(() => {
          if (nativeIosAction) {
            return operation === "save"
              ? saveFile(namedFile.uri)
              : openNativeFile(namedFile.uri, openMimeType);
          }
          return Sharing.shareAsync(namedFile.uri, {
            mimeType: operation === "open" ? openMimeType : info.mimeType,
            dialogTitle: t(`attachment.${operation}`),
          });
        });
        if (Platform.OS === "ios") {
          // Wait for the progress Modal's onDismiss before presenting UIKit.
          // Once handed off, the system share sheet owns cancellation.
          handoffDownloadModal(handle, () => {
            void present().catch(() => {
              if (!handle.controller.signal.aborted) {
                Alert.alert(t("attachment.title"), t(errorKey));
              }
            });
          });
          return;
        }
        await present();
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        // Android compatibility runtimes can reject export, URI grants or the
        // system chooser independently. Preserve the native reason instead of
        // hiding every failure behind the same generic file-action message.
        const detail = Platform.OS === "android" && error instanceof Error ? error.message.trim() : "";
        handoffDownloadAlert(handle, detail ? `${t(errorKey)}\n\n${detail}` : t(errorKey));
      } finally {
        finishDownload(handle);
      }
    },
    [
      beginDownload,
      fetchDownload,
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      t,
      updateDownload,
    ],
  );

  const downloadOriginal = useCallback(
    async (attachment: HistoryAttachment, operation: FileOperation = "save") => {
      if (!mobileFileType(attachment.name)) {
        Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
        return;
      }
      const handle = beginDownload(attachment.path, attachment.name, 0, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      try {
        if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
          const local = new File(attachment.path);
          await openOrShare(
            {
              transferId: "local",
              name: attachment.name,
              mimeType: mimeFor(attachment.name),
              size: local.size,
              contentHash: "",
              previewKind: "file",
              variant: "original",
              chunkBytes: 0,
            },
            local,
            operation,
            handle,
          );
          return;
        }
        const info = await remote.prepareAttachment(
          attachment,
          "original",
          handle.controller.signal,
          () => updateDownload(handle, { phase: "waiting_network" }),
        );
        const cached = remote.cachedAttachment(attachment, "original");
        await openOrShare(info, cached?.file ?? null, operation, handle);
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        handoffDownloadAlert(handle, t("attachment.downloadFailed"));
      } finally {
        finishDownload(handle);
      }
    },
    [beginDownload, finishDownload, handoffDownloadAlert, openOrShare, remote, t, updateDownload],
  );

  // A local-file markdown link/image target: prepare, then dispatch by size and
  // preview kind. Over 10 MB → desktop; image/markdown/text/JSON → in-app preview;
  // anything else → open/save/share action sheet.
  //
  // `nested` marks a link followed from inside a previewed document: the
  // document that linked here is kept below, so the reader can go back to it.
  const openLocalPath = useCallback(
    async (path: string, nested: boolean) => {
      const attachment: HistoryAttachment = { path, name: basename(path) };
      const fileType = mobileFileType(attachment.name);
      if (!fileType) {
        Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
        return;
      }
      const handle = beginDownload(path, attachment.name, 0, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      try {
        const variant = fileType.route === "external" ? "original" : "preview";
        // Paths are mutable identities. Always ask Desktop for the current
        // content hash, then reuse the content-addressed bytes only when that
        // identity still matches.
        const info = await remote.prepareAttachment(
          attachment,
          variant,
          handle.controller.signal,
          () => updateDownload(handle, { phase: "waiting_network" }),
        );
        const cachedPreview = remote.cachedAttachment(attachment, variant);
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        const previewable =
          info.previewKind === "image" ||
          info.previewKind === "markdown" ||
          info.previewKind === "text" ||
          info.previewKind === "json";
        const showActions = () =>
          setFileAction({ info, cachedFile: cachedPreview?.file ?? null });
        if (!previewable) {
          // A file that needs an external reader takes over the screen the way
          // the preview's own menu actions do. The stack closes first: the
          // action sheet is a second Modal, and iOS will not present it while
          // the preview is still on screen.
          handoffDownloadModal(handle, nested ? () => dismissPreviewThen(showActions) : showActions);
          return;
        }
        const file = await fetchDownload(info, cachedPreview?.file ?? null, handle);
        if (!file) return;
        if (info.previewKind === "image") {
          handoffDownloadModal(handle, () => showPreview({ attachment, info, uri: file.uri }, nested));
        } else {
          const content = await readPreviewText(file, handle.controller.signal);
          if (!content) throw new Error("invalid_text_preview");
          const { text: previewText, truncated } = content;
          handoffDownloadModal(handle, () =>
            showPreview({
              attachment,
              info,
              uri: file.uri,
              ...(info.previewKind === "markdown"
                ? { markdown: previewText }
                : { text: previewText }),
              truncated,
            }, nested),
          );
        }
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        const detail = error instanceof Error ? error.message : "";
        const message = refusalMessage(detail, t);
        handoffDownloadAlert(handle, message);
      } finally {
        finishDownload(handle);
      }
    },
    [
      beginDownload,
      dismissPreviewThen,
      fetchDownload,
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      remote,
      showPreview,
      t,
      updateDownload,
    ],
  );

  const openFileLink = useCallback(
    (path: string, _refresh = true) => openLocalPath(path, false),
    [openLocalPath],
  );

  // A link inside a previewed document. The document stays open underneath, so
  // following a link is reversible without losing the reading position.
  const openLinkedFile = useCallback(
    (path: string) => openLocalPath(path, true),
    [openLocalPath],
  );

  return {
    activeDownload,
    activeDownloadFraction,
    previews,
    preview,
    fileAction,
    setFileAction,
    openAttachment,
    openFileLink,
    openLinkedFile,
    downloadOriginal,
    openOrShare,
    closePreview,
    popPreview,
    dismissPreviewThen,
    cancelActiveDownload,
    flushPendingPreviewAction,
    flushPendingDownloadModal,
    onDownloadModalShow,
  };
}
