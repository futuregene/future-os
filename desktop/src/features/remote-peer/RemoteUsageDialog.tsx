import { useTranslation } from "react-i18next";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
import { SessionUsageDialog } from "../agent/SessionUsageDialog";
import { useRemoteUsage } from "./useRemoteUsage";

/**
 * A remote conversation's token usage and amount.
 *
 * The same dialog this app shows for its own conversations, quoting the *host's*
 * figures: the tokens were spent by that machine, so its accounting is the only
 * truthful one, and a dialog that recomputed a cost here would report a number
 * the host never charged.
 *
 * Read when it opens rather than carried with the transcript: nobody looks at
 * this except deliberately, and a state read per streaming push would be a
 * request per delta.
 */
export function RemoteUsageDialog({
  desktopId,
  onClose,
  sessionId,
  title,
}: {
  desktopId: string;
  onClose: () => void;
  sessionId: string;
  title: string;
}) {
  const { t } = useTranslation("remotePeer");
  const { usage, loading, error } = useRemoteUsage(desktopId, sessionId, true);
  // A dialog holds the keyboard while it is up — including this file's own
  // hand-rolled error state below, which is not an `Overlay`.
  useKeyboardOwner(true);

  if (error) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/20" role="dialog">
        <div className="w-full max-w-md rounded-lg border border-line bg-surface p-4 shadow-panel">
          <p className="text-sm text-danger">{t("usageUnavailable", { message: error })}</p>
          <button
            className="mt-3 rounded-md px-2 py-1 text-sm text-ink-soft hover:bg-surface-subtle"
            onClick={onClose}
            type="button"
          >
            {t("filesClose")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <SessionUsageDialog
      // A host that has not reported usage yet shows the dialog's own "not
      // reported" state, not a fabricated ¥0: `undefined` is what says so.
      onClose={onClose}
      open
      title={loading ? `${title}…` : title}
      usage={usage}
    />
  );
}
