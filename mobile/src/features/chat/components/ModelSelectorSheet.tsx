import { Check } from "lucide-react-native";
import {
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import type { TFunction } from "i18next";
import { useRemote } from "../../../remote/RemoteContext";
import { modelReference, type ThinkingLevel } from "../../../remote/types";
import { colors, radius, spacing } from "../../../theme/tokens";

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
      animationType="fade"
      onRequestClose={() => setSelector(null)}
      transparent
      visible={selector !== null}
    >
      <TouchableWithoutFeedback onPress={() => setSelector(null)}>
        <View style={styles.selectorOverlay}>
          <TouchableWithoutFeedback>
            <View style={styles.selectorMenu}>
              <Text style={styles.selectorTitle}>
                {selector === "model" ? t("chat.model") : t("chat.thinkingLevel")}
              </Text>
              <ScrollView bounces={false}>
                {selector === "model"
                  ? remote.models.map(model => {
                      const selected = modelReference(model) === remote.modelId;
                      return (
                        <Pressable
                          key={`${model.provider ?? ""}/${model.id}`}
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
                            <Text numberOfLines={1} style={styles.selectorOptionLabel}>
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
                    })
                  : thinkingLevels.map(level => {
                      const selected = level === remote.thinkingLevel;
                      return (
                        <Pressable
                          key={level}
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
                          <Text style={styles.selectorOptionLabel}>
                            {t(`thinking.${level}`)}
                          </Text>
                          {selected ? <Check color={colors.accent} size={18} /> : null}
                        </Pressable>
                      );
                    })}
              </ScrollView>
            </View>
          </TouchableWithoutFeedback>
        </View>
      </TouchableWithoutFeedback>
    </Modal>
  );
}

const styles = StyleSheet.create({
  selectorOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    padding: spacing.md,
    paddingBottom: spacing.xl,
    backgroundColor: colors.overlay,
  },
  selectorMenu: {
    maxHeight: "60%",
    overflow: "hidden",
    padding: spacing.sm,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  selectorTitle: {
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    color: colors.inkMuted,
    fontSize: 12,
    fontWeight: "700",
  },
  selectorOption: {
    minHeight: 50,
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
  selectorOptionLabel: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  selectorOptionMeta: { marginTop: 2, color: colors.inkMuted, fontSize: 11 },
});
