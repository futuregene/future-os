import { useCallback, useEffect, useRef, useState } from "react";
import { Platform } from "react-native";
import { AppAlert as Alert } from "../../../components/appAlerts";
import type { TFunction } from "i18next";
import { confirmDownload, deferPresentation, PREPARE_REVEAL_DELAY_MS, type ActiveDownload, type DownloadHandle } from "../utils";

export function useTransferOperation(t: TFunction) {
  const [activeDownload, setActiveDownload] = useState<ActiveDownload | null>(null);
  const activeDownloadRef = useRef<DownloadHandle | null>(null);
  const downloadModalPresentedRef = useRef(false);
  // UIKit cannot reliably present a second React Native Modal while the
  // download-progress Modal is still dismissing. Keep the next presentation
  // out of render state until `onDismiss` confirms that first Modal is gone.
  const pendingDownloadModalRef = useRef<(() => void) | null>(null);
  const pendingDownloadHandleRef = useRef<DownloadHandle | null>(null);
  const activeDownloadFraction = activeDownload?.totalBytes
    ? Math.min(1, activeDownload.completedBytes / activeDownload.totalBytes)
    : 0;

  const showDownload = useCallback(
    (handle: DownloadHandle, patch: Partial<Omit<ActiveDownload, "id" | "fileName">> = {}) => {
      if (activeDownloadRef.current?.id !== handle.id) return;
      handle.visible = true;
      if (handle.revealTimer !== null) {
        clearTimeout(handle.revealTimer);
        handle.revealTimer = null;
      }
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

  const beginDownload = useCallback(
    (key: string, fileName: string, totalBytes = 0, visible = true): DownloadHandle | null => {
      if (activeDownloadRef.current) return null;
      const handle: DownloadHandle = {
        id: `${key}:${Date.now().toString(36)}`,
        fileName,
        visible,
        controller: new AbortController(),
        handoffPending: false,
        revealTimer: null,
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
      else {
        // A transfer stays invisible until its size is known, so a cached file
        // never flashes a dialog. Preparing asks the desktop to prepare and
        // hash the file first, though, and that round trip is otherwise a dead
        // screen: an unreachable or busy desktop leaves the user staring at
        // nothing until the 10s/20s retry ladder gives up. Reveal the dialog —
        // it already renders "preparing" and "waiting for network" — once the
        // wait stops looking instant.
        handle.revealTimer = setTimeout(() => {
          if (activeDownloadRef.current?.id !== handle.id || handle.visible) return;
          showDownload(handle);
        }, PREPARE_REVEAL_DELAY_MS);
      }
      return handle;
    },
    [showDownload],
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
    if (handle.revealTimer !== null) {
      clearTimeout(handle.revealTimer);
      handle.revealTimer = null;
    }
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
    if (!handle?.controller.signal.aborted) present?.();
  }, []);

  // The confirmation dialog and the progress Modal cannot share the screen: on
  // Android the Modal is its own window that covers the dialog, and a reveal
  // that fires while the question is up does the same on both platforms. Take
  // the progress surface off screen and resolve once it is really gone, without
  // releasing the transfer — the handle stays the active one, so the dialog
  // comes back and the transfer resumes when the user accepts.
  const hideDownload = useCallback(
    (handle: DownloadHandle): Promise<void> => {
      if (activeDownloadRef.current?.id !== handle.id) return Promise.resolve();
      const onScreen = handle.visible;
      if (handle.revealTimer !== null) {
        clearTimeout(handle.revealTimer);
        handle.revealTimer = null;
      }
      handle.visible = false;
      setActiveDownload(null);
      if (!onScreen) return Promise.resolve();
      // `onDismiss` is iOS-only. Android continues after the state commit.
      return new Promise<void>(resolve => {
        pendingDownloadModalRef.current = () => resolve();
        pendingDownloadHandleRef.current = handle;
        if (Platform.OS !== "ios") deferPresentation(flushPendingDownloadModal);
      });
    },
    [flushPendingDownloadModal],
  );

  /** `confirmDownload`, with the progress dialog out of the way for the answer. */
  const confirmDownloadFor = useCallback(
    async (
      handle: DownloadHandle | undefined,
      title: string,
      message: string,
      cancel: string,
      accept: string,
    ) => {
      if (handle) await hideDownload(handle);
      return confirmDownload(title, message, cancel, accept);
    },
    [hideDownload],
  );

  const handoffDownloadModal = useCallback(
    (handle: DownloadHandle, present: () => void) => {
      if (handle.controller.signal.aborted || activeDownloadRef.current !== handle) return;
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
    const handle = activeDownloadRef.current ?? pendingDownloadHandleRef.current;
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
      pendingDownloadHandleRef.current?.controller.abort();
      activeDownloadRef.current = null;
      pendingDownloadModalRef.current = null;
      pendingDownloadHandleRef.current = null;
    },
    [],
  );

  return { activeDownload, activeDownloadRef, activeDownloadFraction, showDownload, beginDownload, updateDownload, finishDownload, flushPendingDownloadModal, confirmDownloadFor, handoffDownloadModal, handoffDownloadAlert, cancelActiveDownload, onDownloadModalShow };
}
