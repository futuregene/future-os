import { Camera, File, Images } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { ActionMenu } from "../../../components/ActionMenu";
import { useTheme } from "../../../theme/ThemeContext";

/** Where an attachment comes from, for whichever surface is asking. */
export type AttachmentSource = "camera" | "album" | "files";

/**
 * The three sources a message's files can come from.
 *
 * Shared by the composer's paperclip and a waiting queue row's "add": the row
 * being edited does not change where a photo comes from, and two menus that drift
 * apart would offer the same gesture in two different places.
 */
export function AttachmentSourceMenu({
  visible,
  onClose,
  onPick,
}: {
  visible: boolean;
  onClose: () => void;
  onPick: (source: AttachmentSource) => void;
}) {
  const { t } = useTranslation();
  const { colors } = useTheme();
  return (
    <ActionMenu
      title={t("attachment.title")}
      visible={visible}
      onClose={onClose}
      actions={[
        {
          label: t("attachment.takePhoto"),
          icon: <Camera color={colors.inkSoft} size={18} />,
          onPress: () => onPick("camera"),
        },
        {
          label: t("attachment.chooseFromAlbum"),
          icon: <Images color={colors.inkSoft} size={18} />,
          onPress: () => onPick("album"),
        },
        {
          label: t("attachment.chooseFiles"),
          icon: <File color={colors.inkSoft} size={18} />,
          onPress: () => onPick("files"),
        },
      ]}
    />
  );
}
