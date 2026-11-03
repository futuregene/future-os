import type { DesktopFilter } from "../remote/mergeSessions";
import type { PairedDesktop } from "../remote/types";
import type { MachineSummary } from "../remote/desktopStatus";
import { machineStatusKey } from "../remote/desktopStatus";
import {
  ConnectionStatusBlock,
  connectionStateColors,
  machineDotColor,
  useConnectionSummary,
  type ConnectionFacts,
} from "../components/ConnectionStatus";
import { Check, ChevronDown, Layers, X } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { DialogSurface } from "../components/DialogSurface";
import { PeerIconGlyph } from "../remote/PeerIconGlyph";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

/**
 * The device menu above the session list.
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
 * The trigger is also the phone's status dot, and its sheet is also the
 * connection's own explanation. The two used to be separate controls beside each
 * other — a dot that opened a popover with the same machines listed again — and
 * the header paid for both: a touch target, and the question of which of the two
 * to tap when something was wrong. They answer one question ("which of my
 * machines can I use, and what do I do about the rest"), so a tap on either part
 * of the same control opens the one sheet, and the state the dot is painting is
 * the first thing the sheet says.
 */
export function DesktopPicker({
  filter,
  onChange,
  onManage,
  desktops,
  machines,
  onTakeControl,
  connection,
  active = true,
  fullWidth = false,
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
  /**
   * How the phone stands with those machines: the dot's own state, the
   * explanation under it, and the actions that reach them again.
   */
  connection: ConnectionFacts;
  /**
   * Whether the screen this header belongs to is the one on screen.
   *
   * A sheet left open would outlive the screen it was opened from, so the
   * header closes it rather than keeping it for a return that may never come.
   */
  active?: boolean;
  /**
   * Give the trigger the row to itself.
   *
   * At the largest text size the name a reader chose no longer fits beside the
   * list's tools and settings — and the name is the one thing here that must
   * not be cut — so the header above the list puts this trigger on its own line
   * and the tools on the next one.
   */
  fullWidth?: boolean;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  if (!active && open) setOpen(false);
  const status = useConnectionSummary({ machines, ...connection });
  const { dot: dotColor } = connectionStateColors(colors)[status.state];
  const desktopName = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;
  const selected = filter.kind === "desktop"
    ? desktops.find(desktop => desktop.desktopId === filter.desktopId)
    : undefined;
  /** What the trigger reads: the chosen machine, or the merged list's name. */
  const scopeName = selected ? desktopName(selected) : t("desktops.filterAll");
  const stateOf = (desktopId: string) => machines?.find(machine => machine.desktopId === desktopId);
  /**
   * Whether the rows need to answer for themselves.
   *
   * With one machine the block above is already its answer — its title is that
   * machine's state — and a second "已连接" a few lines lower reads as a
   * repeat rather than as a second fact. With several, the block answers for the
   * set ("some desktops are not connected") and the rows are the only place that
   * says which, so they carry their own dot and state.
   */
  const perMachineState = (machines?.length ?? 0) > 1;
  /** Connected, and held by another device. */
  const standby = (desktopId: string) => {
    const machine = stateOf(desktopId);
    return machine?.status === "connected" && machine.control === "standby";
  };

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
        // The name says what the button is, what the list below is showing, and
        // how it stands: the dot is the only other way to learn the last part,
        // and a screen reader cannot read a colour. `desktops.pickerLabel` is
        // still the beginning of the name, so the control is found the same way
        // it always was.
        accessibilityLabel={`${t("desktops.pickerLabel")}: ${scopeName}, ${status.label}`}
        accessibilityHint={t("connection.showDetails")}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen(true)}
        style={({ pressed }) => [
          styles.trigger,
          fullWidth && styles.triggerFullWidth,
          pressed && styles.pressed,
        ]}
      >
        {/* One machine indicator, not two: a chosen machine leads with its own
            glyph — the same one the sheet draws beside its name — and the
            general layers icon stands in only for the merged scope, which has
            no machine of its own to show. */}
        <View style={styles.triggerIcon}>
          {selected ? <PeerIconGlyph id={selected.icon} size={17} /> : <Layers color={colors.accent} size={17} />}
        </View>
        {/* Two lines, not one: the row above the list now shares its width with
            the list's own tools, and a machine name the reader gave it can be
            longer than what is left. Wrapping keeps the whole name on screen
            instead of cutting it to "实验室的…" — the sheet this button opens is
            a place to *choose* a machine, not the only place to find out which
            one is on screen. */}
        <Text numberOfLines={2} style={styles.triggerLabel}>{scopeName}</Text>
        {/* The dot that used to stand on its own beside this trigger, inside it
            now: it was the phone's answer about its machines, and so is this
            button. Tapping it anywhere opens the one sheet. */}
        <View testID="connection-dot" style={[styles.dot, { backgroundColor: dotColor }]} />
        <ChevronDown color={colors.inkSoft} size={14} />
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
          {/* The state the header's dot was painting, in words, with the action
              that reaches the machines again: the dot said something was wrong
              and this is where the reader finds out what. */}
          <ConnectionStatusBlock machines={machines} {...connection} />
          {/* Every row carries the same leading icon column, and the chosen one
              is tinted behind the check: without them "all desktops" sits flush
              with the sheet title and reads as a heading for the machines below
              rather than as one of the answers. */}
          <View style={styles.options}>
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
              const machine = stateOf(desktop.desktopId);
              const isStandby = standby(desktop.desktopId);
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
                    <PeerIconGlyph id={desktop.icon} size={16} />
                  </View>
                  <View style={styles.optionText}>
                    <Text style={[styles.optionLabel, isSelected && styles.optionLabelSelected]}>
                      {desktopName(desktop)}
                    </Text>
                    {/* Under the name, not beside it: a row with a second
                        control in it would make "pick this machine" and "take
                        it over" look like the same kind of answer. The dot is
                        the header's own judgement of that machine — green only
                        for connected and ours to drive — so the tally in the
                        block above and these rows can never say different
                        things. */}
                    {machine && perMachineState ? (
                      <View style={styles.optionState}>
                        <View
                          testID={`machine-dot-${desktop.desktopId}`}
                          style={[styles.stateDot, { backgroundColor: machineDotColor(machine, colors) }]}
                        />
                        <Text style={styles.optionNote}>
                          {isStandby
                            ? t("connection.machineStandby")
                            : t(machineStatusKey(machine.status))}
                        </Text>
                      </View>
                    ) : null}
                  </View>
                  {isStandby && onTakeControl ? (
                    <Pressable
                      accessibilityLabel={`${t("connection.takeControl")} ${desktopName(desktop)}`}
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
          </View>
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
    // where a miss lands on the list's own tools.
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    // Tighter than the sheet's own spacing: this row also holds the list's
    // tools and settings, and every point the chevron and the icon give up is a
    // point the machine's name keeps.
    gap: 6,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    paddingLeft: spacing.sm,
    paddingRight: 6,
    paddingVertical: spacing.xs,
  },
  triggerIcon: { width: 17, alignItems: "center" },
  triggerFullWidth: { flexBasis: "100%" },
  triggerLabel: { flex: 1, minWidth: 0, color: theme.colors.inkStrong, fontSize: theme.font(15), fontWeight: "600" },
  dot: { width: 8, height: 8, borderRadius: 4 },
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
  options: { gap: spacing.xs },
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
  optionText: { flex: 1, minWidth: 0 },
  optionLabel: { color: theme.colors.ink, fontSize: theme.font(15) },
  optionState: { flexDirection: "row", alignItems: "center", gap: spacing.xs, marginTop: 1 },
  stateDot: { width: 6, height: 6, borderRadius: 3 },
  optionNote: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
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
