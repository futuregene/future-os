import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import type { StoredArtifact } from "../../integrations/storage/threadStore";
import type { FileKind } from "../../lib/fileType";
import type { LinkMenuItem } from "../markdown/renderers/LinkContextMenu";
import { open } from "@tauri-apps/plugin-dialog";
import { Upload } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import { FileTypeIcon } from "../../components/ui/FileTypeIcon";
import { TextInput } from "../../components/ui/TextInput";
import {
  deleteArtifact,
  importAttachmentArtifact,
  inspectAttachment,
  openPath,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { formatDateTime } from "../../lib/date";
import { errorMessage } from "../../lib/errors";
import { fileKind } from "../../lib/fileType";
import { formatBytes } from "../../lib/format";
import { emitFutureEvent } from "../../lib/futureEvents";
import { pathBasename, relativizeWorkspacePath } from "../../lib/workspacePath";
import { READ_SOURCE_MAX_BYTES } from "../agent/attachments";
import { FilePreviewOverlay } from "../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../filepreview/previewKind";
import { LinkContextMenu } from "../markdown/renderers/LinkContextMenu";
import { useLinkContextMenu } from "../markdown/renderers/useLinkContextMenu";

export function ArtifactsPanel({
  artifacts,
  threadId,
  workspacePath,
  onChanged,
  onSelectArtifact,
}: {
  artifacts: StoredArtifact[];
  threadId: string;
  workspacePath: string | null;
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
                    workspacePath={workspacePath}
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
  workspacePath,
  onChanged,
  onSelectArtifact,
}: {
  artifact: StoredArtifact;
  workspacePath: string | null;
  onChanged: () => void;
  onSelectArtifact: (artifactId: string) => void;
}) {
  const { i18n, t } = useTranslation("artifacts");
  const menu = useLinkContextMenu();
  const [previewOpen, setPreviewOpen] = useState(false);
  // Only file-backed artifacts can be attached/opened; only image/markdown ones
  // can preview inline — the same distinctions the file manager draws.
  const previewKind = artifact.path ? previewKindForPath(artifact.path) : null;

  async function handleDelete() {
    try {
      await deleteArtifact(artifact.id);
      onChanged();
    }
    catch (error) {
      emitFutureEvent("toast", { message: t("card.deleteFailed", { message: errorMessage(error) }), tone: "error" });
    }
  }

  // Attach the artifact's file as an `@`-mention pill in the active composer.
  // The pill wants a workspace-relative, POSIX-separated path (same as the file
  // manager); a path outside the workspace keeps its absolute form.
  function handleAttach() {
    if (!artifact.path)
      return;
    const relative = relativizeWorkspacePath(artifact.path, workspacePath).replace(/\\/g, "/");
    emitFutureEvent("attach-file-to-context", { name: pathBasename(artifact.path) || artifact.title, path: relative });
  }

  const menuItems: LinkMenuItem[] = [
    { label: t("menu.viewDetails"), onSelect: () => onSelectArtifact(artifact.id) },
    ...(artifact.path
      ? [{ label: t("menu.attach"), onSelect: () => handleAttach() }]
      : []),
    ...(artifact.path && previewKind
      ? [{ label: t("menu.preview"), onSelect: () => setPreviewOpen(true) }]
      : []),
    ...(artifact.path
      ? [{ label: t("menu.open"), onSelect: () => void openPath(artifact.path ?? "").catch(() => {}) }]
      : []),
    { danger: true, divider: true, label: t("menu.delete"), onSelect: () => void handleDelete() },
  ];

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelectArtifact(artifact.id);
    }
  }

  return (
    <>
      <div
        aria-label={t("card.viewArtifact", { title: artifact.title })}
        className="cursor-pointer rounded-md border border-line-soft bg-surface p-3 transition-colors hover:border-line hover:bg-surface-subtle focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
        onClick={() => onSelectArtifact(artifact.id)}
        onContextMenu={menu.open}
        onKeyDown={handleKeyDown}
        role="button"
        tabIndex={0}
        title={t("card.viewDetails")}
      >
        <div className="flex items-start gap-2">
          {artifactIcon(artifact)}
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-ink">{artifact.title}</div>
            <div className="mt-1 text-xs text-ink-muted">{formatDateTime(storedTimeToIso(artifact.createdAt), i18n.language)}</div>
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

      <LinkContextMenu controller={menu} items={menuItems} />

      {artifact.path && previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={artifact.title}
              onClose={() => setPreviewOpen(false)}
              onOpenExternal={() => void openPath(artifact.path ?? "").catch(() => {})}
              open={previewOpen}
              path={artifact.path}
            />
          )
        : null}
    </>
  );
}

/** Left-badge icon by artifact kind, from the `artifactType` hint + extension. */
function artifactIcon(artifact: StoredArtifact) {
  return <FileTypeIcon className="mt-0.5 size-4 shrink-0 text-accent" kind={artifactKind(artifact)} />;
}

/**
 * Map an artifact to a {@link FileKind}. The stored `artifactType` hint wins for
 * image/pdf (an image artifact may lack a file extension); everything else is
 * classified from the path, with `code` as the fallback when the type hint says
 * code but the extension is unknown.
 */
function artifactKind(artifact: StoredArtifact): FileKind {
  if (artifact.artifactType === "image")
    return "image";
  if (artifact.artifactType === "pdf")
    return "pdf";
  const byPath = fileKind(artifact.path ?? "");
  if (byPath !== "text")
    return byPath;
  return artifact.artifactType === "code" ? "code" : "text";
}
