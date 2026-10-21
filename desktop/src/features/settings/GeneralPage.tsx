import type { Language } from "../../i18n";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { FontSizePreference } from "../../lib/fontSize";
import type { ThemeMode } from "../../lib/theme";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { automaticApprovalAvailable, effectiveApprovalTier } from "../../integrations/agent/automaticApproval";
import { useSandboxAvailability } from "../../integrations/agent/useSandboxAvailability";
import { FONT_SIZES, setFontSize, useFontSize } from "../../lib/fontSize";
import { isLinux, isWindows } from "../../lib/platform";
import { setFinalAnswerOnly, useFinalAnswerOnly } from "../../lib/replyDisplay";
import { setThemeMode, THEME_MODES, useThemeMode } from "../../lib/theme";
import { linuxUnavailableReasonKey } from "./linuxSandboxStatus";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function GeneralPage({
  approvalTier,
  futureSessionStatus = "checking",
  onChangeApprovalTier,
  autoUpgradeSkills,
  onToggleAutoUpgradeSkills,
  skillRecommend,
  onToggleSkillRecommend,
  bellOnComplete,
  onToggleBellOnComplete,
  autoTitleFirstTurn,
  onToggleAutoTitleFirstTurn,
  autoResumeInterrupted,
  onToggleAutoResumeInterrupted,
}: {
  approvalTier: ApprovalTier;
  futureSessionStatus?: string;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  autoUpgradeSkills: boolean;
  onToggleAutoUpgradeSkills: (value: boolean) => void;
  skillRecommend: boolean;
  onToggleSkillRecommend: (value: boolean) => void;
  bellOnComplete: boolean;
  onToggleBellOnComplete: (value: boolean) => void;
  autoTitleFirstTurn: boolean;
  onToggleAutoTitleFirstTurn: (value: boolean) => void;
  autoResumeInterrupted: boolean;
  onToggleAutoResumeInterrupted: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");
  const themeMode = useThemeMode();
  const fontSize = useFontSize();
  const finalAnswerOnly = useFinalAnswerOnly();
  const sandboxAvailability = useSandboxAvailability();
  const autoAvailable = automaticApprovalAvailable(futureSessionStatus);
  const visibleApprovalTier = effectiveApprovalTier(approvalTier, futureSessionStatus);

  return (
    <SettingsSection>
      <SettingsList>
        <SettingsRow
          title={t("language.title")}
          description={t("language.description")}
        >
          <Select
            size="sm"
            value={getLanguage()}
            wrapperClassName="w-32"
            onChange={e => setLanguage(e.target.value as Language)}
          >
            {SUPPORTED_LANGUAGES.map(lang => (
              <option key={lang} value={lang}>
                {LANGUAGE_LABELS[lang]}
              </option>
            ))}
          </Select>
        </SettingsRow>
        <SettingsRow
          title={t("theme.title")}
          description={t("theme.description")}
        >
          <Select
            size="sm"
            value={themeMode}
            wrapperClassName="w-32"
            onChange={e => setThemeMode(e.target.value as ThemeMode)}
          >
            {THEME_MODES.map(mode => (
              <option key={mode} value={mode}>
                {t(`theme.${mode}`)}
              </option>
            ))}
          </Select>
        </SettingsRow>
        <SettingsRow
          title={t("fontSize.title")}
          description={t("fontSize.description")}
        >
          <Select
            size="sm"
            value={fontSize}
            wrapperClassName="w-32"
            onChange={e => setFontSize(e.target.value as FontSizePreference)}
          >
            {FONT_SIZES.map(size => (
              <option key={size} value={size}>
                {t(`fontSize.${size}`)}
              </option>
            ))}
          </Select>
        </SettingsRow>
        <SettingsRow
          title={t("finalAnswerOnly.title")}
          description={t("finalAnswerOnly.description")}
        >
          <Switch checked={finalAnswerOnly} label={t("finalAnswerOnly.title")} onChange={setFinalAnswerOnly} />
        </SettingsRow>
        <SettingsRow
          title={t("approvalTier.title")}
          description={t(
            visibleApprovalTier === "sandbox" && isWindows
              ? "approvalTier.description.sandboxWindows"
              : visibleApprovalTier === "sandbox" && isLinux
                ? "approvalTier.description.sandboxLinux"
                : `approvalTier.description.${visibleApprovalTier}`,
          )}
        >
          <Select
            size="sm"
            value={visibleApprovalTier}
            wrapperClassName="w-40"
            onChange={e => onChangeApprovalTier(e.target.value as ApprovalTier)}
          >
            <option value="manual">{t("approvalTier.manual")}</option>
            <option disabled={!sandboxAvailability.available} value="sandbox">
              {t(sandboxAvailability.available
                ? "approvalTier.sandbox"
                : sandboxAvailability.resolved
                  ? "approvalTier.sandboxUnavailable"
                  : "approvalTier.sandboxChecking")}
            </option>
            <option disabled={!sandboxAvailability.available || !autoAvailable} value="auto">
              {t(autoAvailable ? "approvalTier.auto" : futureSessionStatus === "checking" ? "approvalTier.autoChecking" : "approvalTier.autoSignInRequired")}
            </option>
            <option value="off">{t("approvalTier.off")}</option>
          </Select>
        </SettingsRow>
        {visibleApprovalTier === "auto" && sandboxAvailability.resolved && !sandboxAvailability.available
          ? <SettingsRow title={t("approvalTier.auto")} description={t("approvalTier.autoUnavailable")} />
          : null}
        {isLinux && sandboxAvailability.resolved && !sandboxAvailability.available
          ? (
              <SettingsRow
                title={t("approvalTier.linuxUnavailable.title")}
                description={t(`approvalTier.linuxUnavailable.reasons.${linuxUnavailableReasonKey(sandboxAvailability.code)}`, {
                  code: sandboxAvailability.code ?? "probe_failed",
                })}
              />
            )
          : null}
        <SettingsRow
          title={t("autoUpgradeSkills.title")}
          description={t("autoUpgradeSkills.description")}
        >
          <Switch checked={autoUpgradeSkills} label={t("autoUpgradeSkills.title")} onChange={onToggleAutoUpgradeSkills} />
        </SettingsRow>
        <SettingsRow
          title={t("skillRecommend.title")}
          description={t("skillRecommend.description")}
        >
          <Switch checked={skillRecommend} label={t("skillRecommend.title")} onChange={onToggleSkillRecommend} />
        </SettingsRow>
        <SettingsRow
          title={t("autoTitleFirstTurn.title")}
          description={t("autoTitleFirstTurn.description")}
        >
          <Switch checked={autoTitleFirstTurn} label={t("autoTitleFirstTurn.title")} onChange={onToggleAutoTitleFirstTurn} />
        </SettingsRow>
        <SettingsRow
          title={t("bellOnComplete.title")}
          description={t("bellOnComplete.description")}
        >
          <Switch checked={bellOnComplete} label={t("bellOnComplete.title")} onChange={onToggleBellOnComplete} />
        </SettingsRow>
        <SettingsRow
          title={t("autoResumeInterrupted.title")}
          description={t("autoResumeInterrupted.description")}
        >
          <Switch
            checked={autoResumeInterrupted}
            label={t("autoResumeInterrupted.title")}
            onChange={onToggleAutoResumeInterrupted}
          />
        </SettingsRow>
      </SettingsList>
    </SettingsSection>
  );
}
