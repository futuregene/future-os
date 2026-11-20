import type { FutureBalanceStatus, FutureSessionStatus } from "../../components/layout/hooks/useFutureAccount";
import type { UpdateStatus } from "../../components/layout/hooks/useUpdateChecker";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { AppSettings } from "../../integrations/storage/appSettings";
import { Boxes, FlaskConical, HardDrive, Info, Keyboard, MessageSquareText, Palette, Puzzle, RefreshCw, RotateCcw, ShieldCheck, Sparkles, UserRound, UserRoundGroup } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Overlay } from "../../components/ui/Overlay";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { cn } from "../../lib/cn";
import { AboutPage } from "./AboutPage";
import { AccountPage } from "./AccountPage";
import { AppearancePage } from "./AppearancePage";
import { CommunityEditionSection } from "./CommunityEditionSection";
import { EnvironmentPage } from "./EnvironmentPage";
import { FusionSettingsPage } from "./FusionSettingsPage";
import { ModelsPage } from "./ModelsPage";
import { PermissionPage } from "./PermissionPage";
import { ProvidersPage } from "./ProvidersPage";
import { ReplyPage } from "./ReplyPage";
import { ResetPage } from "./ResetPage";
import { ShortcutsPage } from "./ShortcutsPage";
import { SkillsPage } from "./SkillsPage";
import { StoragePage } from "./StoragePage";
import { UpdatePage } from "./UpdatePage";

export type SettingsTab = "appearance" | "shortcuts" | "permission" | "reply" | "storage" | "providers" | "models" | "fusion" | "skills" | "account" | "update" | "about" | "reset" | "environment";

// `devOnly` items are only shown on non-release builds. The group that holds
// them is deliberately its own: a page that vanishes in a release build must not
// sit beside settings that stay, or the group reads as half-missing.
const NAV_GROUPS = [
  {
    items: [
      { icon: Palette, labelKey: "dialog.tabs.appearance", value: "appearance" as const },
      { icon: Keyboard, labelKey: "dialog.tabs.shortcuts", value: "shortcuts" as const },
    ],
    labelKey: "dialog.nav.app",
  },
  {
    items: [
      { icon: ShieldCheck, labelKey: "dialog.tabs.permission", value: "permission" as const },
      { icon: MessageSquareText, labelKey: "dialog.tabs.reply", value: "reply" as const },
      { icon: HardDrive, labelKey: "dialog.tabs.storage", value: "storage" as const },
    ],
    labelKey: "dialog.nav.conversation",
  },
  {
    items: [
      { icon: Boxes, labelKey: "dialog.tabs.providers", value: "providers" as const },
      { icon: Sparkles, labelKey: "dialog.tabs.models", value: "models" as const },
      { icon: Puzzle, labelKey: "dialog.tabs.skills", value: "skills" as const },
    ],
    labelKey: "dialog.nav.models",
  },
  {
    items: [
      { icon: UserRound, labelKey: "dialog.tabs.account", value: "account" as const },
      { icon: RefreshCw, labelKey: "dialog.tabs.update", value: "update" as const },
      { icon: Info, labelKey: "dialog.tabs.about", value: "about" as const },
    ],
    labelKey: "dialog.nav.account",
  },
  {
    items: [
      { icon: RotateCcw, labelKey: "dialog.tabs.reset", value: "reset" as const },
      { icon: UserRoundGroup, labelKey: "dialog.tabs.fusion", value: "fusion" as const, devOnly: true },
      { icon: FlaskConical, labelKey: "dialog.tabs.environment", value: "environment" as const, devOnly: true },
    ],
    labelKey: "dialog.nav.debug",
  },
];

const TAB_TITLE_KEYS: Record<SettingsTab, string> = {
  appearance: "dialog.tabs.appearance",
  shortcuts: "dialog.tabs.shortcuts",
  permission: "dialog.tabs.permission",
  reply: "dialog.tabs.reply",
  storage: "dialog.tabs.storage",
  models: "dialog.tabs.models",
  fusion: "dialog.tabs.fusion",
  providers: "dialog.tabs.providers",
  skills: "dialog.tabs.skills",
  environment: "dialog.tabs.environment",
  reset: "dialog.tabs.reset",
  account: "dialog.tabs.account",
  update: "dialog.tabs.update",
  about: "dialog.tabs.about",
};

