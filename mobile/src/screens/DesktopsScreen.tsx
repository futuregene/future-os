import { iconGlyph } from "../remote/peerIcons";
import { ArrowLeft, Check, Plus, Settings2, Trash2 } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
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
import { SettingsScreen, type SettingsScreenHandle } from "../features/settings/SettingsScreen";
import { useRemoteControls } from "../remote/RemoteContext";
import type { PairedDesktop } from "../remote/types";
import { colors, layout, radius, spacing } from "../theme/tokens";

const label = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;

/**
 * The paired desktops, and the way in to each one's settings.
 *
 * One list serves both jobs: a row picks which machine this phone uses, and the
 * configure action beside it opens that machine's own page — its name and icon,
 * plus everything that is read from or written to it. Settings for a device are
 * not reachable from anywhere else, and the phone's own settings deliberately
 * do not list machines at all.
 */
export function DesktopsScreen({ onBack, onAdd, onOpenConversation }: {
  onBack?(): void;
  onAdd(): void;
  /** Open a session in the chat, from a device page's task runs. */
  onOpenConversation?(sessionId: string): void;
}) {
  const { t } = useTranslation();
  const Alert = useAppDialog();
  const remote = useRemoteControls();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const settingsRef = useRef<SettingsScreenHandle>(null);

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
            const selected = remote.credentials?.pairId === desktop.pairId;
            return (
              <View key={desktop.desktopId} style={[styles.card, selected && styles.cardSelected]}>
                <Pressable
                  accessibilityLabel={label(desktop)}
                  accessibilityRole="button"
                  accessibilityState={{ selected, disabled: busy }}
                  disabled={busy}
                  onPress={() => void select(desktop.desktopId)}
                  style={({ pressed }) => [styles.select, pressed && styles.pressed]}
                >
                  <View style={[styles.desktopIcon, selected && styles.desktopIconSelected]}>
                    {/* The desktop's own mark, exactly as the session list and the
                        device page draw it: the icon the user picked *is* this
                        machine's identity. The generic monitor that used to be here
                        was the one place the choice had no effect — the picker wrote
                        the id and nothing on this screen ever read it back. */}
                    <Text style={styles.desktopGlyph}>{iconGlyph(desktop.icon)}</Text>
                  </View>
                  <View style={styles.identity}>
                    <Text
                      numberOfLines={1}
                      style={[styles.name, desktop.name ? null : styles.nameFallback]}
                    >
                      {label(desktop)}
                    </Text>
                    {desktop.name ? (
                      <Text numberOfLines={1} style={styles.id}>{desktop.desktopId}</Text>
                    ) : null}
                  </View>
                  {selected ? <Check color={colors.accent} size={18} /> : null}
                </Pressable>
                {/* A settings glyph rather than a pencil: the page behind it is the
                    machine's whole configuration, and the name and icon are two
                    fields on it. */}
                <Pressable
                  accessibilityLabel={t("desktops.settings")}
                  accessibilityRole="button"
                  disabled={busy}
                  style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
                  onPress={() => setEditing(desktop.desktopId)}
                >
                  <Settings2 color={colors.inkSoft} size={18} />
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
          ? <SettingsScreen
              deviceId={editing}
              key={editing}
              onClose={() => setEditing(null)}
              onOpenConversation={onOpenConversation}
              ref={settingsRef}
            />
          : null}
      </Modal>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.canvas },
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
  title: { flex: 1, color: colors.inkStrong, fontSize: 22, fontWeight: "700" },
  description: { color: colors.inkSoft, fontSize: 15, lineHeight: 23 },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    minHeight: 80,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  cardSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  select: { flex: 1, minWidth: 0, minHeight: 64, flexDirection: "row", alignItems: "center", gap: spacing.sm, borderRadius: radius.md },
  desktopIcon: { width: 36, height: 44, alignItems: "center", justifyContent: "center", borderRadius: radius.md, backgroundColor: colors.surfaceSubtle },
  desktopIconSelected: { backgroundColor: colors.surface },
  desktopGlyph: { fontSize: 20 },
  iconButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  pressed: { backgroundColor: colors.surfaceSubtle },
  identity: { flex: 1, minWidth: 0, gap: spacing.xs },
  name: { color: colors.inkStrong, fontSize: 16, fontWeight: "600" },
  nameFallback: { color: colors.ink, fontWeight: "500" },
  id: { color: colors.inkMuted, fontSize: 12 },
  empty: { color: colors.inkSoft, fontSize: 15, textAlign: "center", paddingVertical: spacing.xl },
  footer: { marginTop: "auto", paddingTop: spacing.lg, gap: spacing.md },
  error: { color: colors.danger, fontSize: 14, textAlign: "center" },
});
