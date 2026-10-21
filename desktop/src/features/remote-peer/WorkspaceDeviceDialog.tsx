import type { RemotePeer } from "./remotePeerClient";
import { useTranslation } from "react-i18next";
import { DeviceChoiceDialog } from "./DeviceChoiceDialog";

/**
 * Which machine a new workspace is on.
 *
 * The workspace "+" used to open this app's own create-workspace form straight
 * away, whose picker browses *this* computer's disk. With another machine
 * paired that answered a question the user was not asked — they were already
 * looking at a list holding two machines' conversations, and the folder being
 * chosen belonged to one of them. So the machine is picked first, exactly as
 * the phone's new-conversation dialog does, and the form that follows belongs
 * to the machine it names.
 *
 * The same step now stands in front of a new *conversation* (`DeviceChoiceDialog`
 * with its own words), which is why the body lives there and this is only the
 * workspace's wording.
 */
export function WorkspaceDeviceDialog({
  onClose,
  onPickLocal,
  onPickPeer,
  peers,
}: {
  onClose: () => void;
  onPickLocal: () => void;
  onPickPeer: (peer: RemotePeer) => void;
  peers: RemotePeer[];
}) {
  const { t } = useTranslation("remotePeer");

  return (
    <DeviceChoiceDialog
      description={t("chooseDeviceHint")}
      onClose={onClose}
      onPickLocal={onPickLocal}
      onPickPeer={onPickPeer}
      peers={peers}
      title={t("chooseDevice")}
    />
  );
}