export function SettingsDialog({
  appSettings,
  cachedUpdateStatus,
  cachedUpdateCheckedAt,
  futureBalance,
  futureBalanceStatus,
  futureEmail,
  futureSessionStatus,
  onRefreshFutureAuth,
  onRefreshFutureBalance,
  hasUpdate,
  initialTab = "appearance",
  modelOptions,
  onChangeSettings,
  onClose,
  onProvidersChanged,
  onUpdateSeen,
  open,
}: {
  appSettings: AppSettings;
  /** Cached update-check result from the background checker. */
  cachedUpdateStatus?: UpdateStatus | null;
  /** When `cachedUpdateStatus` was produced (epoch ms). */
  cachedUpdateCheckedAt?: number | null;
  /** Shared account state from AppShell; prevents tab-specific profile loads. */
  futureBalance: number | null;
  futureBalanceStatus: FutureBalanceStatus;
  futureEmail: string | null;
  futureSessionStatus: FutureSessionStatus;
  onRefreshFutureAuth: () => void;
  onRefreshFutureBalance: () => void;
  /** Whether a new app version is available (shows a dot on the update nav item). */
  hasUpdate?: boolean;
  /** Tab to show when the dialog opens (e.g. a "Models" quick entry). */
  initialTab?: SettingsTab;
  modelOptions: AgentModelOption[];
  onChangeSettings: (patch: Partial<AppSettings>) => Promise<void>;
  onClose: () => void;
  /** Refresh the available model list after a provider mutation. */
  onProvidersChanged?: () => void;
  /** Called when the user navigates to the "Check for updates" tab. */
  onUpdateSeen?: () => void;
  open: boolean;
}) {
  const { t } = useTranslation("settings");
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const build = useBuildInfo();
  const isTestBuild = build.data?.isRelease === false;

  // Reset to the requested tab each time the dialog opens.
  useEffect(() => {
    if (open) {
      setTab(initialTab);
    }
  }, [open, initialTab]);

  // Never strand on a hidden tab, including when opened through initialTab.
  useEffect(() => {
    if (!isTestBuild && (tab === "environment" || tab === "fusion")) {
      setTab("appearance");
    }
  }, [isTestBuild, tab]);

  return (
    <Overlay onClose={onClose} open={open}>
      <section
        aria-label={t("dialog.ariaLabel")}
        aria-modal="true"
        className="relative z-10 flex h-[min(45rem,calc(100vh-4rem))] w-full max-w-3xl overflow-hidden rounded-xl border border-line-soft bg-surface shadow-dialog"
        role="dialog"
      >
        <nav className="flex w-52 shrink-0 flex-col border-r border-line-soft bg-surface-subtle p-3">
          <div className="flex-1 space-y-3 overflow-y-auto">
            {NAV_GROUPS.map(group => (
              <div key={group.labelKey} className="space-y-1">
                <div className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-ink-muted">{t(group.labelKey)}</div>
                {group.items
                  .filter(item => isTestBuild || !("devOnly" in item && item.devOnly))
                  .map((item) => {
                    const Icon = item.icon;
                    const showDot = item.value === "update" && hasUpdate;
                    // The selected item paints the rail's own `surface`; the
                    // hover tint must stay a *different* fill (`line-soft`) —
                    // an item that hovers to the same block as the selected one
                    // reads as a second selected tab while the pane shows
                    // another page. `aria-current` names the one that counts,
                    // for screen readers and for anything reading the DOM.
                    return (
                      <button
                        aria-current={tab === item.value ? "page" : undefined}
                        key={item.value}
                        className={cn(
                          "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-line-soft hover:text-ink",
                          tab === item.value && "border-line-soft bg-surface text-ink shadow-xs",
                        )}
                        onClick={() => {
                          setTab(item.value);
                          if (item.value === "update")
                            onUpdateSeen?.();
                        }}
                        type="button"
                      >
                        <span className="relative inline-flex shrink-0">
                          <Icon className="size-4" />
                          {showDot ? <span className="absolute -right-1 -top-1 size-2 rounded-full bg-accent" /> : null}
                        </span>
                        <span className="truncate">{t(item.labelKey)}</span>
                      </button>
                    );
                  })}
              </div>
            ))}
          </div>
          {/* The app name + version now live on the About page; only the
              test-build hint stays here (release builds show nothing). The hint
              carries its own soft fill: the nav behind it is `surface-subtle`,
              where the warning ink lands at 4.49:1 in one palette, just under
              AA — the palette contract covers `bg-warning-soft text-warning`.
              Sized to its text (`w-fit`): one word in a block element painted
              the whole 184px nav column and read as a band, not a label — the
              same warning chip the About page shows as a `Badge`. */}
          {build.data && !build.data.isRelease
            ? (
                <div className="mt-3 w-fit max-w-full rounded bg-warning-soft px-2 py-1 text-xs text-warning">{t("dialog.testBuild")}</div>
              )
            : null}
        </nav>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center border-b border-line-soft px-6">
            <h2 className="text-base font-semibold text-ink">{t(TAB_TITLE_KEYS[tab])}</h2>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            {tab === "appearance"
              ? (
                  <AppearancePage
                    bellOnComplete={appSettings.bellOnComplete}
                    onToggleBellOnComplete={value => onChangeSettings({ bellOnComplete: value })}
                  />
                )
              : null}
            {tab === "permission"
              ? (
                  <PermissionPage
                    approvalTier={appSettings.approvalTier}
                    futureSessionStatus={futureSessionStatus}
                    onChangeApprovalTier={value => onChangeSettings({ approvalTier: value })}
                  />
                )
              : null}
            {tab === "reply"
              ? (
                  <ReplyPage
                    choiceCards={appSettings.choiceCards}
                    onToggleChoiceCards={value => onChangeSettings({ choiceCards: value })}
                    autoTitleFirstTurn={appSettings.autoTitleFirstTurn}
                    onToggleAutoTitleFirstTurn={value => onChangeSettings({ autoTitleFirstTurn: value })}
                  />
                )
              : null}
            {tab === "storage"
              ? (
                  <StoragePage
                    archiveRunLogCleanup={appSettings.archiveRunLogCleanup}
                    onToggleArchiveRunLogCleanup={value => onChangeSettings({ archiveRunLogCleanup: value })}
                    autoArchiveEnabled={appSettings.autoArchiveEnabled}
                    onToggleAutoArchive={value => onChangeSettings({ autoArchiveEnabled: value })}
                    autoArchiveIdleDays={appSettings.autoArchiveIdleDays}
                    onChangeAutoArchiveIdleDays={value => onChangeSettings({ autoArchiveIdleDays: value })}
                  />
                )
              : null}
            {tab === "skills"
              ? (
                  <SkillsPage
                    autoUpgradeSkills={appSettings.autoUpgradeSkills}
                    onToggleAutoUpgradeSkills={value => onChangeSettings({ autoUpgradeSkills: value })}
                    skillRecommend={appSettings.skillRecommend}
                    onToggleSkillRecommend={value => onChangeSettings({ skillRecommend: value })}
                  />
                )
              : null}
            {tab === "fusion" && isTestBuild ? <FusionSettingsPage /> : null}
            {tab === "shortcuts" ? <ShortcutsPage /> : null}
            {tab === "account"
              ? (
                  <AccountPage
                    balance={futureBalance}
                    balanceStatus={futureBalanceStatus}
                    communityEdition={appSettings.communityEdition}
                    email={futureEmail}
                    sessionStatus={futureSessionStatus}
                    onRefreshAuth={onRefreshFutureAuth}
                    onRefreshBalance={onRefreshFutureBalance}
                  />
                )
              : null}
            {tab === "update" ? <UpdatePage cachedStatus={cachedUpdateStatus} cachedStatusCheckedAt={cachedUpdateCheckedAt} /> : null}
            {tab === "about" ? <AboutPage communityEdition={appSettings.communityEdition} /> : null}
            {tab === "providers"
              ? (
                  <ProvidersPage
                    communityEdition={appSettings.communityEdition}
                    futureSessionStatus={futureSessionStatus}
                    onOpenAccount={() => setTab("account")}
                    onProvidersChanged={onProvidersChanged}
                    onRefreshFutureAuth={onRefreshFutureAuth}
                  />
                )
              : null}
            {tab === "models"
              ? (
                  <ModelsPage
                    hiddenModels={appSettings.hiddenModels}
                    modelOptions={modelOptions}
                    onChangeHidden={hiddenModels => onChangeSettings({ hiddenModels })}
                  />
                )
              : null}
            {tab === "environment" && isTestBuild
              ? (
                  <div className="space-y-6">
                    <EnvironmentPage />
                    <CommunityEditionSection
                      communityEdition={appSettings.communityEdition}
                      onChangeCommunityEdition={value => onChangeSettings({ communityEdition: value })}
                    />
                  </div>
                )
              : null}
            {tab === "reset" ? <ResetPage /> : null}
          </div>
        </div>
      </section>
    </Overlay>
  );
}
