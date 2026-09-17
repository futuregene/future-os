import { ArrowLeft, Folder, MessageCircle } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ActionMenu, type MenuAction } from "../components/ActionMenu";
import { useRemoteControls } from "../remote/RemoteContext";
import { colors } from "../theme/tokens";
import { useShareIntake } from "./useShareIntake";

export function ShareIntakeMenu() {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const { pending, dismiss, chooseDestination } = useShareIntake();
  const [step, setStep] = useState<"kind" | "new" | "existing">("kind");
  const wrongDesktop = pending?.desktopId !== remote.credentials?.expectedDesktopId;
  const back: MenuAction = {
    label: t("common.back"),
    icon: <ArrowLeft size={18} color={colors.inkSoft} />,
    keepOpen: true,
    onPress: () => setStep("kind"),
  };
  const actions: MenuAction[] = step === "kind" ? [
    {
      label: t("share.newConversation"),
      icon: <MessageCircle size={18} color={colors.accent} />,
      disabled: wrongDesktop,
      keepOpen: true,
      onPress: () => setStep("new"),
    },
    {
      label: t("share.existingConversation"),
      icon: <MessageCircle size={18} color={colors.accent} />,
      disabled: wrongDesktop || remote.sessions.length === 0,
      keepOpen: true,
      onPress: () => setStep("existing"),
    },
  ] : step === "new" ? [
    back,
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
  ] : [
    back,
    ...remote.sessions.map(session => {
      const workspace = remote.workspaces.find(item => item.id === session.workspaceId);
      const title = session.title.trim() || t("sessions.unnamed");
      return {
        label: workspace
          ? t("share.existingWorkspace", { title, name: workspace.name })
          : t("share.existing", { title }),
        icon: <MessageCircle size={18} color={colors.accent} />,
        disabled: wrongDesktop,
        onPress: () => void chooseDestination("session", session.sessionId),
      };
    }),
  ];
  return <ActionMenu
    title={t(step === "kind" ? "share.chooseDestination" : step === "new" ? "share.newConversation" : "share.existingConversation")}
    visible={pending !== null}
    onBack={step === "kind" ? undefined : () => setStep("kind")}
    onClose={() => {
      setStep("kind");
      dismiss();
    }}
    actions={actions}
  />;
}
