import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { AppSettings } from "../../integrations/storage/appSettings";
import { Boxes, Settings2, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { Overlay } from "../../components/ui/Overlay";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { cn } from "../../lib/cn";
import { GeneralPage } from "./GeneralPage";
import { ModelsPage } from "./ModelsPage";
import { ProvidersPage } from "./ProvidersPage";

type SettingsTab = "general" | "providers" | "models";

const NAV_GROUPS = [
  {
    items: [{ icon: Settings2, label: "通用", value: "general" as const }],
    label: "桌面",
  },
  {
    items: [
      { icon: Boxes, label: "提供商", value: "providers" as const },
      { icon: Sparkles, label: "模型", value: "models" as const },
    ],
    label: "服务器",
  },
];

const TAB_TITLES: Record<SettingsTab, string> = {
  general: "通用",
  models: "模型",
  providers: "提供商",
};

export function SettingsDialog({
  appSettings,
  modelOptions,
  onChangeSettings,
  onClose,
  open,
}: {
  appSettings: AppSettings;
  modelOptions: AgentModelOption[];
  onChangeSettings: (patch: Partial<AppSettings>) => void;
  onClose: () => void;
  open: boolean;
}) {
  const [tab, setTab] = useState<SettingsTab>("general");
  const [version, setVersion] = useState("");

  useEffect(() => {
    if (!open || version) {
      return;
    }
    void invokeCommand<string>("app_version").then(setVersion).catch(() => undefined);
  }, [open, version]);

  return (
    <Overlay onClose={onClose} open={open}>
      <section
        aria-label="设置"
        aria-modal="true"
        className="relative z-10 flex h-[560px] w-full max-w-3xl overflow-hidden rounded-xl border border-line-soft bg-surface shadow-dialog"
        role="dialog"
      >
        <nav className="flex w-52 shrink-0 flex-col border-r border-line-soft bg-surface-subtle p-3">
          <div className="flex-1 space-y-4 overflow-y-auto">
            {NAV_GROUPS.map(group => (
              <div key={group.label} className="space-y-1">
                <div className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-ink-muted">{group.label}</div>
                {group.items.map((item) => {
                  const Icon = item.icon;
                  return (
                    <button
                      key={item.value}
                      className={cn(
                        "flex h-8 w-full items-center gap-2 rounded-md border border-transparent px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink",
                        tab === item.value && "border-line-soft bg-surface text-ink shadow-sm",
                      )}
                      onClick={() => setTab(item.value)}
                      type="button"
                    >
                      <Icon className="size-4 shrink-0" />
                      <span className="truncate">{item.label}</span>
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
          <div className="px-2 pt-3 text-xs text-ink-muted">
            <div>FutureOS Desktop</div>
            {version
              ? (
                  <div className="mt-0.5">
                    v
                    {version}
                  </div>
                )
              : null}
          </div>
        </nav>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center border-b border-line-soft px-6">
            <h2 className="text-base font-semibold text-ink">{TAB_TITLES[tab]}</h2>
          </header>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            {tab === "general"
              ? (
                  <GeneralPage
                    autoApprove={appSettings.autoApprove}
                    onToggleAutoApprove={value => onChangeSettings({ autoApprove: value })}
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
          </div>
        </div>
      </section>
    </Overlay>
  );
}
