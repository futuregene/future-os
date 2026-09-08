import type { KeyboardEvent, ReactNode } from "react";
import type { ActivitySection } from "./ActivityRail";
import { Download, Settings, Wallet } from "lucide-react";
import { useCallback, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { IconButton } from "../ui/IconButton";
import { MenuPanel } from "../ui/MenuPanel";

export function ActivityRailAccountFooter({
  active,
  balance,
  communityEdition,
  expanded,
  hasUpdate,
  onChange,
  onOpenUpdate,
  onRecharge,
  userEmail,
}: {
  active: ActivitySection;
  balance: number | null;
  communityEdition?: boolean;
  expanded: boolean;
  hasUpdate?: boolean;
  onChange: (section: ActivitySection) => void;
  onOpenUpdate?: () => void;
  onRecharge?: () => void;
  userEmail?: string | null;
}) {
  const { t } = useTranslation("layout");
  return (
    <div className="shrink-0 border-t border-line-soft/40 p-2">
      {expanded
        ? (
            userEmail && !communityEdition
              ? (
                  <AccountMenuButton
                    balance={balance}
                    email={userEmail}
                    hasUpdate={hasUpdate}
                    onOpenSettings={() => onChange("settings")}
                    onOpenUpdate={onOpenUpdate}
                    onRecharge={onRecharge}
                  />
                )
              : (
                  <button
                    className={cn(
                      "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
                      active === "settings" && "border-accent bg-accent-soft text-accent",
                    )}
                    onClick={() => onChange("settings")}
                    type="button"
                  >
                    <span className="relative inline-flex shrink-0">
                      <Settings className="size-4" />
                      {hasUpdate ? <span className="absolute -right-1 -top-1 size-2 rounded-full bg-accent" /> : null}
                    </span>
                    <span className="truncate">{t("activityRail.settings")}</span>
                  </button>
                )
          )
        : (
            <IconButton
              icon={(
                <span className="relative inline-flex">
                  <Settings className="size-4" />
                  {hasUpdate ? <span className="absolute -right-1 -top-1 size-2 rounded-full bg-accent" /> : null}
                </span>
              )}
              label={t("activityRail.settings")}
              active={active === "settings"}
              onClick={() => onChange("settings")}
            />
          )}
    </div>
  );
}

function AccountMenuButton({
  balance,
  email,
  hasUpdate,
  onOpenSettings,
  onOpenUpdate,
  onRecharge,
}: {
  balance: number | null;
  email: string;
  hasUpdate?: boolean;
  onOpenSettings: () => void;
  onOpenUpdate?: () => void;
  onRecharge?: () => void;
}) {
  const { t } = useTranslation("layout");
  const [open, setOpen] = useState(false);
  const menuId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => setOpen(false), []);
  const layerRef = useDismissableLayer<HTMLDivElement>({
    enabled: open,
    onDismiss: close,
    onEscapeDismiss: () => {
      close();
      triggerRef.current?.focus();
    },
  });
  const prefix = email.split("@")[0] || email;
  const initial = (prefix[0] ?? "?").toUpperCase();

  function handleMenuKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const delta = event.key === "ArrowDown" ? 1 : -1;
      items[(index + delta + items.length) % items.length]?.focus();
    }
    else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      items[event.key === "Home" ? 0 : items.length - 1]?.focus();
    }
  }

  function openMenu() {
    setOpen(true);
    window.requestAnimationFrame(() => layerRef.current?.querySelector<HTMLButtonElement>("[role=menuitem]")?.focus());
  }

  return (
    <div className="relative" ref={layerRef}>
      <div className="flex w-full items-center rounded-md border border-transparent transition-colors hover:bg-surface-subtle">
        <button
          ref={triggerRef}
          aria-controls={menuId}
          aria-expanded={open}
          aria-haspopup="menu"
          className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onClick={() => open ? close() : openMenu()}
          type="button"
        >
          <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-accent-soft text-sm font-semibold uppercase leading-none text-accent">
            {initial}
          </span>
          <span className="min-w-0 flex-1 truncate text-sm font-medium text-ink">{prefix}</span>
        </button>
        {hasUpdate
          ? (
              <button
                aria-label={t("userMenu.upgrade")}
                className="mr-1 inline-flex size-7 shrink-0 items-center justify-center rounded text-accent transition-colors hover:bg-accent-soft focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
                onClick={() => {
                  onOpenUpdate?.();
                  close();
                }}
                title={t("userMenu.upgrade")}
                type="button"
              >
                <Download className="size-4" />
              </button>
            )
          : null}
      </div>
      {open
        ? (
            <MenuPanel
              id={menuId}
              className="absolute bottom-full left-0 right-0 z-40 mb-2 overflow-hidden p-0"
              onKeyDown={handleMenuKeyDown}
              role="menu"
            >
              <MenuRow
                icon={<Settings className="size-4 shrink-0 text-ink-soft" />}
                label={t("activityRail.settings")}
                onClick={() => {
                  onOpenSettings();
                  close();
                }}
              />
              <MenuRow
                action={<ActionBadge>{t("userMenu.recharge")}</ActionBadge>}
                icon={<Wallet className="size-4 shrink-0 text-ink-soft" />}
                label={t("userMenu.balance", { credits: balance != null ? Math.trunc(balance) : "—" })}
                onClick={() => {
                  onRecharge?.();
                  close();
                }}
              />
            </MenuPanel>
          )
        : null}
    </div>
  );
}

function MenuRow({ action, icon, label, onClick }: { action?: ReactNode; icon: ReactNode; label: string; onClick: () => void }) {
  return (
    <button
      className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-sm text-ink transition-colors hover:bg-surface-subtle focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-focus"
      onClick={onClick}
      role="menuitem"
      type="button"
    >
      {icon}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {action ?? null}
    </button>
  );
}

function ActionBadge({ children }: { children: ReactNode }) {
  return (
    <span className="shrink-0 rounded bg-accent px-1.5 py-0.5 text-[11px] font-medium leading-none text-white">
      {children}
    </span>
  );
}
