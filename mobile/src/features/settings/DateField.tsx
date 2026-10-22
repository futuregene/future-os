import { ChevronDown } from "lucide-react-native";
import { useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { layout, radius, spacing } from "../../theme/tokens";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";

/** A two-digit month/day, or `null` when the text is not `YYYY-MM-DD`. */
function splitDate(value: string): { year: number; month: number; day: number } | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1 || day > daysIn(year, month)) return null;
  return { year, month, day };
}

/** Days in a month, so a picker never offers a 31st to February. */
function daysIn(year: number, month: number): number {
  return new Date(Date.UTC(year, month, 0)).getUTCDate();
}

const pad = (value: number) => String(value).padStart(2, "0");
/** Chips are 48pt wide with a 4pt gap: the stride one of them advances by. */
const CHIP_WIDTH = 48;
const CHIP_STRIDE = CHIP_WIDTH + 4;
/** How far ahead a one-off may be scheduled. */
const YEARS_AHEAD = 6;

/**
 * A calendar date chosen from a picker instead of typed.
 *
 * The desktop's form is an `<input type="date">`, so its picker is the
 * operating system's; the phone has no such element, and a text field with a
 * `2026-12-24` placeholder read as a date already chosen — the same trap the
 * desktop removed. This is the same choice in three rows: year, month, day,
 * no keyboard, and the day row follows the month it belongs to.
 *
 * Each tap rewrites the `YYYY-MM-DD` string, so the draft stays a string like
 * the desktop's own, and closing is its own button rather than a tap on the
 * value that was just picked.
 */
export function DateField({
  label,
  value,
  disabled = false,
  onChange,
}: {
  label: string;
  /** The draft's date as `YYYY-MM-DD`; anything unreadable opens on today. */
  value: string;
  disabled?: boolean;
  onChange(value: string): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const today = useMemo(() => new Date(), []);
  const parsed = splitDate(value) ?? {
    year: today.getFullYear(),
    month: today.getMonth() + 1,
    day: today.getDate(),
  };
  const shown = splitDate(value) ? value.trim() : "";
  const years = useMemo(
    () => Array.from({ length: YEARS_AHEAD }, (_, index) => today.getFullYear() + index),
    [today],
  );
  const pick = (year: number, month: number, day: number) =>
    onChange(`${year}-${pad(month)}-${pad(Math.min(day, daysIn(year, month)))}`);

  return (
    <View style={styles.field}>
      <Pressable
        accessibilityHint={t("tasks.form.pickDate")}
        accessibilityLabel={label}
        accessibilityRole="button"
        accessibilityState={{ disabled, expanded: open }}
        accessibilityValue={{ text: shown || t("tasks.form.dateUnset") }}
        disabled={disabled}
        onPress={() => setOpen(current => !current)}
        style={({ pressed }) => [
          styles.trigger,
          pressed && styles.pressed,
          disabled && styles.disabled,
        ]}
      >
        <Text style={[styles.value, !shown && styles.unset]}>
          {shown || t("tasks.form.dateUnset")}
        </Text>
        <ChevronDown color={colors.inkMuted} size={18} />
      </Pressable>
      {open
        ? (
            <View style={styles.panel}>
              <Text style={styles.groupLabel}>{t("tasks.form.year")}</Text>
              <ScrollView horizontal keyboardShouldPersistTaps="handled" showsHorizontalScrollIndicator={false}>
                <View style={styles.row}>
                  {years.map(year => (
                    <Chip
                      disabled={disabled}
                      key={year}
                      label={t("tasks.form.yearOption", { year })}
                      onPress={() => pick(year, parsed.month, parsed.day)}
                      selected={parsed.year === year}
                      text={String(year)}
                    />
                  ))}
                </View>
              </ScrollView>
              <Text style={styles.groupLabel}>{t("tasks.form.month")}</Text>
              <View style={styles.grid}>
                {Array.from({ length: 12 }, (_, index) => index + 1).map(month => (
                  <Chip
                    disabled={disabled}
                    key={month}
                    label={t("tasks.form.monthOption", { month })}
                    onPress={() => pick(parsed.year, month, parsed.day)}
                    selected={parsed.month === month}
                    text={pad(month)}
                  />
                ))}
              </View>
              <Text style={styles.groupLabel}>{t("tasks.form.dayOfMonth")}</Text>
              <ScrollView
                contentOffset={{ x: Math.max(0, (parsed.day - 1) * CHIP_STRIDE - CHIP_STRIDE), y: 0 }}
                horizontal
                keyboardShouldPersistTaps="handled"
                showsHorizontalScrollIndicator={false}
              >
                <View style={styles.row}>
                  {Array.from({ length: daysIn(parsed.year, parsed.month) }, (_, index) => index + 1).map(day => (
                    <Chip
                      disabled={disabled}
                      key={day}
                      label={t("tasks.form.dayOption", { day })}
                      onPress={() => pick(parsed.year, parsed.month, day)}
                      selected={parsed.day === day}
                      text={pad(day)}
                    />
                  ))}
                </View>
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

function Chip({ label, text, selected, disabled, onPress }: {
  label: string;
  text: string;
  selected: boolean;
  disabled: boolean;
  onPress(): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityLabel={label}
      accessibilityRole="radio"
      accessibilityState={{ selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.chip,
        selected && styles.chipSelected,
        pressed && styles.pressed,
      ]}
    >
      <Text style={[styles.chipText, selected && { color: colors.accent }]}>{text}</Text>
    </Pressable>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  field: { gap: spacing.sm },
  trigger: {
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: spacing.md,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
  },
  value: { color: theme.colors.inkStrong, fontSize: theme.font(15), fontVariant: ["tabular-nums"] },
  unset: { color: theme.colors.inkMuted, fontVariant: undefined },
  disabled: { opacity: 0.5 },
  pressed: { opacity: 0.75 },
  panel: { gap: spacing.sm, padding: spacing.md, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surfaceSubtle },
  groupLabel: { color: theme.colors.inkMuted, fontSize: theme.font(12), fontWeight: "600" },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: spacing.xs },
  row: { flexDirection: "row", gap: spacing.xs, paddingRight: spacing.md },
  chip: {
    width: CHIP_WIDTH,
    minHeight: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
  },
  chipSelected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  chipText: { color: theme.colors.ink, fontSize: theme.font(14), fontVariant: ["tabular-nums"] },
  actions: { flexDirection: "row", justifyContent: "flex-end", gap: spacing.sm },
}));

/** Exported for tests: the date string a picker tap produces. */
export { splitDate };
