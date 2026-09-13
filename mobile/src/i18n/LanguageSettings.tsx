import { useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { colors, layout, radius, spacing } from "../theme/tokens";
import type { LanguagePreference } from "./index";
import { getLanguagePreference, setLanguagePreference, subscribeLanguagePreference } from "./preferences";

export function LanguageSettings() {
  const { t } = useTranslation();
  const preference = useSyncExternalStore(subscribeLanguagePreference, getLanguagePreference);
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);

  const select = async (value: LanguagePreference) => {
    if (saving || value === preference) return;
    setSaving(true);
    setFailed(false);
    try {
      await setLanguagePreference(value);
    } catch {
      setFailed(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <View style={styles.section}>
      <Text accessibilityRole="header" style={styles.label}>{t("language.title")}</Text>
      <View accessibilityRole="radiogroup" style={styles.options}>
        {(["system", "zh", "en"] as const).map(value => (
          <Pressable
            key={value}
            accessibilityRole="radio"
            accessibilityLabel={t(`language.${value}`)}
            accessibilityState={{ checked: preference === value, disabled: saving }}
            disabled={saving}
            onPress={() => void select(value)}
            style={({ pressed }) => [styles.option, preference === value && styles.selected, (pressed || saving) && styles.dimmed]}
          >
            <Text style={styles.optionText}>{t(`language.${value}`)}</Text>
          </Pressable>
        ))}
      </View>
      {failed && <Text accessibilityRole="alert" style={styles.error}>{t("language.saveFailed")}</Text>}
    </View>
  );
}

const styles = StyleSheet.create({
  section: { gap: spacing.sm },
  label: { color: colors.inkMuted, fontSize: 13, fontWeight: "600" },
  options: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
  option: {
    minHeight: layout.touchTarget,
    justifyContent: "center",
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  selected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  dimmed: { opacity: 0.6 },
  optionText: { color: colors.ink, fontSize: 14 },
  error: { color: colors.danger, fontSize: 13 },
});
