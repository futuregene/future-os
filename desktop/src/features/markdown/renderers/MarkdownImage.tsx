import type { StoredFile } from "../../../integrations/storage/types";
import { localFilePath } from "@future-os/markdown";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { prepareImagePreviewUrl } from "../../../integrations/storage/files";
import { isStoredFile } from "../../../integrations/storage/typeGuards";
import { useAsyncResource } from "../../../lib/useAsyncResource";
import { useFutureReference } from "../futureReferenceStore";
import { usePreviewMarkdown } from "../PreviewMarkdownContext";
import { usePreviewLinkPath } from "../usePreviewLinkPath";
import { SafeImage } from "./SafeLink";

export function MarkdownImage({
  alt,
  src,
  title,
  workspaceId,
}: {
  alt: string;
  src: string;
  title?: string;
  workspaceId?: string | null;
}) {
  const preview = usePreviewMarkdown();
  const path = localFilePath(src);
  if (!path)
    return <SafeImage alt={alt} src={src} title={title} />;
  if (preview)
    return <PreviewLocalImage alt={alt} basePath={preview.basePath} target={path} title={title} />;
  return <WorkspaceLocalImage alt={alt} target={path} title={title} workspaceId={workspaceId} />;
}

function WorkspaceLocalImage({
  alt,
  target,
  title,
  workspaceId,
}: {
  alt: string;
  target: string;
  title?: string;
  workspaceId?: string | null;
}) {
  const resolved = useFutureReference(workspaceId, { targetId: target, targetType: "file" });
  // Only inline images that live inside the workspace: a model-authored absolute
  // path (e.g. `![x](/etc/passwd)`) must not become a read-arbitrary-file channel
  // through the asset protocol. Out-of-workspace targets render as a fallback chip.
  const file = resolved?.status === "resolved" && resolved.targetType === "file"
    && isStoredFile(resolved.data) && resolved.data.insideWorkspace
    ? resolved.data
    : null;
  return file
    ? <ResolvedLocalImage alt={alt} file={file} title={title} />
    : <LocalImageFallback alt={alt} path={target} />;
}

function PreviewLocalImage({
  alt,
  basePath,
  target,
  title,
}: {
  alt: string;
  basePath: string;
  target: string;
  title?: string;
}) {
  const resolved = usePreviewLinkPath(basePath, target);
  if (!resolved)
    return <LocalImageFallback alt={alt} path={target} />;
  return (
    <ResolvedLocalImage
      alt={alt}
      file={{ insideWorkspace: false, name: resolved.name, path: resolved.path, relativePath: null }}
      title={title}
    />
  );
}

function ResolvedLocalImage({ alt, file, title }: { alt: string; file: StoredFile; title?: string }) {
  const { data: imageUrl, error, loading } = useAsyncResource<string | null>(
    () => prepareImagePreviewUrl(file.path),
    [file.path],
    null,
  );
  const [failedPath, setFailedPath] = useState<string | null>(null);
  const failed = failedPath === file.path;

  if (loading || error || !imageUrl || failed)
    return <LocalImageFallback alt={alt} path={file.path} />;

  return (
    <img
      alt={alt}
      className="my-2 max-h-80 max-w-full rounded-md border border-line-soft object-contain"
      onError={() => setFailedPath(file.path)}
      src={imageUrl}
      title={title ?? file.path}
    />
  );
}

function LocalImageFallback({ alt, path }: { alt: string; path: string }) {
  const { t } = useTranslation("markdown");
  return (
    <span
      className="inline-flex max-w-full items-center rounded-md border border-dashed border-line-soft bg-surface-subtle px-2 py-1 text-sm text-ink-muted"
      title={path}
    >
      {alt || t("image.unavailable")}
    </span>
  );
}
