import { X } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { colors, radius, spacing } from "../theme/tokens";

/**
 * Map a raw transport/backend error onto a localized, human-readable message.
 * The connection/operation layer records raw codes and server strings
 * ("503", "HTTP 503", "…unavailable while the Agent is offline") — surfacing
 * them verbatim (audit 05 L5) tells the user nothing; this is the single
 * presentation-side translation layer. Unknown errors keep their raw text as
 * the interpolation so debugging context survives.
 */
function friendlyError(
  message: string,
  t: (key: string, opts?: Record<string, unknown>) => string,
): string {
  const trimmed = message.trim();
  const code = /^(?:HTTP\s*)?(\d{3})$/.exec(trimmed)?.[1];
  if (code) {
    if (code === "401" || code === "403") return t("connection.errorAuth");
    if (code === "404") return t("connection.errorNotFound");
    if (code === "429") return t("connection.errorRateLimit");
    if (code.startsWith("5")) return t("connection.errorService");
  }
  if (/agent.*(offline|unavailable)|history is unavailable/i.test(trimmed)) {
    return t("connection.errorAgentOffline");
  }
  if (/time-?out|timed out/i.test(trimmed)) return t("connection.errorTimeout");
  if (/network|unreachable|load failed|fetch failed|econn|connection (refused|reset)/i.test(trimmed)) {
    return t("connection.errorNetwork");
  }
  return t("connection.errorWithReason", { reason: message });
}

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
      <Text style={styles.text}>{friendlyError(message, t)}</Text>
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
