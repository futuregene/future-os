import { X } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { colors, radius, spacing } from "../theme/tokens";

/**
 * A dismissible inline banner for the last connection/operation error. The
 * connection FSM records the error (`remote.error`) but no screen rendered it,
 * so a failed `selectSession`/`sendMessage` was silent (audit 05 L5). This is
 * the presentation half of that signal — the error text stays visible until the
 * user dismisses it or the next successful operation clears it.
 */
export function ErrorBanner({
  message,
  onDismiss,
}: {
  message: string;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  return (
    <View style={styles.banner}>
      <Text style={styles.text}>{message}</Text>
      <Pressable
        accessibilityLabel={t("common.close")}
        accessibilityRole="button"
        onPress={onDismiss}
        style={styles.dismiss}
      >
        <X color={colors.danger} size={16} />
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
    backgroundColor: colors.dangerSoft,
    borderWidth: 1,
    borderColor: colors.dangerLine,
  },
  text: { flex: 1, color: colors.danger, fontSize: 13, lineHeight: 18 },
  dismiss: { padding: 2 },
});
