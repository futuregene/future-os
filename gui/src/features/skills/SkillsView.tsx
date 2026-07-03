import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import { Blocks, Download, RotateCcw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import {
  installSkill,
  listAvailableSkills,
  listInstalledSkills,
  uninstallSkill,
} from "../../integrations/skills/skillsClient";
import { cn } from "../../lib/cn";

type SkillsTab = "installed" | "all";

export function SkillsView() {
  const { t } = useTranslation("skills");
  const [tab, setTab] = useState<SkillsTab>("installed");
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [available, setAvailable] = useState<AvailableSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [availableError, setAvailableError] = useState<string | null>(null);
  // Skill ids with an install/uninstall in flight (disables their buttons).
  const [busy, setBusy] = useState<Record<string, boolean>>({});

  const installedIds = useMemo(
    () => new Set(installed.map(skill => skill.id)),
    [installed],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    // Installed comes from the agent; the catalogue needs the platform reachable
    // and may fail independently — keep the installed tab usable either way.
    const [installedResult, availableResult] = await Promise.allSettled([
      listInstalledSkills(),
      listAvailableSkills(),
    ]);
    if (installedResult.status === "fulfilled")
      setInstalled(installedResult.value);
    if (availableResult.status === "fulfilled") {
      setAvailable(availableResult.value);
      setAvailableError(null);
    }
    else {
      setAvailable([]);
      setAvailableError(
        availableResult.reason instanceof Error
          ? availableResult.reason.message
          : String(availableResult.reason),
      );
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const runAction = useCallback(async (id: string, action: () => Promise<unknown>) => {
    setBusy(current => ({ ...current, [id]: true }));
    try {
      await action();
      await refresh();
    }
    finally {
      setBusy(current => ({ ...current, [id]: false }));
    }
  }, [refresh]);

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header className="border-b border-line-soft px-8 pb-3 pt-6">
        <h1 className="text-base font-semibold text-ink">{t("title")}</h1>
        <p className="mt-1 text-sm text-ink-muted">{t("subtitle")}</p>
        <div className="mt-4 grid w-64 grid-cols-2 gap-1 rounded-md bg-surface-subtle p-1">
          <TabButton active={tab === "installed"} label={t("tab.installed")} onClick={() => setTab("installed")} />
          <TabButton active={tab === "all"} label={t("tab.all")} onClick={() => setTab("all")} />
        </div>
      </header>

      <div className="floating-scrollbar min-h-0 flex-1 overflow-auto px-8 py-5">
        <div className="mx-auto w-full max-w-3xl space-y-3">
          {tab === "installed"
            ? (
                <InstalledTab
                  loading={loading}
                  skills={installed}
                  busy={busy}
                  onUninstall={id => void runAction(id, () => uninstallSkill(id))}
                />
              )
            : (
                <AllTab
                  loading={loading}
                  skills={available}
                  installedIds={installedIds}
                  error={availableError}
                  busy={busy}
                  onInstall={(id, version) => void runAction(id, () => installSkill(id, version))}
                  onUninstall={id => void runAction(id, () => uninstallSkill(id))}
                  onRetry={() => void refresh()}
                />
              )}
        </div>
      </div>
    </section>
  );
}

function TabButton({ active, label, onClick }: { active: boolean; label: string; onClick: () => void }) {
  return (
    <button
      className={cn(
        "h-8 rounded text-sm font-medium transition-colors",
        active ? "bg-surface text-ink shadow-xs" : "text-ink-muted hover:text-ink",
      )}
      onClick={onClick}
      type="button"
    >
      {label}
    </button>
  );
}

function InstalledTab({
  busy,
  loading,
  onUninstall,
  skills,
}: {
  busy: Record<string, boolean>;
  loading: boolean;
  onUninstall: (id: string) => void;
  skills: InstalledSkill[];
}) {
  const { t } = useTranslation("skills");
  if (loading && skills.length === 0)
    return <LoadingRow />;
  if (skills.length === 0)
    return <EmptyState title={t("installed.emptyTitle")} detail={t("installed.emptyDetail")} />;

  return (
    <>
      {skills.map(skill => (
        <SkillRow
          key={skill.id}
          name={skill.name}
          description={skill.description}
          version={skill.version}
          action={(
            <UninstallButton busy={busy[skill.id]} onClick={() => onUninstall(skill.id)} />
          )}
        />
      ))}
    </>
  );
}

function AllTab({
  busy,
  error,
  installedIds,
  loading,
  onInstall,
  onRetry,
  onUninstall,
  skills,
}: {
  busy: Record<string, boolean>;
  error: string | null;
  installedIds: Set<string>;
  loading: boolean;
  onInstall: (id: string, version: string) => void;
  onRetry: () => void;
  onUninstall: (id: string) => void;
  skills: AvailableSkill[];
}) {
  const { t } = useTranslation("skills");
  if (loading && skills.length === 0)
    return <LoadingRow />;
  if (error) {
    return (
      <div className="space-y-3">
        <div className="rounded-md border border-danger-line bg-danger-soft p-3 text-sm text-danger">
          {t("all.loadError")}
          {error}
        </div>
        <Button leftIcon={<RotateCcw className="size-3.5" />} onClick={onRetry} size="sm" variant="secondary">
          {t("all.retry")}
        </Button>
      </div>
    );
  }
  if (skills.length === 0)
    return <EmptyState title={t("all.emptyTitle")} detail={t("all.emptyDetail")} />;

  return (
    <>
      {skills.map((skill) => {
        const isInstalled = installedIds.has(skill.id);
        const canInstall = Boolean(skill.latestVersion);
        return (
          <SkillRow
            key={skill.id}
            name={skill.name || skill.id}
            description={skill.description}
            version={skill.latestVersion}
            meta={skill.category || skill.price || skill.formats || undefined}
            action={
              isInstalled
                ? <UninstallButton busy={busy[skill.id]} onClick={() => onUninstall(skill.id)} />
                : (
                    <Button
                      disabled={busy[skill.id] || !canInstall}
                      leftIcon={<Download className="size-3.5" />}
                      onClick={() => skill.latestVersion && onInstall(skill.id, skill.latestVersion)}
                      size="sm"
                      variant="primary"
                    >
                      {busy[skill.id] ? t("install.installing") : canInstall ? t("install.install") : t("install.noVersion")}
                    </Button>
                  )
            }
          />
        );
      })}
    </>
  );
}

function SkillRow({
  action,
  description,
  meta,
  name,
  version,
}: {
  action: React.ReactNode;
  description: string;
  meta?: string;
  name: string;
  version: string | null;
}) {
  return (
    <div className="flex items-start gap-3 rounded-md border border-line-soft bg-surface p-3">
      <Blocks className="mt-0.5 size-5 shrink-0 text-ink-soft" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium text-ink">{name}</span>
          {version ? <Badge tone="neutral">{`v${version}`}</Badge> : null}
        </div>
        {description
          ? <p className="mt-1 line-clamp-2 text-xs leading-5 text-ink-muted">{description}</p>
          : null}
        {meta ? <p className="mt-1 truncate text-xs text-ink-muted">{meta}</p> : null}
      </div>
      <div className="shrink-0">{action}</div>
    </div>
  );
}

function UninstallButton({ busy, onClick }: { busy?: boolean; onClick: () => void }) {
  const { t } = useTranslation("skills");
  const [confirming, setConfirming] = useState(false);
  if (!confirming) {
    return (
      <Button
        disabled={busy}
        leftIcon={<Trash2 className="size-3.5" />}
        onClick={() => setConfirming(true)}
        size="sm"
        variant="danger-soft"
      >
        {busy ? t("uninstall.uninstalling") : t("uninstall.uninstall")}
      </Button>
    );
  }
  return (
    <div className="flex items-center gap-2">
      <Button disabled={busy} onClick={() => setConfirming(false)} size="sm" variant="ghost">
        {t("uninstall.cancel")}
      </Button>
      <Button disabled={busy} onClick={onClick} size="sm" variant="danger">
        {busy ? t("uninstall.uninstalling") : t("uninstall.confirm")}
      </Button>
    </div>
  );
}

function LoadingRow() {
  const { t } = useTranslation("skills");
  return <div className="rounded-md border border-line-soft bg-surface p-3 text-sm text-ink-muted">{t("loading")}</div>;
}
