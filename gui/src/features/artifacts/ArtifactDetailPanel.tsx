import type { StoredArtifact } from "../../integrations/storage/threadStore";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, Download, ExternalLink, Maximize2, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import {
  deleteArtifact,
  exportArtifactFile,
  inspectAttachment,
  openPath,
  readTextFilePreview,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { formatDateTime } from "../../lib/date";
import { errorMessage } from "../../lib/errors";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { pathBasename } from "../../lib/workspacePath";
import { FilePreviewOverlay } from "../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../filepreview/previewKind";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { PdfPreview } from "./PdfPreview";

interface ArtifactDetailPanelProps {
  artifact: StoredArtifact;
  onBack: () => void;
  onChanged: () => void;
}

export function ArtifactDetailPanel({ artifact, onBack, onChanged }: ArtifactDetailPanelProps) {
  const { i18n, t } = useTranslation("artifacts");
  const [error, setError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<"delete" | "export" | "open" | null>(null);
  const { copiedKey, copy } = useCopyState<"content" | "path">();
  const [imageFailed, setImageFailed] = useState(false);
  const [enlarged, setEnlarged] = useState(false);
  const imageSrc = useMemo(
    () => artifact.path && isImageArtifact(artifact) ? convertFileSrc(artifact.path) : null,
    [artifact],
  );
  // Kind used by the fullscreen overlay + Markdown rendering — extension-based,
  // the same detection the middle-panel file preview uses (image / markdown).
  const previewKind = artifact.path ? previewKindForPath(artifact.path) : null;
  const isMarkdown = previewKind === "markdown";
  const openExternal = useCallback(() => {
    if (artifact.path)
      void openPath(artifact.path);
  }, [artifact.path]);
  // File-backed previews take priority over inline `content`; the stored
  // content is only a fallback (see the render order below). Load the text
  // preview whenever the path is a text/markdown file, regardless of content.
  const shouldLoadTextPreview = Boolean(artifact.path && isTextPreviewArtifact(artifact));
  const shouldShowPdfPreview = Boolean(artifact.path && isPdfArtifact(artifact));

  // Detect a missing file (deleted / moved) so we can say so explicitly instead
  // of falling back to a stale preview or a generic error the user reads as a
  // system bug. `inspect_attachment` rejects when the path can't be stat'd.
  const { error: statError, loading: statLoading } = useAsyncResource<{ isDir: boolean; size: number; isBinary: boolean } | null>(
    () => (artifact.path ? inspectAttachment(artifact.path) : Promise.resolve(null)),
    [artifact.path],
    null,
  );
  const fileMissing = Boolean(artifact.path) && !statLoading && statError !== null;

  const { data: filePreview, error: previewError, loading: previewLoading } = useAsyncResource<{ content: string; size: number; truncated: boolean } | null>(
    () => (shouldLoadTextPreview && artifact.path
      ? readTextFilePreview({ maxBytes: 200 * 1024, path: artifact.path })
      : Promise.resolve(null)),
    [artifact.path, shouldLoadTextPreview],
    null,
  );

  // Inline content is the fallback shown only when no file-backed preview
  // (image / PDF / text-file) applies — e.g. a pathless inline artifact, or a
  // file type we can't preview inline but that carries stored content.
  const showInlineContent = !fileMissing
    && Boolean(artifact.content)
    && !imageSrc
    && !shouldShowPdfPreview
    && !shouldLoadTextPreview;

  const showUnsupportedPreview = Boolean(
    artifact.path
    && !fileMissing
    && !artifact.content
    && !filePreview
    // Don't flash "Preview unavailable" while the text preview is still loading.
    && !previewLoading
    && !imageSrc
    && !previewError
    && !shouldShowPdfPreview,
  );

  // Reset the image-load failure flag whenever the previewed file changes.
  useEffect(() => {
    setImageFailed(false);
  }, [artifact.path]);

  async function runAction(action: "delete" | "export" | "open", task: () => Promise<void>) {
    setBusyAction(action);
    setError(null);
    try {
      await task();
      if (action !== "open") {
        onChanged();
      }
      if (action === "delete") {
        onBack();
      }
    }
    catch (nextError) {
      setError(errorMessage(nextError));
    }
    finally {
      setBusyAction(null);
    }
  }

  async function handleExport() {
    const destinationPath = await save({
      defaultPath: artifactFileName(artifact),
      title: t("detail.exportTitle"),
    });
    if (!destinationPath)
      return;

    await runAction("export", () =>
      exportArtifactFile({
        content: artifact.path ? null : artifact.content ?? filePreview?.content ?? null,
        destinationPath,
        sourcePath: artifact.path ?? null,
      }));
  }

  return (
    <div className="space-y-3">
      <button
        className="inline-flex h-8 items-center gap-1.5 rounded-md px-1.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink"
        onClick={onBack}
        type="button"
      >
        <ArrowLeft className="size-3.5" />
        {t("detail.back")}
      </button>

      <section className="rounded-md border border-line-soft bg-surface p-3">
        <div className="min-w-0">
          <h3 className="wrap-break-word text-sm font-semibold leading-5 text-ink">{artifact.title}</h3>
          <div className="mt-2 text-xs text-ink-muted">{formatDateTime(storedTimeToIso(artifact.createdAt), i18n.language)}</div>
        </div>

        {artifact.path
          ? (
              <div className="mt-3 flex items-start gap-2 rounded-md bg-surface-subtle px-2 py-1.5 text-xs leading-5 text-ink-muted">
                <span className="min-w-0 flex-1 wrap-break-word" title={artifact.path}>{artifact.path}</span>
                <CopyButton
                  className="shrink-0"
                  copied={copiedKey === "path"}
                  label={t("detail.copyPath")}
                  onCopy={() => void copy(artifact.path ?? "", "path")}
                  variant="inline"
                />
              </div>
            )
          : null}
        {fileMissing
          ? (
              <div className="mt-3 rounded-md bg-warning-soft p-3">
                <div className="text-sm font-medium text-warning">{t("detail.fileMissingTitle")}</div>
                <p className="mt-1 text-xs leading-5 text-warning">{t("detail.fileMissingDetail")}</p>
              </div>
            )
          : null}
        {!fileMissing && imageSrc
          ? (
              <div className="relative mt-3 overflow-hidden rounded-md border border-line-soft bg-surface-subtle">
                {imageFailed
                  ? (
                      <PreviewFallback
                        detail={t("detail.imagePreviewUnavailableDetail")}
                        title={t("detail.imagePreviewUnavailableTitle")}
                      />
                    )
                  : (
                      <>
                        {previewKind === "image"
                          ? <EnlargeButton className="right-1.5 top-1.5" label={t("detail.enlarge")} onClick={() => setEnlarged(true)} />
                          : null}
                        <img
                          alt={artifact.title}
                          className="max-h-80 w-full object-contain"
                          onError={() => setImageFailed(true)}
                          src={imageSrc}
                        />
                      </>
                    )}
              </div>
            )
          : null}
        {!fileMissing && filePreview
          ? (
              <div className="relative mt-3">
                {isMarkdown
                  ? <EnlargeButton className="right-9 top-1.5" label={t("detail.enlarge")} onClick={() => setEnlarged(true)} />
                  : null}
                <CopyButton
                  copied={copiedKey === "content"}
                  label={t("detail.copyPreview")}
                  onCopy={() => void copy(filePreview.content, "content")}
                  variant="floating"
                />
                {isMarkdown
                  ? (
                      <div className="max-h-96 overflow-auto rounded-md bg-surface-subtle p-3 pr-16">
                        <MarkdownContent basePath={artifact.path ?? undefined} content={filePreview.content} workspaceId={artifact.workspaceId} />
                      </div>
                    )
                  : (
                      <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft">
                        <code>{filePreview.content}</code>
                      </pre>
                    )}
                <div className="mt-1 text-[11px] text-ink-muted">
                  {filePreview.truncated
                    ? t("detail.sizeBytesTruncated", { size: filePreview.size.toLocaleString() })
                    : t("detail.sizeBytes", { size: filePreview.size.toLocaleString() })}
                </div>
              </div>
            )
          : null}
        {!fileMissing && shouldShowPdfPreview
          ? (
              <div className="mt-3">
                <PdfPreview path={artifact.path!} />
              </div>
            )
          : null}
        {showInlineContent
          ? (
              <div className="relative mt-3">
                <CopyButton
                  copied={copiedKey === "content"}
                  label={t("detail.copyContent")}
                  onCopy={() => void copy(artifact.content ?? "", "content")}
                  variant="floating"
                />
                <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft">
                  <code>{artifact.content}</code>
                </pre>
              </div>
            )
          : null}
        {showUnsupportedPreview
          ? (
              <div className="mt-3">
                <PreviewFallback
                  detail={t("detail.previewUnavailableDetail")}
                  title={t("detail.previewUnavailableTitle")}
                />
              </div>
            )
          : null}
        {!fileMissing && previewError ? <div className="mt-3 rounded-md bg-warning-soft p-2 text-xs leading-5 text-warning">{previewError}</div> : null}
        {error ? <div className="mt-3 rounded-md bg-danger-soft p-2 text-xs leading-5 text-danger">{error}</div> : null}
      </section>

      <div className="flex flex-wrap gap-2">
        {artifact.path
          ? (
              <Button
                disabled={busyAction !== null}
                leftIcon={<ExternalLink className="size-3.5" />}
                onClick={() => void runAction("open", () => openPath(artifact.path ?? ""))}
                size="sm"
                variant="toolbar"
              >
                {busyAction === "open" ? t("detail.opening") : t("detail.open")}
              </Button>
            )
          : null}
        <Button
          disabled={busyAction !== null || (!artifact.path && !artifact.content && !filePreview)}
          leftIcon={<Download className="size-3.5" />}
          onClick={() => void handleExport()}
          size="sm"
          variant="toolbar"
        >
          {busyAction === "export" ? t("detail.exporting") : t("detail.export")}
        </Button>
        <Button
          disabled={busyAction !== null}
          leftIcon={<Trash2 className="size-3.5" />}
          onClick={() => void runAction("delete", async () => {
            await deleteArtifact(artifact.id);
          })}
          size="sm"
          variant="danger-soft"
        >
          {busyAction === "delete" ? t("detail.deleting") : t("detail.deleteAction")}
        </Button>
      </div>

      {previewKind && artifact.path
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={artifact.title}
              onClose={() => setEnlarged(false)}
              onOpenExternal={openExternal}
              open={enlarged}
              path={artifact.path}
            />
          )
        : null}
    </div>
  );
}

function EnlargeButton({
  className,
  label,
  onClick,
}: {
  className?: string;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className={cn(
        "absolute inline-flex size-6 items-center justify-center rounded-md bg-surface/90 text-ink-muted shadow-xs ring-1 ring-line-soft transition-colors hover:text-ink",
        className,
      )}
      onClick={onClick}
      title={label}
      type="button"
    >
      <Maximize2 className="size-3.5" />
    </button>
  );
}

function PreviewFallback({
  detail,
  title,
}: {
  detail: string;
  title: string;
}) {
  return (
    <div className="rounded-md border border-dashed border-line-soft bg-surface-subtle p-3">
      <div className="text-sm font-medium text-ink-soft">{title}</div>
      <p className="mt-1 text-xs leading-5 text-ink-muted">{detail}</p>
    </div>
  );
}

function artifactFileName(artifact: StoredArtifact) {
  if (artifact.path) {
    return pathBasename(artifact.path) || sanitizeFileName(artifact.title);
  }
  const extension = artifact.artifactType === "data" ? "json" : "txt";
  return `${sanitizeFileName(artifact.title || "artifact")}.${extension}`;
}

function sanitizeFileName(value: string) {
  const sanitized = value.replace(/[\\/:*?"<>|]+/g, "_").trim();
  return sanitized || "artifact";
}

function isImageArtifact(artifact: StoredArtifact) {
  return artifact.artifactType === "image" || /\.(?:avif|bmp|gif|jpe?g|png|svg|webp)$/i.test(artifact.path ?? "");
}

function isTextPreviewArtifact(artifact: StoredArtifact) {
  return ["code", "data", "document", "text"].includes(artifact.artifactType)
    || /\.(?:css|csv|html?|js|json|jsonl|jsx|md|py|rs|toml|ts|tsx|txt|xml|ya?ml)$/i.test(artifact.path ?? "");
}

function isPdfArtifact(artifact: StoredArtifact) {
  return artifact.artifactType === "pdf" || /\.pdf$/i.test(artifact.path ?? "");
}
