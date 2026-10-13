import type { DeviceFilter } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { peerBadgeText } from "../../features/remote-peer/peerIcons";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { MenuPanel } from "../ui/MenuPanel";
import { useDropUpMenu } from "./hooks/useDropUpMenu";
import { RailHeaderButton } from "./RailChrome";

/**
 * "New conversation", and the machine it belongs to.
 *
 * A conversation lives on one machine, so the control has to know which — and the
 * list it sits in can be showing one machine or all of them:
 *
 * - **One machine selected**: there is nothing to ask. The control creates the
 *   conversation there, whether that is this machine or another one. It used to
 *   be offered only for this machine, so filtering to a host left the reader with
 *   no way to start a conversation on the host they had just chosen to look at.
 * - **All machines**: the machine *is* the choice, so the control asks which —
 *   this machine's name first, then each paired one.
 *
 * The names are the ones the rail already uses for those machines, so a
 * conversation started from here lands where the reader expects.
 */
export function RailNewChatButton({
  filter,
  onNewChatLocal,
  onNewChatRemote,
  peers,
}: {
  filter: DeviceFilter;
  /** Start a conversation on this machine. */
  onNewChatLocal?: () => void;
  /** Start a conversation on another machine (no workspace in particular). */
  onNewChatRemote?: (desktopId: string, workspaceId: string | null) => void;
  peers: RemotePeer[];
}) {
  const { t } = useTranslation("layout");
  const { t: tPeer } = useTranslation("remotePeer");
  const [open, setOpen] = useState(false);
  const layerRef = useDismissableLayer<HTMLDivElement>({
    enabled: open,
    onDismiss: () => setOpen(false),
  });
  const { menuRef, dropUp } = useDropUpMenu(open);
  const label = t("activityRail.newChatShort");

  // One machine on screen: the control acts, rather than asking a question with a
  // single answer.
  if (filter.kind === "device") {
    const handler = filter.desktopId === null
      ? onNewChatLocal
      : onNewChatRemote
        ? () => onNewChatRemote(filter.desktopId!, null)
        : undefined;
    if (!handler)
      return null;
    return <RailHeaderButton icon={<Plus className="size-3.5" />} label={label} onClick={handler} />;
  }

  // The whole list: which machine is the choice. With nothing to choose between —
  // no machines paired — the plain local control is what is left.
  if (peers.length === 0) {
    if (!onNewChatLocal)
      return null;
    return <RailHeaderButton icon={<Plus className="size-3.5" />} label={label} onClick={onNewChatLocal} />;
  }

  return (
    <div className="relative" ref={layerRef}>
      <RailHeaderButton
        icon={<Plus className="size-3.5" />}
        label={label}
        onClick={() => setOpen(value => !value)}
      />
      {open
        ? (
            <MenuPanel
              className={cn("absolute right-0 z-40 w-56 p-1", dropUp ? "bottom-7" : "top-7")}
              ref={menuRef}
              role="menu"
            >
              {onNewChatLocal
                ? (
                    <Item
                      onClick={() => {
                        setOpen(false);
                        onNewChatLocal();
                      }}
                    >
                      {tPeer("selector.local")}
                    </Item>
                  )
                : null}
              {peers.map(peer => (
                <Item
                  key={peer.desktopId}
                  onClick={() => {
                    setOpen(false);
                    onNewChatRemote?.(peer.desktopId, null);
                  }}
                >
                  {peerBadgeText(peer, peer.desktopId)}
                </Item>
              ))}
            </MenuPanel>
          )
        : null}
    </div>
  );
}

/** One machine in the menu. */
function Item({ children, onClick }: { children: string; onClick: () => void }) {
  return (
    <button
      className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
      onClick={onClick}
      role="menuitem"
      type="button"
    >
      <span className="min-w-0 flex-1 truncate">{children}</span>
    </button>
  );
}
