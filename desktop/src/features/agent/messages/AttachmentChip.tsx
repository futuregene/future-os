import type { MessageAttachment } from "@future-os/thread-projection";
import { convertFileSrc } from "@tauri-apps/api/core";
import { FileText, Paperclip } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openPath } from "../../../integrations/storage/threadStore";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { FilePreviewOverlay } from "../../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../../filepreview/previewKind";
import { useRemoteAttachmentOrigin } from "../../remote-peer/RemoteAttachmentOrigin";
import { previewRemoteFile } from "../../remote-peer/remotePeerClient";

export function AttachmentChip({ attachment }: { attachment: MessageAttachment }) {
  // A thumbnail (images now; PDF page previews later) renders as a small preview.
  // If it's absent (generation failed, or the thread's image dir was reclaimed)
  // or fails to load, fall back to the named pill instead of a blank box.
  const { t } = useTranslation("agent");
  const [failed, setFailed] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [fetching, setFetching] = useState(false);
  /**
   * What the overlay reads once a remote attachment has been fetched: this
   * machine's cache entry, and the host's own name for the file.
   */
  const [fetched, setFetched] = useState<{ name: string; path: string } | null>(null);
  const origin = useRemoteAttachmentOrigin();
  // `attachment.path` names the file on whichever machine the conversation runs
  // on. For a remote one that is not this machine, so the path is only good for
  // asking the host — never for opening or thumbnailing here.
  const localPath = origin ? fetched?.path : attachment.path;
  const previewKind = previewKindForPath(origin ? fetched?.name ?? attachment.name : attachment.path);
  const missingMessage = origin
    ? t("attachment.remoteUnavailable", { name: attachment.name })
    : t("attachment.fileMissing", { name: attachment.name });

  async function openRemote() {
    // A second look at the same file is the cache's job, not another transfer:
    // the entry it lands on is keyed by machine, session and path.
    if (fetched) {
      setPreviewOpen(Boolean(previewKindForPath(fetched.name)));
      if (!previewKindForPath(fetched.name))
        void openPath(fetched.path).catch(() => emitFutureEvent("toast", { message: missingMessage, tone: "error" }));
      return;
    }
    setFetching(true);
    try {
      const value = await previewRemoteFile({
        desktopId: origin!.desktopId,
        sessionId: origin!.sessionId,
        path: attachment.path,
        name: attachment.name,
      });
      setFetched(value);
      if (previewKindForPath(value.name))
        setPreviewOpen(true);
      else
        // Nothing to draw (a zip, a font): the bytes are here now, so hand them
        // to the OS the way a local chip would — the same gesture, one transfer
        // later.
        await openPath(value.path);
    }
    catch {
      // The host is unreachable, the file is gone there, or the transfer failed.
      // Whoever is looking is told at the moment they asked; saying "moved or
      // deleted" would blame this machine for a file that may be right where the
      // other one left it.
      emitFutureEvent("toast", { message: missingMessage, tone: "error" });
    }
    finally {
      setFetching(false);
    }
  }

  function handleOpen() {
    if (origin) {
      void openRemote();
      return;
    }
    if (localPath && previewKind) {
      setPreviewOpen(true);
      return;
    }
    void openPath(attachment.path).catch(() => {
      emitFutureEvent("toast", { message: missingMessage, tone: "error" });
    });
  }

  // A remote image's `thumbnail` is a path on the host's disk: an <img> pointed
  // at it can only fail, after a request this machine has no way to serve. Draw
  // the pill and let the click fetch the bytes instead.
  if (!origin && attachment.thumbnail && !failed) {
    return (
      <>
        <button
          className="inline-flex max-w-64 items-center overflow-hidden rounded-md"
          onClick={handleOpen}
          title={attachment.name}
          type="button"
        >
          <img
            alt={attachment.name}
            className="max-h-64 max-w-64 object-contain"
            onError={() => setFailed(true)}
            src={convertFileSrc(attachment.thumbnail)}
          />
        </button>
        {/* Preview the full-size original. If it's gone (moved/reclaimed), toast
            that it's damaged and close — the 96px thumbnail isn't worth previewing. */}
        <FilePreviewOverlay
          kind={previewKind ?? "image"}
          name={attachment.name}
          onClose={() => setPreviewOpen(false)}
          open={previewOpen}
          path={attachment.path}
          unavailableMessage={missingMessage}
        />
      </>
    );
  }
  return (
    <>
      <button
        aria-busy={fetching || undefined}
        className="inline-flex max-w-72 items-center gap-1.5 rounded-md bg-surface px-2 py-1 text-xs text-ink-soft ring-1 ring-line-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:opacity-60"
        disabled={fetching}
        onClick={handleOpen}
        title={origin ? attachment.name : attachment.path}
        type="button"
      >
        {attachment.kind === "file"
          ? <FileText className="size-3 shrink-0" />
          : <Paperclip className="size-3 shrink-0" />}
        <span className="truncate">{attachment.name}</span>
      </button>
      {previewKind && localPath
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={fetched?.name ?? attachment.name}
              onClose={() => setPreviewOpen(false)}
              open={previewOpen}
              path={localPath}
              unavailableMessage={missingMessage}
            />
          )
        : null}
    </>
  );
}
