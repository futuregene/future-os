import { X } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { colors, radius, spacing } from "../theme/tokens";
import { friendlyError } from "./errorMessage";

/**
 * Map a raw transport/backend error onto a localized, human-readable message.
 * The connection/operation layer records raw codes and server strings
 * ("503", "HTTP 503", "…unavailable while the Agent is offline") — surfacing
 * them verbatim (audit 05 L5) tells the user nothing; this is the single
 * presentation-side translation layer. Unknown details stay in the console
 * and collapse to a stable generic action here.
 */
/**
 * A dismissible inline banner for the last unexpected error. The connection
 * FSM records the error (`remote.error`) but no screen rendered it, so a
 * failed `selectSession`/`sendMessage` was silent (audit 05 L5). This is the
 * presentation half of that signal — the error text stays visible until the
 * user dismisses it or the connection returns to ready. Expected reachability
 * states (desktop asleep, reconnecting) never reach the banner; the badge and
 * offline empty state are their dedicated UI.
 */
export function ErrorBanner({ message, onDismiss }: { message: string; onDismiss?: () => void }) {
  const { t } = useTranslation();
  return (
    <View style={styles.banner}>
      <Text style={styles.text}>{friendlyError(message, t)}</Text>
      {onDismiss ? (
        <Pressable
          accessibilityLabel={t("common.close")}
          accessibilityRole="button"
          onPress={onDismiss}
          style={styles.dismiss}
        >
          <X color={colors.danger} size={16} />
        </Pressable>
      ) : null}
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
