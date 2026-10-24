import { FileText, Paperclip } from "lucide-react-native";
import { Pressable, Text } from "react-native";
import type { HistoryAttachment } from "../../../remote/types";
import { layout } from "../../../theme/tokens";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";

/**
 * The chip draws as one 26px line (13px glyph and 12px name in a 4px pad) inside
 * a message, so its tap area is the line plus this slop: 26 + 9 + 9 = the 44px
 * `layout.touchTarget` — the same arithmetic the transcript's copy button does
 * from its 28px box. It is a file's only way open from the transcript, so the
 * name alone was a target too small to hit on a phone.
 */
const CHIP_HIT_SLOP = { top: (layout.touchTarget - 26) / 2, bottom: (layout.touchTarget - 26) / 2 };

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
