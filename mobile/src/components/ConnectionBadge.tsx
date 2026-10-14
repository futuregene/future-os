import { useRef, useState } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react-native";
import type { ConnectionPresentation } from "../remote/connectionPresentation";
import type { MachineStatus, MachineSummary } from "../remote/desktopStatus";
import { machineStatusKey, machinesConnected } from "../remote/desktopStatus";
import { disconnectedCopy } from "../remote/disconnectedCopy";
import { iconGlyph } from "../remote/peerIcons";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { Button } from "./Button";

/** The dot a machine's own row carries, in the same palette as the badge. */
function statusColor(status: MachineStatus): string {
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
const stateColors: Record<DotState, { dot: string; text: string }> = {
  up: { dot: colors.success, text: colors.success },
  partial: { dot: colors.warningDot, text: colors.warning },
  down: { dot: colors.danger, text: colors.danger },
};

export function ConnectionBadge({
  presentation,
  disconnectReason,
  error,
  onReconnect,
  onUnpair,
  active = true,
  machines,
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
}) {
  const { t } = useTranslation();
  const dimensions = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const trigger = useRef<View>(null);
  const frameKey = `${dimensions.width}:${dimensions.height}:${dimensions.fontScale}:${insets.top}:${insets.right}:${insets.bottom}:${insets.left}`;
  const [anchor, setAnchor] = useState<{ right: number; top: number; frameKey: string } | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
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
  // The dot answers for every paired machine when there is a list to judge, and
  // for the one connection the app renders from when there is not.
  const state: DotState = summary && machines!.length > 0
    ? summary.all ? "up" : summary.any ? "partial" : "down"
    : presentation.level === "connected" ? "up" :
      presentation.level === "connecting" ? "partial" : "down";
  const { dot: dotColor, text: titleColor } = stateColors[state];
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
  const label = t(statusKey);
  const reason = disconnected
    ? t(`connection.disconnectDetails.${copy}.reason`)
    : t(`connection.statusDetails.${detailState}.reason`);
  const solution = disconnected
    ? t(`connection.disconnectDetails.${copy}.solution`)
    : t(`connection.statusDetails.${detailState}.solution`);
  const reasonWithCode =
    reason + (presentation.supportCode ? ` (${presentation.supportCode})` : "");
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
                  {machines!.map(machine => (
                    <View key={machine.desktopId} style={styles.machineRow}>
                      <View
                        testID={`machine-dot-${machine.desktopId}`}
                        style={[
                          styles.machineDot,
                          { backgroundColor: statusColor(machine.status) },
                        ]}
                      />
                      <Text numberOfLines={1} style={styles.machineName}>
                        {`${iconGlyph(machine.icon)} ${machine.label}`}
                      </Text>
                      <Text style={styles.machineState}>
                        {t(machineStatusKey(machine.status))}
                      </Text>
                    </View>
                  ))}
                  <Text accessibilityLiveRegion="polite" style={styles.machinesSummary}>
                    {t("connection.machinesSummary", {
                      connected: summary?.connected ?? 0,
                      total: summary?.total ?? 0,
                    })}
                  </Text>
                </View>
              )}
              <Text accessibilityLiveRegion="polite" style={[styles.title, { color: titleColor }]}>{label}</Text>
              <Text style={styles.hint}>{reasonWithCode}</Text>
              {solution && <Text style={styles.hint}>{solution}</Text>}
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

const styles = StyleSheet.create({
  badge: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
  },
  pressed: { backgroundColor: colors.surfaceSubtle },
  dot: { width: 8, height: 8, borderRadius: 4 },
  overlay: { flex: 1 },
  popover: {
    position: "absolute",
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.surface,
    boxShadow: "0 4px 16px rgba(15, 23, 42, 0.15)",
    overflow: "hidden",
  },
  header: { flexDirection: "row", alignItems: "center", paddingLeft: spacing.lg },
  heading: { flex: 1, color: colors.inkSoft, fontSize: 13, fontWeight: "600" },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  content: { paddingHorizontal: spacing.lg, paddingBottom: spacing.lg, gap: spacing.sm },
  machines: { gap: spacing.xs },
  machineRow: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  machineDot: { width: 8, height: 8, borderRadius: 4 },
  machineName: { flex: 1, minWidth: 0, color: colors.ink, fontSize: 14 },
  machineState: { color: colors.inkMuted, fontSize: 13 },
  machinesSummary: { color: colors.inkMuted, fontSize: 12 },
  title: { fontSize: 15, fontWeight: "600" },
  hint: { color: colors.inkSoft, fontSize: 14, lineHeight: 21 },
});
