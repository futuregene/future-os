import type { StoredArtifact } from "../../integrations/storage/threadStore";
import { open } from "@tauri-apps/plugin-dialog";
import { Eye, FileText, Trash2, Upload } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import { TextInput } from "../../components/ui/TextInput";
import {
  deleteArtifact,
  importAttachmentArtifact,
  inspectAttachment,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { formatTime } from "../../lib/date";
import { errorMessage } from "../../lib/errors";
import { formatBytes } from "../../lib/format";
import { emitFutureEvent } from "../../lib/futureEvents";
import { READ_SOURCE_MAX_BYTES } from "../agent/attachments";

export function ArtifactsPanel({
  artifacts,
  threadId,
  onChanged,
  onSelectArtifact,
}: {
  artifacts: StoredArtifact[];
  threadId: string;
  onChanged: () => void;
  onSelectArtifact: (artifactId: string) => void;
}) {
  const { t } = useTranslation("artifacts");
  const [filter, setFilter] = useState("");
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);

  const hasArtifacts = artifacts.length > 0;
  const query = filter.trim().toLowerCase();
  const filtered = query
    ? artifacts.filter(artifact => artifact.title.toLowerCase().includes(query))
    : artifacts;

  async function handleUpload() {
    if (uploading)
      return;
    setUploadError(null);
    const selected = await open({ multiple: false, title: t("panel.uploadDialogTitle") });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path)
      return;

    setUploading(true);
    try {
      const info = await inspectAttachment(path);
      if (info.isDir) {
        setUploadError(t("panel.uploadNotFile"));
        return;
      }
      if (info.size > READ_SOURCE_MAX_BYTES) {
        setUploadError(t("panel.uploadTooLarge", { max: formatBytes(READ_SOURCE_MAX_BYTES) }));
        return;
      }
      await importAttachmentArtifact({ threadId, path });
      onChanged();
    }
    catch (error) {
      setUploadError(t("panel.uploadFailed", { message: errorMessage(error) }));
    }
    finally {
      setUploading(false);
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        {hasArtifacts
          ? (
              <TextInput
                className="h-8 flex-1"
                onChange={event => setFilter(event.target.value)}
                placeholder={t("panel.filterPlaceholder")}
                value={filter}
              />
            )
          : null}
        <Button
          className={hasArtifacts ? undefined : "ml-auto"}
          disabled={uploading}
          leftIcon={<Upload className="size-3.5" />}
          onClick={() => void handleUpload()}
          size="sm"
          variant="toolbar"
        >
          {uploading ? t("panel.uploading") : t("panel.upload")}
        </Button>
      </div>
      {uploadError ? <div className="rounded-md bg-danger-soft p-2 text-xs leading-5 text-danger">{uploadError}</div> : null}

      {hasArtifacts
        ? filtered.length > 0
          ? (
              <div className="space-y-3">
                {filtered.map(artifact => (
                  <ArtifactCard
                    artifact={artifact}
                    key={artifact.id}
                    onChanged={onChanged}
                    onSelectArtifact={onSelectArtifact}
                  />
                ))}
              </div>
            )
          : <EmptyState title={t("panel.noMatch")} />
        : <EmptyState title={t("panel.emptyTitle")} detail={t("panel.emptyDetail")} />}
    </div>
  );
}

function ArtifactCard({
  artifact,
  onChanged,
  onSelectArtifact,
}: {
  artifact: StoredArtifact;
  onChanged: () => void;
  onSelectArtifact: (artifactId: string) => void;
}) {
  const { i18n, t } = useTranslation("artifacts");
  async function handleDelete() {
    try {
      await deleteArtifact(artifact.id);
      onChanged();
    }
    catch (error) {
      emitFutureEvent("toast", { message: t("card.deleteFailed", { message: errorMessage(error) }), tone: "error" });
    }
  }

  return (
    <div className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <FileText className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold text-ink">{artifact.title}</div>
              <div className="mt-1 text-xs text-ink-muted">{formatTime(storedTimeToIso(artifact.createdAt), i18n.language)}</div>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <button
                aria-label={t("card.viewArtifact", { title: artifact.title })}
                className="inline-flex size-7 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink"
                onClick={() => onSelectArtifact(artifact.id)}
                title={t("card.viewDetails")}
                type="button"
              >
                <Eye className="size-3.5" />
              </button>
              <button
                aria-label={t("card.deleteArtifact", { title: artifact.title })}
                className="inline-flex size-7 items-center justify-center rounded-md text-ink-muted transition-colors hover:bg-danger-soft hover:text-danger"
                onClick={() => void handleDelete()}
                title={t("card.delete")}
                type="button"
              >
                <Trash2 className="size-3.5" />
              </button>
            </div>
          </div>
          {artifact.path
            ? (
                <div className="mt-2 truncate rounded-md bg-surface-subtle px-2 py-1.5 text-xs text-ink-muted" title={artifact.path}>
                  {artifact.path}
                </div>
              )
            : null}
          {!artifact.path && artifact.content
            ? (
                <pre className="mt-2 max-h-32 overflow-auto rounded-md bg-surface-subtle p-2 text-[11px] leading-4 text-ink-soft">
                  <code>{artifact.content}</code>
                </pre>
              )
            : null}
        </div>
      </div>
    </div>
  );
}
