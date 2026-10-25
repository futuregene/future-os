import { PeerIconChip } from "../remote/PeerIconChip";
import { ArrowLeft, Pencil, Plus, Settings2, Trash2 } from "lucide-react-native";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BackHandler,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { useAppDialog } from "../components/useAppDialog";
import { DesktopScope, useRemoteControls } from "../remote/RemoteContext";
import { machineConfigurable, machineStatusKey, pairedMachines } from "../remote/desktopStatus";
import { SettingsScreen, type SettingsScreenHandle } from "../features/settings/SettingsScreen";
import type { PairedDesktop } from "../remote/types";
import { layout, radius, slopToTouchTarget, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { DesktopLabelDialog } from "./DesktopLabelDialog";

const label = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;

/**
 * The paired desktops: the phone's device management page, and the way in to
 * each one's settings.
 *
 * A row is a description, not a control. Tapping it used to switch which
 * machine the app is talking to and leave for the session list — a move nobody
 * asked for on a page they opened to look at the machines, and one that also
 * threw away the page they were reading. Switching has its own control where
 * the sessions are (the device picker on the session list). What a row offers
 * here is the two things that are about the machine: the configure action
 * beside it — everything that is read from or written to it — and unpairing.
 * That page is offered only while the machine answers and is this phone's to
 * drive, because every row on it reads or writes that machine; the row says
 * why when it does not (see `stateLine`), and taking a held machine back is
 * offered where a machine is picked and on the connection sheet.
 *
 * Its name and icon are the phone's own label rather than the machine's, so
 * they are edited on the row itself, by the mark and the name they describe —
 * and they keep working while the machine is off, which the page behind the
 * gear cannot. Settings for a device are not reachable from anywhere else, and
 * the phone's own settings deliberately do not list machines at all.
 *
 * The one list still serves as the startup picker, for a phone that has paired
 * desktops but no usable active pair: App renders this screen with no back
 * action there, and with nothing behind it the row itself is the choice (see
 * `picking`, and the comment on the row).
 */
export function DesktopsScreen({ onBack, onAdd, onOpenConversation }: {
  onBack?(): void;
  onAdd(): void;
  /** Open a session in the chat, from a device page's task runs. */
  onOpenConversation?(sessionId: string): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const Alert = useAppDialog();
  const remote = useRemoteControls();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  /** The machine whose name and icon are being edited, if any. */
  const [labeling, setLabeling] = useState<string | null>(null);
  const settingsRef = useRef<SettingsScreenHandle>(null);
  /** A phone with paired desktops but no active pair (see App) gets this list
   * with nothing behind it, so there — and only there — a row picks the machine
   * the app uses. Opened from the session list it is the device page instead. */
  const picking = !onBack;
  /** The machine the label dialog is for — looked up on every render, so a
   * machine unpaired while it is open simply closes the dialog. */
  const labelled = labeling ? remote.desktops.find(item => item.desktopId === labeling) : undefined;
  /**
   * How each paired machine stands, in the same judgement the session list's
   * picker and status dot use — one list, so a row cannot offer this phone a
   * machine another surface has already called unreachable.
   */
  const machines = useMemo(() => pairedMachines({
    desktops: remote.desktops,
    activeDesktopId: remote.credentials?.expectedDesktopId ?? null,
    presence: remote.presence,
    presentation: remote.connectionPresentation,
    others: remote.catalogs.machines,
  }), [remote.catalogs.machines, remote.connectionPresentation, remote.credentials?.expectedDesktopId, remote.desktops, remote.presence]);

  useEffect(() => {
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (!onBack) return false;
      if (!busy) onBack();
      return true;
    });
    return () => subscription.remove();
  }, [busy, onBack]);

  const remove = (desktop: PairedDesktop) => {
    Alert.alert(
      t("sessions.unpair"),
      t("sessions.unpairConfirm", { name: label(desktop) }),
      [
        { text: t("chat.cancel"), style: "cancel" },
        {
          text: t("sessions.unpair"),
          style: "destructive",
          onPress: () => {
            setBusy(true);
            setFailed(null);
            void remote.removeDesktop(desktop.desktopId)
              .catch(() => setFailed("desktops.actionFailed"))
              .finally(() => setBusy(false));
          },
        },
      ],
    );
  };

  const select = async (desktopId: string) => {
    setBusy(true);
    setFailed(null);
    try {
      await remote.switchDesktop(desktopId);
      onBack?.();
    } catch {
      setFailed("desktops.switchFailed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <SafeAreaView style={styles.safe}>
      {Alert.dialog}
      <View style={styles.column}>
        <View style={styles.topbar}>
          {onBack ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("common.back")}
              disabled={busy}
              onPress={onBack}
              style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
            >
              <ArrowLeft color={colors.ink} size={22} />
            </Pressable>
          ) : null}
          <Text accessibilityRole="header" style={styles.title}>{t("desktops.title")}</Text>
        </View>
        <ScrollView contentContainerStyle={styles.scroll}>
          <View style={styles.header}>
            <Text style={styles.description}>{t("desktops.description")}</Text>
          </View>
          {remote.desktops.map((desktop) => {
            // Whether this is the machine the phone is reading is *not* drawn on
            // the row: beside the two actions a row carries, a highlight and a
            // check mark read as a third control, and the one thing a row here
            // cannot do is act. Assistive tech still gets it, and which machine a
            // conversation is created on is asked for where one is created,
            // marked there against the machine in use.
            const selected = remote.credentials?.pairId === desktop.pairId;
            const machine = machines.find(item => item.desktopId === desktop.desktopId);
            // The gear opens what this machine answers, so it is offered only
            // while the machine answers and is this phone's to drive. A machine
            // that is off, reconnecting or still starting has nothing behind
            // those rows, and one another device holds refuses every one of them
            // — both would be a page that opens only to say so. The reason is
            // drawn on the row instead (`stateLine`), because a greyed control
            // with no explanation reads as a broken app.
            const configurable = machineConfigurable(machine);
            const held = machine?.status === "connected" && machine.control === "standby";
            const holderName = held ? machine?.holder?.name?.trim() : undefined;
            // `machine` undefined cannot be here: an unreported machine is
            // configurable (see `machineConfigurable`).
            const stateLine = configurable || !machine
              ? null
              : held
                ? (holderName ? t("desktops.heldBy", { name: holderName }) : t("connection.machineStandbyHint"))
                : t(machineStatusKey(machine.status));
            const identity = (
              <>
                {/* The desktop's own mark, exactly as the session list and the
                    device page draw it: the icon the user picked *is* this
                    machine's identity, on the shared set's chip in that icon's
                    own colour. */}
                <PeerIconChip id={desktop.icon} size={{ height: 44, width: 36 }} />
                <View style={styles.identity}>
                  <View style={styles.nameRow}>
                    <Text
                      numberOfLines={1}
                      style={[styles.name, desktop.name ? null : styles.nameFallback]}
                    >
                      {label(desktop)}
                    </Text>
                    {/* Beside the label it edits, not among the row's actions:
                        the name and the icon are this phone's own record, so
                        they are changed here even while the machine is off —
                        unlike everything behind the gear. Not drawn on the
                        startup picker, where the whole row is the choice and a
                        second control inside it would only take taps away. */}
                    {picking
                      ? null
                      : (
                          <Pressable
                            accessibilityLabel={t("desktops.editLabel")}
                            accessibilityRole="button"
                            disabled={busy}
                            hitSlop={slopToTouchTarget(24)}
                            onPress={() => setLabeling(desktop.desktopId)}
                            style={({ pressed }) => [styles.editButton, pressed && styles.pressed]}
                          >
                            <Pencil color={colors.inkMuted} size={14} />
                          </Pressable>
                        )}
                  </View>
                  {/* What the row says under the name: the id the phone files
                      the machine under, unless something about the machine
                      stands in the way — then the state, in place of the id,
                      because that is the fact the reader needs and the one
                      explaining the gear beside it. */}
                  {stateLine
                    ? <Text numberOfLines={1} style={[styles.id, held ? styles.held : null]}>{stateLine}</Text>
                    : desktop.name
                      ? <Text numberOfLines={1} style={styles.id}>{desktop.desktopId}</Text>
                      : null}
                </View>
              </>
            );
            return (
              <View key={desktop.desktopId} style={styles.card}>
                {picking
                  ? (
                      <Pressable
                        accessibilityLabel={label(desktop)}
                        accessibilityRole="button"
                        accessibilityState={{ selected, disabled: busy }}
                        disabled={busy}
                        onPress={() => void select(desktop.desktopId)}
                        style={({ pressed }) => [styles.select, pressed && styles.pressed]}
                      >
                        {identity}
                      </Pressable>
                    )
                  : (
                      /* The same drawing as the picker's row, and nothing to press:
                         on this page the two actions beside it are what a tap can do. */
                      <View
                        accessibilityLabel={label(desktop)}
                        accessibilityState={{ selected }}
                        accessible
                        style={styles.select}
                      >
                        {identity}
                      </View>
                    )}
                {/* A settings glyph rather than a pencil: the page behind it is
                    everything the machine reads and writes, and the name and
                    icon — the phone's own record — are beside this row instead.
                    Off, still connecting, or in somebody else's hands there is
                    nothing to read or write there, so the row does not offer it
                    and says why underneath.*/}
                <Pressable
                  accessibilityLabel={t("desktops.settings")}
                  accessibilityRole="button"
                  accessibilityState={{ disabled: busy || !configurable }}
                  disabled={busy || !configurable}
                  onPress={() => setEditing(desktop.desktopId)}
                  style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
                >
                  {/* Dimmed by opacity rather than a lighter ink: `inkSoft` and
                      `inkMuted` are two greys that differ in the dark theme and
                      are nearly the same in the light one, so a token swap would
                      read as "no change" exactly where the phone is easiest to
                      read. */}
                  <View style={configurable ? null : styles.iconDisabled}>
                    <Settings2 color={colors.inkSoft} size={18} />
                  </View>
                </Pressable>
                <Pressable
                  accessibilityLabel={t("sessions.unpair")}
                  accessibilityRole="button"
                  disabled={busy}
                  style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
                  onPress={() => remove(desktop)}
                >
                  <Trash2 color={colors.danger} size={18} />
                </Pressable>
              </View>
            );
          })}
          {remote.desktops.length === 0 && (
            <Text style={styles.empty}>{t("desktops.empty")}</Text>
          )}
        <View style={styles.footer}>
          {failed && (
            <Text accessibilityRole="alert" style={styles.error}>{t(failed)}</Text>
          )}
          <Button disabled={busy} icon={<Plus color={colors.surface} size={20} />} label={t("desktops.add")} onPress={onAdd} />
        </View>
        </ScrollView>
      </View>

      {/* A full screen rather than a floating card: a machine's settings are a
          stack of pages (models, providers, skills, tasks), not a form that
          fits in a dialog. The stack owns its own levels and back handling. */}
      <Modal
        animationType="slide"
        onDismiss={() => setEditing(null)}
        onRequestClose={() => settingsRef.current?.goBack()}
        presentationStyle="fullScreen"
        visible={editing !== null}
      >
        {editing
          ? <DesktopScope desktopId={editing}>
              <SettingsScreen
                deviceId={editing}
                key={editing}
                onClose={() => setEditing(null)}
                onOpenConversation={onOpenConversation}
                ref={settingsRef}
              />
            </DesktopScope>
          : null}
      </Modal>

      {/* A dialog rather than a level inside the machine's settings: the name and
          icon are the phone's own label, so they are edited where that label is
          drawn, and the settings page keeps to what the machine itself answers.
          Resolved from the list rather than held as an object: a machine can be
          unpaired while the dialog is open, and there is nothing left to edit. */}
      {labelled ? <DesktopLabelDialog desktop={labelled} key={labelled.desktopId} onClose={() => setLabeling(null)} /> : null}
    </SafeAreaView>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  safe: { flex: 1, backgroundColor: theme.colors.canvas },
  // One centered column keeps the list readable on tablets and desktop-sized
  // windows instead of stretching the cards edge to edge.
  column: {
    flex: 1,
    width: "100%",
    maxWidth: layout.formMaxWidth,
    alignSelf: "center",
    paddingHorizontal: layout.gutter,
    paddingTop: spacing.sm,
    paddingBottom: spacing.lg,
  },
  topbar: { flexDirection: "row", alignItems: "center", gap: spacing.sm, minHeight: 56, marginBottom: spacing.md },
  scroll: { flexGrow: 1, paddingBottom: spacing.sm, gap: spacing.md },
  header: { gap: spacing.sm, paddingBottom: spacing.md },
  title: { flex: 1, color: theme.colors.inkStrong, fontSize: theme.font(22), fontWeight: "700" },
  description: { color: theme.colors.inkSoft, fontSize: theme.font(15), lineHeight: theme.leading(23) },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    minHeight: 80,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.lg,
    backgroundColor: theme.colors.surface,
  },
  select: { flex: 1, minWidth: 0, minHeight: 64, flexDirection: "row", alignItems: "center", gap: spacing.sm, borderRadius: radius.md },
  iconButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  // Half-drawn, not recoloured: an icon at 40% is unmistakably unavailable in
  // either theme, and the state line underneath says which state it is.
  iconDisabled: { opacity: 0.4 },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  identity: { flex: 1, minWidth: 0, gap: spacing.xs },
  // The name and the control that edits it share a line, so the row keeps its
  // two actions and a long name truncates instead of wrapping.
  nameRow: { flexDirection: "row", alignItems: "center", gap: spacing.xs, minWidth: 0 },
  name: { flexShrink: 1, color: theme.colors.inkStrong, fontSize: theme.font(16), fontWeight: "600" },
  nameFallback: { color: theme.colors.ink, fontWeight: "500" },
  editButton: {
    width: 24,
    height: 24,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.sm,
  },
  // A machine another device holds is not a fault — nothing is broken, and the
  // row still offers unpairing and its label — so the line is amber rather
  // than the danger tint an error would use.
  held: { color: theme.colors.warning },
  id: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  empty: { color: theme.colors.inkSoft, fontSize: theme.font(15), textAlign: "center", paddingVertical: spacing.xl },
  footer: { marginTop: "auto", paddingTop: spacing.lg, gap: spacing.md },
  error: { color: theme.colors.danger, fontSize: theme.font(14), textAlign: "center" },
}));
