import { MonitorOff } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { StyleSheet, Text, View } from "react-native";
import { Button } from "../components/Button";
import { spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

/**
 * The page a phone with one pairing sees while that desktop is in use elsewhere.
 *
 * With a single machine there is nowhere else to read, and a taken-over one
 * answers the list with refusals: every command comes back null and sending is
 * blocked. Drawing the conversations first asked the user to browse a list they
 * could not act on, while the only way out — the takeover — waited behind the
 * status dot. So the page asks for it directly.
 *
 * It is only the list that is replaced: the header above still holds the picker,
 * the connection badge and settings, because pairing and settings stay reachable
 * while the machine is someone else's.
 */
export function StandbyScreen({
  holder,
  onTakeControl,
}: {
  holder?: { name?: string } | null;
  onTakeControl(): Promise<void>;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const name = holder?.name?.trim();

  const take = async () => {
    if (busy) return;
    setBusy(true);
    setFailed(false);
    try {
      await onTakeControl();
    }
    catch {
      // The desktop refused (or the request never left): the page stays up with
      // the reason beside the button, so the next attempt is a tap away.
      setFailed(true);
    }
    finally {
      setBusy(false);
    }
  };

  return (
    <View style={styles.page}>
      <View style={styles.content}>
        <View style={styles.iconSlot}>
          <MonitorOff color={colors.warning} size={34} />
        </View>
        <Text accessibilityLiveRegion="polite" style={styles.title}>
          {name
            ? t("connection.controlHeldBy", { name })
            : t("connection.controlHeldByUnknown")}
        </Text>
        <View style={styles.details}>
          <Text style={styles.hint}>{t("connection.standbyPageHint")}</Text>
          <Text style={styles.hint}>
            {name
              ? t("connection.takeControlHint", { name })
              : t("connection.takeControlHintUnknown")}
          </Text>
          {failed && <Text style={styles.failed}>{t("connection.takeControlFailed")}</Text>}
        </View>
        <View style={styles.actionSlot}>
          <Button
            label={t("connection.takeControl")}
            loading={busy}
            onPress={() => void take()}
          />
        </View>
      </View>
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  page: { flex: 1, backgroundColor: theme.colors.surface, justifyContent: "center" },
  content: { alignItems: "center", padding: spacing.xl, gap: spacing.md },
  iconSlot: {
    width: 64,
    height: 64,
    borderRadius: 32,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.warningSoft,
  },
  title: {
    color: theme.colors.ink,
    textAlign: "center",
    fontSize: theme.font(18),
    fontWeight: "600",
  },
  details: { alignSelf: "stretch", alignItems: "center", gap: spacing.xs },
  hint: { color: theme.colors.inkMuted, textAlign: "center", fontSize: theme.font(14) },
  failed: { color: theme.colors.danger, textAlign: "center", fontSize: theme.font(14) },
  actionSlot: { minHeight: 48, alignItems: "center", justifyContent: "center" },
}));
