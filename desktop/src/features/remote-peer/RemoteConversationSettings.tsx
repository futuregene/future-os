import type { RemoteConversationSettingsState } from "./useRemoteConversationSettings";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { remoteModelReference } from "./remotePeerClient";

/**
 * The thinking levels the agent accepts.
 *
 * A fixed list, mirroring the desktop's own picker and the phone's: the host does
 * not publish one, and inventing a value would be a `set_thinking_level` it
 * rejects.
 */
const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

/**
 * A remote conversation's model and thinking level.
 *
 * Both are the *host's* settings for that conversation, applied there and run
 * there — which is why the labels say so and why a change is confirmed by the
 * host's own announcement rather than by this control.
 *
 * Disabled while a change is in flight: the host applies one at a time, and a
 * second choice fired at it would be a race the user cannot see.
 */
export function RemoteConversationSettings({
  settings,
}: {
  settings: RemoteConversationSettingsState;
}) {
  const { t } = useTranslation("remotePeer");
  const busy = settings.saving;

  return (
    <div className="flex shrink-0 items-center gap-2">
      <Select
        aria-label={t("modelLabel")}
        disabled={busy || settings.models.length === 0}
        onChange={event => void settings.setModel(event.target.value)}
        size="xs"
        value={settings.model ?? ""}
        wrapperClassName="max-w-56"
      >
        {/* The host's catalogue can arrive without the current model in it (a
            model removed there, or one it resolves itself). Keeping that value
            as an option is what stops the select from silently showing the first
            entry as if it were selected. */}
        {settings.model && !settings.models.some(model => remoteModelReference(model) === settings.model)
          ? <option value={settings.model}>{settings.model}</option>
          : null}
        {settings.models.map(model => (
          <option key={remoteModelReference(model)} value={remoteModelReference(model)}>
            {model.label ?? model.id}
            {model.provider ? ` · ${model.provider}` : ""}
          </option>
        ))}
      </Select>
      <Select
        aria-label={t("thinkingLevelLabel")}
        disabled={busy}
        onChange={event => void settings.setThinkingLevel(event.target.value)}
        size="xs"
        value={settings.thinkingLevel ?? "off"}
        wrapperClassName="w-28"
      >
        {THINKING_LEVELS.map(level => (
          <option key={level} value={level}>{t(`thinkingLevel.${level}`)}</option>
        ))}
      </Select>
    </div>
  );
}
