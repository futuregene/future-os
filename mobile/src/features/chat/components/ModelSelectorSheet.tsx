import { Check, X } from "lucide-react-native";
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
import { useRemote } from "../../../remote/RemoteContext";
import { modelReference, type ThinkingLevel } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";

type Remote = ReturnType<typeof useRemote>;

const thinkingLevels: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];

export function ModelSelectorSheet({
  selector,
  setSelector,
  remote,
  t,
}: {
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
  remote: Remote;
  t: TFunction;
}) {
  return (
    <Modal
      animationType="slide"
      onRequestClose={() => setSelector(null)}
      transparent
      visible={selector !== null}
    >
        <SafeAreaView style={styles.selectorOverlay}>
          <Pressable
            accessible={false}
            onPress={() => setSelector(null)}
            style={StyleSheet.absoluteFill}
          />
            <View accessibilityViewIsModal style={styles.selectorMenu}>
              <View style={styles.selectorHeader}>
                <Text accessibilityRole="header" style={styles.selectorTitle}>
                  {t(selector === "model" ? "chat.model" : "chat.thinkingLevel")}
                </Text>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={t("common.close")}
                  onPress={() => setSelector(null)}
                  style={({ pressed }) => [styles.closeButton, pressed && styles.selectorOptionPressed]}
                >
                  <X color={colors.inkSoft} size={20} />
                </Pressable>
              </View>
              <ScrollView bounces={false} keyboardShouldPersistTaps="handled" contentContainerStyle={styles.options}>
                {selector === "model" && remote.models.length === 0 && (
                  <Text style={styles.selectorOptionLabel}>{t("connection.noModels")}</Text>
                )}
                {selector === "model" && remote.models.map(model => {
                      const selected = modelReference(model) === remote.modelId;
                      return (
                        <Pressable
                          key={`${model.provider ?? ""}/${model.id}`}
                          accessibilityRole="radio"
                          accessibilityState={{ checked: selected }}
                          onPress={() => {
                            setSelector(null);
                            void remote.setModel(modelReference(model));
                          }}
                          style={({ pressed }) => [
                            styles.selectorOption,
                            selected && styles.selectorOptionSelected,
                            pressed && styles.selectorOptionPressed,
                          ]}
                        >
                          <View style={styles.selectorOptionCopy}>
                            <Text style={styles.selectorOptionLabel}>
                              {model.label || model.id}
                            </Text>
                            {model.provider ? (
                              <Text numberOfLines={1} style={styles.selectorOptionMeta}>
                                {model.provider}
                              </Text>
                            ) : null}
                          </View>
                          {selected ? <Check color={colors.accent} size={18} /> : null}
                        </Pressable>
                      );
                    })}
                {selector === "thinking" && thinkingLevels.map(level => {
                      const selected = level === remote.thinkingLevel;
                      return (
                        <Pressable
                          key={level}
                          accessibilityRole="radio"
                          accessibilityState={{ checked: selected }}
                          onPress={() => {
                            setSelector(null);
                            void remote.setThinkingLevel(level);
                          }}
                          style={({ pressed }) => [
                            styles.selectorOption,
                            selected && styles.selectorOptionSelected,
                            pressed && styles.selectorOptionPressed,
                          ]}
                        >
                          <Text style={styles.selectorOptionLabel}>{t(`thinking.${level}`)}</Text>
                          {selected ? <Check color={colors.accent} size={18} /> : null}
                        </Pressable>
                      );
                    })}
              </ScrollView>
            </View>
        </SafeAreaView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  selectorOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    padding: layout.gutter,
    backgroundColor: colors.overlay,
  },
  selectorMenu: {
    width: "100%",
    maxWidth: layout.formMaxWidth,
    alignSelf: "center",
    maxHeight: "85%",
    overflow: "hidden",
    padding: spacing.sm,
    borderRadius: radius.xl,
    backgroundColor: colors.surface,
  },
  selectorHeader: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.md, marginBottom: spacing.sm },
  closeButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.pill },
  options: { gap: spacing.xs, paddingBottom: spacing.sm },
  selectorTitle: {
    flex: 1,
    paddingVertical: spacing.sm,
    color: colors.inkStrong,
    fontSize: 18,
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
  selectorOptionSelected: { backgroundColor: colors.accentSoft },
  selectorOptionPressed: { opacity: 0.72 },
  selectorOptionCopy: { minWidth: 0, flex: 1 },
  selectorOptionLabel: { flexShrink: 1, color: colors.ink, fontSize: 15, fontWeight: "600" },
  selectorOptionMeta: { marginTop: 2, color: colors.inkMuted, fontSize: 12 },
});
