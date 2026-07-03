import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { AppSettings } from "../../integrations/storage/appSettings";
import { Boxes, FlaskConical, RotateCcw, Settings2, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Overlay } from "../../components/ui/Overlay";
import { cn } from "../../lib/cn";
import { useBuildInfo } from "../../lib/useBuildInfo";
import { EnvironmentPage } from "./EnvironmentPage";
import { GeneralPage } from "./GeneralPage";
import { ModelsPage } from "./ModelsPage";
import { ProvidersPage } from "./ProvidersPage";
import { ResetPage } from "./ResetPage";

export type SettingsTab = "general" | "providers" | "models" | "environment" | "reset";

// `devOnly` items are only shown on non-release builds — the environment switch
// is a dev affordance; release builds are production-locked.
const NAV_GROUPS = [
  {
    items: [{ icon: Settings2, labelKey: "dialog.tabs.general", value: "general" as const }],
    labelKey: "dialog.nav.desktop",
  },
  {
    items: [
      { icon: Boxes, labelKey: "dialog.tabs.providers", value: "providers" as const },
      { icon: Sparkles, labelKey: "dialog.tabs.models", value: "models" as const },
    ],
    labelKey: "dialog.nav.server",
  },
  {
    items: [
      { icon: FlaskConical, labelKey: "dialog.tabs.environment", value: "environment" as const, devOnly: true },
      { icon: RotateCcw, labelKey: "dialog.tabs.reset", value: "reset" as const },
    ],
    labelKey: "dialog.nav.debug",
  },
];

const TAB_TITLE_KEYS: Record<SettingsTab, string> = {
  general: "dialog.tabs.general",
  models: "dialog.tabs.models",
  providers: "dialog.tabs.providers",
  environment: "dialog.tabs.environment",
  reset: "dialog.tabs.reset",
};

export function SettingsDialog({
  appSettings,
  initialTab = "general",
  modelOptions,
  onChangeSettings,
  onClose,
  open,
}: {
  appSettings: AppSettings;
  /** Tab to show when the dialog opens (e.g. a "Models" quick entry). */
  initialTab?: SettingsTab;
  modelOptions: AgentModelOption[];
  onChangeSettings: (patch: Partial<AppSettings>) => void;
  onClose: () => void;
  open: boolean;
}) {
  const { t } = useTranslation("settings");
  const [tab, setTab] = useState<SettingsTab>(initialTab);
  const build = useBuildInfo();
  // Environment switching is a dev-build affordance; release builds hide it.
  const showEnvironment = Boolean(build.data && !build.data.isRelease);

  // Reset to the requested tab each time the dialog opens.
  useEffect(() => {
    if (open) {
      setTab(initialTab);
    }
  }, [open, initialTab]);

  // Never strand on the (hidden) environment tab on a release build.
  useEffect(() => {
    if (!showEnvironment && tab === "environment") {
      setTab("reset");
    }
  }, [showEnvironment, tab]);

  return (
    <Overlay onClose={onClose} open={open}>
      <section
        aria-label={t("dialog.ariaLabel")}
        aria-modal="true"
        className="relative z-10 flex h-140 w-full max-w-3xl overflow-hidden rounded-xl border border-line-soft bg-surface shadow-dialog"
        role="dialog"
      >
        <nav className="flex w-52 shrink-0 flex-col border-r border-line-soft bg-surface-subtle p-3">
          <div className="flex-1 space-y-4 overflow-y-auto">
            {NAV_GROUPS.map(group => (
              <div key={group.labelKey} className="space-y-1">
                <div className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-ink-muted">{t(group.labelKey)}</div>
                {group.items
                  .filter(item => showEnvironment || !("devOnly" in item && item.devOnly))
                  .map((item) => {
                    const Icon = item.icon;
                    return (
                      <button
                        key={item.value}
                        className={cn(
                          "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink",
                          tab === item.value && "border-line-soft bg-surface text-ink shadow-xs",
                        )}
                        onClick={() => setTab(item.value)}
                        type="button"
                      >
                        <Icon className="size-4 shrink-0" />
                        <span className="truncate">{t(item.labelKey)}</span>
                      </button>
                    );
                  })}
              </div>
            ))}
          </div>
          <div className="px-2 pt-3 text-xs text-ink-muted">
            <div>{t("dialog.appName")}</div>
            {build.data
              ? (
                  <div className="mt-0.5">
                    {t("dialog.version", { version: build.data.version })}
                  </div>
                )
              : null}
            {build.data && !build.data.isRelease
              ? (
                  <div className="mt-0.5 text-warning">{t("dialog.testBuild")}</div>
                )
              : null}
          </div>
        </nav>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center border-b border-line-soft px-6">
            <h2 className="text-base font-semibold text-ink">{t(TAB_TITLE_KEYS[tab])}</h2>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            {tab === "general"
              ? (
                  <GeneralPage
                    autoApprove={appSettings.autoApprove}
                    onToggleAutoApprove={value => onChangeSettings({ autoApprove: value })}
                    showThinking={appSettings.showThinking}
                    onToggleShowThinking={value => onChangeSettings({ showThinking: value })}
                  />
                )
              : null}
            {tab === "providers" ? <ProvidersPage /> : null}
            {tab === "models"
              ? (
                  <ModelsPage
                    hiddenModels={appSettings.hiddenModels}
                    modelOptions={modelOptions}
                    onChangeHidden={hiddenModels => onChangeSettings({ hiddenModels })}
                  />
                )
              : null}
            {tab === "environment" && showEnvironment ? <EnvironmentPage /> : null}
            {tab === "reset" ? <ResetPage /> : null}
          </div>
        </div>
      </section>
    </Overlay>
  );
}
