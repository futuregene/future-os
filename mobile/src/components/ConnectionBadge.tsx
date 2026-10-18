import { useRef, useState } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react-native";
import type { ConnectionPresentation } from "../remote/connectionPresentation";
import type { MachineStatus, MachineSummary } from "../remote/desktopStatus";
import { machineStatusKey, machinesConnected, machinesPartlyConnected } from "../remote/desktopStatus";
import { disconnectedCopy } from "../remote/disconnectedCopy";
import { iconGlyph } from "../remote/peerIcons";
import { layout, radius, spacing, type Palette } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { Button } from "./Button";

/** The dot a machine's own row carries, in the same palette as the badge. */
function statusColor(status: MachineStatus, colors: Palette): string {
  if (status === "connected") return colors.success;
  return status === "offline" ? colors.danger : colors.warningDot;
}

type DotState = "up" | "partial" | "down";

/**
 * The three states the dot and the title share, and the two hues they wear.
 *
 * The dot is yellow at `partial`: it and the title are drawn on the same light
 * surface, but one is an 8px shape and the other a line of text, and the hues
 * those two want are not the same. `warning` is darkened so it can be read as
 * text; at the size of a dot that same orange is close enough to the `danger`
 * red of the state below that "some machines are down" reads as "all of them
 * are" — the one distinction this dot exists to make. So the dot takes the
 * yellow, and the text keeps the amber it has contrast for.
 */
function stateColors(colors: Palette): Record<DotState, { dot: string; text: string }> {
  return {
    up: { dot: colors.success, text: colors.success },
    partial: { dot: colors.warningDot, text: colors.warning },
    down: { dot: colors.danger, text: colors.danger },
  };
}

