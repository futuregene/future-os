import type { VibeCellSettings } from "./vibeCellSettings";
import { useTranslation } from "react-i18next";
import { thinkingLevelName } from "../../integrations/agent/agentClient";

/**
 * The read-only line: `DeepSeek V4 Pro · medium · 完全放开`.
 *
 * Identical on the reading and the input layer — the reader may be in either when the
 * question arrives, and a value that moved or vanished between layers would be a value
 * they stop trusting. Read-only on purpose: the change lives in the panel's menu, one
 * `←`/`→` away, and a dropdown under a thumbnail-sized cell was the noise this line
 * replaced (`VIBE.md` §5.6).
 */
export function VibeSettingsLine({ settings }: { settings: VibeCellSettings }) {
  const { t } = useTranslation("agent");
  const text = [
    settings.modelName,
    settings.supportsThinking ? thinkingLevelName(settings.thinkingLevel) : null,
    t(`composer.approvalTier.${settings.approvalTier}`),
  ].filter(Boolean).join(" · ");
  return (
    <span
      className="min-w-0 truncate font-mono text-[11px] text-ink-muted"
      data-vibe-approval={settings.approvalTier}
      data-vibe-model={settings.modelId}
      data-vibe-settings=""
      title={text}
    >
      {text}
    </span>
  );
}
