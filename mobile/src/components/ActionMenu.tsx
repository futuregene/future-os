import { useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Platform, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { X } from "lucide-react-native";
import { colors, layout, radius, spacing } from "../theme/tokens";

export interface MenuAction {
  label: string;
  icon?: ReactNode;
  destructive?: boolean;
  disabled?: boolean;
  /** Navigate within this sheet without discarding its pending payload. */
  keepOpen?: boolean;
  onPress(): void;
}

/** App-styled action sheet. Release the native modal before navigation or an alert. */
export function ActionMenu({ title, visible, actions, onClose }: {
  title: string;
  visible: boolean;
  actions: MenuAction[];
  onClose(): void;
}) {
  const { t } = useTranslation();
  const [expandedTitle, setExpandedTitle] = useState<string | null>(null);
  if (expandedTitle !== null && (!visible || expandedTitle !== title)) setExpandedTitle(null);
  const titleExpanded = expandedTitle !== null && expandedTitle === title;
  const pending = useRef<(() => void) | null>(null);
  const dismiss = () => {
    setExpandedTitle(null);
    pending.current = null;
    onClose();
  };
  const flush = () => {
    const action = pending.current;
    pending.current = null;
    action?.();
  };
  return (
    <Modal transparent animationType="slide" visible={visible} onRequestClose={dismiss} onDismiss={flush}>
      <SafeAreaView style={styles.overlay}>
        <Pressable accessible={false} style={StyleSheet.absoluteFill} onPress={dismiss} />
        <View accessibilityViewIsModal style={styles.menu}>
          <View style={styles.header}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={title}
              accessibilityHint={t(titleExpanded ? "common.hideFullTitle" : "common.showFullTitle")}
              accessibilityState={{ expanded: titleExpanded }}
              onPress={() => setExpandedTitle(titleExpanded ? null : title)}
              style={styles.titleButton}
            >
              <Text accessibilityRole="header" numberOfLines={1} ellipsizeMode="middle" style={styles.title}>{title}</Text>
            </Pressable>
            <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={dismiss} style={styles.close}>
              <X color={colors.inkSoft} size={20} />
            </Pressable>
          </View>
          <ScrollView bounces={false} keyboardShouldPersistTaps="handled" contentContainerStyle={styles.options}>
            {titleExpanded && <Text selectable={Platform.OS === "ios"} style={styles.fullTitle}>{title}</Text>}
            {actions.map((action, index) => (
              <Pressable
                key={`${index}:${action.label}`}
                accessibilityRole="button"
                accessibilityLabel={action.label}
                accessibilityState={{ disabled: !!action.disabled }}
                disabled={action.disabled}
                onPress={() => {
                  if (pending.current) return;
                  if (action.keepOpen) {
                    action.onPress();
                    return;
                  }
                  pending.current = action.onPress;
                  onClose();
                  if (Platform.OS !== "ios") setTimeout(flush, 0);
                }}
                style={({ pressed }) => [styles.option, pressed && styles.pressed, action.disabled && styles.disabled]}
              >
                {action.icon}
                <Text style={[styles.label, action.destructive && styles.danger]}>{action.label}</Text>
              </Pressable>
            ))}
            <Pressable accessibilityRole="button" accessibilityLabel={t("chat.cancel")} onPress={dismiss} style={styles.option}>
              <Text style={styles.cancel}>{t("chat.cancel")}</Text>
            </Pressable>
          </ScrollView>
        </View>
      </SafeAreaView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  overlay: { flex: 1, justifyContent: "flex-end", padding: layout.gutter, backgroundColor: colors.overlay },
  menu: { width: "100%", maxWidth: layout.formMaxWidth, alignSelf: "center", maxHeight: "85%", overflow: "hidden", padding: spacing.sm, borderRadius: radius.xl, backgroundColor: colors.surface },
  header: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.md, marginBottom: spacing.sm },
  titleButton: { flex: 1, minWidth: 0, minHeight: layout.touchTarget, justifyContent: "center", paddingVertical: spacing.sm },
  title: { color: colors.inkStrong, fontSize: 18, fontWeight: "700" },
  fullTitle: { paddingHorizontal: spacing.md, paddingVertical: spacing.sm, color: colors.inkSoft, fontSize: 14, lineHeight: 21 },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  options: { gap: spacing.xs, paddingBottom: spacing.sm },
  option: { minHeight: 56, flexDirection: "row", alignItems: "center", gap: spacing.md, paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderRadius: radius.md },
  label: { flexShrink: 1, color: colors.ink, fontSize: 15, fontWeight: "600" },
  cancel: { color: colors.inkMuted, fontSize: 15 },
  danger: { color: colors.danger },
  pressed: { backgroundColor: colors.surfaceSubtle },
  disabled: { opacity: 0.4 },
});
