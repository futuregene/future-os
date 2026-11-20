import type { LanguagePreference } from "../../i18n";
import type { FontSizePreference } from "../../lib/fontSize";
import type { ReplyDetail } from "../../lib/replyDisplay";
import type { ThemeMode } from "../../lib/theme";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { getLanguagePreference, LANGUAGE_LABELS, LANGUAGE_PREFERENCES, setLanguage } from "../../i18n";
import { FONT_SIZES, setFontSize, useFontSize } from "../../lib/fontSize";
import { REPLY_DETAIL_LEVELS, setReplyDetail, useReplyDetail } from "../../lib/replyDisplay";
import { setThemeMode, THEME_MODES, useThemeMode } from "../../lib/theme";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";
import { ThemePalettePicker } from "./ThemePalettePicker";

/**
 * This app's own preferences: language, light/dark, palette, text size, how a
 * finished reply is shown, and whether finishing one rings the bell.
 *
 * None of them reach the agent, and none are visible from a paired phone — the
 * bell is played by this machine, and a phone carries its own reminder switch.
 * That is the line this page keeps: what this client does, not what a reply does.
 */
export function AppearancePage({ bellOnComplete, onToggleBellOnComplete }: {
  bellOnComplete: boolean;
  onToggleBellOnComplete: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");
  const themeMode = useThemeMode();
  const fontSize = useFontSize();
  const replyDetail = useReplyDetail();

  return (
    <div className="space-y-6">
      <SettingsSection>
        <SettingsList>
          <SettingsRow
            title={t("language.title")}
            description={t("language.description")}
          >
            <Select
              size="sm"
              value={getLanguagePreference()}
              wrapperClassName="w-32"
              onChange={e => setLanguage(e.target.value as LanguagePreference)}
            >
              {LANGUAGE_PREFERENCES.map(preference => (
                <option key={preference} value={preference}>
                  {preference === "system" ? t("language.system") : LANGUAGE_LABELS[preference]}
                </option>
              ))}
            </Select>
          </SettingsRow>
          <SettingsRow
            title={t("appearance.title")}
            description={t("appearance.description")}
          >
            <Select
              size="sm"
              value={themeMode}
              wrapperClassName="w-32"
              onChange={e => setThemeMode(e.target.value as ThemeMode)}
            >
              {THEME_MODES.map(mode => (
                <option key={mode} value={mode}>
                  {t(`appearance.${mode}`)}
                </option>
              ))}
            </Select>
          </SettingsRow>
          {/* The palette is a second, independent axis: it works in both
            renditions, so it gets its own row instead of a combined list. The
            picker owns that row itself — it is the one setting whose shape
            changes with its own state (compact value beside the label until the
            catalogue opens, then a grid under it). */}
          <ThemePalettePicker />
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
            title={t("replyDetail.title")}
            description={t(`replyDetail.description.${replyDetail}`)}
          >
            <Select
              size="sm"
              value={replyDetail}
              wrapperClassName="w-32"
              onChange={e => setReplyDetail(e.target.value as ReplyDetail)}
            >
              {REPLY_DETAIL_LEVELS.map(level => (
                <option key={level} value={level}>
                  {t(`replyDetail.${level}`)}
                </option>
              ))}
            </Select>
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      {/* The completion bell is this machine's own notification: the sound plays
          here, a paired phone never sees the row (it has its own reminder
          switch), and the agent does not know it exists. It sat on the replies
          page for a while, which read as something a reply does — but a reply is
          the same on every client, and this is not. */}
      <SettingsSection title={t("alerts.section")}>
        <SettingsList>
          <SettingsRow
            title={t("bellOnComplete.title")}
            description={t("bellOnComplete.description")}
          >
            <Switch checked={bellOnComplete} label={t("bellOnComplete.title")} onChange={onToggleBellOnComplete} />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
