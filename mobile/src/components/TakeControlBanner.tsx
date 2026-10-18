import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { Button } from "./Button";

/**
 * "Another device is driving this desktop."
 *
 * The desktop does not hand the machine to whoever connects — connecting is not
 * taking over. A second device comes up in standby and the phone says who holds
 * it; only this button moves control, and only after the person holding the
 * phone has read what that costs. It is deliberately not a dialog: the user may
 * well want to read the conversation first, and a banner can wait while a modal
 * cannot.
 */
export function TakeControlBanner({
  holder,
  onTakeControl,
}: {
  /** The device holding the desktop, as it described itself. */
  holder?: { name?: string } | null;
  onTakeControl: () => Promise<void> | void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const name = holder?.name?.trim();
  const title = name
    ? t("connection.controlHeldBy", { name })
    : t("connection.controlHeldByUnknown");

  const take = async () => {
    if (busy) return;
    setBusy(true);
    setFailed(false);
    try {
      await onTakeControl();
    }
    catch {
      setFailed(true);
    }
    finally {
      setBusy(false);
    }
  };

  return (
    <View accessibilityLiveRegion="polite" style={styles.banner}>
      <View style={styles.copy}>
        <Text style={styles.title}>{title}</Text>
        <Text style={styles.hint}>
          {failed ? t("connection.takeControlFailed") : t("connection.takeControlHint")}
        </Text>
      </View>
      <Button
        disabled={busy}
        label={t("connection.takeControl")}
        onPress={() => void take()}
        variant="secondary"
      />
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  banner: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    marginHorizontal: layout.gutter,
    marginBottom: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
    backgroundColor: theme.colors.warningSoft,
    borderWidth: 1,
    borderColor: theme.colors.warning,
  },
  copy: { flex: 1 },
  title: { color: theme.colors.ink, fontSize: theme.font(13), fontWeight: "600" },
  hint: { color: theme.colors.inkMuted, fontSize: theme.font(12), marginTop: spacing.xs },
}));
