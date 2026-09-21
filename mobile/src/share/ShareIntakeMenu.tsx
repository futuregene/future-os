import { ArrowLeft, Folder, MessageCircle } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ActionMenu, type MenuAction } from "../components/ActionMenu";
import { useRemoteControls } from "../remote/RemoteContext";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import { colors } from "../theme/tokens";
import { shareSessionGroups } from "./shareSessionGroups";
import { useShareIntake } from "./useShareIntake";

export function ShareIntakeMenu() {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const { pending, dismiss, chooseDestination } = useShareIntake();
  const [step, setStep] = useState<"kind" | "new" | "existing">("kind");
  const [query, setQuery] = useState("");
  const wrongDesktop = pending?.desktopId !== remote.credentials?.expectedDesktopId;
  /** A search inside the tree unwinds one level at a time: query, then step. */
  const up = () => {
    if (step === "existing" && query) setQuery("");
    else setStep("kind");
  };
  const back: MenuAction = {
    label: t("common.back"),
    icon: <ArrowLeft size={18} color={colors.inkSoft} />,
    keepOpen: true,
    onPress: up,
  };
  const searching = query.trim().length > 0;
  // The picker files sessions the way the session list does; a search keeps its
  // hits in their groups, but they are shown flat so the query reads as results.
  const groups = shareSessionGroups(remote.sessions, remote.workspaces, query);
  const destination = (session: RemoteSession, workspace: RemoteWorkspace | null): MenuAction => {
    const title = session.title.trim() || t("sessions.unnamed");
    return {
      // Browsing shows the title alone under its workspace heading, so a flat
      // hit has to name the workspace it came from.
      label: searching
        ? workspace
          ? t("share.existingWorkspace", { name: workspace.name.trim() || t("sessions.workspace"), title })
          : t("share.existing", { title })
        : title,
      icon: <MessageCircle size={18} color={colors.accent} />,
      nested: !searching && workspace !== null,
      disabled: wrongDesktop,
      onPress: () => void chooseDestination("session", session.sessionId),
    };
  };
  const existing: MenuAction[] = groups.length === 0
    ? [back, { label: t("sessions.noResults"), heading: true }]
    : [
        back,
        // Workspace-less conversations are roots here, as they are in the list.
        ...groups.flatMap(group => [
          ...(group.workspace && !searching
            ? [{
                label: group.workspace.name.trim() || t("sessions.workspace"),
                icon: <Folder size={18} color={colors.inkSoft} />,
                heading: true,
              } as MenuAction]
            : []),
          ...group.sessions.map(session => destination(session, group.workspace)),
        ]),
      ];
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
  ] : existing;
  return <ActionMenu
    title={t(step === "kind" ? "share.chooseDestination" : step === "new" ? "share.newConversation" : "share.existingConversation")}
    visible={pending !== null}
    onBack={step === "kind" ? undefined : up}
    onClose={() => {
      setStep("kind");
      setQuery("");
      dismiss();
    }}
    search={step === "existing" ? {
      value: query,
      label: t("share.search"),
      placeholder: t("share.search"),
      onChangeText: setQuery,
    } : undefined}
    actions={actions}
  />;
}
