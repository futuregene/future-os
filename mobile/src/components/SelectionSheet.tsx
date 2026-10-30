import { Check, X } from "lucide-react-native";
import { useEffect, useState } from "react";
import { Keyboard, KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

export interface SelectionSheetOption {
  value: string;
  label: string;
  description?: string;
  searchText?: string;
}
interface SelectionSheetProps {
  visible: boolean;
  title: string;
  closeLabel: string;
  emptyLabel: string;
  options: readonly SelectionSheetOption[];
  selectedValue: string | null;
  onClose(): void;
  onSelect(value: string): void;
  disabled?: boolean;
  searchPlaceholder?: string;
}

/** Shared model/thinking bottom sheet; callers own data and selection effects. */
export function SelectionSheet(props: SelectionSheetProps) {
  useEffect(() => {
    // A Modal has its own window. Clear the composer's IME on opening so the
    // bottom rows remain reachable; searchable sheets can open it again safely.
    if (props.visible) Keyboard.dismiss();
  }, [props.visible]);
  return <Modal animationType="slide" onRequestClose={props.onClose} transparent visible={props.visible}>
    {props.visible ? <SelectionSheetContent {...props} /> : null}
  </Modal>;
}

function SelectionSheetContent({ title, closeLabel, emptyLabel, options, selectedValue, onClose, onSelect, disabled = false, searchPlaceholder }: SelectionSheetProps) {
  const styles = useStyles();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const matches = searchPlaceholder ? options.filter(option => `${option.label} ${option.description ?? ""} ${option.searchText ?? ""}`.toLowerCase().includes(needle)) : options;
  return <KeyboardAvoidingView
    enabled={!!searchPlaceholder}
    behavior={Platform.OS === "ios" ? "padding" : "height"}
    style={[styles.selectorOverlay, {
      paddingTop: insets.top + layout.gutter,
      paddingBottom: insets.bottom + layout.gutter,
      paddingLeft: insets.left + layout.gutter,
      paddingRight: insets.right + layout.gutter,
    }]}
  >
    <Pressable accessible={false} onPress={onClose} style={StyleSheet.absoluteFill} />
    <View accessibilityViewIsModal style={styles.selectorMenu}>
      <View style={styles.selectorHeader}>
        <Text accessibilityRole="header" style={styles.selectorTitle}>{title}</Text>
        <Pressable accessibilityRole="button" accessibilityLabel={closeLabel} onPress={onClose} style={({ pressed }) => [styles.closeButton, pressed && styles.selectorOptionPressed]}>
          <X color={colors.inkSoft} size={20} />
        </Pressable>
      </View>
      {searchPlaceholder ? <TextInput accessibilityLabel={searchPlaceholder} placeholder={searchPlaceholder} placeholderTextColor={colors.inkMuted} value={query} onChangeText={setQuery} style={styles.search} /> : null}
      <ScrollView bounces={false} keyboardShouldPersistTaps="handled" contentContainerStyle={styles.options}>
        {matches.length === 0 ? <Text style={styles.selectorOptionLabel}>{emptyLabel}</Text> : null}
        {matches.map(option => {
          const selected = option.value === selectedValue;
          return <Pressable
            key={option.value}
            accessibilityRole="radio"
            accessibilityState={{ checked: selected, disabled }}
            disabled={disabled}
            onPress={() => {
              if (disabled) return;
              onClose();
              onSelect(option.value);
            }}
            style={({ pressed }) => [styles.selectorOption, selected && styles.selectorOptionSelected, pressed && styles.selectorOptionPressed, disabled && styles.selectorOptionDisabled]}
          >
            <View style={styles.selectorOptionCopy}>
              <Text style={styles.selectorOptionLabel}>{option.label}</Text>
              {option.description ? <Text numberOfLines={1} style={styles.selectorOptionMeta}>{option.description}</Text> : null}
            </View>
            {selected ? <Check color={colors.accent} size={18} /> : null}
          </Pressable>;
        })}
      </ScrollView>
    </View>
  </KeyboardAvoidingView>;
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  selectorOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    padding: layout.gutter,
    backgroundColor: theme.colors.overlay,
  },
  selectorMenu: {
    width: "100%",
    maxWidth: layout.formMaxWidth,
    alignSelf: "center",
    maxHeight: "85%",
    overflow: "hidden",
    padding: spacing.sm,
    borderRadius: radius.xl,
    backgroundColor: theme.colors.surface,
  },
  selectorHeader: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.md, marginBottom: spacing.sm },
  closeButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.pill },
  options: { gap: spacing.xs, paddingBottom: spacing.sm },
  selectorTitle: {
    flex: 1,
    paddingVertical: spacing.sm,
    color: theme.colors.inkStrong,
    fontSize: theme.font(18),
    fontWeight: "700",
  },
  selectorOption: {
    minHeight: 56,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  selectorOptionSelected: { backgroundColor: theme.colors.accentSoft },
  selectorOptionPressed: { opacity: 0.72 },
  selectorOptionDisabled: { opacity: 0.5 },
  search: { marginHorizontal: spacing.md, marginBottom: spacing.sm, minHeight: layout.touchTarget, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surface, color: theme.colors.ink, fontSize: theme.font(15), padding: spacing.md },
  selectorOptionCopy: { minWidth: 0, flex: 1 },
  selectorOptionLabel: { flexShrink: 1, color: theme.colors.ink, fontSize: theme.font(15), fontWeight: "600" },
  selectorOptionMeta: { marginTop: 2, color: theme.colors.inkMuted, fontSize: theme.font(12) },
}));
