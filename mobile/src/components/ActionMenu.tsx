import { useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import {
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Search, X } from "lucide-react-native";
import { colors, layout, radius, spacing } from "../theme/tokens";

export interface MenuAction {
  label: string;
  icon?: ReactNode;
  destructive?: boolean;
  disabled?: boolean;
  /** Navigate within this sheet without discarding its pending payload. */
  keepOpen?: boolean;
  /** A label for the actions under it, not a choice of its own: the share
   * sheet's workspace tree, where conversations sit under their workspace. */
  heading?: boolean;
  /** Filed under the heading above it, so the tree's levels read as levels. */
  nested?: boolean;
  onPress?(): void;
}

/** Filter field for a long action list. The caller owns the query and does the
 * filtering, so the sheet stays a plain presenter of whatever it is handed. */
export interface MenuSearch {
  value: string;
  label: string;
  placeholder: string;
  onChangeText(value: string): void;
}

/** App-styled action sheet. Release the native modal before navigation or an alert. */
export function ActionMenu({ title, visible, actions, onClose, onBack, search }: {
  title: string;
  visible: boolean;
  actions: MenuAction[];
  onClose(): void;
  /** Pop an inner step on system back; explicit cancel still closes the sheet. */
  onBack?(): void;
  search?: MenuSearch;
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
    <Modal transparent animationType="slide" visible={visible} onRequestClose={onBack ?? dismiss} onDismiss={flush}>
      <SafeAreaView style={styles.overlay}>
        <Pressable accessible={false} style={StyleSheet.absoluteFill} onPress={dismiss} />
        <View accessibilityViewIsModal onAccessibilityEscape={onBack ?? dismiss} style={styles.menu}>
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
          {search && (
            <View style={styles.search}>
              <Search size={16} color={colors.inkMuted} />
              <TextInput
                accessibilityLabel={search.label}
                placeholder={search.placeholder}
                placeholderTextColor={colors.inkMuted}
                value={search.value}
                onChangeText={search.onChangeText}
                returnKeyType="search"
                autoCapitalize="none"
                autoCorrect={false}
                style={styles.searchInput}
              />
              {!!search.value && (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={t("sessions.clearSearch")}
                  onPress={() => search.onChangeText("")}
                  style={styles.searchClear}
                >
                  <X size={16} color={colors.inkMuted} />
                </Pressable>
              )}
            </View>
          )}
          <ScrollView bounces={false} keyboardShouldPersistTaps="handled" contentContainerStyle={styles.options}>
            {titleExpanded && <Text selectable={Platform.OS === "ios"} style={styles.fullTitle}>{title}</Text>}
            {actions.map((action, index) => action.heading ? (
              <View key={`${index}:${action.label}`} accessibilityRole="header" style={styles.groupHeading}>
                {action.icon}
                <Text numberOfLines={1} style={styles.headingLabel}>{action.label}</Text>
              </View>
            ) : (
              <Pressable
                key={`${index}:${action.label}`}
                accessibilityRole="button"
                accessibilityLabel={action.label}
                accessibilityState={{ disabled: !!action.disabled }}
                disabled={action.disabled}
                onPress={() => {
                  if (!action.onPress || pending.current) return;
                  if (action.keepOpen) {
                    action.onPress();
                    return;
                  }
                  pending.current = action.onPress;
                  onClose();
                  if (Platform.OS !== "ios") setTimeout(flush, 0);
                }}
                style={({ pressed }) => [
                  styles.option,
                  action.nested && styles.nested,
                  pressed && styles.pressed,
                  action.disabled && styles.disabled,
                ]}
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
  search: {
    flexDirection: "row",
    alignItems: "center",
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
    paddingLeft: spacing.md,
    borderRadius: radius.lg,
    backgroundColor: colors.surfaceSubtle,
  },
  searchInput: { flex: 1, minWidth: 0, minHeight: 44, paddingHorizontal: spacing.sm, paddingVertical: spacing.sm, fontSize: 14, color: colors.ink },
  searchClear: { width: 44, minHeight: 44, alignItems: "center", justifyContent: "center" },
  options: { gap: spacing.xs, paddingBottom: spacing.sm },
  option: { minHeight: 56, flexDirection: "row", alignItems: "center", gap: spacing.md, paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderRadius: radius.md },
  // The group's own row: a label, not a choice, so it is lighter than an option
  // and carries no touch target of its own.
  groupHeading: { minHeight: 32, flexDirection: "row", alignItems: "center", gap: spacing.md, marginTop: spacing.sm, paddingHorizontal: spacing.md },
  headingLabel: { flexShrink: 1, color: colors.inkSoft, fontSize: 13, fontWeight: "700" },
  // Start a nested row where its heading's label starts — the option's inset
  // plus the heading's icon column (18) and gap — so the tree's levels line up
  // the way they do in the session list. See `option` and `groupHeading`.
  nested: { paddingLeft: spacing.md + 18 + spacing.md },
  label: { flexShrink: 1, color: colors.ink, fontSize: 15, fontWeight: "600" },
  cancel: { color: colors.inkMuted, fontSize: 15 },
  danger: { color: colors.danger },
  pressed: { backgroundColor: colors.surfaceSubtle },
  disabled: { opacity: 0.4 },
});
