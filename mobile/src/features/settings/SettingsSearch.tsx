import { Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { useTheme } from "../../theme/ThemeContext";
import type { TitledSettingsPage } from "./settingsPages";
import { settingsPageTitleKey, type MobileSettingsMatch } from "./settingsIndex";
import { SettingsLink, SettingsSection, useSettingsStyles } from "./SettingsPrimitives";

/**
 * The one field both homes open with.
 *
 * A filter rather than a second screen: a home lists at most eight pages, so
 * typing narrows the list in place and a result opens the page the reader would
 * have tapped anyway.
 */
export function SettingsSearchField({ value, onChange }: { value: string; onChange(value: string): void }) {
  const { t } = useTranslation();
  const { colors } = useTheme();
  const settingsStyles = useSettingsStyles();
  return <TextInput
    accessibilityLabel={t("desktopSettings.search")}
    autoCapitalize="none"
    autoCorrect={false}
    onChangeText={onChange}
    placeholder={t("desktopSettings.searchPlaceholder")}
    placeholderTextColor={colors.inkMuted}
    returnKeyType="search"
    style={settingsStyles.search}
    value={value}
  />;
}

/** Matches in place of the home's sections, each naming the page it opens. */
export function SettingsSearchResults({ matches, onOpen }: {
  matches: readonly MobileSettingsMatch[];
  onOpen(page: TitledSettingsPage): void;
}) {
  const { t } = useTranslation();
  const settingsStyles = useSettingsStyles();
  if (!matches.length)
    return <View><Text style={settingsStyles.description}>{t("desktopSettings.searchEmpty")}</Text></View>;
  return <SettingsSection title={t("desktopSettings.searchResults")}>
    {matches.map(match => <SettingsLink
      key={`${match.page}:${match.labelKey}`}
      // A page's own entry has no row of its own, so its name must not be
      // printed twice on one line.
      description={match.labelKey === settingsPageTitleKey(match.page) ? undefined : t(settingsPageTitleKey(match.page))}
      label={t(match.labelKey)}
      onPress={() => onOpen(match.page)}
    />)}
  </SettingsSection>;
}
