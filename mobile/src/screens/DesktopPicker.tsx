import type { DesktopFilter } from "../remote/mergeSessions";
import type { PairedDesktop } from "../remote/types";
import { Check, ChevronDown, Monitor } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { DialogSurface } from "../components/DialogSurface";
import { iconGlyph } from "../remote/peerIcons";
import { colors, radius, spacing } from "../theme/tokens";

/**
 * The desktop picker above the session list.
 *
 * Two states the user has to tell apart at a glance — "all desktops" (a merged
 * list) and "one desktop" — because the list below looks otherwise identical
 * either way. An offline desktop stays selectable: its rows are simply absent,
 * and disabling the entry would leave no way to see that it is offline rather
 * than removed.
 */
export function DesktopPicker({
  filter,
  onChange,
  onManage,
  desktops,
  activeDesktopId,
}: {
  filter: DesktopFilter;
  onChange: (filter: DesktopFilter) => void;
  onManage: () => void;
  desktops: PairedDesktop[];
  activeDesktopId: string | null;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const label = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;
  const selected = filter.kind === "desktop"
    ? desktops.find(desktop => desktop.desktopId === filter.desktopId)
    : undefined;

  function choose(next: DesktopFilter) {
    onChange(next);
    setOpen(false);
  }

  return (
    <>
      <Pressable
        accessibilityLabel={t("desktops.pickerLabel")}
        accessibilityRole="button"
        onPress={() => setOpen(true)}
        style={({ pressed }) => [styles.trigger, pressed && styles.pressed]}
      >
        <View style={styles.triggerIcon}>
          <Monitor color={colors.accent} size={20} />
        </View>
        <Text numberOfLines={1} style={styles.triggerLabel}>
          {selected ? `${iconGlyph(selected.icon)} ${label(selected)}` : t("desktops.filterAll")}
        </Text>
        <ChevronDown color={colors.inkSoft} size={16} />
      </Pressable>

      <Modal
        animationType="fade"
        onRequestClose={() => setOpen(false)}
        transparent
        visible={open}
      >
        <DialogSurface>
          <Text style={styles.title}>{t("desktops.pickerLabel")}</Text>
          <ScrollView style={styles.list}>
            <Pressable
              accessibilityRole="button"
              accessibilityState={{ selected: filter.kind === "all" }}
              onPress={() => choose({ kind: "all" })}
              style={styles.option}
            >
              <Text style={[styles.optionLabel, filter.kind === "all" && styles.optionSelected]}>
                {t("desktops.filterAll")}
              </Text>
              {filter.kind === "all" ? <Check color={colors.accent} size={16} /> : null}
            </Pressable>
            {desktops.map((desktop) => {
              const isSelected = filter.kind === "desktop" && filter.desktopId === desktop.desktopId;
              return (
                <Pressable
                  accessibilityRole="button"
                  accessibilityState={{ selected: isSelected }}
                  key={desktop.desktopId}
                  onPress={() => choose({ kind: "desktop", desktopId: desktop.desktopId })}
                  style={styles.option}
                >
                  <Text style={[styles.optionLabel, isSelected && styles.optionSelected]}>
                    {iconGlyph(desktop.icon)} {label(desktop)}
                  </Text>
                  {desktop.desktopId === activeDesktopId
                    ? <Text style={styles.badge}>{t("desktops.filterConnected")}</Text>
                    : null}
                  {isSelected ? <Check color={colors.accent} size={16} /> : null}
                </Pressable>
              );
            })}
            <Pressable
              accessibilityRole="button"
              onPress={() => {
                setOpen(false);
                onManage();
              }}
              style={styles.option}
            >
              <Text style={styles.manage}>{t("desktops.manage")}</Text>
            </Pressable>
          </ScrollView>
        </DialogSurface>
      </Modal>
    </>
  );
}

const styles = StyleSheet.create({
  trigger: {
    flex: 1,
    minWidth: 0,
    // 44pt is the smallest comfortable target, and this one sits in a header
    // where a miss lands on the connection badge instead.
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    paddingHorizontal: spacing.sm,
  },
  triggerIcon: { width: 24, alignItems: "center" },
  triggerLabel: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 15, fontWeight: "600" },
  pressed: { opacity: 0.7 },
  title: { color: colors.inkStrong, fontSize: 18, fontWeight: "700" },
  list: { maxHeight: 420 },
  option: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingVertical: spacing.md,
  },
  optionLabel: { flex: 1, minWidth: 0, color: colors.ink, fontSize: 15 },
  optionSelected: { color: colors.accent, fontWeight: "600" },
  badge: { color: colors.inkMuted, fontSize: 12 },
  manage: { flex: 1, color: colors.accent, fontSize: 15 },
});
