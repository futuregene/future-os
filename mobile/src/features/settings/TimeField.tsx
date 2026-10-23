import { ChevronDown } from "lucide-react-native";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { layout, radius, spacing } from "../../theme/tokens";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";

/** A two-digit clock value, or `null` when the text is not `HH:MM`. */
function splitTime(value: string): { hour: number; minute: number } | null {
  const match = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  if (hour > 23 || minute > 59) return null;
  return { hour, minute };
}

const pad = (value: number) => String(value).padStart(2, "0");
const HOURS = Array.from({ length: 24 }, (_, hour) => hour);
const MINUTES = Array.from({ length: 60 }, (_, minute) => minute);
/** Width of one chip, and the distance between two of them (width + gap). */
const CHIP_WIDTH = 48;
const CHIP_STRIDE = CHIP_WIDTH + 4;

/**
 * A wall-clock time chosen from a picker instead of typed.
 *
 * The desktop's form is an `<input type="time">`, so its own picker is the
 * operating system's; the phone has no such element, and the typed field it had
 * was both awkward on a phone keyboard and impossible to reach once the
 * keyboard covered the bottom of the form. This is the same choice in two rows
 * of chips: every hour, every minute, no keyboard.
 *
 * It edits the draft directly (each tap rewrites the `HH:MM` string), so the
 * picker can stay open while the user moves from hour to minute, and closing it
 * is its own button rather than a tap on the value that has just been chosen.
 */
export function TimeField({
  label,
  value,
  disabled = false,
  onChange,
}: {
  label: string;
  /** The draft's time as `HH:MM`; anything unreadable opens at 09:00. */
  value: string;
  disabled?: boolean;
  onChange(value: string): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const parsed = splitTime(value) ?? { hour: 9, minute: 0 };
  const shown = `${pad(parsed.hour)}:${pad(parsed.minute)}`;
  const pick = (hour: number, minute: number) => onChange(`${pad(hour)}:${pad(minute)}`);

  return (
    <View style={styles.field}>
      <Pressable
        accessibilityHint={t("tasks.form.pickTime")}
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
        <Text style={styles.value}>{shown}</Text>
        <ChevronDown color={colors.inkMuted} size={18} />
      </Pressable>
      {open
        ? (
            <View style={styles.panel}>
              <Text style={styles.groupLabel}>{t("tasks.form.hour")}</Text>
              <View style={styles.hourGrid}>
                {HOURS.map(hour => (
                  <Chip
                    disabled={disabled}
                    key={hour}
                    label={t("tasks.form.hourOption", { hour: pad(hour) })}
                    onPress={() => pick(hour, parsed.minute)}
                    selected={parsed.hour === hour}
                    text={pad(hour)}
                  />
                ))}
              </View>
              <Text style={styles.groupLabel}>{t("tasks.form.minute")}</Text>
              {/* Opens on the stored minute: 60 chips do not fit one phone width,
                  and starting at :00 would make a stored :59 a long swipe away. */}
              <ScrollView
                contentOffset={{ x: Math.max(0, parsed.minute * CHIP_STRIDE - CHIP_STRIDE), y: 0 }}
                horizontal
                keyboardShouldPersistTaps="handled"
                showsHorizontalScrollIndicator={false}
              >
                <View style={styles.minuteRow}>
                  {MINUTES.map(minute => (
                    <Chip
                      disabled={disabled}
                      key={minute}
                      label={t("tasks.form.minuteOption", { minute: pad(minute) })}
                      onPress={() => pick(parsed.hour, minute)}
                      selected={parsed.minute === minute}
                      text={pad(minute)}
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
  disabled: { opacity: 0.5 },
  pressed: { opacity: 0.75 },
  panel: { gap: spacing.sm, padding: spacing.md, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surfaceSubtle },
  groupLabel: { color: theme.colors.inkMuted, fontSize: theme.font(12), fontWeight: "600" },
  // Wraps to as many 48pt chips as the panel holds (six on a 390pt phone, five
  // on a 320pt one), so no tap target falls below the 44pt minimum and 24 hours
  // never need scrolling.
  hourGrid: { flexDirection: "row", flexWrap: "wrap", gap: spacing.xs },
  minuteRow: { flexDirection: "row", gap: spacing.xs, paddingRight: spacing.md },
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
