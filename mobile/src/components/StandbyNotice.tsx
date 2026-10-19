import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { useState } from "react";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles } from "../theme/ThemeContext";

/**
 * One line where the sending happens: this desktop is taken over, so nothing
 * sent here will arrive.
 *
 * The status dot explains the refusal, but only to someone who thinks to tap it.
 * In a conversation the user is looking at the composer, not the header, so the
 * reason has to be where the send button is — and with the way out beside it,
 * because "you cannot send" without "here is what to do" is the same dead end in
 * a different place.
 */
export function StandbyNotice({
  holder,
  onTakeControl,
}: {
  holder?: { name?: string } | null;
  onTakeControl: () => Promise<void> | void;
}) {
  const styles = useStyles();
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const name = holder?.name?.trim();

  const take = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await onTakeControl();
    }
    finally {
      setBusy(false);
    }
  };

  return (
    <View accessibilityLiveRegion="polite" style={styles.row}>
      <Text numberOfLines={2} style={styles.text}>
        {name ? t("chat.standbyCannotSend", { name }) : t("chat.standbyCannotSendUnknown")}
      </Text>
      <Pressable
        accessibilityLabel={t("connection.takeControl")}
        accessibilityRole="button"
        disabled={busy}
        onPress={() => void take()}
        style={({ pressed }) => [styles.action, pressed && styles.pressed]}
      >
        <Text style={styles.actionLabel}>
          {busy ? "…" : t("connection.takeControl")}
        </Text>
      </Pressable>
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    marginHorizontal: layout.gutter,
    marginBottom: spacing.xs,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.xs,
    borderRadius: radius.md,
    backgroundColor: theme.colors.warningSoft,
    borderWidth: 1,
    borderColor: theme.colors.warning,
  },
  text: { flex: 1, minWidth: 0, color: theme.colors.ink, fontSize: theme.font(12) },
  action: {
    minHeight: 28,
    justifyContent: "center",
    paddingHorizontal: spacing.md,
    borderRadius: radius.pill,
    borderWidth: 1,
    borderColor: theme.colors.warning,
  },
  actionLabel: { color: theme.colors.ink, fontSize: theme.font(12), fontWeight: "600" },
  pressed: { opacity: 0.7 },
}));
