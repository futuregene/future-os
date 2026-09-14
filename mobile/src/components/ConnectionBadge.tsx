import { useRef, useState } from "react";
import { Modal, Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react-native";
import type { ConnectionPresentation } from "../remote/connectionPresentation";
import { disconnectedCopy } from "../remote/disconnectedCopy";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { Button } from "./Button";

export function ConnectionBadge({
  presentation,
  disconnectReason,
  error,
  onReconnect,
  onUnpair,
  active = true,
}: {
  presentation: ConnectionPresentation;
  disconnectReason?: string;
  error?: string | null;
  onReconnect?: () => void;
  onUnpair?: () => void;
  active?: boolean;
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
  const color = presentation.level === "connected" ? colors.success :
    presentation.level === "connecting" ? colors.warning : colors.danger;
  const reconnectable = !!onReconnect && (
    presentation.action === "reconnect" ||
    presentation.action === "retry" ||
    presentation.action === "checkNetwork"
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
        <View style={[styles.dot, { backgroundColor: color }]} />
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
              <Text accessibilityLiveRegion="polite" style={[styles.title, { color }]}>{label}</Text>
              <Text style={styles.hint}>{reasonWithCode}</Text>
              {solution && <Text style={styles.hint}>{solution}</Text>}
              {(reconnectable || unpairable) && (
                <Button
                  compact
                  label={t(unpairable ? "sessions.unpair" : "connection.reconnect")}
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
  title: { fontSize: 15, fontWeight: "600" },
  hint: { color: colors.inkSoft, fontSize: 14, lineHeight: 21 },
});
