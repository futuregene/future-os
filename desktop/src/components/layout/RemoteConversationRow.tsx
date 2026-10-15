import type { MergedConversation } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { iconGlyph } from "../../features/remote-peer/peerIcons";
import { RemoteConversationMenu } from "../../features/remote-peer/RemoteConversationMenu";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { RailRow, RailRowIndicator, RailRowMenuButton, RailRowSpacer, RailRowTitle, RailRowTreeToggle } from "./RailRow";

/**
 * A remote conversation in the shared list.
 *
 * Its own component rather than a synthesized `StoredThread` fed to
 * `ThreadListItem`, because almost every affordance of the local row is wrong
 * here: rename/pin/delete act on the *local* store, the run menu reads the
 * local agent, and selection writes local state. A remote row reads, is
 * prompted, and its actions are commands issued to the host that owns it.
 *
 * It is built on the same `RailRow` as the local row, though — the frame, the
 * run indicator's reserved slot and the actions trigger are shared, because
 * those are what a reader sees when scanning a list that holds both kinds, and
 * so is the tree toggle: a session on another machine is forked the same way, and
 * the host reports the parent it came from. The parts that differ are the parts
 * that mean something different: the selection checkbox (selection is a
 * local-store operation) and the machine's icon.
 */
export function RemoteConversationRow({
  active,
  compact,
  conversation,
  depth = 0,
  expanded = false,
  hasChildren = false,
  onChanged,
  onDeletedRemote,
  onOpen,
  onOpenRename,
  onToggleExpanded,
  onToggleSelection,
  peer,
  selected,
  selectionMode,
  showSource,
  unread,
}: {
  active: boolean;
  /** Whether this row sits inside a workspace group, as the local row does. */
  compact?: boolean;
  conversation: MergedConversation;
  depth?: number;
  /** Whether this row's forks are on screen. Only meaningful with `hasChildren`. */
  expanded?: boolean;
  /**
   * Whether the host reports a conversation forked from this one.
   *
   * The same control as the local row's, for the same reason: a fork is an
   * indented row under the turn it came from, and without the toggle its parent
   * is a conversation with children the reader cannot put away.
   */
  hasChildren?: boolean;
  /** Re-read the owning host's catalogue after a row action. */
  onChanged: () => void;
  /** This conversation was deleted on the host. */
  onDeletedRemote: (conversation: MergedConversation) => void;
  onOpen: (conversation: MergedConversation) => void;
  onOpenRename: (conversation: MergedConversation) => void;
  /** Fold or unfold this conversation's forks. */
  onToggleExpanded?: (conversation: MergedConversation) => void;
  /** Tick or untick this row while a batch is being assembled. */
  onToggleSelection?: (conversation: MergedConversation) => void;
  peer: RemotePeer | undefined;
  selected?: boolean;
  /**
   * Whether this row shows a checkbox.
   *
   * The same slot and the same control the local row uses, because a batch can
   * hold both machines' conversations: a remote row without one would be a row
   * the user could see the batch around and not add to it.
   */
  selectionMode?: boolean;
  /**
   * Whether the row still needs to say which machine it is on.
   *
   * False when the list is already filtered to one machine: every row is that
   * machine's, so the badge repeats what the selector above says. The title
   * does not carry the machine either way (the row has a title and a time
   * already), so the badge is only ever a qualifier.
   */
  showSource?: boolean;
  unread?: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  // The tree toggle is the local row's control, so it is named with the local
  // row's own words rather than a second set written for this list.
  const { t: tLayout } = useTranslation("layout");
  const [menuOpen, setMenuOpen] = useState(false);
  const itemRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => setMenuOpen(false),
  });

  return (
    <RailRow
      active={active}
      compact={compact}
      depth={depth}
      rowRef={itemRef}
      title={`${conversation.title} · ${peer?.name ?? conversation.desktopId ?? ""}`}
      onContextMenu={() => setMenuOpen(true)}
      onSelect={() => onOpen(conversation)}
    >
      {/* The fork toggle, in the slot the local row uses: a `+`/`−` tree node,
          named with the local row's own words so the two lists say the same
          thing about the same control. */}
      {hasChildren
        ? (
            <RailRowTreeToggle
              expanded={expanded}
              label={tLayout(expanded
                ? "activityRail.collapseThread"
                : "activityRail.expandThread", { title: conversation.title })}
              onToggle={() => onToggleExpanded?.(conversation)}
            />
          )
        : null}
      <RailRowTitle grow={false}>{conversation.title}</RailRowTitle>
      {/* The machine, immediately after the title it qualifies: the title is what
          the row is about, and the icon says whose it is. Aligned with the
          title rather than with the row's right edge — at the far edge it read
          as a second, unrelated column, and on a narrow rail it was further
          from the title than from the next row. */}
      {showSource === false
        ? null
        : (
            <span aria-hidden className="pointer-events-none shrink-0 text-xs" data-testid="remote-source-icon">
              {iconGlyph(peer?.icon)}
            </span>
          )}
      <RailRowSpacer />
      {selectionMode
        ? (
            // The same control, in the same slot, as the local row's: the
            // checkbox replaces the run indicator *and* the actions trigger
            // rather than joining them, so the two kinds of row line up while a
            // batch is being assembled.
            <input
              aria-label={t("selectConversation", { title: conversation.title })}
              checked={selected === true}
              className="relative z-10 size-5 shrink-0 rounded border-line accent-accent"
              onChange={(event) => {
                event.stopPropagation();
                onToggleSelection?.(conversation);
              }}
              type="checkbox"
            />
          )
        : (
            <>
              <RailRowIndicator status={conversation.streaming ? "running" : unread ? "completed" : undefined} unread={unread} />
              {/* The actions trigger is part of what the checkbox replaces, as
                  it is on the local row: while a batch is being assembled the
                  pointer is over the list ticking rows, and a `…` that appears
                  under it offers rename/delete/fork for the one row the user is
                  about to tick — the same menu, meaning a different thing, in
                  the one state where the row is answering "in or out of the
                  batch". */}
              <RailRowMenuButton
                label={t("conversationActions")}
                menuOpen={menuOpen}
                onToggle={() => setMenuOpen(open => !open)}
              />
            </>
          )}
      {menuOpen
        ? (
            <RemoteConversationMenu
              conversation={conversation}
              onChanged={onChanged}
              onClose={() => setMenuOpen(false)}
              onDeleted={() => onDeletedRemote(conversation)}
              onOpenRename={() => {
                setMenuOpen(false);
                onOpenRename(conversation);
              }}
            />
          )
        : null}
    </RailRow>
  );
}
