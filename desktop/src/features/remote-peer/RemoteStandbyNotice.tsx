import { useState } from "react";
import { useTranslation } from "react-i18next";

/**
 * One line where the writing happens: another device drives this machine, so
 * nothing sent from here would arrive.
 *
 * The machine's own row already says it is taken over, and the device picker
 * offers the way back — but neither is where a reader is while they are reading
 * or typing a conversation. The phone put the same sentence beside its composer
 * for exactly that reason (`StandbyNotice`), and the way out belongs with it:
 * "you cannot send" without "here is what to do" is the same dead end in a
 * different place.
 *
 * What it states is the *write* fact, which is `control`, not liveness: the host
 * refuses this client's business commands with `superseded` while another paired
 * device holds it, whether or not the link happens to be up this second. So the
 * caller decides from `control === "standby"` alone rather than from
 * `peerHeld`, which also asks whether the link is up.
 *
 * The frame is this app's own conversation notice (`AgentThread`'s
 * `ConnectionNotice`): same width, same `warning` pair, same action button at the
 * right — the two banners answer the same question ("why can I not use this
 * conversation?") about two different machines.
 */
export function RemoteStandbyNotice({
  holderName,
  onTakeControl,
}: {
  /** Who holds the machine, when it said who it is. */
  holderName: string | null;
  /** Ask the host to let this computer drive it. */
  onTakeControl: () => Promise<unknown> | void;
}) {
  const { t } = useTranslation("remotePeer");
  const [taking, setTaking] = useState(false);

  /**
   * One takeover at a time.
   *
   * The control is disabled while it is in flight, so `taking` is the only guard
   * a second press would need — and a refusal is *not* reported here: the shell
   * re-reads the machine afterwards, which is what makes the banner (and the
   * machine's own row) state the truth. A sentence about a failed takeover on a
   * banner that is already explaining the machine would be a second opinion
   * about the same fact.
   */
  async function take() {
    if (taking)
      return;
    setTaking(true);
    try {
      await onTakeControl();
    }
    finally {
      setTaking(false);
    }
  }

  return (
    <div
      className="pointer-events-auto mx-auto w-full max-w-3xl rounded-md border border-warning-line bg-warning-soft px-3 py-2 text-xs leading-5 text-warning shadow-xs"
      // The row is announced when it appears, which is the moment control moves
      // under the reader: the send button closing without a word would read as a
      // broken composer.
      role="status"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">
          {holderName ? t("heldTitle", { name: holderName }) : t("heldTitleUnknown")}
        </span>
        <button
          aria-label={t("takeOver")}
          className="h-7 rounded-md bg-surface px-2 text-xs font-medium text-warning ring-1 ring-warning-line transition-colors hover:bg-warning-line disabled:opacity-60"
          disabled={taking}
          onClick={() => void take()}
          type="button"
        >
          {taking ? t("takingOver") : t("takeOver")}
        </button>
      </div>
      <div className="mt-1 text-warning">{t("heldCannotSend")}</div>
    </div>
  );
}
