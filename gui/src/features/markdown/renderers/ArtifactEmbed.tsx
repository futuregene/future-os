import type { StoredArtifact } from "../../../integrations/storage/types";
import type { FutureReference } from "../futureMarkdownTypes";
import { Check, Clipboard, ExternalLink, FileText, Maximize2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../../components/ui/Button";
import { useCopyState } from "../../../components/ui/useCopyState";
import { openPath, storedTimeToIso } from "../../../integrations/storage/threadStore";
import { formatTime } from "../../../lib/date";
import { emitFutureEvent } from "../../../lib/futureEvents";

export function ArtifactEmbed({
  artifact,
  reference,
}: {
  artifact: StoredArtifact;
  reference: FutureReference;
}) {
  const { t } = useTranslation("markdown");
  const { copiedKey, copy } = useCopyState();

  function inspectArtifact() {
    emitFutureEvent("inspect-artifact", { artifactId: artifact.id });
  }

  return (
    <article className="rounded-md border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-2">
        <FileText className="mt-0.5 size-4 shrink-0 text-accent" />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h4 className="truncate text-sm font-semibold text-ink">{artifact.title || reference.label || artifact.id}</h4>
            <span className="shrink-0 rounded bg-surface-subtle px-1.5 py-0.5 text-[11px] text-ink-muted">
              {artifact.artifactType}
            </span>
          </div>
          <div className="mt-1 text-xs text-ink-muted">{formatTime(storedTimeToIso(artifact.createdAt))}</div>
          {artifact.summary ? <p className="mt-2 text-sm leading-5 text-ink-soft">{artifact.summary}</p> : null}
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
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              leftIcon={<Maximize2 className="size-3.5" />}
              onClick={inspectArtifact}
              size="xs"
              variant="toolbar"
            >
              {t("artifactEmbed.details")}
            </Button>
            {artifact.path
              ? (
                  <Button
                    leftIcon={copiedKey ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
                    onClick={() => void copy(artifact.path ?? "")}
                    size="xs"
                    variant="toolbar"
                  >
                    {t("artifactEmbed.copyPath")}
                  </Button>
                )
              : null}
            {artifact.path
              ? (
                  <Button
                    leftIcon={<ExternalLink className="size-3.5" />}
                    onClick={() => void openPath(artifact.path ?? "")}
                    size="xs"
                    variant="toolbar"
                  >
                    {t("artifactEmbed.open")}
                  </Button>
                )
              : null}
          </div>
        </div>
      </div>
    </article>
  );
}
