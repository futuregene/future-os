import { Pencil, X } from "lucide-react-native";
import {
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { TFunction } from "i18next";
import { SafeAreaView } from "react-native-safe-area-context";
import type { RemoteSessionUsage } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
import { formatCostCny } from "../utils";

/**
 * The conversation's own account book, opened from the amount in the chat top
 * bar: the session title (with the rename entry that used to sit in the top
 * bar) plus the token/amount breakdown.
 *
 * The rows are priced per category by the agent from the model's rates; the
 * total is the agent's authoritative figure when the provider bills itself
 * (the Future platform), so the rows are an estimate and need not add up to it
 * exactly. An unpriced model reports zeros — then rows show tokens only.
 */
export function SessionUsageSheet({
  title,
  usage,
  visible,
  onClose,
  onRename,
  t,
}: {
  title: string;
  usage: RemoteSessionUsage | null;
  visible: boolean;
  onClose: () => void;
  onRename: () => void;
  t: TFunction;
}) {
  const priced = usage
    ? usage.costInputCny + usage.costOutputCny + usage.costCacheReadCny + usage.costCacheWriteCny > 0
    : false;
  // `inputTokens` includes the cached subset; bill the remainder as plain input
  // so the categories line up with the same token accounting the agent used.
  const uncachedInput = usage
    ? Math.max(0, usage.inputTokens - usage.cacheReadTokens - usage.cacheWriteTokens)
    : 0;
  const formatCount = (value: number) => new Intl.NumberFormat(undefined).format(value);
  const rows = usage
    ? [
        { label: t("chat.usageInput"), tokens: uncachedInput, cost: usage.costInputCny },
        { label: t("chat.usageOutput"), tokens: usage.outputTokens, cost: usage.costOutputCny },
        { label: t("chat.usageCacheRead"), tokens: usage.cacheReadTokens, cost: usage.costCacheReadCny },
        { label: t("chat.usageCacheWrite"), tokens: usage.cacheWriteTokens, cost: usage.costCacheWriteCny },
      ]
    : [];

  return (
    <Modal animationType="slide" onRequestClose={onClose} transparent visible={visible}>
      <SafeAreaView style={styles.overlay}>
        <Pressable accessible={false} onPress={onClose} style={StyleSheet.absoluteFill} />
        <View accessibilityViewIsModal style={styles.sheet}>
          <View style={styles.header}>
            <Text accessibilityRole="header" style={styles.title} numberOfLines={2}>
              {t("chat.usageTitle")}
            </Text>
            <Pressable
              accessibilityLabel={t("common.close")}
              accessibilityRole="button"
              onPress={onClose}
              style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
            >
              <X color={colors.inkSoft} size={20} />
            </Pressable>
          </View>
          <ScrollView bounces={false} contentContainerStyle={styles.body}>
            {/* The rename entry that used to live in the top bar now lives
                here, beside the title it edits. */}
            <View style={styles.subject}>
              <Text numberOfLines={2} style={styles.subjectTitle}>{title}</Text>
              <Pressable
                accessibilityLabel={t("chat.rename")}
                accessibilityRole="button"
                onPress={onRename}
                style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
              >
                <Pencil color={colors.ink} size={18} />
              </Pressable>
            </View>

            {usage ? (
              <View style={styles.table}>
                <View style={[styles.row, styles.headRow]}>
                  <Text style={[styles.headCell, styles.grow]}>{t("chat.usageCategory")}</Text>
                  <Text style={[styles.headCell, styles.numberCell]}>{t("chat.usageTokens")}</Text>
                  <Text style={[styles.headCell, styles.numberCell]}>{t("chat.usageCost")}</Text>
                </View>
                {rows.map(row => (
                  <View key={row.label} style={styles.row}>
                    <Text style={[styles.cell, styles.grow]}>{row.label}</Text>
                    <Text style={[styles.cell, styles.numberCell]}>{formatCount(row.tokens)}</Text>
                    <Text style={[styles.cell, styles.numberCell, styles.muted]}>
                      {priced ? formatCostCny(row.cost) : "—"}
                    </Text>
                  </View>
                ))}
                <View style={[styles.row, styles.totalRow]}>
                  <Text style={[styles.totalCell, styles.grow]}>{t("chat.usageTotal")}</Text>
                  <Text style={[styles.totalCell, styles.numberCell]} />
                  <Text style={[styles.totalCell, styles.numberCell]}>{formatCostCny(usage.costCny)}</Text>
                </View>
              </View>
            ) : (
              <Text style={styles.hint}>{t("chat.usageUnavailable")}</Text>
            )}
            {usage ? (
              <Text style={styles.hint}>
                {priced ? t("chat.usageHintPriced") : t("chat.usageHintUnpriced")}
              </Text>
            ) : null}
          </ScrollView>
        </View>
      </SafeAreaView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  overlay: {
    flex: 1,
    justifyContent: "flex-end",
    padding: layout.gutter,
    backgroundColor: colors.overlay,
  },
  sheet: {
    width: "100%",
    maxWidth: layout.formMaxWidth,
    alignSelf: "center",
    maxHeight: "85%",
    overflow: "hidden",
    padding: spacing.sm,
    borderRadius: radius.xl,
    backgroundColor: colors.surface,
  },
  header: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.md, marginBottom: spacing.sm },
  title: { flex: 1, paddingVertical: spacing.sm, color: colors.inkStrong, fontSize: 18, fontWeight: "700" },
  iconButton: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
  },
  pressed: { backgroundColor: colors.surfaceSubtle },
  body: { gap: spacing.sm, paddingBottom: spacing.sm },
  subject: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingLeft: spacing.md,
  },
  subjectTitle: { flex: 1, color: colors.inkStrong, fontSize: 15, fontWeight: "600" },
  table: { borderRadius: radius.md, overflow: "hidden", borderWidth: 1, borderColor: colors.line },
  row: { flexDirection: "row", alignItems: "center", gap: spacing.sm, paddingHorizontal: spacing.md, paddingVertical: spacing.sm },
  headRow: { backgroundColor: colors.surfaceSubtle },
  totalRow: { borderTopWidth: 1, borderTopColor: colors.line, backgroundColor: colors.surfaceSubtle },
  headCell: { color: colors.inkMuted, fontSize: 12, fontWeight: "600" },
  cell: { color: colors.ink, fontSize: 14 },
  totalCell: { color: colors.inkStrong, fontSize: 14, fontWeight: "600" },
  muted: { color: colors.inkSoft },
  grow: { flex: 1, minWidth: 0 },
  numberCell: { minWidth: 78, textAlign: "right" },
  hint: { paddingHorizontal: spacing.md, color: colors.inkMuted, fontSize: 12, lineHeight: 18 },
});