export function ConnectionBadge({
  presentation,
  disconnectReason,
  error,
  onReconnect,
  onUnpair,
  active = true,
  machines,
  standby = false,
  holder = null,
  onTakeControl,
}: {
  presentation: ConnectionPresentation;
  disconnectReason?: string;
  error?: string | null;
  onReconnect?: () => void;
  onUnpair?: () => void;
  active?: boolean;
  /**
   * Every paired machine this phone holds, the active one included.
   *
   * The dot answers for all of them: a phone paired to three desktops is green
   * only when all three are connected, because a merged list that is missing a
   * machine is exactly the state the reader has to be able to see.
   */
  machines?: MachineSummary[];
  /**
   * Whether the machine the badge is rendered for is held by another device.
   *
   * Only read when there is no machine list to judge (the single-device
   * fallback): with a list, every machine's own `control` answers this.
   */
  standby?: boolean;
  /**
   * The device holding the machine, when another one does. Only used to name it
   * in the block's own words; the per-machine rows read their own state.
   */
  holder?: { name?: string } | null;
  /**
   * Ask `desktopId` to let this phone drive it.
   *
   * The popover is where a refusal is explained, so it is also where the way out
   * of it belongs: the dot says something is wrong, this sheet says what, and
   * the action is one tap from the explanation.
   */
  onTakeControl?: (desktopId: string) => Promise<void> | void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const dimensions = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const trigger = useRef<View>(null);
  const frameKey = `${dimensions.width}:${dimensions.height}:${dimensions.fontScale}:${insets.top}:${insets.right}:${insets.bottom}:${insets.left}`;
  const [anchor, setAnchor] = useState<{ right: number; top: number; frameKey: string } | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  /** The machine whose takeover is in flight, so its button cannot be doubled. */
  const [busy, setBusy] = useState<string | null>(null);
  // Dismiss rather than leave the bubble detached from its dot after layout changes.
  if (modalOpen && (!active || anchor?.frameKey !== frameKey)) setModalOpen(false);
  const visible = active && modalOpen && anchor !== null && anchor.frameKey === frameKey;
  const close = () => setModalOpen(false);
  // Keep the last anchor while the native fade-out is running so the bubble does not
  // jump to its fallback position before it disappears.
  const open = () => {
    trigger.current?.measureInWindow((x, y, width, height) => {
      setAnchor({
        frameKey,
        right: dimensions.width - x - width,
        top: y + height + spacing.xs,
      });
      setModalOpen(true);
    });
  };
  // Several machines in one list: the header says how many are up, and the
  // button offers to reach all of them rather than only the active one.
  const severalMachines = (machines?.length ?? 0) > 1;
  const summary = machines ? machinesConnected(machines) : null;
  // ...and the block under the rows answers for the same set. A phone paired to
  // three desktops, two of them up, is not "no connection" — which is what the
  // one machine's own state says when that machine is the one that is down.
  const partial = summary !== null && machinesPartlyConnected(summary, presentation);
  // "Connected" alone is not green: a machine that answers while another paired
  // device holds it cannot be read from or commanded, and painting that green
  // told the user everything was fine while every action failed. Green means
  // connected *and* ours to drive; yellow covers both a partly-connected set and
  // a fully-connected one that nobody here may drive.
  const state: DotState = summary && machines!.length > 0
    ? summary.all
      ? (summary.drivable > 0 ? "up" : "partial")
      : summary.any
        ? "partial"
        : "down"
    : presentation.level === "connected"
      ? (standby ? "partial" : "up")
      : presentation.level === "connecting" ? "partial" : "down";
  const { dot: dotColor, text: titleColor } = stateColors(colors)[state];
  // A connection the app still reports as healthy (or still trying) is where a
  // silently dead transport hides: those states keep the same manual reconnect
  // the failure states get, so recovering never requires unpairing first.
  const reconnectable = !!onReconnect && (
    presentation.action === "reconnect" ||
    presentation.action === "retry" ||
    presentation.action === "checkNetwork" ||
    presentation.action === "none" ||
    presentation.action === "wait"
  );
  const unpairable = !!onUnpair && presentation.action === "pairAgain";
  const disconnected = presentation.level === "disconnected";
  const copy = disconnectedCopy(presentation, disconnectReason, error);
  const statusKey = disconnected
    ? presentation.customerState === "pairingExpired"
      ? "connection.pairingExpired"
      : "connection.disconnected"
    : presentation.titleKey;
  const detailState = presentation.customerState === "devicePreparing"
    ? "devicePreparing"
    : presentation.customerState === "waitingDesktop"
      ? "waitingDesktop"
      : presentation.level;
  // Every machine answers, and none of them is ours to drive: the set is up but
  // held by another paired device. Distinct from `partial` (some machines are
  // down): here the block has to say *why* nothing can be done, and name who is
  // holding it when there is one machine and it said who it is.
  const standbyOnly = summary !== null
    && summary.any
    && summary.drivable === 0
    && !partial;
  const holderName = machines?.length === 1 ? holder?.name?.trim() ?? null : null;
  const label = partial
    ? t("connection.machinesPartial")
    : standbyOnly
      ? t("connection.connectedStandby")
      : t(statusKey);
  const reason = partial
    ? t("connection.machinesSummary", {
      connected: summary!.connected,
      total: summary!.total,
    })
    : standbyOnly
      ? (holderName
          ? t("connection.controlHeldBy", { name: holderName })
          : t("connection.controlHeldByUnknown"))
      : disconnected
        ? t(`connection.disconnectDetails.${copy}.reason`)
        : t(`connection.statusDetails.${detailState}.reason`);
  const solution = partial
    ? t("connection.machinesPartialHint")
    : standbyOnly
      ? (holderName
          ? t("connection.takeControlHint", { name: holderName })
          : t("connection.takeControlHintUnknown"))
      : disconnected
        ? t(`connection.disconnectDetails.${copy}.solution`)
        : t(`connection.statusDetails.${detailState}.solution`);
  // The set's own state carries no support code: there is no single failure to
  // look up, and the rows above name which machines it is about. The open
  // machine's code is still the phone's when nothing answers at all, where
  // `partial` is false.
  const reasonWithCode = partial
    ? reason
    : reason + (presentation.supportCode ? ` (${presentation.supportCode})` : "");
  /**
   * Take one machine, then close.
   *
   * Closing is part of the answer: the sheet exists to explain why the phone
   * cannot work, and once it can, leaving the explanation on screen would be the
   * last thing the user sees.
   */
  async function take(desktopId: string) {
    if (!onTakeControl || busy) return;
    setBusy(desktopId);
    try {
      await onTakeControl(desktopId);
      close();
    }
    catch {
      // Stay open: the machine refused, and the sheet is where that is read.
    }
    finally {
      setBusy(null);
    }
  }

  const leftEdge = insets.left + layout.gutter;
  const rightEdge = insets.right + layout.gutter;
  const popoverWidth = Math.min(320, dimensions.width - leftEdge - rightEdge);
  const right = Math.max(rightEdge, Math.min(anchor?.right ?? rightEdge, dimensions.width - leftEdge - popoverWidth));
  const top = Math.max(insets.top + spacing.sm, Math.min(anchor?.top ?? 0, dimensions.height - insets.bottom - 120));

  return (
    <>
      <Pressable
        ref={trigger}
        collapsable={false}
        accessibilityLabel={`${t("connection.status")}: ${label}`}
        accessibilityHint={t("connection.showDetails")}
        accessibilityRole="button"
        accessibilityState={{ expanded: visible }}
        onPress={open}
        style={({ pressed }) => [styles.badge, (pressed || visible) && styles.pressed]}
      >
        <View testID="connection-dot" style={[styles.dot, { backgroundColor: dotColor }]} />
      </Pressable>
      <Modal
        transparent
        statusBarTranslucent
        navigationBarTranslucent
        animationType="fade"
        visible={visible}
        onRequestClose={close}
      >
        <View style={styles.overlay}>
          <Pressable accessible={false} style={StyleSheet.absoluteFill} onPress={close} />
          <View
            accessibilityViewIsModal
            style={[styles.popover, { top, right, width: popoverWidth, maxHeight: dimensions.height - top - insets.bottom - spacing.sm }]}
          >
            <View style={styles.header}>
              <Text accessibilityRole="header" style={styles.heading}>{t("connection.status")}</Text>
              <Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} onPress={close} style={styles.close}>
                <X color={colors.inkSoft} size={18} />
              </Pressable>
            </View>
            <ScrollView bounces={false} contentContainerStyle={styles.content}>
              {severalMachines && (
                <View style={styles.machines}>
                  {machines!.map((machine) => {
                    // A machine another device holds answers every heartbeat and
                    // still cannot be read from, so its row wears the same yellow
                    // the header dot does rather than the green of "connected".
                    const held = machine.status === "connected" && machine.control === "standby";
                    return (
                      <View key={machine.desktopId} style={styles.machineRow}>
                        <View
                          testID={`machine-dot-${machine.desktopId}`}
                          style={[
                            styles.machineDot,
                            { backgroundColor: held ? colors.warningDot : statusColor(machine.status, colors) },
                          ]}
                        />
                        <View style={styles.machineText}>
                          <Text numberOfLines={1} style={styles.machineName}>
                            {`${iconGlyph(machine.icon)} ${machine.label}`}
                          </Text>
                          <Text numberOfLines={1} style={styles.machineState}>
                            {held
                              ? t("connection.machineStandby")
                              : t(machineStatusKey(machine.status))}
                          </Text>
                        </View>
                        {held && onTakeControl ? (
                          <Button
                            compact
                            variant="secondary"
                            label={busy === machine.desktopId ? "…" : t("connection.takeControl")}
                            onPress={() => void take(machine.desktopId)}
                          />
                        ) : null}
                      </View>
                    );
                  })}
                  {/* The count is the block's own description when only some
                      machines answer; written here it would repeat it a line
                      above its own reason. */}
                  {!partial && (
                    <Text accessibilityLiveRegion="polite" style={styles.machinesSummary}>
                      {t("connection.machinesSummary", {
                        connected: summary?.connected ?? 0,
                        total: summary?.total ?? 0,
                      })}
                    </Text>
                  )}
                </View>
              )}
              <Text accessibilityLiveRegion="polite" style={[styles.title, { color: titleColor }]}>{label}</Text>
              <Text style={styles.hint}>{reasonWithCode}</Text>
              {solution && <Text style={styles.hint}>{solution}</Text>}
              {!severalMachines && standby && onTakeControl && machines?.[0] ? (
                <Button
                  compact
                  variant="secondary"
                  label={busy ? "…" : t("connection.takeControl")}
                  onPress={() => void take(machines[0]!.desktopId)}
                />
              ) : null}
              {(reconnectable || unpairable) && (
                <Button
                  compact
                  variant={presentation.level === "disconnected" ? "primary" : "secondary"}
                  label={t(unpairable
                    ? "sessions.unpair"
                    : severalMachines ? "connection.reconnectAll" : "connection.reconnect")}
                  onPress={() => {
                    close();
                    if (unpairable) onUnpair?.();
                    else onReconnect?.();
                  }}
                />
              )}
            </ScrollView>
          </View>
        </View>
      </Modal>
    </>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  badge: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
  },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  dot: { width: 8, height: 8, borderRadius: 4 },
  overlay: { flex: 1 },
  popover: {
    position: "absolute",
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: theme.colors.line,
    backgroundColor: theme.colors.surface,
    boxShadow: "0 4px 16px rgba(15, 23, 42, 0.15)",
    overflow: "hidden",
  },
  header: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.lg },
  heading: { flex: 1, color: theme.colors.inkSoft, fontSize: theme.font(13), fontWeight: "600" },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  content: { paddingHorizontal: spacing.lg, paddingBottom: spacing.lg, gap: spacing.sm },
  machines: { gap: spacing.xs },
  machineRow: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  machineDot: { width: 8, height: 8, borderRadius: 4 },
  machineText: { flex: 1, minWidth: 0 },
  machineName: { color: theme.colors.ink, fontSize: theme.font(14) },
  machineState: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  machinesSummary: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  title: { fontSize: theme.font(15), fontWeight: "600" },
  hint: { color: theme.colors.inkSoft, fontSize: theme.font(14), lineHeight: theme.leading(21) },
}));
