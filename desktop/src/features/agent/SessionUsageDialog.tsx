import type { AgentSessionUsage } from "../../integrations/agent/agentStateCache";
import { useTranslation } from "react-i18next";
import { Dialog } from "../../components/ui/Dialog";
import { formatCostCny, formatNumber } from "../../lib/format";

/**
 * Where the money went, for the conversation the header button belongs to.
 *
 * The rows are priced per category by the agent from the resolved model's
 * per-1M-token rates; the total is the agent's authoritative figure when the
 * provider bills itself (the Future platform), so the rows are an estimate and
 * need not add up to it exactly. An unpriced model reports zeros — then the
 * rows show tokens only, without a fabricated ¥0.
 */
export function SessionUsageDialog({
  onClose,
  open,
  title,
  usage,
}: {
  onClose: () => void;
  open: boolean;
  /** Session title, so the dialog names the conversation it accounts for. */
  title: string;
  usage: AgentSessionUsage | undefined;
}) {
  const { t, i18n } = useTranslation("agent");
  const priced = usage
    ? usage.costInputCny + usage.costOutputCny + usage.costCacheReadCny + usage.costCacheWriteCny > 0
    : false;
  // `inputTokens` includes the cached subset; bill the remainder as plain
  // input so the categories add up against the same token accounting the agent
  // used (see `Cost::estimate`).
  const uncachedInput = usage
    ? Math.max(0, usage.inputTokens - usage.cacheReadTokens - usage.cacheWriteTokens)
    : 0;
  const rows = usage
    ? [
        { label: t("usage.input"), tokens: uncachedInput, cost: usage.costInputCny },
        { label: t("usage.output"), tokens: usage.outputTokens, cost: usage.costOutputCny },
        { label: t("usage.cacheRead"), tokens: usage.cacheReadTokens, cost: usage.costCacheReadCny },
        { label: t("usage.cacheWrite"), tokens: usage.cacheWriteTokens, cost: usage.costCacheWriteCny },
      ]
    : [];

  return (
    <Dialog
      className="max-w-sm"
      description={title}
      onClose={onClose}
      open={open}
      title={t("usage.title")}
    >
      {usage
        ? (
            <div className="space-y-3">
              <div className="overflow-hidden rounded-lg border border-line-soft">
                <div className="flex items-center gap-2 border-b border-line-soft bg-surface-subtle px-3 py-1.5 text-xs font-medium text-ink-muted">
                  <span className="flex-1">{t("usage.category")}</span>
                  <span className="w-24 text-right">{t("usage.tokens")}</span>
                  <span className="w-20 text-right">{t("usage.cost")}</span>
                </div>
                {rows.map(row => (
                  <div
                    className="flex items-center gap-2 px-3 py-2 text-sm text-ink"
                    data-testid={`usage-row-${row.label}`}
                    key={row.label}
                  >
                    <span className="flex-1 text-ink-soft">{row.label}</span>
                    <span className="w-24 text-right tabular-nums">
                      {formatNumber(row.tokens, i18n.language)}
                    </span>
                    <span className="w-20 text-right tabular-nums text-ink-soft">
                      {priced ? formatCostCny(row.cost) : "—"}
                    </span>
                  </div>
                ))}
                <div className="flex items-center gap-2 border-t border-line-soft bg-surface-subtle px-3 py-2 text-sm font-medium text-ink">
                  <span className="flex-1">{t("usage.total")}</span>
                  <span className="w-24" />
                  <span className="w-20 text-right tabular-nums" data-testid="usage-total">
                    {formatCostCny(usage.costCny)}
                  </span>
                </div>
              </div>
              <p className="text-xs text-ink-muted">
                {priced ? t("usage.hintPriced") : t("usage.hintUnpriced")}
              </p>
            </div>
          )
        : <p className="text-sm text-ink-muted">{t("usage.unavailable")}</p>}
    </Dialog>
  );
}
