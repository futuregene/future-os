import { Check, Monitor, Trash2 } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { useRemoteControls } from "../remote/RemoteContext";
import { colors, radius, spacing } from "../theme/tokens";

export function DesktopsScreen({ onBack, onAdd }: { onBack(): void; onAdd(): void }) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const [switching, setSwitching] = useState(false);
  const [failed, setFailed] = useState(false);

  const remove = (desktopId: string) => {
    Alert.alert(t("sessions.unpair"), `${desktopId}\n\n${t("sessions.unpairConfirm")}`, [
      { text: t("chat.cancel"), style: "cancel" },
      {
        text: t("sessions.unpair"),
        style: "destructive",
        onPress: () => {
          setSwitching(true);
          setFailed(false);
          void remote.removeDesktop(desktopId)
            .catch(() => setFailed(true))
            .finally(() => setSwitching(false));
        },
      },
    ]);
  };

  const select = async (desktopId: string) => {
    setSwitching(true);
    setFailed(false);
    try {
      await remote.switchDesktop(desktopId);
      onBack();
    } catch {
      setFailed(true);
    } finally {
      setSwitching(false);
    }
  };

  return (
    <SafeAreaView style={styles.page}>
      <Text style={styles.title}>{t("desktops.title")}</Text>
      <Text style={styles.description}>{t("desktops.description")}</Text>
      <ScrollView contentContainerStyle={styles.list}>
        {remote.desktops.map((desktop) => {
          const selected = remote.credentials?.pairId === desktop.pairId;
          return (
            <View key={desktop.desktopId} style={styles.desktop}>
              <Pressable
                accessibilityRole="button"
                accessibilityState={{ selected, disabled: switching }}
                disabled={switching}
                onPress={() => void select(desktop.desktopId)}
                style={styles.select}
              >
                <Monitor color={colors.accent} size={22} />
                <View style={styles.identity}>
                  <Text style={styles.name}>{t("desktops.desktop")}</Text>
                  <Text style={styles.id}>{desktop.desktopId}</Text>
                </View>
                {selected && <Check color={colors.accent} size={20} />}
              </Pressable>
              <Pressable
                accessibilityLabel={t("sessions.unpair")}
                accessibilityRole="button"
                disabled={switching}
                onPress={() => remove(desktop.desktopId)}
                hitSlop={8}
              >
                <Trash2 color={colors.danger} size={20} />
              </Pressable>
            </View>
          );
        })}
        {remote.desktops.length === 0 && (
          <Text style={styles.description}>{t("desktops.empty")}</Text>
        )}
      </ScrollView>
      {failed && (
        <Text accessibilityRole="alert" style={styles.error}>{t("desktops.switchFailed")}</Text>
      )}
      <Button disabled={switching} label={t("desktops.add")} onPress={onAdd} />
      <Button
        disabled={switching}
        loading={switching}
        label={t("common.close")}
        onPress={onBack}
        variant="secondary"
      />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  page: { flex: 1, backgroundColor: colors.canvas, padding: spacing.xl, gap: spacing.lg },
  title: { color: colors.inkStrong, fontSize: 28, fontWeight: "700" },
  description: { color: colors.inkSoft, fontSize: 15, lineHeight: 22 },
  list: { gap: spacing.md },
  desktop: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    padding: spacing.lg,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    backgroundColor: colors.surface,
  },
  select: { flex: 1, flexDirection: "row", alignItems: "center", gap: spacing.md },
  identity: { flex: 1, gap: spacing.sm },
  name: { color: colors.inkStrong, fontSize: 16, fontWeight: "600" },
  id: { color: colors.inkSoft, fontSize: 12 },
  error: { color: colors.danger, fontSize: 14 },
});
