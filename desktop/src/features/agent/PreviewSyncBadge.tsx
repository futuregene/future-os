import type { PreviewSyncStall } from "@future-os/thread-projection";
import { ChevronDown, ChevronUp, RefreshCw } from "lucide-react";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * "Sync paused" note for a reply whose live preview cannot read the run's
 * events: the text on screen is the last projection the client managed to
 * build, not the model having stopped.
 *
 * Deliberately not an error and not a spinner: nothing is wrong with the reply,
 * the client is only behind, and the note disappears on its own with the first
 * successful read (`previewStalled` is cleared there). It starts collapsed so it
 * costs one short line, and opening it names the stage and the raw reason —
 * otherwise the only account of a frozen reply is a console warning.
 */
export const PreviewSyncBadge = memo(({ stall }: { stall: PreviewSyncStall }) => {
  const { t } = useTranslation("agent");
  const [expanded, setExpanded] = useState(false);
  const Chevron = expanded ? ChevronUp : ChevronDown;

  return (
    <div className="flex min-w-0 flex-col gap-1 text-xs text-ink-muted">
      <button
        aria-expanded={expanded}
        className="flex max-w-full cursor-pointer items-center gap-1.5 self-start rounded text-left hover:text-ink"
        onClick={() => setExpanded(value => !value)}
        type="button"
      >
        <RefreshCw className="size-3 shrink-0" />
        <span>{t("message.syncing")}</span>
        <Chevron className="size-3 shrink-0" />
      </button>
      {expanded
        ? (
            <div className="flex min-w-0 flex-col gap-0.5 pl-4 text-[11px] leading-5">
              <span>{t(stall.stage === "read" ? "message.syncingRead" : "message.syncingProjection")}</span>
              <span>{t("message.syncingAttempts", { count: stall.attempts })}</span>
              <span className="break-all font-mono opacity-80">{stall.reason}</span>
            </div>
          )
        : null}
    </div>
  );
});
