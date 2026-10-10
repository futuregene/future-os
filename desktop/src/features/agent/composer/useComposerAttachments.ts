import type { MessageAttachment } from "@future-os/thread-projection";
import type { ComposerDragState } from "../Composer";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { deleteTempAttachment, readNativeClipboardFilePaths, savePastedFile, savePastedImage } from "../../../integrations/storage/threadStore";
import { formatBytes } from "../../../lib/format";
import { useCommittedRef } from "../../../lib/useCommittedRef";
import { classifyAttachment, fileNameFromPath, imageExtensionFromMime, MAX_IMAGES_PER_TURN, READ_SOURCE_MAX_BYTES } from "../attachments";

interface Options {
  disabled?: boolean;
  onDragStateChange?: (state: ComposerDragState) => void;
  captureOperation: () => () => boolean;
}

const MAX_COPIED_FILES_PER_PASTE = 10;
const MAX_COPIED_FILE_BYTES = 10 * 1024 * 1024;
const MAX_COPIED_FILES_TOTAL_BYTES = 20 * 1024 * 1024;

export function useComposerAttachments({ disabled, onDragStateChange, captureOperation }: Options) {
  const { t } = useTranslation("agent");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
  const attachmentsRef = useCommittedRef(attachments);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [dragState, setDragState] = useState<ComposerDragState>(null);
  const addAttachmentPaths = useCallback(async (paths: string[], temporary = false, names?: Map<string, string>, isCurrent = captureOperation()) => {
    const classified = await Promise.all(
      paths.map(async path => ({ path, result: await classifyAttachment(path) })),
    );
    if (!isCurrent()) {
      if (temporary)
        await Promise.all(paths.map(path => deleteTempAttachment(path).catch(() => {})));
      return;
    }
    // Compute next/rejected against the current attachments, then call both
    // setters — never call setAttachError inside a setAttachments updater
    // (updaters must be pure; StrictMode/concurrent React may run them twice).
    // Classification is asynchronous and several sources (picker, paste, drag)
    // may finish out of order. Merge into the live ref, not the render-time
    // closure, so a later completion cannot overwrite an earlier one.
    const next = [...attachmentsRef.current];
    const rejected: string[] = [];
    for (const { path, result } of classified) {
      const name = names?.get(path) ?? fileNameFromPath(path);
      if (next.some(attachment => attachment.path === path))
        continue;
      if (result.kind === null) {
        rejected.push(t("composer.attachRejectedReason", { name, reason: result.reason }));
        continue;
      }
      // Images carry a per-message count cap regardless of model (a text-only model
      // still receives the paths, but keeping the same ceiling avoids surprises
      // when switching models mid-draft). Every other file type is unlimited —
      // the agent reads local paths on demand with its own tools.
      if (result.kind === "image") {
        const imageCount = next.filter(attachment => attachment.kind === "image").length;
        if (imageCount >= MAX_IMAGES_PER_TURN) {
          rejected.push(t("composer.attachRejectedLimit", { name, count: MAX_IMAGES_PER_TURN }));
          continue;
        }
      }
      next.push({ kind: result.kind, name, path, ...(temporary ? { temporary: true } : {}) });
    }
    attachmentsRef.current = next;
    setAttachments(next);
    setAttachError(rejected.length > 0 ? t("composer.attachIgnored", { items: rejected.join("，") }) : null);
  }, [attachmentsRef, captureOperation, t]);

  async function attachImageFiles(files: File[], isCurrent: () => boolean) {
    // Save every file first, then attach in ONE addAttachmentPaths call:
    // calling it per file inside the loop reuses the same closure over the
    // pre-paste `attachments`, so each iteration's setAttachments overwrites
    // the previous one and only the last image survives.
    const saved: string[] = [];
    const rejected: string[] = [];
    for (const file of files) {
      if (!isCurrent())
        break;
      if (file.size > READ_SOURCE_MAX_BYTES) {
        rejected.push(t("composer.attachRejectedReason", {
          name: file.name,
          reason: t("attachment.imageTooLarge", { max: formatBytes(READ_SOURCE_MAX_BYTES) }),
        }));
        continue;
      }
      try {
        const buffer = await file.arrayBuffer();
        const result = await savePastedImage({
          bytes: Array.from(new Uint8Array(buffer)),
          extension: imageExtensionFromMime(file.type) ?? "png",
        });
        saved.push(result.path);
      }
      catch {
        rejected.push(t("composer.attachRejectedReason", { name: file.name, reason: t("attachment.readFailed") }));
      }
    }
    if (saved.length > 0) {
      await addAttachmentPaths(saved, true, undefined, isCurrent);
      const accepted = new Set(attachmentsRef.current.map(attachment => attachment.path));
      // Files written for this paste but rejected by classification/model limits
      // are no longer referenced by the draft and can be reclaimed immediately.
      await Promise.all(saved.filter(path => !accepted.has(path)).map(path => deleteTempAttachment(path).catch(() => {})));
    }
    if (isCurrent() && rejected.length > 0)
      setAttachError(t("composer.attachIgnored", { items: rejected.join("，") }));
  }

  async function attachPastedFiles(files: File[]) {
    if (disabled)
      return;
    const isCurrent = captureOperation();
    // Finder uses a native file-URL pasteboard type which WKWebView turns into
    // opaque File objects. Recover the original paths before copying bytes.
    const nativePaths = await readNativeClipboardFilePaths().catch(() => []);
    if (!isCurrent())
      return;
    if (nativePaths.length > 0) {
      await addAttachmentPaths(nativePaths, false, undefined, isCurrent);
      return;
    }
    const copiedFiles = files.filter(file => !file.type.startsWith("image/"));
    const imageFiles = files.filter(file => file.type.startsWith("image/"));
    if (imageFiles.length > 0)
      await attachImageFiles(imageFiles, isCurrent);
    if (!isCurrent())
      return;
    const candidates = copiedFiles.slice(0, MAX_COPIED_FILES_PER_PASTE);
    const total = candidates.reduce((sum, file) => sum + file.size, 0);
    const rejected: string[] = [];
    if (copiedFiles.length > MAX_COPIED_FILES_PER_PASTE)
      rejected.push(t("composer.attachCopiedCountLimit", { count: MAX_COPIED_FILES_PER_PASTE }));
    if (total > MAX_COPIED_FILES_TOTAL_BYTES) {
      setAttachError(t("composer.attachIgnored", { items: t("composer.attachCopiedTotalLimit", { max: formatBytes(MAX_COPIED_FILES_TOTAL_BYTES) }) }));
      return;
    }
    const saved: string[] = [];
    const names = new Map<string, string>();
    for (const file of candidates) {
      if (!isCurrent())
        break;
      if (file.size > MAX_COPIED_FILE_BYTES) {
        rejected.push(t("composer.attachRejectedReason", { name: file.name, reason: t("composer.attachCopiedFileLimit", { max: formatBytes(MAX_COPIED_FILE_BYTES) }) }));
        continue;
      }
      try {
        const buffer = await file.arrayBuffer();
        const result = await savePastedFile({ bytes: Array.from(new Uint8Array(buffer)), name: file.name });
        saved.push(result.path);
        names.set(result.path, file.name);
      }
      catch {
        rejected.push(t("composer.attachRejectedReason", { name: file.name, reason: t("attachment.readFailed") }));
      }
    }
    if (saved.length > 0) {
      await addAttachmentPaths(saved, true, names, isCurrent);
      const accepted = new Set(attachmentsRef.current.map(attachment => attachment.path));
      await Promise.all(saved.filter(path => !accepted.has(path)).map(path => deleteTempAttachment(path).catch(() => {})));
    }
    if (isCurrent() && rejected.length > 0)
      setAttachError(t("composer.attachIgnored", { items: rejected.join("，") }));
  }

  async function handleAttachFiles() {
    if (disabled)
      return;
    const isCurrent = captureOperation();

    // Any file type is acceptable; classification handles unsupported paths.
    const selected = await open({
      multiple: true,
      title: t("composer.attachDialogTitle"),
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    if (!isCurrent() || paths.length === 0)
      return;

    await addAttachmentPaths(paths);
  }

  function removeAttachment(path: string) {
    const removed = attachmentsRef.current.find(attachment => attachment.path === path);
    const next = attachmentsRef.current.filter(attachment => attachment.path !== path);
    attachmentsRef.current = next;
    setAttachments(next);
    if (removed?.temporary)
      void deleteTempAttachment(path).catch(() => {});
  }

  // Keep the native drag subscription stable while translations change.
  const addAttachmentPathsRef = useCommittedRef(addAttachmentPaths);

  // Update the verdict and (when the parent opts in) report it, so the parent
  // can draw the drag highlight around a larger card. Ref-backed for the same
  // reason as above: the drag listener must not re-subscribe when the callback
  // identity changes. `dragStateRef` lets `over` read the current verdict
  // without a stale closure.
  const dragStateRef = useRef<ComposerDragState>(null);
  const onDragStateChangeRef = useCommittedRef(onDragStateChange);
  const setDrag = useCallback((next: ComposerDragState) => {
    dragStateRef.current = next;
    setDragState(next);
    onDragStateChangeRef.current?.(next);
  }, [onDragStateChangeRef]);

  useEffect(() => {
    if (disabled)
      return;

    let active = true;
    let dispose: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter") {
          // Any file is acceptable at drag time (the agent reads paths with its
          // own tools; images degrade to a path for text-only models). Directory
          // drops are caught by classification on release.
          setDrag(event.payload.paths.length > 0 ? "accept" : "reject");
        }
        else if (event.payload.type === "over") {
          // `over` has no paths; keep the verdict decided on `enter`.
          setDrag(dragStateRef.current ?? "accept");
        }
        else if (event.payload.type === "leave") {
          setDrag(null);
        }
        else if (event.payload.type === "drop") {
          setDrag(null);
          // Forward every dropped path; classification in addAttachmentPaths
          // rejects the unsupported ones (e.g. directories) with a reason.
          if (event.payload.paths.length > 0)
            void addAttachmentPathsRef.current(event.payload.paths);
        }
      })
      .then((unlisten) => {
        if (active)
          dispose = unlisten;
        else
          unlisten();
      });

    return () => {
      active = false;
      dispose?.();
      setDrag(null);
    };
  }, [addAttachmentPathsRef, disabled, setDrag]);

  return { attachments, attachmentsRef, setAttachments, attachError, setAttachError, dragState, addAttachmentPaths, attachPastedFiles, handleAttachFiles, removeAttachment };
}
