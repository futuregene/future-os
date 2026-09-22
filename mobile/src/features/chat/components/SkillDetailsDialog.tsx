import { X } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { DialogSurface } from "../../../components/DialogSurface";
import type { RemoteSkill } from "../../../remote/types";
import { colors, layout, radius, spacing } from "../../../theme/tokens";

/**
 * One skill's full description, opened from the `/` menu's info button.
 *
 * The picker panel is capped to a slice of the screen above the composer, so a
 * description expanded inside its row could only ever show a few lines. The
 * dialog gets the whole viewport instead, and its state lives with the composer
 * rather than the picker: the picker unmounts as soon as the slash token does.
 */
export function SkillDetailsDialog({ skill, onClose }: { skill: RemoteSkill | null; onClose: () => void }) {
  const { t, i18n } = useTranslation();
  if (!skill) return null;
  const useZh = i18n.language.startsWith("zh");
  // The row shows the localized name; the command is only news when the two
  // differ, otherwise it would repeat the title verbatim.
  const nameZh = useZh ? skill.nameZh : undefined;
  const description = useZh ? skill.descriptionZh || skill.description : skill.description;
  return (
    <Modal animationType="fade" onRequestClose={onClose} transparent visible>
      <DialogSurface>
        <View style={styles.header}>
          <Text accessibilityRole="header" numberOfLines={3} style={styles.title}>{nameZh || skill.name}</Text>
          <Pressable accessibilityLabel={t("common.close")} accessibilityRole="button" onPress={onClose}
            style={({ pressed }) => [styles.close, pressed && styles.pressed]}>
            <X color={colors.inkSoft} size={20} />
          </Pressable>
        </View>
        {nameZh ? <Text style={styles.command}>{`/${skill.name}`}</Text> : null}
        <Text style={styles.description}>{description}</Text>
      </DialogSurface>
    </Modal>
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "flex-start", gap: spacing.sm },
  title: { flex: 1, minWidth: 0, paddingVertical: spacing.sm, color: colors.inkStrong, fontSize: 18, fontWeight: "700" },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center", borderRadius: radius.pill },
  pressed: { backgroundColor: colors.surfaceSubtle },
  command: { fontSize: 13, color: colors.inkMuted },
  description: { fontSize: 14, lineHeight: 21, color: colors.inkSoft },
});
