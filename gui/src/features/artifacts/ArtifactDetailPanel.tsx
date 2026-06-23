import type { StoredArtifact } from "../../integrations/storage/threadStore";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, BookMarked, Check, Clipboard, Download, ExternalLink, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Badge } from "../../components/ui/Badge";
import {
  deleteArtifact,
  exportArtifactFile,
  openPath,
  promoteArtifactToResearch,
  readTextFilePreview,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { copyText } from "../../lib/clipboard";
import { formatTime } from "../../lib/date";
import { PdfPreview } from "./PdfPreview";

interface ArtifactDetailPanelProps {
  artifact: StoredArtifact;
  onBack: () => void;
  onChanged: () => void;
}

export function ArtifactDetailPanel({ artifact, onBack, onChanged }: ArtifactDetailPanelProps) {
  const [error, setError] = useState<string | null>(null);
  const [filePreview, setFilePreview] = useState<{ content: string; size: number; truncated: boolean } | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<"delete" | "export" | "open" | "promote" | null>(null);
  const [copied, setCopied] = useState<"content" | "path" | null>(null);
  const [imageFailed, setImageFailed] = useState(false);
  const imageSrc = useMemo(
    () => artifact.path && isImageArtifact(artifact) ? convertFileSrc(artifact.path) : null,
    [artifact],
  );
  const shouldLoadTextPreview = Boolean(artifact.path && !artifact.content && isTextPreviewArtifact(artifact));
  const shouldShowPdfPreview = Boolean(artifact.path && isPdfArtifact(artifact));
  const showUnsupportedPreview = Boolean(
    artifact.path
    && !artifact.content
    && !filePreview
    && !imageSrc
    && !previewError
    && !shouldShowPdfPreview,
  );

  useEffect(() => {
    let cancelled = false;
    setFilePreview(null);
    setPreviewError(null);
    setImageFailed(false);

    if (!shouldLoadTextPreview || !artifact.path)
      return;

    async function loadPreview() {
      try {
        const preview = await readTextFilePreview({ maxBytes: 200 * 1024, path: artifact.path ?? "" });
        if (!cancelled) {
          setFilePreview(preview);
        }
      }
      catch (nextError) {
        if (!cancelled) {
          setPreviewError(nextError instanceof Error ? nextError.message : String(nextError));
        }
      }
    }

    void loadPreview();
    return () => {
      cancelled = true;
    };
  }, [artifact.path, shouldLoadTextPreview]);

  async function runAction(action: "delete" | "export" | "open" | "promote", task: () => Promise<void>) {
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
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    }
    finally {
      setBusyAction(null);
    }
  }

  async function handleCopy(kind: "content" | "path", value: string) {
    await copyText(value);
    setCopied(kind);
    window.setTimeout(setCopied, 1400, null);
  }

  async function handleExport() {
    const destinationPath = await save({
      defaultPath: artifactFileName(artifact),
      title: "Export artifact",
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
        Artifacts
      </button>

      <section className="rounded-md border border-line-soft bg-surface p-3">
        <div className="min-w-0">
          <h3 className="break-words text-sm font-semibold leading-5 text-ink">{artifact.title}</h3>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <Badge>{artifact.artifactType}</Badge>
            {artifact.contentStorage ? <Badge>{artifact.contentStorage}</Badge> : null}
            <span className="text-xs text-ink-muted">{formatTime(storedTimeToIso(artifact.createdAt))}</span>
          </div>
        </div>

        {artifact.summary ? <p className="mt-3 text-sm leading-5 text-ink-soft">{artifact.summary}</p> : null}
        {artifact.path
          ? (
              <div className="mt-3 flex items-start gap-2 rounded-md bg-surface-subtle px-2 py-1.5 text-xs leading-5 text-ink-muted">
                <span className="min-w-0 flex-1 break-words" title={artifact.path}>{artifact.path}</span>
                <button
                  aria-label="Copy artifact path"
                  className="inline-flex size-6 shrink-0 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface hover:text-ink"
                  onClick={() => void handleCopy("path", artifact.path ?? "")}
                  title="Copy path"
                  type="button"
                >
                  {copied === "path" ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
                </button>
              </div>
            )
          : null}
        {artifact.content
          ? (
              <div className="relative mt-3">
                <button
                  aria-label="Copy artifact content"
                  className="absolute right-1.5 top-1.5 inline-flex size-6 items-center justify-center rounded-md bg-surface/90 text-ink-muted shadow-sm ring-1 ring-line-soft transition-colors hover:text-ink"
                  onClick={() => void handleCopy("content", artifact.content ?? "")}
                  title="Copy content"
                  type="button"
                >
                  {copied === "content" ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
                </button>
                <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft">
                  <code>{artifact.content}</code>
                </pre>
              </div>
            )
          : null}
        {!artifact.content && imageSrc
          ? (
              <div className="mt-3 overflow-hidden rounded-md border border-line-soft bg-surface-subtle">
                {imageFailed
                  ? (
                      <PreviewFallback
                        detail="The image could not be loaded. You can still open or export the original file."
                        title="Image preview unavailable"
                      />
                    )
                  : (
                      <img
                        alt={artifact.title}
                        className="max-h-80 w-full object-contain"
                        onError={() => setImageFailed(true)}
                        src={imageSrc}
                      />
                    )}
              </div>
            )
          : null}
        {!artifact.content && filePreview
          ? (
              <div className="relative mt-3">
                <button
                  aria-label="Copy artifact preview"
                  className="absolute right-1.5 top-1.5 inline-flex size-6 items-center justify-center rounded-md bg-surface/90 text-ink-muted shadow-sm ring-1 ring-line-soft transition-colors hover:text-ink"
                  onClick={() => void handleCopy("content", filePreview.content)}
                  title="Copy preview"
                  type="button"
                >
                  {copied === "content" ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
                </button>
                <pre className="max-h-80 overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 pr-9 text-[11px] leading-4 text-ink-soft">
                  <code>{filePreview.content}</code>
                </pre>
                <div className="mt-1 text-[11px] text-ink-muted">
                  {filePreview.size.toLocaleString()}
                  {" "}
                  bytes
                  {filePreview.truncated ? " · preview truncated" : ""}
                </div>
              </div>
            )
          : null}
        {shouldShowPdfPreview
          ? (
              <div className="mt-3">
                <PdfPreview path={artifact.path!} />
              </div>
            )
          : null}
        {showUnsupportedPreview
          ? (
              <div className="mt-3">
                <PreviewFallback
                  detail="This file type does not have an inline preview yet. Use Open or Export to inspect the original file."
                  title="Preview unavailable"
                />
              </div>
            )
          : null}
        {previewError ? <div className="mt-3 rounded-md bg-amber-50 p-2 text-xs leading-5 text-amber-700">{previewError}</div> : null}
        {error ? <div className="mt-3 rounded-md bg-red-50 p-2 text-xs leading-5 text-red-700">{error}</div> : null}
      </section>

      <div className="flex flex-wrap gap-2">
        {artifact.path
          ? (
              <button
                className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
                disabled={busyAction !== null}
                onClick={() => void runAction("open", () => openPath(artifact.path ?? ""))}
                type="button"
              >
                <ExternalLink className="size-3.5" />
                {busyAction === "open" ? "Opening" : "Open"}
              </button>
            )
          : null}
        <button
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
          disabled={busyAction !== null}
          onClick={() => void runAction("promote", async () => {
            await promoteArtifactToResearch(artifact.id);
          })}
          type="button"
        >
          <BookMarked className="size-3.5" />
          {busyAction === "promote" ? "Adding" : "Add to Research"}
        </button>
        <button
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
          disabled={busyAction !== null || (!artifact.path && !artifact.content && !filePreview)}
          onClick={() => void handleExport()}
          type="button"
        >
          <Download className="size-3.5" />
          {busyAction === "export" ? "Exporting" : "Export"}
        </button>
        <button
          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-red-200 bg-red-50 px-2.5 text-xs font-medium text-red-700 transition-colors hover:bg-red-100 disabled:cursor-not-allowed disabled:opacity-60"
          disabled={busyAction !== null}
          onClick={() => void runAction("delete", async () => {
            await deleteArtifact(artifact.id);
          })}
          type="button"
        >
          <Trash2 className="size-3.5" />
          {busyAction === "delete" ? "Deleting" : "Delete"}
        </button>
      </div>
    </div>
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
    return artifact.path.split(/[/\\]/).filter(Boolean).pop() ?? sanitizeFileName(artifact.title);
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
