import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Platform } from "react-native";
import * as Network from "expo-network";
import * as Sharing from "expo-sharing";
import { openFile as openAndroidFile } from "future-file-handler";
import { File } from "expo-file-system";
import * as LegacyFileSystem from "expo-file-system/legacy";
import type { TFunction } from "i18next";
import { useRemote } from "../../remote/RemoteContext";
import { basename } from "../../remote/localPath";
import { mobileFileType, mobilePreviewRoute } from "../../remote/fileTypes";
import { supportedExternalMime } from "../../remote/fileHandler";
import {
  MAX_FILE_BYTES,
  mimeFor,
  namedExternalFile,
  TransferCancelledError,
} from "../../remote/files";
import type { DownloadInfo, HistoryAttachment } from "../../remote/types";
import {
  confirmDownload,
  deferPresentation,
  formatBytes,
  MARKDOWN_RENDER_BYTES,
  plainText,
  showToast,
  type ActiveDownload,
  type DownloadHandle,
  type FileAction,
} from "./utils";

type Remote = ReturnType<typeof useRemote>;

export interface PreviewState {
  attachment: HistoryAttachment;
  info: DownloadInfo;
  uri: string;
  markdown?: string;
  text?: string;
  truncated?: boolean;
}

export interface FileDownloadApi {
  activeDownload: ActiveDownload | null;
  activeDownloadFraction: number;
  preview: PreviewState | null;
  fileAction: FileAction | null;
  setFileAction: (action: FileAction | null) => void;
  openAttachment: (attachment: HistoryAttachment) => Promise<void>;
  openFileLink: (path: string) => Promise<void>;
  downloadOriginal: (attachment: HistoryAttachment) => Promise<void>;
  openOrShare: (
    info: DownloadInfo,
    cachedFile: File | null,
    save: boolean,
    existingHandle?: DownloadHandle,
    openMimeType?: string,
  ) => Promise<void>;
  closePreview: () => void;
  dismissPreviewThen: (action: () => void) => void;
  cancelActiveDownload: () => void;
  flushPendingPreviewAction: () => void;
  flushPendingDownloadModal: () => void;
  onDownloadModalShow: () => void;
}

