import type { MessageAttachment } from "@future-os/thread-projection";
import { convertFileSrc } from "@tauri-apps/api/core";
import { FileText, Paperclip } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openPath } from "../../../integrations/storage/threadStore";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { FilePreviewOverlay } from "../../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../../filepreview/previewKind";

export function AttachmentChip({ attachment }: { attachment: MessageAttachment }) {
  // A thumbnail (images now; PDF page previews later) renders as a small preview.
  // If it's absent (generation failed, or the thread's image dir was reclaimed)
  // or fails to load, fall back to the named pill instead of a blank box.
  const { t } = useTranslation("agent");
  const [failed, setFailed] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const previewKind = previewKindForPath(attachment.path);
  const missingMessage = t("attachment.fileMissing", { name: attachment.name });

  function handleOpen() {
    if (previewKind) {
      setPreviewOpen(true);
      return;
    }
    void openPath(attachment.path).catch(() => {
      emitFutureEvent("toast", { message: missingMessage, tone: "error" });
    });
  }

  if (attachment.thumbnail && !failed) {
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
        className="inline-flex max-w-72 items-center gap-1.5 rounded-md bg-surface px-2 py-1 text-xs text-ink-soft ring-1 ring-line-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={handleOpen}
        title={attachment.path}
        type="button"
      >
        {attachment.kind === "file"
          ? <FileText className="size-3 shrink-0" />
          : <Paperclip className="size-3 shrink-0" />}
        <span className="truncate">{attachment.name}</span>
      </button>
      {previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={attachment.name}
              onClose={() => setPreviewOpen(false)}
              open={previewOpen}
              path={attachment.path}
              unavailableMessage={missingMessage}
            />
          )
        : null}
    </>
  );
}
