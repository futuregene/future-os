import { ChevronDown } from "lucide-react-native";
import { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { layout, radius, spacing } from "../../theme/tokens";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";
import { matchTimezones } from "./timezones";

/**
 * The zone a scheduled task's trigger times are read on.
 *
 * The zone belongs to the schedule, not to the task, so this sits inside the
 * trigger and only for a mode that reads a clock — the same place the desktop
 * asks for it. Empty is not "no zone": it means "whatever the desktop running
 * the task is set to", which is why it is offered as its own row rather than as
 * a blank field.
 *
 * Every zone the runtime knows is listed, because a task may be scheduled for a
 * machine the user is not carrying: the phone's own zone is not the answer, and
 * four fixed choices were not a list. The search field narrows it, and the rows
 * live in a bounded scroll view — a fixed short prefix would have hidden every
 * zone past the letter E, which is where most of them are.
 */
export function TimezoneField({
  label,
  hint,
  value,
  disabled = false,
  onChange,
}: {
  label: string;
  hint: string;
  /** Empty means the desktop's own zone. */
  value: string;
  disabled?: boolean;
  onChange(value: string): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const current = value.trim();
  const matches = useMemo(() => matchTimezones(query, current), [query, current]);
  const shown = current || t("tasks.form.timezoneDesktop");

  return (
    <View style={styles.field}>
      <Pressable
        accessibilityHint={hint}
        accessibilityLabel={label}
        accessibilityRole="button"
        accessibilityState={{ disabled, expanded: open }}
        accessibilityValue={{ text: shown }}
        disabled={disabled}
        onPress={() => setOpen(current => !current)}
        style={({ pressed }) => [
          styles.trigger,
          pressed && styles.pressed,
          disabled && styles.disabled,
        ]}
      >
        <Text numberOfLines={1} style={[styles.value, !current && styles.unset]}>{shown}</Text>
        <ChevronDown color={colors.inkMuted} size={18} />
      </Pressable>
      <Text style={styles.hint}>{hint}</Text>
      {open
        ? (
            <View style={styles.panel}>
              <TextInput
                accessibilityLabel={t("tasks.form.timezoneSearch")}
                autoCapitalize="none"
                autoCorrect={false}
                onChangeText={setQuery}
                placeholder={t("tasks.form.timezonePlaceholder")}
                placeholderTextColor={colors.inkMuted}
                style={styles.search}
                value={query}
              />
              <ScrollView keyboardShouldPersistTaps="handled" style={styles.list}>
                <Option
                  disabled={disabled}
                  label={t("tasks.form.timezoneDesktop")}
                  onPress={() => onChange("")}
                  selected={current === ""}
                  text={t("tasks.form.timezoneDesktop")}
                />
                {matches.map(zone => (
                  <Option
                    disabled={disabled}
                    key={zone}
                    label={zone}
                    onPress={() => onChange(zone)}
                    selected={current === zone}
                    text={zone}
                  />
                ))}
                {matches.length === 0
                  ? <Text style={styles.empty}>{t("tasks.form.timezoneNoMatch")}</Text>
                  : null}
              </ScrollView>
              <View style={styles.actions}>
                <Button compact label={t("tasks.form.done")} onPress={() => setOpen(false)} variant="secondary" />
              </View>
            </View>
          )
        : null}
    </View>
  );
}

function Option({ label, text, selected, disabled, onPress }: {
  label: string;
  text: string;
  selected: boolean;
  disabled: boolean;
  onPress(): void;
}) {
  const styles = useStyles();
  return (
    <Pressable
      accessibilityLabel={label}
      accessibilityRole="radio"
      accessibilityState={{ checked: selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [styles.option, selected && styles.optionSelected, pressed && styles.pressed]}
    >
      <Text style={[styles.optionText, selected && styles.optionTextSelected]}>{text}</Text>
    </Pressable>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  field: { gap: spacing.xs },
  trigger: {
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
  },
  value: { flex: 1, color: theme.colors.inkStrong, fontSize: theme.font(15) },
  unset: { color: theme.colors.inkMuted },
  hint: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  disabled: { opacity: 0.5 },
  pressed: { opacity: 0.75 },
  panel: { gap: spacing.sm, padding: spacing.md, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surfaceSubtle },
  search: {
    minHeight: layout.touchTarget,
    paddingHorizontal: spacing.md,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
    color: theme.colors.inkStrong,
    fontSize: theme.font(15),
  },
  // A bounded window rather than the whole zone database: the panel sits inside
  // a form that is already scrolling, and a nested unbounded list would fight it.
  list: { maxHeight: 220 },
  option: { minHeight: layout.touchTarget, justifyContent: "center", paddingHorizontal: spacing.md, borderRadius: radius.md },
  optionSelected: { backgroundColor: theme.colors.accentSoft },
  optionText: { color: theme.colors.ink, fontSize: theme.font(14) },
  optionTextSelected: { color: theme.colors.accent, fontWeight: "600" },
  empty: { color: theme.colors.inkMuted, padding: spacing.md, fontSize: theme.font(13) },
  actions: { flexDirection: "row", justifyContent: "flex-end", gap: spacing.sm },
}));
