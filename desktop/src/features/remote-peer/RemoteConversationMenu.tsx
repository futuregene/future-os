import type { ReactNode } from "react";
import type { MergedConversation } from "./mergeConversations";
import { Pencil, Pin, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useDropUpMenu } from "../../components/layout/hooks/useDropUpMenu";
import { MenuPanel } from "../../components/ui/MenuPanel";
import { cn } from "../../lib/cn";
import {
  deleteRemoteConversation,
  pinRemoteConversation,
} from "./remotePeerClient";

/**
 * The actions a remote conversation supports, issued to the host that owns it.
 *
 * Not a reuse of the local `ThreadItemMenu`: every action there calls the local
 * store or the local agent, and the local store has never heard of a remote
 * session. Sharing the component would mean sharing the *dispatch*, and a rename
 * would then land on a local thread that happens to have the same id.
 *
 * Unlike the local menu, an item does not close the menu before running: a
 * remote action can fail on the wire, and the message would otherwise vanish
 * with the menu that produced it. The menu closes once the host accepted the
 * command.
 */
export function RemoteConversationMenu({
  conversation,
  onChanged,
  onClose,
  onOpenRename,
}: {
  conversation: MergedConversation;
  /** Re-read the host's catalogue once the change has landed. */
  onChanged: () => void;
  onClose: () => void;
  onOpenRename: () => void;
}) {
  const { t } = useTranslation("layout");
  const { menuRef, dropUp } = useDropUpMenu();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const desktopId = conversation.desktopId;

  // Local rows are rendered by the local row component, which owns their menu.
  // Returning nothing keeps that invariant here rather than at every call site.
  if (!desktopId)
    return null;

  const address = { sessionId: conversation.id, threadId: conversation.threadId };
  // A host that never reported a thread id: pin and delete have nothing to
  // address, and the session id is not a substitute for one.
  const pinnable = Boolean(conversation.threadId);

  /**
   * The items are `disabled` while a command is outstanding, so this needs no
   * in-flight guard of its own: one mechanism, and it is the one a user sees.
   */
  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
      onChanged();
      onClose();
    }
    catch (err) {
      // Shown inside the panel: the user's next move depends on what went
      // wrong, and a message that disappears with the menu is no message.
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <MenuPanel
      ref={menuRef}
      className={cn("absolute right-1 z-40 w-36 p-1", dropUp ? "bottom-7" : "top-7")}
      role="menu"
    >
      {error ? <p className="px-2 py-1 text-xs text-danger">{error}</p> : null}
      <Item icon={<Pencil className="size-3.5" />} onClick={onOpenRename}>
        {t("activityRail.rename")}
      </Item>
      <Item
        icon={<Pin className="size-3.5" />}
        onClick={() => void run(() => pinRemoteConversation(desktopId, address, !conversation.pinned))}
        disabled={busy || !pinnable}
      >
        {conversation.pinned ? t("activityRail.unpin") : t("activityRail.pin")}
      </Item>
      <Item
        danger
        icon={<Trash2 className="size-3.5" />}
        onClick={() => void run(() => deleteRemoteConversation(desktopId, address))}
        disabled={busy || !pinnable}
      >
        {t("activityRail.delete")}
      </Item>
    </MenuPanel>
  );
}

/**
 * A menu row that stays open while its action runs.
 *
 * The local menu's items dismiss first and then act; for a host command that
 * would throw away the only place an error can be read.
 */
function Item({
  children,
  danger,
  disabled,
  icon,
  onClick,
}: {
  children: string;
  danger?: boolean;
  disabled?: boolean;
  icon: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium transition-colors",
        danger ? "text-danger hover:bg-danger-soft" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
        disabled && "cursor-not-allowed opacity-50",
      )}
      disabled={disabled}
      onClick={onClick}
      role="menuitem"
      type="button"
    >
      {icon}
      <span className="whitespace-nowrap">{children}</span>
    </button>
  );
}
