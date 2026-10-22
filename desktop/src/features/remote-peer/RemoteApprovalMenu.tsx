import type { RemotePeer } from "./remotePeerClient";
import { ChevronDown, ShieldCheck, ShieldKeyhole, ShieldOff, ShieldQuestion } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../components/ui/SelectMenu";
import { getRemoteApprovalSettings, setRemoteApprovalTier } from "./remotePeerClient";

/**
 * The host's approval mode, in the composer's control row.
 *
 * This is the mode the *host* will run this conversation under, so it is offered
 * here — beside the box whose messages it governs — and not in the settings
 * page, where it would read as a property of the machine rather than of the next
 * thing you send. This app's own composer places its approval control in exactly
 * this slot.
 *
 * Each tier is offered or refused by what the host reports about itself: a host
 * whose sandbox is missing cannot be asked for `sandbox`, and a host that
 * answered `manual` when asked for `sandbox` is shown as `manual`, because that
 * is the mode it is actually in.
 */
const TIERS = ["manual", "sandbox", "auto", "off"] as const;

function tierIcon(tier: string, className: string) {
  if (tier === "auto")
    return <ShieldCheck className={className} />;
  if (tier === "sandbox")
    return <ShieldKeyhole className={className} />;
  if (tier === "off")
    return <ShieldOff className={className} />;
  return <ShieldQuestion className={className} />;
}

export function RemoteApprovalMenu({
  desktopId,
  onChanged,
  peer,
}: {
  desktopId: string;
  /** The host settled on a new mode; re-read whatever depends on it. */
  onChanged?: () => void;
  peer: RemotePeer | undefined;
}) {
  const { t } = useTranslation("remotePeer");
  const [tier, setTier] = useState<string | null>(null);
  const [sandboxAvailable, setSandboxAvailable] = useState(false);
  const [sandboxCode, setSandboxCode] = useState<string>();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  // A capability the host declares, not something to assume: an older build that
  // cannot review automatically must not be offered the mode.
  const supportsAuto = peer?.features.includes("auto_approval_v1") === true;

  useEffect(() => {
    let cancelled = false;
    void getRemoteApprovalSettings(desktopId).then((settings) => {
      if (cancelled)
        return;
      setTier(settings.approvalTier);
      setSandboxAvailable(settings.sandboxAvailable);
      setSandboxCode(settings.sandboxCode);
    }).catch(() => {
      // A host too old to report its settings has no control to offer.
      if (!cancelled)
        setTier(null);
    });
    return () => {
      cancelled = true;
    };
  }, [desktopId]);

  // Nothing to show until the host has said what it is in: a control that
  // guessed would report a mode the host is not running.
  if (tier === null)
    return null;

  async function choose(next: string) {
    setOpen(false);
    setBusy(true);
    try {
      const settled = await setRemoteApprovalTier(desktopId, next);
      setTier(settled);
      onChanged?.();
    }
    catch {
      // The host refused; its own mode is unchanged, and the control keeps
      // showing that.
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <SelectMenu
      align="left"
      open={open}
      onDismiss={() => setOpen(false)}
      panelClassName="w-64 overflow-hidden"
      trigger={(
        <button
          className="inline-flex h-7 max-w-40 min-w-0 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:opacity-40"
          disabled={busy}
          onClick={() => setOpen(value => !value)}
          title={t("approvalTier")}
          type="button"
        >
          {tierIcon(tier, "size-3 shrink-0")}
          <span className="truncate">{t(`approvalTierLabel.${tier}`, { defaultValue: tier })}</span>
          <ChevronDown className="size-3 shrink-0" />
        </button>
      )}
    >
      {TIERS.map(item => (
        <SelectMenuItem
          className="py-1.5"
          disabled={((item === "sandbox" || item === "auto") && !sandboxAvailable) || (item === "auto" && !supportsAuto)}
          key={item}
          onSelect={() => void choose(item)}
          selected={tier === item}
        >
          {tierIcon(item, "size-4 shrink-0 text-ink-soft")}
          <span className="min-w-0 flex-1 space-y-0.5">
            <span className="block truncate font-medium leading-tight text-ink">
              {/* A mode this host cannot run says so in its own label, the words
                  the settings page uses for the same state. */}
              {item === "sandbox" && !sandboxAvailable
                ? t("approvalTierLabel.sandboxUnavailable")
                : item === "auto" && !supportsAuto
                  ? t("approvalTierLabel.autoUnsupported")
                  : t(`approvalTierLabel.${item}`, { defaultValue: item })}
            </span>
            <span className="block text-xs leading-tight text-ink-muted">
              {sandboxAvailable && sandboxCode === "available_write_only" && (item === "sandbox" || item === "auto")
                ? t("approvalTierUpgrade")
                : t(`approvalTierDescription.${item}`, { defaultValue: t("approvalTierDescription.default") })}
            </span>
          </span>
        </SelectMenuItem>
      ))}
    </SelectMenu>
  );
}
