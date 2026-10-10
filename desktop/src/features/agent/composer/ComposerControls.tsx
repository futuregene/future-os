import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../../integrations/storage/appSettings";
import { ArrowUp, ChevronDown, Loader2, Paperclip, ShieldCheck, ShieldKeyhole, ShieldOff, ShieldQuestion, Square } from "lucide-react";
import { useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../../components/ui/SelectMenu";
import { localizedModelDescription, modelKey, modelLabel, modelOption, modelSupportsThinking, normalizeThinkingLevel, thinkingLevels } from "../../../integrations/agent/agentClient";
import { automaticApprovalAvailable, effectiveApprovalTier } from "../../../integrations/agent/automaticApproval";
import { useProviderNames } from "../../../integrations/agent/useProviderNames";
import { useSandboxAvailability } from "../../../integrations/agent/useSandboxAvailability";
import { isLinux, isWindows } from "../../../lib/platform";

/** Approval-tier order for the composer dropdown (availability is host-gated). */
const APPROVAL_TIERS: ApprovalTier[] = ["manual", "sandbox", "auto", "off"];

/**
 * Icon per approval tier, shared between the dropdown rows and the trigger so
 * the button always mirrors the selected tier. The check identifies automatic
 * review, while the keyhole identifies sandbox protection.
 */
function tierIcon(tier: ApprovalTier, className: string) {
  if (tier === "auto")
    return <ShieldCheck className={className} />;
  if (tier === "sandbox")
    return <ShieldKeyhole className={className} />;
  if (tier === "off")
    return <ShieldOff className={className} />;
  return <ShieldQuestion className={className} />;
}

interface Props {
  disabled?: boolean;
  modelId?: string;
  modelOptions: AgentModelOption[];
  modelsEmptyReason?: "no_models" | "all_disabled";
  onModelChange?: (modelId: string) => void;
  thinkingLevel?: string;
  onThinkingLevelChange?: (level: string) => void;
  approvalTier?: ApprovalTier;
  futureSessionStatus: string;
  onChangeApprovalTier?: (tier: ApprovalTier) => void;
  sending?: boolean;
  onAbort?: () => void;
  handleAttachFiles: () => Promise<void>;
  inputEmpty: boolean;
  attachmentCount: number;
  sendPending: boolean;
  recommendPending: boolean;
  compactionPending: boolean;
}

export function ComposerControls({ disabled, modelId, modelOptions, modelsEmptyReason, onModelChange, thinkingLevel, onThinkingLevelChange, approvalTier, futureSessionStatus, onChangeApprovalTier, sending, onAbort, handleAttachFiles, inputEmpty, attachmentCount, sendPending, recommendPending, compactionPending }: Props) {
  const { t, i18n } = useTranslation("agent");
  const sandboxAvailability = useSandboxAvailability();
  const autoAvailable = automaticApprovalAvailable(futureSessionStatus);
  const visibleApprovalTier = effectiveApprovalTier(approvalTier ?? "off", futureSessionStatus);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [thinkingMenuOpen, setThinkingMenuOpen] = useState(false);
  const [approvalMenuOpen, setApprovalMenuOpen] = useState(false);
  const providerNames = useProviderNames();
  const activeModelId = modelId || (modelOptions[0] ? modelKey(modelOptions[0]) : "");
  const activeModel = modelOption(activeModelId, modelOptions);
  const supportsThinking = modelSupportsThinking(activeModelId, modelOptions);
  const activeThinkingLevel = supportsThinking ? normalizeThinkingLevel(thinkingLevel) : "off";
  // Localized thinking-level label; unknown levels fall back to the raw value.
  const thinkingLevelLabel = (level: string) => t(`composer.thinkingLevelLabels.${level}`, { defaultValue: level });

  return (
    <>
      {/* Wraps rather than overflows: a narrow center cannot fit the model /
          thinking / send group beside the attach / approval group, so the
          right-hand group drops to a second row instead of pushing the send
          button past the pane's edge. */}
      <div className="flex flex-wrap items-center justify-between gap-y-1 pt-1">
        <div className="flex min-w-0 items-center gap-1">
          <button
            className="inline-flex size-7 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
            disabled={disabled}
            onClick={() => void handleAttachFiles()}
            type="button"
            aria-label={t("composer.attachFiles")}
            title={t("composer.attachFilesHint")}
          >
            <Paperclip className="size-3.5" />
          </button>
          {onChangeApprovalTier
            ? (
                <SelectMenu
                  align="left"
                  open={approvalMenuOpen}
                  onDismiss={() => setApprovalMenuOpen(false)}
                  panelClassName="w-64 overflow-hidden"
                  trigger={(
                    <button
                      className="inline-flex h-7 max-w-40 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                      onClick={() => {
                        setModelMenuOpen(false);
                        setThinkingMenuOpen(false);
                        setApprovalMenuOpen(open => !open);
                      }}
                      type="button"
                      title={t("composer.approval")}
                    >
                      {tierIcon(visibleApprovalTier, "size-3 shrink-0")}
                      <span className="truncate">{t(`composer.approvalTier.${visibleApprovalTier}`)}</span>
                      <ChevronDown className="size-3 shrink-0" />
                    </button>
                  )}
                >
                  {APPROVAL_TIERS.map(tier => (
                    <SelectMenuItem
                      className="py-1.5"
                      disabled={((tier === "sandbox" || tier === "auto") && !sandboxAvailability.available) || (tier === "auto" && !autoAvailable)}
                      key={tier}
                      selected={visibleApprovalTier === tier}
                      onSelect={() => {
                        onChangeApprovalTier(tier);
                        setApprovalMenuOpen(false);
                      }}
                    >
                      {tierIcon(tier, "size-4 shrink-0 text-ink-soft")}
                      <span className="min-w-0 flex-1 space-y-0.5">
                        <span className="block truncate font-medium leading-tight text-ink">{t(`composer.approvalTier.${tier}`)}</span>
                        <span className="block text-xs leading-tight text-ink-muted">
                          {tier === "auto" && !autoAvailable
                            ? t(futureSessionStatus === "checking" ? "composer.approvalTierDesc.autoChecking" : "composer.approvalTierDesc.autoSignInRequired")
                            : (tier === "sandbox" || tier === "auto") && !sandboxAvailability.resolved
                                ? t("composer.approvalTierDesc.sandboxChecking")
                                : (tier === "sandbox" || tier === "auto") && !sandboxAvailability.available
                                    ? t("composer.approvalTierDesc.sandboxUnavailable")
                                    : tier === "off"
                                      ? <Trans t={t} i18nKey="composer.approvalTierDesc.off" components={{ em: <span className="font-semibold" /> }} />
                                      : t(tier === "sandbox" && isWindows
                                          ? "composer.approvalTierDesc.sandboxWindows"
                                          : tier === "sandbox" && isLinux
                                            ? "composer.approvalTierDesc.sandboxLinux"
                                            : `composer.approvalTierDesc.${tier}`)}
                        </span>
                      </span>
                    </SelectMenuItem>
                  ))}
                </SelectMenu>
              )
            : null}
        </div>
        <div className="ms-auto flex min-w-0 items-center gap-2">
          <SelectMenu
            className="hidden md:block"
            open={modelMenuOpen}
            onDismiss={() => setModelMenuOpen(false)}
            panelClassName="max-h-[40vh] w-56 overflow-y-auto"
            trigger={(
              <button
                className="inline-flex h-7 max-w-48 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                onClick={() => {
                  setThinkingMenuOpen(false);
                  setApprovalMenuOpen(false);
                  setModelMenuOpen(open => !open);
                }}
                type="button"
                title={t("composer.model")}
              >
                <span className="truncate">{modelLabel(activeModelId, modelOptions) ?? t("common:modelFallback")}</span>
                <ChevronDown className="size-3 shrink-0" />
              </button>
            )}
          >
            {modelOptions.length === 0
              ? (
                  <div className="px-3 py-2 text-sm text-ink-muted">
                    {modelsEmptyReason === "all_disabled"
                      ? t("composer.allModelsDisabled")
                      : t("composer.startAgentForModels")}
                  </div>
                )
              : null}
            {modelOptions.map(model => (
              <SelectMenuItem
                className="py-1"
                key={`${model.provider}/${model.id}`}
                selected={model === activeModel}
                onSelect={() => {
                  onModelChange?.(modelKey(model));
                  setModelMenuOpen(false);
                }}
                title={localizedModelDescription(model, i18n.language) ?? undefined}
              >
                <span className="min-w-0 flex-1 space-y-0.5">
                  <span className="block truncate font-medium leading-tight text-ink">{model.label}</span>
                  <span className="block truncate text-xs leading-tight text-ink-muted">
                    {providerNames[model.provider] ?? model.provider}
                  </span>
                </span>
              </SelectMenuItem>
            ))}
          </SelectMenu>
          <SelectMenu
            className="hidden md:block"
            open={thinkingMenuOpen && supportsThinking}
            onDismiss={() => setThinkingMenuOpen(false)}
            panelClassName="w-40 overflow-hidden"
            trigger={(
              <button
                className="inline-flex h-7 max-w-40 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
                onClick={() => {
                  setModelMenuOpen(false);
                  setApprovalMenuOpen(false);
                  setThinkingMenuOpen(open => !open);
                }}
                type="button"
                aria-label={t("composer.thinkingLevel")}
                disabled={!supportsThinking}
                title={supportsThinking ? t("composer.thinkingLevel") : t("composer.thinkingUnsupported")}
              >
                <span className="truncate">{thinkingLevelLabel(activeThinkingLevel)}</span>
                <ChevronDown className="size-3 shrink-0" />
              </button>
            )}
          >
            {thinkingLevels.map(level => (
              <SelectMenuItem
                key={level}
                selected={activeThinkingLevel === level}
                onSelect={() => {
                  onThinkingLevelChange?.(level);
                  setThinkingMenuOpen(false);
                }}
              >
                <span className="min-w-0 flex-1 truncate font-medium text-ink">{thinkingLevelLabel(level)}</span>
              </SelectMenuItem>
            ))}
          </SelectMenu>
          {sending
            ? (
                <button
                  className="inline-flex size-7 items-center justify-center rounded-md bg-ink text-surface transition-colors hover:bg-ink-soft"
                  onClick={() => onAbort?.()}
                  type="button"
                  aria-label={t("composer.stop")}
                  title={t("composer.stop")}
                >
                  <Square className="size-3 fill-current" />
                </button>
              )
            : (
                <>
                  {/* Compaction blocks submission for as long as the agent
                      takes to summarize (minutes on a long conversation) and
                      the send button below has no room to say why. Without
                      this the composer looks inert: typed text stays, Enter
                      does nothing. */}
                  {compactionPending
                    ? (
                        <span
                          className="shrink-0 text-xs whitespace-nowrap text-ink-muted"
                          role="status"
                        >
                          {t("composer.compacting")}
                        </span>
                      )
                    : null}
                  {/* The recommendation wait: same reasoning as compaction,
                      with the spinner on the button itself. */}
                  {recommendPending
                    ? (
                        <span
                          className="shrink-0 text-xs whitespace-nowrap text-ink-muted"
                          role="status"
                        >
                          {t("composer.recommending")}
                        </span>
                      )
                    : null}
                  <button
                    className="inline-flex size-7 items-center justify-center rounded-md bg-accent text-white transition-colors hover:bg-accent-hover disabled:bg-accent-disabled"
                    disabled={
                      (inputEmpty && attachmentCount === 0)
                      || disabled
                      || sendPending
                      || compactionPending
                      || recommendPending
                    }
                    type="submit"
                    aria-label={recommendPending
                      ? t("composer.recommending")
                      : compactionPending ? t("composer.compacting") : t("composer.send")}
                    title={recommendPending
                      ? t("composer.recommending")
                      : compactionPending ? t("composer.compacting") : t("composer.send")}
                  >
                    {recommendPending || compactionPending
                      ? <Loader2 className="size-3.5 animate-spin" />
                      : <ArrowUp className="size-3.5" />}
                  </button>
                </>
              )}
        </div>
      </div>
    </>
  );
}
