import { ArrowLeft, Check, Monitor, Pencil, Plus, Trash2 } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BackHandler,
  Keyboard,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { DialogSurface } from "../components/DialogSurface";
import { useAppDialog } from "../components/useAppDialog";
import { useRemoteControls } from "../remote/RemoteContext";
import type { PairedDesktop } from "../remote/types";
import { colors, layout, radius, spacing } from "../theme/tokens";

const MAX_NAME_LENGTH = 40;
const renameDefault = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;

export function DesktopsScreen({ onBack, onAdd }: { onBack?(): void; onAdd(): void }) {
  const { t } = useTranslation();
  const Alert = useAppDialog();
  const remote = useRemoteControls();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<PairedDesktop | null>(null);
  const [renameValue, setRenameValue] = useState("");

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
      t("sessions.unpairConfirm", { name: renameDefault(desktop) }),
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

  const submitRename = async () => {
    const desktop = renameTarget;
    if (!desktop) return;
    const name = renameValue.trim();
    setRenameTarget(null);
    if (name === (desktop.name ?? "")) return;
    try {
      await remote.renameDesktop(desktop.desktopId, name);
    } catch {
      setFailed("desktops.actionFailed");
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
                  accessibilityRole="button"
                  accessibilityState={{ selected, disabled: busy }}
                  disabled={busy}
                  onPress={() => void select(desktop.desktopId)}
                  style={({ pressed }) => [styles.select, pressed && styles.pressed]}
                >
                  <View style={[styles.desktopIcon, selected && styles.desktopIconSelected]}>
                    <Monitor color={selected ? colors.accent : colors.inkSoft} size={22} />
                  </View>
                  <View style={styles.identity}>
                    <Text
                      numberOfLines={1}
                      style={[styles.name, desktop.name ? null : styles.nameFallback]}
                    >
                      {renameDefault(desktop)}
                    </Text>
                    {desktop.name ? (
                      <Text numberOfLines={1} style={styles.id}>{desktop.desktopId}</Text>
                    ) : null}
                  </View>
                  {selected ? <Check color={colors.accent} size={18} /> : null}
                </Pressable>
                <Pressable
                  accessibilityLabel={t("desktops.rename")}
                  accessibilityRole="button"
                  disabled={busy}
                  style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
                  onPress={() => {
                    setRenameTarget(desktop);
                    setRenameValue(desktop.name ?? "");
                  }}
                >
                  <Pencil color={colors.inkSoft} size={18} />
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

      <Modal
        animationType="fade"
        onRequestClose={() => setRenameTarget(null)}
        transparent
        visible={renameTarget !== null}
      >
        <DialogSurface>
            <Text style={styles.dialogTitle}>{t("desktops.rename")}</Text>
            <TextInput
              accessibilityLabel={t("desktops.rename")}
              autoCapitalize="none"
              autoCorrect={false}
              autoFocus
              maxLength={MAX_NAME_LENGTH}
              onChangeText={setRenameValue}
              onSubmitEditing={() => {
                Keyboard.dismiss();
                void submitRename();
              }}
              placeholder={renameTarget?.desktopId}
              placeholderTextColor={colors.inkMuted}
              returnKeyType="done"
              selectTextOnFocus
              style={styles.input}
              value={renameValue}
            />
            <View style={styles.dialogActions}>
              <View style={styles.dialogAction}>
                <Button
                  compact
                  label={t("chat.cancel")}
                  onPress={() => setRenameTarget(null)}
                  variant="secondary"
                />
              </View>
              <View style={styles.dialogAction}>
                <Button compact label={t("chat.save")} onPress={() => void submitRename()} />
              </View>
            </View>
        </DialogSurface>
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
  iconButton: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  pressed: { backgroundColor: colors.surfaceSubtle },
  identity: { flex: 1, minWidth: 0, gap: spacing.xs },
  name: { color: colors.inkStrong, fontSize: 16, fontWeight: "600" },
  nameFallback: { color: colors.ink, fontWeight: "500" },
  id: { color: colors.inkMuted, fontSize: 12 },
  empty: { color: colors.inkSoft, fontSize: 15, textAlign: "center", paddingVertical: spacing.xl },
  footer: { marginTop: "auto", paddingTop: spacing.lg, gap: spacing.md },
  error: { color: colors.danger, fontSize: 14, textAlign: "center" },
  dialogTitle: { color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  input: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    color: colors.ink,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    fontSize: 15,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
