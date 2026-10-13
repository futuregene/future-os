import { Check } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { iconGlyph } from "../../remote/peerIcons";
import { useRemoteControls } from "../../remote/RemoteContext";
import { colors, radius, spacing } from "../../theme/tokens";
import { SettingsLink, settingsStyles } from "./SettingsPrimitives";

/**
 * Which desktop this phone connects to.
 *
 * A phone-side choice rather than a desktop setting: it has to work while the
 * current desktop is offline, which is exactly when someone comes looking for
 * it. Selecting a machine reconnects the phone, and the settings screen above
 * this page is keyed by the pairing, so a successful switch remounts the stack
 * on the new desktop instead of leaving a level editing the old one.
 */
export function DesktopsSettingsPage({ onSwitched, onManage }: {
  /** The switch completed; leave this level (the screen itself may already be remounting). */
  onSwitched?(): void;
  /** Hand over to the full pairing list to add, rename or remove a desktop. */
  onManage?(): void;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const [busy, setBusy] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  // The connection this page belongs to can be replaced (or the whole settings
  // screen remounted) while a switch is in flight; nothing after that belongs
  // to this render.
  const active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const currentPairId = remote.credentials?.pairId;

  const select = async (desktopId: string) => {
    if (busy) return;
    setBusy(desktopId);
    setFailed(false);
    try {
      await remote.switchDesktop(desktopId);
      if (active.current) onSwitched?.();
    } catch {
      if (active.current) setFailed(true);
    } finally {
      if (active.current) setBusy(null);
    }
  };

  return <ScrollView contentContainerStyle={settingsStyles.content}>
    <Text style={settingsStyles.description}>{t("desktopSettings.desktopsHint")}</Text>
    {failed ? <Text accessibilityRole="alert" style={settingsStyles.error}>{t("desktops.switchFailed")}</Text> : null}
    <View accessibilityRole="radiogroup" style={styles.list}>
      {remote.desktops.map(desktop => {
        const selected = desktop.pairId === currentPairId;
        const connecting = busy === desktop.desktopId;
        const label = desktop.name || desktop.desktopId;
        return <Pressable
          accessibilityLabel={label}
          accessibilityRole="radio"
          accessibilityState={{ checked: selected, disabled: busy !== null, busy: connecting }}
          disabled={busy !== null}
          key={desktop.desktopId}
          onPress={() => void select(desktop.desktopId)}
          style={({ pressed }) => [settingsStyles.card, styles.card, selected && styles.cardSelected, pressed && settingsStyles.pressed]}
        >
          <View style={[styles.icon, selected && styles.iconSelected]}>
            {/* The machine's own mark, as the paired-desktop list draws it: the
                fixed monitor here was the third place the user's icon for a
                desktop had no effect. */}
            <Text style={styles.iconGlyph}>{iconGlyph(desktop.icon)}</Text>
          </View>
          <View style={styles.identity}>
            <Text numberOfLines={1} style={styles.name}>{label}</Text>
            {desktop.name ? <Text numberOfLines={1} style={styles.id}>{desktop.desktopId}</Text> : null}
          </View>
          {/* One spinner for the machine being connected, so a slow pairing
              does not read as a tap that did nothing. */}
          {connecting ? <ActivityIndicator color={colors.accent} size="small" /> : selected ? <Check color={colors.accent} size={18} /> : null}
        </Pressable>;
      })}
    </View>
    {remote.desktops.length === 0 ? <Text style={settingsStyles.description}>{t("desktops.empty")}</Text> : null}
    {onManage ? <SettingsLink label={t("desktops.manage")} onPress={onManage} /> : null}
  </ScrollView>;
}

const styles = StyleSheet.create({
  list: { gap: spacing.sm },
  card: { flexDirection: "row", alignItems: "center", gap: spacing.sm, minHeight: 72 },
  cardSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  icon: { width: 36, height: 36, alignItems: "center", justifyContent: "center", borderRadius: radius.md, backgroundColor: colors.surfaceSubtle },
  iconSelected: { backgroundColor: colors.surface },
  iconGlyph: { fontSize: 20 },
  identity: { flex: 1, minWidth: 0, gap: spacing.xs },
  name: { color: colors.inkStrong, fontSize: 15, fontWeight: "600" },
  id: { color: colors.inkMuted, fontSize: 12 },
});
