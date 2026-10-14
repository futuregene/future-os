import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { FONT_SIZE_PREFERENCE_ORDER, FONT_SIZE_SCALE, layout, radius, spacing } from "./tokens";
import { getAppearance, setAppearance, type Appearance, type ThemeMode } from "./preferences";
import { createThemedStyles, useAppearance, useTheme } from "./ThemeContext";

/** The three schemes, in the order the reader meets them. */
const THEME_MODES: readonly ThemeMode[] = ["system", "light", "dark"];

/**
 * This phone's appearance: the scheme and the text size.
 *
 * Both are local to the phone — no desktop is read or written — and both are
 * stored before they are applied, so a choice that cannot be saved leaves the
 * screen on the value the store actually holds.
 */
export function AppearanceSettings() {
  const styles = useStyles();
  const theme = useTheme();
  const { t } = useTranslation();
  const { themeMode, fontSize } = useAppearance();
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);

  const select = async (patch: Partial<Appearance>) => {
    if (saving) return;
    const current = getAppearance();
    if (Object.entries(patch).every(([key, value]) => current[key as keyof Appearance] === value)) return;
    setSaving(true);
    setFailed(false);
    try {
      await setAppearance(patch);
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <View style={styles.section}>
      <Text accessibilityRole="header" style={styles.label}>{t("appearance.theme")}</Text>
      <View accessibilityRole="radiogroup" style={styles.options}>
        {THEME_MODES.map(value => (
          <Pressable
            accessibilityLabel={t(`appearance.themeMode.${value}`)}
            accessibilityRole="radio"
            accessibilityState={{ checked: themeMode === value, disabled: saving }}
            disabled={saving}
            key={value}
            onPress={() => void select({ themeMode: value })}
            style={({ pressed }) => [styles.option, themeMode === value && styles.selected, (pressed || saving) && styles.dimmed]}
          >
            <Text style={styles.optionText}>{t(`appearance.themeMode.${value}`)}</Text>
          </Pressable>
        ))}
      </View>
      <Text accessibilityRole="header" style={styles.label}>{t("appearance.fontSize")}</Text>
      <View accessibilityRole="radiogroup" style={styles.options}>
        {FONT_SIZE_PREFERENCE_ORDER.map(value => (
          <Pressable
            accessibilityLabel={t(`appearance.fontSizeName.${value}`)}
            accessibilityRole="radio"
            accessibilityState={{ checked: fontSize === value, disabled: saving }}
            disabled={saving}
            key={value}
            onPress={() => void select({ fontSize: value })}
            style={({ pressed }) => [styles.option, fontSize === value && styles.selected, (pressed || saving) && styles.dimmed]}
          >
            {/* Each chip is set at the size it picks, so the choice reads as a
                size and not only as a word. The demo keeps its proportions at
                every choice — the page's own typesetting still scales with the
                reader. */}
            <Text style={[styles.optionText, { fontSize: theme.font(SAMPLE_SIZE * FONT_SIZE_SCALE[value]) }]}>
              {t(`appearance.fontSizeName.${value}`)}
            </Text>
          </Pressable>
        ))}
      </View>
      {failed ? <Text accessibilityRole="alert" style={styles.error}>{t("appearance.saveFailed")}</Text> : null}
    </View>
  );
}

/** The size the demo chips are drawn at before their own scale is applied. */
const SAMPLE_SIZE = 14;

const useStyles = createThemedStyles(theme => StyleSheet.create({
  section: { gap: spacing.sm },
  label: { color: theme.colors.inkMuted, fontSize: theme.font(13), fontWeight: "600" },
  options: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
  option: {
    minHeight: layout.touchTarget,
    justifyContent: "center",
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
  },
  selected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  dimmed: { opacity: 0.6 },
  optionText: { color: theme.colors.ink },
  error: { color: theme.colors.danger, fontSize: theme.font(13) },
}));
