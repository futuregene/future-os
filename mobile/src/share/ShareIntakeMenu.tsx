import { Folder, MessageCircle } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { ActionMenu } from "../components/ActionMenu";
import { useRemoteControls } from "../remote/RemoteContext";
import { colors } from "../theme/tokens";
import { useShareIntake } from "./useShareIntake";

export function ShareIntakeMenu() {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const { pending, dismiss, chooseDestination } = useShareIntake();
  const wrongDesktop = pending?.desktopId !== remote.credentials?.expectedDesktopId;
  return <ActionMenu
    title={t("share.newConversation")}
    visible={pending !== null}
    onClose={dismiss}
    actions={[
      {
        label: t("share.chat"),
        icon: <MessageCircle size={18} color={colors.accent} />,
        disabled: wrongDesktop,
        onPress: () => void chooseDestination("chat"),
      },
      ...remote.workspaces.map(workspace => ({
        label: t("share.workspace", { name: workspace.name }),
        icon: <Folder size={18} color={colors.accent} />,
        disabled: wrongDesktop,
        onPress: () => void chooseDestination("workspace", workspace.id),
      })),
    ]}
  />;
}
