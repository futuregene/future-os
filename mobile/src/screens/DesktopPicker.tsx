import type { DesktopFilter } from "../remote/mergeSessions";
import type { PairedDesktop } from "../remote/types";
import type { MachineSummary } from "../remote/desktopStatus";
import { Check, ChevronDown, Layers, Monitor, X } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { DialogSurface } from "../components/DialogSurface";
import { iconGlyph } from "../remote/peerIcons";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

/**
 * The desktop picker above the session list.
 *
 * Two states the user has to tell apart at a glance — "all desktops" (a merged
 * list) and "one desktop" — because the list below looks otherwise identical
 * either way. An offline desktop stays selectable: its rows are simply absent,
 * and disabling the entry would leave no way to see that it is offline rather
 * than removed. Selecting one is not a dead end either: the list below then shows
 * that machine's own state, with the action that reaches it, because showing the
 * machine the phone happens to be reading under another machine's name is the
 * one thing the list must never do.
 *
 * Which desktop the phone is actually connected to is deliberately not repeated
 * on the row: the header's status dot already answers that, and a label on the
 * one connected row made it look like a different kind of row than the rest.
 */
export function DesktopPicker({
  filter,
  onChange,
  onManage,
  desktops,
  machines,
  onTakeControl,
}: {
  filter: DesktopFilter;
  onChange: (filter: DesktopFilter) => void;
  onManage: () => void;
  desktops: PairedDesktop[];
  /**
   * Each paired machine's live control state, by desktop id.
   *
   * The picker is where the user chooses which machine to work on, so it is also
   * where "this one is connected but another device is driving it" has to be
   * visible — otherwise the only way to learn it is to select the machine and
   * find every action refused.
   */
  machines?: MachineSummary[];
  /** Ask `desktopId` to let this phone drive it. */
  onTakeControl?: (desktopId: string) => Promise<void> | void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const label = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;
  const selected = filter.kind === "desktop"
    ? desktops.find(desktop => desktop.desktopId === filter.desktopId)
    : undefined;
  const stateOf = (desktopId: string) => machines?.find(machine => machine.desktopId === desktopId);
  /** Connected, and held by another device. */
  const standby = (desktopId: string) => {
    const machine = stateOf(desktopId);
    return machine?.status === "connected" && machine.control === "standby";
  };
  const offline = (desktopId: string) => stateOf(desktopId)?.status === "offline";

  async function take(desktopId: string) {
    if (!onTakeControl || busy) return;
    setBusy(desktopId);
    try {
      await onTakeControl(desktopId);
    }
    catch {
      // The row keeps its state; the machine's own refusal is what the user
      // will see if it persists, and a spinner stuck on is worse than a retry.
    }
    finally {
      setBusy(null);
    }
  }

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
          <View style={styles.header}>
            <Text accessibilityRole="header" style={styles.title}>{t("desktops.pickerLabel")}</Text>
            {/* Every row in the list commits a choice, and the sheet has no
                outside-tap dismissal, so without this the only way out is the
                system back gesture — which iOS does not have. */}
            <Pressable
              accessibilityLabel={t("common.close")}
              accessibilityRole="button"
              onPress={() => setOpen(false)}
              style={({ pressed }) => [styles.closeButton, pressed && styles.pressed]}
            >
              <X color={colors.inkSoft} size={20} />
            </Pressable>
          </View>
          <ScrollView style={styles.list}>
            {/* Every row carries the same leading icon column, and the chosen
                one is tinted behind the check: without them "all desktops" sits
                flush with the sheet title and reads as a heading for the
                machines below rather than as one of the answers. */}
            <Pressable
              accessibilityRole="radio"
              accessibilityState={{ checked: filter.kind === "all" }}
              onPress={() => choose({ kind: "all" })}
              style={({ pressed }) => [
                styles.option,
                filter.kind === "all" && styles.optionSelected,
                pressed && styles.pressed,
              ]}
            >
              <View style={styles.optionIcon}>
                <Layers color={colors.inkSoft} size={18} />
              </View>
              <Text style={[styles.optionLabel, filter.kind === "all" && styles.optionLabelSelected]}>
                {t("desktops.filterAll")}
              </Text>
              {filter.kind === "all" ? <Check color={colors.accent} size={16} /> : null}
            </Pressable>
            {desktops.map((desktop) => {
              const isSelected = filter.kind === "desktop" && filter.desktopId === desktop.desktopId;
              const isStandby = standby(desktop.desktopId);
              const isOffline = offline(desktop.desktopId);
              return (
                <Pressable
                  accessibilityRole="radio"
                  accessibilityState={{ checked: isSelected }}
                  key={desktop.desktopId}
                  onPress={() => choose({ kind: "desktop", desktopId: desktop.desktopId })}
                  style={({ pressed }) => [
                    styles.option,
                    isSelected && styles.optionSelected,
                    pressed && styles.pressed,
                  ]}
                >
                  <View style={styles.optionIcon}>
                    <Text style={styles.optionGlyph}>{iconGlyph(desktop.icon)}</Text>
                  </View>
                  <View style={styles.optionText}>
                    <Text style={[styles.optionLabel, isSelected && styles.optionLabelSelected]}>
                      {label(desktop)}
                    </Text>
                    {/* Under the name, not beside it: a row with a second
                        control in it would make "pick this machine" and "take
                        it over" look like the same kind of answer. */}
                    {isStandby ? (
                      <Text style={styles.optionNote}>{t("connection.machineStandbyHint")}</Text>
                    ) : isOffline ? (
                      <Text style={styles.optionNote}>{t("connection.machine.offline")}</Text>
                    ) : null}
                  </View>
                  {isStandby && onTakeControl ? (
                    <Pressable
                      accessibilityLabel={`${t("connection.takeControl")} ${label(desktop)}`}
                      accessibilityRole="button"
                      disabled={busy !== null}
                      onPress={() => void take(desktop.desktopId)}
                      style={({ pressed }) => [styles.takeControl, pressed && styles.pressed]}
                    >
                      <Text style={styles.takeControlLabel}>
                        {busy === desktop.desktopId ? "…" : t("connection.takeControl")}
                      </Text>
                    </Pressable>
                  ) : null}
                  {isSelected ? <Check color={colors.accent} size={16} /> : null}
                </Pressable>
              );
            })}
            <View style={styles.separator} />
            <Pressable
              accessibilityRole="button"
              onPress={() => {
                setOpen(false);
                onManage();
              }}
              style={({ pressed }) => [styles.option, pressed && styles.pressed]}
            >
              {/* An action, not a choice — the rule above separates it from the
                  answers, and the empty column keeps its text on their edge. */}
              <View style={styles.optionIcon} />
              <Text style={styles.manage}>{t("desktops.manage")}</Text>
            </Pressable>
          </ScrollView>
        </DialogSurface>
      </Modal>
    </>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
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
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    paddingHorizontal: spacing.sm,
  },
  triggerIcon: { width: 24, alignItems: "center" },
  triggerLabel: { flex: 1, minWidth: 0, color: theme.colors.inkStrong, fontSize: theme.font(15), fontWeight: "600" },
  pressed: { opacity: 0.7 },
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  title: { flex: 1, minWidth: 0, color: theme.colors.inkStrong, fontSize: theme.font(18), fontWeight: "700" },
  closeButton: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
  },
  list: { maxHeight: 420 },
  option: {
    minHeight: 48,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  // Same tint the other single-choice sheets use for the answer in force.
  optionSelected: { backgroundColor: theme.colors.accentSoft },
  optionIcon: { width: 24, alignItems: "center", justifyContent: "center" },
  optionGlyph: { fontSize: theme.font(16) },
  optionText: { flex: 1, minWidth: 0 },
  optionLabel: { color: theme.colors.ink, fontSize: theme.font(15) },
  optionNote: { color: theme.colors.inkMuted, fontSize: theme.font(12), marginTop: 1 },
  takeControl: {
    minHeight: 32,
    justifyContent: "center",
    paddingHorizontal: spacing.md,
    borderRadius: radius.pill,
    backgroundColor: theme.colors.warningSoft,
    borderWidth: 1,
    borderColor: theme.colors.warning,
  },
  takeControlLabel: { color: theme.colors.ink, fontSize: theme.font(13), fontWeight: "600" },
  optionLabelSelected: { color: theme.colors.accent, fontWeight: "600" },
  separator: {
    height: StyleSheet.hairlineWidth,
    marginVertical: spacing.xs,
    backgroundColor: theme.colors.lineSoft,
  },
  manage: { flex: 1, color: theme.colors.accent, fontSize: theme.font(15) },
}));
