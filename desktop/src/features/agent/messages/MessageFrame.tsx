import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../../lib/cn";
import { formatDateTime, formatMessageTimestamp } from "../../../lib/date";
import { useNow } from "../../../lib/useNow";

/**
 * The frame a conversation draws every message in.
 *
 * Both views use it — this app's own `MessageBlock` and the remote one — because
 * these classes *are* the look: an author line above the text, ruled off on the
 * assistant's side, and the two roles' bodies distinguished by the bubble and
 * the width rather than by colour. Two copies of that would be two conversations
 * that read as different products, which is what the remote view was: accent-
 * coloured user bubbles and a bordered card for the assistant, neither of them
 * anything this app draws.
 *
 * What a caller adds inside is its own: the activity lines, the reply steps, the
 * failure notice, the fork control. What it cannot change is the geometry, the
 * typography and the timestamps — a message's place on the page is not a
 * decoration a view gets to pick.
 */
export function MessageFrame({
  author,
  bodyClassName,
  children,
  createdAt,
  dataMessageId,
  onEnter,
  onLeave,
  role,
}: {
  /** Already localized: "You" / "Research Copilot". */
  author: string;
  /** Extra classes for the body, e.g. a streaming block's containment. */
  bodyClassName?: string;
  children: ReactNode;
  /** ISO timestamp, as the projection's messages carry it. */
  createdAt: string;
  dataMessageId?: string;
  onEnter?: () => void;
  onLeave?: () => void;
  role: "assistant" | "user";
}) {
  const { t, i18n } = useTranslation("agent");
  // Re-render on a minute cadence so the relative timestamp stays accurate as it
  // ages, instead of freezing at its first-render value.
  const now = useNow();
  const isUser = role === "user";

  return (
    <article className="flex justify-center">
      <div
        className="min-w-0 w-full max-w-3xl"
        data-message-id={dataMessageId}
        onPointerLeave={onLeave}
        onPointerOver={onEnter}
      >
        <div
          className={cn(
            "flex items-center gap-2",
            // The user's own line sits above their bubble, right-aligned with it;
            // the assistant's is ruled off, because everything below it is one
            // reply and the rule is what separates replies.
            isUser ? "mb-1 justify-end" : "mb-3 border-b border-line-soft/40 pb-2",
          )}
        >
          <span className="text-sm font-semibold text-ink">{author}</span>
          <span className="text-xs text-ink-muted" title={formatDateTime(createdAt, i18n.language)}>
            {formatMessageTimestamp(createdAt, i18n.language, {
              now,
              justNowLabel: t("message.justNow"),
            })}
          </span>
        </div>
        <div
          className={cn(
            "text-sm leading-6 text-ink",
            isUser
              ? "ml-auto w-fit max-w-2xl wrap-break-word rounded-lg bg-surface-subtle px-4 py-3 text-left"
              : "w-full",
            bodyClassName,
          )}
        >
          {children}
        </div>
      </div>
    </article>
  );
}
