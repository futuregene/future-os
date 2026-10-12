import type { RemoteConversationSettingsState } from "./useRemoteConversationSettings";
import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../components/ui/SelectMenu";
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
 * **The same two controls as this app's own composer, in the same slot.** A
 * native `<select>` next to this app's dropdowns is the one place a reader sees
 * the two side by side, and it shows: a bordered control with the platform's own
 * menu, at a different height and with a different chevron. These are
 * `SelectMenu`s now, built from the local composer's trigger and item markup —
 * the shape is what makes a remote conversation's control row read as this app's.
 *
 * Disabled while a change is in flight: the host applies one at a time, and a
 * second choice fired at it would be a race the user cannot see.
 *
 * A refusal is *shown*, not merely held: the control falls back to what the host
 * still has, and without the reason beside it the user sees a value snap back
 * for no stated cause. The message is a shell of one line under the pickers, and
 * carries the full text in its tooltip because the header has a fixed height.
 */
export function RemoteConversationSettings({
  settings,
}: {
  settings: RemoteConversationSettingsState;
}) {
  const { t } = useTranslation("remotePeer");
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [thinkingMenuOpen, setThinkingMenuOpen] = useState(false);
  const busy = settings.saving;
  const activeModel = settings.models.find(model => remoteModelReference(model) === settings.model);
  // The thinking level this control shows, and the one it started on when the
  // host has not said: `off` is what the host's own picker falls back to.
  const activeThinkingLevel = settings.thinkingLevel ?? "off";

  return (
    <div className="flex shrink-0 flex-col items-end">
      <div className="flex shrink-0 items-center gap-2">
        <SelectMenu
          open={modelMenuOpen}
          onDismiss={() => setModelMenuOpen(false)}
          panelClassName="max-h-[40vh] w-56 overflow-y-auto"
          trigger={(
            <button
              aria-label={t("modelLabel")}
              className="inline-flex h-7 max-w-48 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
              disabled={busy || settings.models.length === 0}
              onClick={() => {
                setThinkingMenuOpen(false);
                setModelMenuOpen(open => !open);
              }}
              title={t("modelLabel")}
              type="button"
            >
              {/* The host's catalogue can arrive without the current model in it (a
                  model removed there, or one it resolves itself). Keeping that value
                  on screen is what stops the control from silently showing the first
                  entry as if it were selected. */}
              <span className="truncate">
                {(activeModel?.label ?? settings.model ?? "") || t("modelLabel")}
              </span>
              <ChevronDown className="size-3 shrink-0" />
            </button>
          )}
        >
          {settings.model && !activeModel
            ? (
                <SelectMenuItem className="py-1" selected onSelect={() => setModelMenuOpen(false)}>
                  <span className="min-w-0 flex-1 truncate font-medium leading-tight text-ink">{settings.model}</span>
                </SelectMenuItem>
              )
            : null}
          {settings.models.map(model => (
            <SelectMenuItem
              className="py-1"
              key={remoteModelReference(model)}
              selected={remoteModelReference(model) === settings.model}
              onSelect={() => {
                setModelMenuOpen(false);
                void settings.setModel(remoteModelReference(model));
              }}
            >
              <span className="min-w-0 flex-1 space-y-0.5">
                <span className="block truncate font-medium leading-tight text-ink">{model.label ?? model.id}</span>
                {model.provider
                  ? <span className="block truncate text-xs leading-tight text-ink-muted">{model.provider}</span>
                  : null}
              </span>
            </SelectMenuItem>
          ))}
        </SelectMenu>
        <SelectMenu
          open={thinkingMenuOpen}
          onDismiss={() => setThinkingMenuOpen(false)}
          panelClassName="w-40 overflow-hidden"
          trigger={(
            <button
              aria-label={t("thinkingLevelLabel")}
              className="inline-flex h-7 max-w-40 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
              disabled={busy}
              onClick={() => {
                setModelMenuOpen(false);
                setThinkingMenuOpen(open => !open);
              }}
              title={t("thinkingLevelLabel")}
              type="button"
            >
              <span className="truncate">{t(`thinkingLevel.${activeThinkingLevel}`, { defaultValue: activeThinkingLevel })}</span>
              <ChevronDown className="size-3 shrink-0" />
            </button>
          )}
        >
          {THINKING_LEVELS.map(level => (
            <SelectMenuItem
              key={level}
              selected={activeThinkingLevel === level}
              onSelect={() => {
                setThinkingMenuOpen(false);
                void settings.setThinkingLevel(level);
              }}
            >
              <span className="min-w-0 flex-1 truncate font-medium text-ink">{t(`thinkingLevel.${level}`)}</span>
            </SelectMenuItem>
          ))}
        </SelectMenu>
      </div>
      {settings.error
        ? (
            <p
              className="mt-0.5 max-w-72 truncate text-xs text-danger"
              role="alert"
              title={t("settingsSaveFailed", { message: settings.error })}
            >
              {t("settingsSaveFailed", { message: settings.error })}
            </p>
          )
        : null}
    </div>
  );
}
