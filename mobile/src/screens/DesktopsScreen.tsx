import { Check, Monitor, Pencil, Trash2 } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Alert,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { useRemoteControls } from "../remote/RemoteContext";
import type { PairedDesktop } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

const MAX_NAME_LENGTH = 40;
const renameDefault = (desktop: PairedDesktop) => desktop.name ?? desktop.desktopId;

export function DesktopsScreen({ onBack, onAdd }: { onBack(): void; onAdd(): void }) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<PairedDesktop | null>(null);
  const [renameValue, setRenameValue] = useState("");

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
      onBack();
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
    <SafeAreaView edges={["top", "bottom"]} style={styles.safe}>
      <View style={styles.column}>
        <ScrollView contentContainerStyle={styles.scroll}>
          <View style={styles.header}>
            <Text style={styles.title}>{t("desktops.title")}</Text>
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
                  style={styles.select}
                >
                  <Monitor color={selected ? colors.accent : colors.inkMuted} size={20} />
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
                  hitSlop={8}
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
                  hitSlop={8}
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
        </ScrollView>
        <View style={styles.footer}>
          {failed && (
            <Text accessibilityRole="alert" style={styles.error}>{t(failed)}</Text>
          )}
          <Button disabled={busy} label={t("desktops.add")} onPress={onAdd} />
          <Button
            disabled={busy}
            label={t("common.close")}
            onPress={onBack}
            variant="secondary"
          />
        </View>
      </View>

      <Modal
        animationType="fade"
        onRequestClose={() => setRenameTarget(null)}
        transparent
        visible={renameTarget !== null}
      >
        <KeyboardAvoidingView
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          style={styles.overlay}
        >
          <View style={styles.dialog}>
            <Text style={styles.dialogTitle}>{t("desktops.rename")}</Text>
            <TextInput
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
          </View>
        </KeyboardAvoidingView>
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
    maxWidth: 560,
    alignSelf: "center",
    paddingHorizontal: spacing.xl,
    paddingTop: spacing.xl,
    paddingBottom: spacing.lg,
  },
  scroll: { paddingBottom: spacing.xl, gap: spacing.md },
  header: { gap: spacing.sm, paddingBottom: spacing.sm },
  title: { color: colors.inkStrong, fontSize: 26, fontWeight: "700", textAlign: "center" },
  description: { color: colors.inkSoft, fontSize: 15, lineHeight: 22, textAlign: "center" },
  card: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.lg,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.md,
    minHeight: 60,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  cardSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  select: { flex: 1, flexDirection: "row", alignItems: "center", gap: spacing.md },
  identity: { flex: 1, gap: spacing.xs },
  name: { color: colors.inkStrong, fontSize: 16, fontWeight: "600" },
  nameFallback: { color: colors.ink, fontWeight: "500" },
  id: { color: colors.inkMuted, fontSize: 12 },
  empty: { color: colors.inkSoft, fontSize: 15, textAlign: "center", paddingVertical: spacing.xl },
  footer: { gap: spacing.md },
  error: { color: colors.danger, fontSize: 14, textAlign: "center" },
  overlay: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: spacing.xl,
    backgroundColor: colors.overlay,
  },
  dialog: {
    width: "100%",
    maxWidth: 420,
    padding: spacing.xl,
    gap: spacing.lg,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
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
