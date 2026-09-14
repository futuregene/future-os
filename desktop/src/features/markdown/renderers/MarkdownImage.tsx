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
import { MarkdownImageView } from "./MarkdownImageView";
import { SafeImage } from "./SafeLink";

export function MarkdownImage({
  alt,
  src,
  title,
  workspaceId,
  linked,
}: {
  linked?: boolean;
  alt: string;
  src: string;
  title?: string;
  workspaceId?: string | null;
}) {
  const preview = usePreviewMarkdown();
  const path = localFilePath(src);
  if (!path)
    return <SafeImage alt={alt} linked={linked} src={src} title={title} />;
  if (preview)
    return <PreviewLocalImage alt={alt} basePath={preview.basePath} key={`${preview.basePath}:${path}`} linked={linked} target={path} title={title} />;
  return <WorkspaceLocalImage alt={alt} key={`${workspaceId}:${path}`} linked={linked} target={path} title={title} workspaceId={workspaceId} />;
}

function WorkspaceLocalImage({
  alt,
  target,
  title,
  workspaceId,
  linked,
}: {
  linked?: boolean;
  alt: string;
  target: string;
  title?: string;
  workspaceId?: string | null;
}) {
  const resolved = useFutureReference(workspaceId, { targetId: target, targetType: "file" });
  // Only workspace-contained images auto-load. Reading any other model-authored
  // path requires a deliberate click; the backend still canonicalizes/checks it.
  const [requestedPath, setRequestedPath] = useState<string | null>(null);
  const file = resolved?.status === "resolved" && resolved.targetType === "file"
    && isStoredFile(resolved.data)
    ? resolved.data
    : null;
  return file && (file.insideWorkspace || requestedPath === file.path)
    ? <ResolvedLocalImage alt={alt} file={file} key={file.path} linked={linked} title={title} />
    : <LocalImageFallback alt={alt} onLoad={file && !linked ? () => setRequestedPath(file.path) : undefined} path={target} />;
}

function PreviewLocalImage({
  alt,
  basePath,
  target,
  title,
  linked,
}: {
  linked?: boolean;
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
      key={resolved.path}
      linked={linked}
      file={{ insideWorkspace: false, name: resolved.name, path: resolved.path, relativePath: null }}
      title={title}
    />
  );
}

function ResolvedLocalImage({ alt, file, title, linked }: { alt: string; file: StoredFile; title?: string; linked?: boolean }) {
  const { data: imageUrl, error, loading } = useAsyncResource<string | null>(
    () => prepareImagePreviewUrl(file.path),
    [file.path],
    null,
  );
  if (loading || error || !imageUrl)
    return <LocalImageFallback alt={alt} path={file.path} />;

  return (
    <MarkdownImageView
      alt={alt}
      linked={linked}
      src={imageUrl}
      title={title ?? file.path}
    />
  );
}

function LocalImageFallback({ alt, path, onLoad }: { alt: string; path: string; onLoad?: () => void }) {
  const { t } = useTranslation("markdown");
  return (
    <span
      className="inline-flex max-w-full flex-wrap items-center gap-2 rounded-md border border-dashed border-line-soft bg-surface-subtle px-2 py-1 text-sm text-ink-muted"
      title={path}
    >
      <span className="break-all">{alt || t("image.unavailable")}</span>
      {onLoad && <button className="text-accent hover:underline" onClick={onLoad} type="button">{t("image.load")}</button>}
    </span>
  );
}
