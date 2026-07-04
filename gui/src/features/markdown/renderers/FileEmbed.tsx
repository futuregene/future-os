import type { StoredFile } from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { Check, Clipboard, ExternalLink, FileText } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../components/ui/Button";
import { useCopyState } from "../../../components/ui/useCopyState";
import { openPath } from "../../../integrations/storage/threadStore";

/// Card for a `futureos://file/<path>` reference resolved to a workspace file on
/// disk. Unlike {@link ArtifactEmbed} there is no stored artifact record, so it
/// offers only path-level actions (copy / open) — no "details" inspector.
export function FileEmbed({
  file,
  reference,
}: {
  file: StoredFile;
  reference: FutureReference;
}) {
  const { t } = useTranslation("markdown");
  const { copiedKey, copy } = useCopyState();
  const name = file.name || reference.label || file.path;

  return (
    <article className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <FileText className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h4 className="truncate text-sm font-semibold text-ink">{name}</h4>
            <span className="shrink-0 rounded bg-surface-subtle px-1.5 py-0.5 text-[11px] text-ink-muted">
              {file.artifactType}
            </span>
          </div>
          <div className="mt-2 truncate rounded-md bg-surface-subtle px-2 py-1.5 text-xs text-ink-muted" title={file.path}>
            {file.path}
          </div>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              leftIcon={copiedKey ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
              onClick={() => void copy(file.path)}
              size="xs"
              variant="toolbar"
            >
              {t("artifactEmbed.copyPath")}
            </Button>
            <Button
              leftIcon={<ExternalLink className="size-3.5" />}
              onClick={() => void openPath(file.path)}
              size="xs"
              variant="toolbar"
            >
              {t("artifactEmbed.open")}
            </Button>
          </div>
        </div>
      </div>
    </article>
  );
}
