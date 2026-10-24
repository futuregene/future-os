import { FileText, Paperclip } from "lucide-react-native";
import { Pressable, Text } from "react-native";
import type { HistoryAttachment } from "../../../remote/types";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";
import { CHIP_HIT_SLOP } from "./touchTargets";

/**
 * The chip draws as one 26px line (13px glyph and 12px name in a 4px pad) inside
 * a message, so its tap area is that line plus `CHIP_HIT_SLOP`: the same
 * arithmetic the transcript's copy button does from its own box. It is a file's
 * only way open from the transcript, so the name alone was a target too small to
 * hit on a phone. The slop lives in `touchTargets.ts` — `react-native-web`
 * ignores `hitSlop`, so a capture cannot show it.
 */
export function AttachmentChip({
  attachment,
  onOpen,
}: {
  attachment: HistoryAttachment;
  onOpen?(): void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityLabel={attachment.name}
      accessibilityRole="button"
      disabled={!onOpen}
      hitSlop={CHIP_HIT_SLOP}
      onPress={onOpen}
      style={({ pressed }) => [styles.attachmentChip, pressed && styles.attachmentChipPressed]}
    >
      {attachment.kind === "file" ? (
        <FileText color={colors.inkSoft} size={13} />
      ) : (
        <Paperclip color={colors.inkSoft} size={13} />
      )}
      <Text numberOfLines={1} style={styles.attachmentName}>
        {attachment.name}
      </Text>
    </Pressable>
  );
}
