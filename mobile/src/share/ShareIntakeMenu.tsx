import { ArrowLeft, Folder, MessageCircle } from "lucide-react-native";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActionMenu, type MenuAction } from "../components/ActionMenu";
import { showToast } from "../features/chat/utils";
import { PeerIconGlyph } from "../remote/PeerIconGlyph";
import { useRemoteControls } from "../remote/RemoteContext";
import { pairedMachines } from "../remote/desktopStatus";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import { useTheme } from "../theme/ThemeContext";
import { needsDeviceStep, shareDevices, type ShareDevice } from "./shareDevices";
import { shareSessionGroups } from "./shareSessionGroups";
import { useShareIntake } from "./useShareIntake";

export function ShareIntakeMenu() {
  const { colors } = useTheme();
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const { pending, dismiss, chooseDestination } = useShareIntake();
  const [step, setStep] = useState<"kind" | "new" | "existing">("kind");
  /** The machine the user named; null means the question has not been asked. */
  const [deviceId, setDeviceId] = useState<string | null>(null);
  /** A machine the phone is not reading is asked for its workspaces before the
   * next step, because its own list is what that step must offer. */
  const [deviceWorkspaces, setDeviceWorkspaces] = useState<RemoteWorkspace[] | null>(null);
  const [query, setQuery] = useState("");
  /** Bumped when the machine step is left, so a read still in flight from a
   * machine the user has moved away from cannot reopen it underneath them. */
  const deviceEpoch = useRef(0);
  const activeDesktopId = remote.credentials?.expectedDesktopId ?? null;
  const devices = shareDevices(pairedMachines({
    desktops: remote.desktops,
    activeDesktopId,
    presence: remote.presence,
    presentation: remote.connectionPresentation,
    others: remote.catalogs.machines,
  }));
  // The machine step is not a stored step: it is what the sheet shows while no
  // machine has been named and there is one to name, exactly as the
  // new-conversation dialog stops asking for a single machine.
  const askingDevice = deviceId === null && needsDeviceStep(devices, activeDesktopId);
  const shown = askingDevice ? "device" : step;
  /** The machine the destinations belong to: the one named, else the open one. */
  const targetId = deviceId ?? activeDesktopId ?? "";
  const readingOpenMachine = targetId === activeDesktopId;
  const sessions: RemoteSession[] = readingOpenMachine
    ? remote.sessions
    : remote.catalogs.catalogs.find(entry => entry.desktopId === targetId)?.sessions ?? [];
  const workspaces: RemoteWorkspace[] = readingOpenMachine
    ? remote.workspaces
    : deviceWorkspaces ?? [];
  // A destination is offered only on a machine that will take the share: the
  // catalogue of a machine that went away would otherwise still list rows.
  const canTake = devices.find(device => device.desktopId === targetId)?.canTake === true;
  /** Leave the machine step, discarding whatever was read for its machine. */
  const forgetDevice = () => {
    deviceEpoch.current += 1;
    setDeviceId(null);
    setDeviceWorkspaces(null);
  };
  /** Take the machine the user named, reading it before the next step. */
  const pickDevice = async (device: ShareDevice) => {
    if (!device.canTake) return;
    const epoch = deviceEpoch.current + 1;
    deviceEpoch.current = epoch;
    if (device.desktopId === activeDesktopId) {
      setDeviceId(device.desktopId);
      setDeviceWorkspaces(null);
      return;
    }
    try {
      const read = await remote.catalogs.readWorkspaces(device.desktopId);
      if (deviceEpoch.current !== epoch) return;
      setDeviceWorkspaces(read);
      setDeviceId(device.desktopId);
    } catch {
      if (deviceEpoch.current !== epoch) return;
      showToast(t("common.error"));
    }
  };
  /** One step back: a search unwinds first, then the machine, then the mode. */
  const pop = () => {
    if (shown === "existing" && query) setQuery("");
    else if (shown === "kind" && deviceId !== null) forgetDevice();
    else setStep("kind");
  };
  const atRoot = shown === "device" || (shown === "kind" && deviceId === null);
  const back: MenuAction = {
    label: t("common.back"),
    icon: <ArrowLeft size={18} color={colors.inkSoft} />,
    keepOpen: true,
    onPress: pop,
  };
  const searching = query.trim().length > 0;
  // The picker files sessions the way the session list does; a search keeps its
  // hits in their groups, but they are shown flat so the query reads as results.
  const groups = shareSessionGroups(sessions, workspaces, query);
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
      disabled: !canTake,
      onPress: () => void chooseDestination(targetId, "session", session.sessionId),
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
  const actions: MenuAction[] = shown === "device" ? devices.map(device => ({
    label: device.label,
    icon: <PeerIconGlyph id={device.icon} size={18} />,
    disabled: !device.canTake,
    keepOpen: true,
    onPress: () => void pickDevice(device),
  })) : shown === "kind" ? [
    // The machine step is above this one only when it was asked, and the mode
    // is the question that followed it.
    ...(deviceId !== null ? [back] : []),
    {
      label: t("share.newConversation"),
      icon: <MessageCircle size={18} color={colors.accent} />,
      disabled: !canTake,
      keepOpen: true,
      onPress: () => setStep("new"),
    },
    {
      label: t("share.existingConversation"),
      icon: <MessageCircle size={18} color={colors.accent} />,
      disabled: !canTake || sessions.length === 0,
      keepOpen: true,
      onPress: () => setStep("existing"),
    },
  ] : shown === "new" ? [
    back,
    {
      label: t("share.chat"),
      icon: <MessageCircle size={18} color={colors.accent} />,
      disabled: !canTake,
      onPress: () => void chooseDestination(targetId, "chat"),
    },
    ...workspaces.map(workspace => ({
      label: t("share.workspace", { name: workspace.name }),
      icon: <Folder size={18} color={colors.accent} />,
      disabled: !canTake,
      onPress: () => void chooseDestination(targetId, "workspace", workspace.id),
    })),
  ] : existing;
  return <ActionMenu
    title={t(shown === "device" ? "share.chooseDevice" : shown === "kind" ? "share.chooseDestination" : shown === "new" ? "share.newConversation" : "share.existingConversation")}
    visible={pending !== null}
    onBack={atRoot ? undefined : pop}
    onClose={() => {
      deviceEpoch.current += 1;
      setStep("kind");
      setDeviceId(null);
      setDeviceWorkspaces(null);
      setQuery("");
      dismiss();
    }}
    search={shown === "existing" ? {
      value: query,
      label: t("share.search"),
      placeholder: t("share.search"),
      onChangeText: setQuery,
    } : undefined}
    actions={actions}
  />;
}
