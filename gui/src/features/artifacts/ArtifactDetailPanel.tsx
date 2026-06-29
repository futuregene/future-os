import type { StoredArtifact } from "../../integrations/storage/threadStore";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { ArrowLeft, BookMarked, Download, ExternalLink, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import {
  deleteArtifact,
  exportArtifactFile,
  openPath,
  promoteArtifactToResearch,
  readTextFilePreview,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { formatTime } from "../../lib/date";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { PdfPreview } from "./PdfPreview";

interface ArtifactDetailPanelProps {
  artifact: StoredArtifact;
  onBack: () => void;
  onChanged: () => void;
}

export function ArtifactDetailPanel({ artifact, onBack, onChanged }: ArtifactDetailPanelProps) {
  const [error, setError] = useState<string | null>(null);
  const [busyAction, setBusyAction] = useState<"delete" | "export" | "open" | "promote" | null>(null);
  const { copiedKey, copy } = useCopyState<"content" | "path">();
  const [imageFailed, setImageFailed] = useState(false);
  const imageSrc = useMemo(
    () => artifact.path && isImageArtifact(artifact) ? convertFileSrc(artifact.path) : null,
    [artifact],
  );
  const shouldLoadTextPreview = Boolean(artifact.path && !artifact.content && isTextPreviewArtifact(artifact));
  const shouldShowPdfPreview = Boolean(artifact.path && isPdfArtifact(artifact));

  const { data: filePreview, error: previewError } = useAsyncResource<{ content: string; size: number; truncated: boolean } | null>(
    () => (shouldLoadTextPreview && artifact.path
      ? readTextFilePreview({ maxBytes: 200 * 1024, path: artifact.path })
      : Promise.resolve(null)),
    [artifact.path, shouldLoadTextPreview],
    null,
  );

  const showUnsupportedPreview = Boolean(
    artifact.path
    && !artifact.content
    && !filePreview
    && !imageSrc
    && !previewError
    && !shouldShowPdfPreview,
  );

  // Reset the image-load failure flag whenever the previewed file changes.
  useEffect(() => {
    setImageFailed(false);
  }, [artifact.path]);

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
                <CopyButton
                  className="shrink-0"
                  copied={copiedKey === "path"}
                  label="Copy path"
                  onCopy={() => void copy(artifact.path ?? "", "path")}
                  variant="inline"
                />
              </div>
            )
          : null}
        {artifact.content
          ? (
              <div className="relative mt-3">
                <CopyButton
                  copied={copiedKey === "content"}
                  label="Copy content"
                  onCopy={() => void copy(artifact.content ?? "", "content")}
                  variant="floating"
                />
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
                <CopyButton
                  copied={copiedKey === "content"}
                  label="Copy preview"
                  onCopy={() => void copy(filePreview.content, "content")}
                  variant="floating"
                />
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
        {previewError ? <div className="mt-3 rounded-md bg-warning-soft p-2 text-xs leading-5 text-warning">{previewError}</div> : null}
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
                {busyAction === "open" ? "Opening" : "Open"}
              </Button>
            )
          : null}
        <Button
          disabled={busyAction !== null}
          leftIcon={<BookMarked className="size-3.5" />}
          onClick={() => void runAction("promote", async () => {
            await promoteArtifactToResearch(artifact.id);
          })}
          size="sm"
          variant="toolbar"
        >
          {busyAction === "promote" ? "Adding" : "Add to Research"}
        </Button>
        <Button
          disabled={busyAction !== null || (!artifact.path && !artifact.content && !filePreview)}
          leftIcon={<Download className="size-3.5" />}
          onClick={() => void handleExport()}
          size="sm"
          variant="toolbar"
        >
          {busyAction === "export" ? "Exporting" : "Export"}
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
          {busyAction === "delete" ? "Deleting" : "Delete"}
        </Button>
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
