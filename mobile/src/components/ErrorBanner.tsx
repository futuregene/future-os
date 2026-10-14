import { X } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { friendlyError } from "./errorMessage";

/**
 * Map a raw transport/backend error onto a localized, human-readable message.
 * The connection/operation layer records raw codes and server strings
 * ("503", "HTTP 503", "…unavailable while the Agent is offline") — surfacing
 * them verbatim (audit 05 L5) tells the user nothing; this is the single
 * presentation-side translation layer. Unknown details stay in the console
 * and collapse to a stable generic action here. Screens render this banner
 * only while connected, for operation failures such as opening, sending, or
 * deleting. Connection lifecycle errors belong to the shared status page.
 */
export function ErrorBanner({ message, onDismiss }: { message: string; onDismiss?: () => void }) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  return (
    <View accessibilityLiveRegion="polite" style={styles.banner}>
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
    backgroundColor: theme.colors.dangerSoft,
    borderWidth: 1,
    borderColor: theme.colors.dangerLine,
  },
  text: { flex: 1, color: theme.colors.danger, fontSize: theme.font(13), lineHeight: theme.leading(18) },
  dismiss: { width: layout.touchTarget, minHeight: layout.touchTarget, alignItems: "center", justifyContent: "center" },
}));