export function useFileDownload(
  remote: Remote,
  t: TFunction,
  setTransferProgress: (value: number | null) => void,
): FileDownloadApi {
  const [activeDownload, setActiveDownload] = useState<ActiveDownload | null>(null);
  const activeDownloadRef = useRef<DownloadHandle | null>(null);
  const downloadModalPresentedRef = useRef(false);
  // UIKit cannot reliably present a second React Native Modal while the
  // download-progress Modal is still dismissing. Keep the next presentation
  // out of render state until `onDismiss` confirms that first Modal is gone.
  const pendingDownloadModalRef = useRef<(() => void) | null>(null);
  const pendingDownloadHandleRef = useRef<DownloadHandle | null>(null);
  const pendingPreviewActionRef = useRef<(() => void) | null>(null);
  const [preview, setPreview] = useState<PreviewState | null>(null);
  // Prepared non-previewable attachment awaiting an Android open/save choice;
  // iOS immediately continues to its system share sheet.
  const [fileAction, setFileAction] = useState<FileAction | null>(null);

  const activeDownloadFraction = activeDownload?.totalBytes
    ? Math.min(1, activeDownload.completedBytes / activeDownload.totalBytes)
    : 0;

  const beginDownload = useCallback(
    (key: string, fileName: string, totalBytes = 0, visible = true): DownloadHandle | null => {
      if (activeDownloadRef.current) return null;
      const handle = {
        id: `${key}:${Date.now().toString(36)}`,
        fileName,
        visible,
        controller: new AbortController(),
        handoffPending: false,
      };
      activeDownloadRef.current = handle;
      if (visible) {
        setActiveDownload({
          id: handle.id,
          fileName,
          phase: "preparing",
          completedBytes: 0,
          totalBytes,
        });
      }
      return handle;
    },
    [],
  );

  const showDownload = useCallback(
    (handle: DownloadHandle, patch: Partial<Omit<ActiveDownload, "id" | "fileName">> = {}) => {
      if (activeDownloadRef.current?.id !== handle.id) return;
      handle.visible = true;
      setActiveDownload({
        id: handle.id,
        fileName: handle.fileName,
        phase: "preparing",
        completedBytes: 0,
        totalBytes: 0,
        ...patch,
      });
    },
    [],
  );

  const updateDownload = useCallback(
    (handle: DownloadHandle, patch: Partial<Omit<ActiveDownload, "id" | "fileName">>) => {
      if (activeDownloadRef.current?.id !== handle.id || !handle.visible) return;
      setActiveDownload(current =>
        current?.id === handle.id ? { ...current, ...patch } : current,
      );
    },
    [],
  );

  const finishDownload = useCallback((handle: DownloadHandle, force = false) => {
    // An iOS handoff requested before Modal.onShow must keep the progress
    // Modal mounted until UIKit confirms presentation. onShow then forces the
    // state clear, and only onDismiss is allowed to present the next surface.
    if (handle.handoffPending && !force) return;
    if (activeDownloadRef.current?.id !== handle.id) return;
    activeDownloadRef.current = null;
    setActiveDownload(null);
  }, []);

  const flushPendingDownloadModal = useCallback(() => {
    downloadModalPresentedRef.current = false;
    const handle = pendingDownloadHandleRef.current;
    if (handle) {
      handle.handoffPending = false;
    }
    pendingDownloadHandleRef.current = null;
    const present = pendingDownloadModalRef.current;
    pendingDownloadModalRef.current = null;
    present?.();
  }, []);

  const handoffDownloadModal = useCallback(
    (handle: DownloadHandle, present: () => void) => {
      if (!handle.visible) {
        finishDownload(handle);
        present();
        return;
      }
      pendingDownloadModalRef.current = present;
      pendingDownloadHandleRef.current = handle;
      // `onDismiss` is iOS-only. Android continues after the state commit.
      // On iOS, if the work finishes before onShow, keep the Modal visible
      // until onShow and then dismiss it. This avoids the invalid state where
      // UIKit is still presenting the progress controller while a timer tries
      // to present the preview/share controller on top of it.
      if (Platform.OS !== "ios") {
        finishDownload(handle, true);
        deferPresentation(flushPendingDownloadModal);
      } else {
        handle.handoffPending = true;
        if (downloadModalPresentedRef.current) finishDownload(handle, true);
      }
    },
    [finishDownload, flushPendingDownloadModal],
  );

  const handoffDownloadAlert = useCallback(
    (handle: DownloadHandle, message: string) => {
      handoffDownloadModal(handle, () => Alert.alert(t("attachment.title"), message));
    },
    [handoffDownloadModal, t],
  );

  const cancelActiveDownload = useCallback(() => {
    const handle = activeDownloadRef.current;
    if (!handle) return;
    pendingDownloadModalRef.current = null;
    pendingDownloadHandleRef.current = null;
    handle.handoffPending = false;
    handle.controller.abort();
    // Local cancellation must be immediate even while an underlying NATS
    // request is waiting for its transport timeout. Late callbacks are scoped
    // to this handle and are ignored once it has been released.
    finishDownload(handle, true);
  }, [finishDownload]);

  const closePreview = useCallback(() => {
    pendingPreviewActionRef.current = null;
    setPreview(null);
  }, []);

  const dismissPreviewThen = useCallback(
    (action: () => void) => {
      if (!preview) {
        action();
        return;
      }
      pendingPreviewActionRef.current = action;
      setPreview(null);
      if (Platform.OS !== "ios") {
        deferPresentation(() => {
          if (pendingPreviewActionRef.current !== action) return;
          pendingPreviewActionRef.current = null;
          action();
        });
      }
    },
    [preview],
  );

  const flushPendingPreviewAction = useCallback(() => {
    if (Platform.OS !== "ios") return;
    const action = pendingPreviewActionRef.current;
    pendingPreviewActionRef.current = null;
    action?.();
  }, []);

  const openAttachment = useCallback(
    async (attachment: HistoryAttachment) => {
      let handle: DownloadHandle | null = null;
      try {
        const fileType = mobileFileType(attachment.name);
        if (!fileType) {
          Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
          return;
        }
        const openMimeType =
          fileType.route === "external" ? await supportedExternalMime(attachment.name) : null;
        if (fileType.route === "external" && !openMimeType) {
          Alert.alert(t("attachment.title"), t("attachment.noHandler"));
          return;
        }
        // The just-sent optimistic bubble still points at this phone's local
        // picker URI. Open it directly; durable history later replaces this
        // with the desktop path used by the NATS download flow.
        if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
          const local = new File(attachment.path);
          if (fileType.route === "image" && !attachment.mobilePreviewUnsupported) {
            setPreview({
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
              openMimeType: openMimeType!,
            });
          } else {
            const bytes = await local.bytes();
            const text = plainText(bytes);
            if (text === null) {
              Alert.alert(t("attachment.title"), t("attachment.previewOnDesktop"));
              return;
            }
            const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
            const previewText =
              visible.byteLength === bytes.byteLength ? text : new TextDecoder().decode(visible);
            const markdown = fileType.route === "markdown";
            const richJson = mobilePreviewRoute(attachment.name, local.size) === "json";
            setPreview({
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
              truncated: bytes.byteLength > visible.byteLength,
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
        let cachedPreview = remote.cachedAttachment(attachment, variant);
        const info =
          cachedPreview?.info ??
          (await remote.prepareAttachment(
            attachment,
            variant,
            handle.controller.signal,
            () => handle && updateDownload(handle, { phase: "waiting_network" }),
          ));
        // prepareAttachment records the returned content identity before it
        // resolves. Re-read the index so a persistent disk-cache hit is used
        // immediately instead of showing a progress Modal and verifying the
        // same file a second time in downloadAttachment.
        cachedPreview ??= remote.cachedAttachment(attachment, variant);
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        if (info.previewKind === "file") {
          if (!openMimeType) {
            handoffDownloadAlert(handle, t("attachment.noHandler"));
            return;
          }
          handoffDownloadModal(handle, () =>
            setFileAction({ info, cachedFile: cachedPreview?.file ?? null, openMimeType }),
          );
          return;
        }
        let file = cachedPreview?.file ?? null;
        if (!file) {
          const network = await Network.getNetworkStateAsync();
          if (
            network.type === Network.NetworkStateType.CELLULAR ||
            network.type === Network.NetworkStateType.UNKNOWN
          ) {
            const accepted = await confirmDownload(
              t("attachment.downloadTitle"),
              t("attachment.cellularWarning", { size: formatBytes(info.size) }),
              t("chat.cancel"),
              t("attachment.download"),
            );
            if (!accepted) return;
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
            setPreview({ attachment, info, uri: file.uri });
          });
        } else {
          const bytes = await file.bytes();
          const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
          const previewText = new TextDecoder().decode(visible);
          handoffDownloadModal(handle, () =>
            setPreview({
              attachment,
              info,
              uri: file.uri,
              ...(info.previewKind === "markdown"
                ? { markdown: previewText }
                : { text: previewText }),
              truncated: bytes.byteLength > visible.byteLength,
            }),
          );
        }
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        const detail = error instanceof Error ? error.message : "";
        const message =
          detail.includes("view it on desktop") || detail.includes("GIF preview")
            ? t("attachment.previewOnDesktop")
            : t("attachment.downloadFailed");
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
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      remote,
      showDownload,
      t,
      updateDownload,
    ],
  );

  // Download `info` to a cached File, prompting on cellular. Returns the file,
  // or null when the user declines the cellular download.
  const fetchDownload = useCallback(
    async (
      info: DownloadInfo,
      cachedFile: File | null,
      handle?: DownloadHandle,
    ): Promise<File | null> => {
      if (cachedFile) return cachedFile;
      const network = await Network.getNetworkStateAsync();
      if (
        network.type === Network.NetworkStateType.CELLULAR ||
        network.type === Network.NetworkStateType.UNKNOWN
      ) {
        const accepted = await confirmDownload(
          t("attachment.downloadTitle"),
          t("attachment.cellularWarning", { size: formatBytes(info.size) }),
          t("chat.cancel"),
          t("attachment.download"),
        );
        if (!accepted) return null;
      }
      if (handle && !cachedFile) {
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
    [remote, setTransferProgress, showDownload, t, updateDownload],
  );

  // Non-previewable file: download then hand it to the OS share sheet, which is
  // the cross-platform "open with external app / save to files" surface.
  const openOrShare = useCallback(
    async (
      info: DownloadInfo,
      cachedFile: File | null,
      save: boolean,
      existingHandle?: DownloadHandle,
      openMimeType = info.mimeType,
    ) => {
      const handle =
        existingHandle ?? beginDownload(info.transferId || info.name, info.name, info.size, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      try {
        const file = await fetchDownload(info, cachedFile, handle);
        if (!file) return;
        if (save && Platform.OS === "android") {
          updateDownload(handle, { phase: "saving" });
          const permission =
            await LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync();
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
        if (!save && Platform.OS === "android") {
          updateDownload(handle, { phase: "opening" });
          const namedFile = await namedExternalFile(file, info.name);
          await openAndroidFile(namedFile.uri, openMimeType);
          return;
        }
        if (!(await Sharing.isAvailableAsync())) {
          handoffDownloadAlert(handle, t("attachment.shareUnavailable"));
          return;
        }
        updateDownload(handle, { phase: save ? "saving" : "opening" });
        const namedFile = await namedExternalFile(file, info.name);
        if (Platform.OS === "ios") {
          // UIActivityViewController cannot be presented reliably while the
          // React Native download Modal is still on screen. Dismiss it first
          // and wait for Modal.onDismiss before handing the file to UIKit.
          // Once handed off, the system share sheet owns cancellation.
          handoffDownloadModal(handle, () => {
            void Sharing.shareAsync(namedFile.uri, {
              mimeType: save ? info.mimeType : openMimeType,
              dialogTitle: save ? t("attachment.save") : t("attachment.open"),
            }).catch(() => Alert.alert(t("attachment.title"), t("attachment.downloadFailed")));
          });
          return;
        }
        await Sharing.shareAsync(namedFile.uri, {
          mimeType: save ? info.mimeType : openMimeType,
          dialogTitle: save ? t("attachment.save") : t("attachment.open"),
        });
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        handoffDownloadAlert(handle, t("attachment.downloadFailed"));
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
    async (attachment: HistoryAttachment) => {
      const handle = beginDownload(attachment.path, attachment.name, 0, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
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
          true,
          handle,
        );
        return;
      }
      try {
        let cached = remote.cachedAttachment(attachment, "original");
        const info =
          cached?.info ??
          (await remote.prepareAttachment(attachment, "original", handle.controller.signal, () =>
            updateDownload(handle, { phase: "waiting_network" }),
          ));
        cached ??= remote.cachedAttachment(attachment, "original");
        await openOrShare(info, cached?.file ?? null, true, handle);
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        handoffDownloadAlert(handle, t("attachment.downloadFailed"));
      }
    },
    [beginDownload, handoffDownloadAlert, openOrShare, remote, t, updateDownload],
  );

  // A local-file markdown link/image target: prepare, then dispatch by size and
  // preview kind. Over 10 MB → desktop; image/markdown/text/JSON → in-app preview;
  // anything else → open/save action sheet.
  const openFileLink = useCallback(
    async (path: string) => {
      const attachment: HistoryAttachment = { path, name: basename(path) };
      const fileType = mobileFileType(attachment.name);
      if (!fileType) {
        Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
        return;
      }
      const openMimeType =
        fileType.route === "external" ? await supportedExternalMime(attachment.name) : null;
      if (fileType.route === "external" && !openMimeType) {
        Alert.alert(t("attachment.title"), t("attachment.noHandler"));
        return;
      }
      const handle = beginDownload(path, attachment.name, 0, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      try {
        const variant = fileType.route === "external" ? "original" : "preview";
        let cachedPreview = remote.cachedAttachment(attachment, variant);
        const info =
          cachedPreview?.info ??
          (await remote.prepareAttachment(attachment, variant, handle.controller.signal, () =>
            updateDownload(handle, { phase: "waiting_network" }),
          ));
        cachedPreview ??= remote.cachedAttachment(attachment, variant);
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        const previewable =
          info.previewKind === "image" ||
          info.previewKind === "markdown" ||
          info.previewKind === "text" ||
          info.previewKind === "json";
        if (!previewable) {
          if (!openMimeType) {
            handoffDownloadAlert(handle, t("attachment.noHandler"));
            return;
          }
          handoffDownloadModal(handle, () =>
            setFileAction({ info, cachedFile: cachedPreview?.file ?? null, openMimeType }),
          );
          return;
        }
        const file = await fetchDownload(info, cachedPreview?.file ?? null, handle);
        if (!file) return;
        if (info.previewKind === "image") {
          handoffDownloadModal(handle, () => setPreview({ attachment, info, uri: file.uri }));
        } else {
          const bytes = await file.bytes();
          const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
          const previewText = new TextDecoder().decode(visible);
          handoffDownloadModal(handle, () =>
            setPreview({
              attachment,
              info,
              uri: file.uri,
              ...(info.previewKind === "markdown"
                ? { markdown: previewText }
                : { text: previewText }),
              truncated: bytes.byteLength > visible.byteLength,
            }),
          );
        }
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        const detail = error instanceof Error ? error.message : "";
        const message =
          detail.includes("view it on desktop") || detail.includes("GIF preview")
            ? t("attachment.previewOnDesktop")
            : t("attachment.downloadFailed");
        handoffDownloadAlert(handle, message);
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
      remote,
      t,
      updateDownload,
    ],
  );

  const onDownloadModalShow = useCallback(() => {
    downloadModalPresentedRef.current = true;
    const handle = pendingDownloadHandleRef.current ?? activeDownloadRef.current;
    if (handle) {
      if (handle.handoffPending) finishDownload(handle, true);
    }
  }, [finishDownload]);

  useEffect(
    () => () => {
      activeDownloadRef.current?.controller.abort();
      pendingDownloadModalRef.current = null;
      pendingDownloadHandleRef.current = null;
      pendingPreviewActionRef.current = null;
    },
    [],
  );

  return {
    activeDownload,
    activeDownloadFraction,
    preview,
    fileAction,
    setFileAction,
    openAttachment,
    openFileLink,
    downloadOriginal,
    openOrShare,
    closePreview,
    dismissPreviewThen,
    cancelActiveDownload,
    flushPendingPreviewAction,
    flushPendingDownloadModal,
    onDownloadModalShow,
  };
}
