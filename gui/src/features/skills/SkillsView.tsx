import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import type { SkillFilters } from "./skillsFilter";
import { Blocks, Download, RotateCcw, Search, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import {
  installSkill,
  listAvailableSkills,
  listInstalledSkills,
  uninstallSkill,
} from "../../integrations/skills/skillsClient";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import {
  allCategoriesValue,
  matchesAvailableSkill,
  matchesInstalledSkill,
  uniqueSorted,
} from "./skillsFilter";

type SkillsTab = "installed" | "all";

const emptyFilters: SkillFilters = { category: allCategoriesValue, query: "" };

export function SkillsView() {
  const { t } = useTranslation("skills");
  const [tab, setTab] = useState<SkillsTab>("installed");
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [available, setAvailable] = useState<AvailableSkill[]>([]);
  const [installedFilters, setInstalledFilters] = useState<SkillFilters>(emptyFilters);
  const [allFilters, setAllFilters] = useState<SkillFilters>(emptyFilters);
  const [loading, setLoading] = useState(true);
  const [installedError, setInstalledError] = useState<string | null>(null);
  const [availableError, setAvailableError] = useState<string | null>(null);
  // Skill ids with an install/uninstall in flight (disables their buttons).
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const hasResolvedInitialTabRef = useRef(false);

  const installedIds = useMemo(
    () => new Set(installed.map(skill => skill.id)),
    [installed],
  );

  const allCategories = useMemo(
    () => uniqueSorted(available.map(skill => skill.category)),
    [available],
  );

  // Category lookup for installed skills (from catalogue). Used for filtering
  // and the category dropdown — uncategorized skills are excluded when a
  // specific category is selected.
  const installedCategoryMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const a of available) {
      if (a.category) map.set(a.id, a.category);
    }
    return map;
  }, [available]);

  // Categories that have at least one installed skill (matched via catalogue).
  const installedCategories = useMemo(() => {
    const catSet = new Set<string>();
    for (const s of installed) {
      const cat = installedCategoryMap.get(s.id);
      if (cat) catSet.add(cat);
    }
    return [...catSet].sort();
  }, [installed, installedCategoryMap]);

  const filteredInstalled = useMemo(
    () => installed.filter(skill => matchesInstalledSkill(skill, installedFilters, installedCategoryMap.get(skill.id))),
    [installed, installedFilters, installedCategoryMap],
  );

  const filteredAvailable = useMemo(
    () => available.filter(skill => matchesAvailableSkill(skill, allFilters)),
    [allFilters, available],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    // Installed comes from the agent; the catalogue needs the platform reachable
    // and may fail independently — keep the installed tab usable either way.
    const [installedResult, availableResult] = await Promise.allSettled([
      listInstalledSkills(),
      listAvailableSkills(),
    ]);
    if (installedResult.status === "fulfilled") {
      setInstalled(installedResult.value);
      setInstalledError(null);
      if (!hasResolvedInitialTabRef.current) {
        hasResolvedInitialTabRef.current = true;
        if (installedResult.value.length === 0)
          setTab("all");
      }
    }
    else {
      // Don't let a failed load masquerade as an empty Installed tab.
      setInstalled([]);
      setInstalledError(errorMessage(installedResult.reason));
    }
    if (availableResult.status === "fulfilled") {
      setAvailable(availableResult.value);
      setAvailableError(null);
    }
    else {
      setAvailable([]);
      setAvailableError(errorMessage(availableResult.reason));
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
    catch (error) {
      // Every caller is `void runAction(...)`, so a rejected install/uninstall
      // would otherwise vanish — surface it as a toast.
      emitFutureEvent("toast", { message: t("actionFailed", { message: errorMessage(error) }), tone: "error" });
    }
    finally {
      setBusy(current => ({ ...current, [id]: false }));
    }
  }, [refresh, t]);

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
                  categories={installedCategories}
                  filters={installedFilters}
                  onFiltersChange={setInstalledFilters}
                  resultCount={filteredInstalled.length}
                  skills={filteredInstalled}
                  totalCount={installed.length}
                  error={installedError}
                  busy={busy}
                  catalogue={available}
                  onUninstall={id => void runAction(id, () => uninstallSkill(id))}
                  onRetry={() => void refresh()}
                />
              )
            : (
                <AllTab
                  loading={loading}
                  categories={allCategories}
                  filters={allFilters}
                  onFiltersChange={setAllFilters}
                  resultCount={filteredAvailable.length}
                  skills={filteredAvailable}
                  totalCount={available.length}
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
  catalogue,
  categories,
  error,
  filters,
  loading,
  onFiltersChange,
  onRetry,
  onUninstall,
  resultCount,
  skills,
  totalCount,
}: {
  busy: Record<string, boolean>;
  catalogue: AvailableSkill[];
  categories: string[];
  error: string | null;
  filters: SkillFilters;
  loading: boolean;
  onFiltersChange: (filters: SkillFilters) => void;
  onRetry: () => void;
  onUninstall: (id: string) => void;
  resultCount: number;
  skills: InstalledSkill[];
  totalCount: number;
}) {
  const { i18n, t } = useTranslation("skills");
  const useChinese = i18n.language !== "en";
  const catalogueByName = useMemo(() => {
    const map = new Map<string, AvailableSkill>();
    for (const s of catalogue) {
      if (!map.has(s.id))
        map.set(s.id, s);
    }
    return map;
  }, [catalogue]);
  if (loading && totalCount === 0)
    return <LoadingRow />;
  if (error) {
    return (
      <div className="space-y-3">
        <div className="rounded-md border border-danger-line bg-danger-soft p-3 text-sm text-danger">
          {t("installed.loadError")}
          {error}
        </div>
        <Button leftIcon={<RotateCcw className="size-3.5" />} onClick={onRetry} size="sm" variant="secondary">
          {t("all.retry")}
        </Button>
      </div>
    );
  }
  if (totalCount === 0)
    return <EmptyState title={t("installed.emptyTitle")} detail={t("installed.emptyDetail")} />;

  return (
    <>
      <SkillFiltersBar
        categories={categories}
        filters={filters}
        onChange={onFiltersChange}
        resultCount={resultCount}
        totalCount={totalCount}
      />
      {skills.length === 0
        ? <EmptyState title={t("filter.emptyTitle")} detail={t("filter.emptyDetail")} />
        : null}
      {skills.map((skill) => {
        const cat = catalogueByName.get(skill.id);
        const name = useChinese
          ? cat?.nameZh || skill.nameZh || skill.name
          : cat?.name || skill.name;
        const description = useChinese
          ? cat?.descriptionZh || skill.descriptionZh || skill.description
          : cat?.description || skill.description;
        return (
          <SkillRow
            key={skill.id}
            name={name || skill.id}
            description={description}
            version={skill.version}
            meta={cat?.category || undefined}
            action={(
              <UninstallButton busy={busy[skill.id]} onClick={() => onUninstall(skill.id)} />
            )}
          />
        );
      })}
    </>
  );
}

function AllTab({
  busy,
  categories,
  error,
  filters,
  installedIds,
  loading,
  onFiltersChange,
  onInstall,
  onRetry,
  onUninstall,
  resultCount,
  skills,
  totalCount,
}: {
  busy: Record<string, boolean>;
  categories: string[];
  error: string | null;
  filters: SkillFilters;
  installedIds: Set<string>;
  loading: boolean;
  onFiltersChange: (filters: SkillFilters) => void;
  onInstall: (id: string, version: string) => void;
  onRetry: () => void;
  onUninstall: (id: string) => void;
  resultCount: number;
  skills: AvailableSkill[];
  totalCount: number;
}) {
  const { i18n, t } = useTranslation("skills");
  const useChineseCatalogueText = i18n.language !== "en";
  if (loading && totalCount === 0)
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
  if (totalCount === 0)
    return <EmptyState title={t("all.emptyTitle")} detail={t("all.emptyDetail")} />;

  return (
    <>
      <SkillFiltersBar
        categories={categories}
        filters={filters}
        onChange={onFiltersChange}
        resultCount={resultCount}
        totalCount={totalCount}
      />
      {skills.length === 0
        ? <EmptyState title={t("filter.emptyTitle")} detail={t("filter.emptyDetail")} />
        : null}
      {skills.map((skill) => {
        const isInstalled = installedIds.has(skill.id);
        const canInstall = Boolean(skill.latestVersion);
        const name = useChineseCatalogueText ? skill.nameZh || skill.name : skill.name;
        const description = useChineseCatalogueText ? skill.descriptionZh || skill.description : skill.description;
        return (
          <SkillRow
            key={skill.id}
            name={name || skill.id}
            description={description}
            version={skill.latestVersion}
            meta={skill.category || undefined}
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

function SkillFiltersBar({
  categories,
  filters,
  onChange,
  resultCount,
  showCategory = true,
  totalCount,
}: {
  categories: string[];
  filters: SkillFilters;
  onChange: (filters: SkillFilters) => void;
  resultCount: number;
  showCategory?: boolean;
  totalCount: number;
}) {
  const { t } = useTranslation("skills");
  const hasActiveFilters = (showCategory && filters.category !== allCategoriesValue) || filters.query.trim().length > 0;

  return (
    <div className="flex flex-col gap-2 rounded-md border border-line-soft bg-surface p-3 sm:flex-row sm:items-center">
      {showCategory
        ? (
            <Select
              aria-label={t("filter.categoryLabel")}
              onChange={event => onChange({ ...filters, category: event.target.value })}
              size="sm"
              value={filters.category}
              wrapperClassName="w-full sm:w-48"
            >
              <option value={allCategoriesValue}>{t("filter.allCategories")}</option>
              {categories.map(category => (
                <option key={category} value={category}>{category}</option>
              ))}
            </Select>
          )
        : null}
      <div className="relative min-w-0 flex-1">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-ink-muted" />
        <TextInput
          aria-label={t("filter.keywordLabel")}
          className="h-8 pl-8"
          onChange={event => onChange({ ...filters, query: event.target.value })}
          placeholder={t("filter.keywordPlaceholder")}
          value={filters.query}
        />
      </div>
      <div className="flex items-center justify-between gap-2 sm:justify-end">
        <span className="whitespace-nowrap text-xs text-ink-muted">
          {t("filter.count", { count: resultCount, total: totalCount })}
        </span>
        {hasActiveFilters
          ? (
              <Button onClick={() => onChange(emptyFilters)} size="sm" variant="ghost">
                {t("filter.clear")}
              </Button>
            )
          : null}
      </div>
    </div>
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
          ? <p className="mt-1 text-xs leading-5 text-ink-muted">{description}</p>
          : null}
        {meta
          ? (
              <div className="mt-2">
                <Badge tone="info">{meta}</Badge>
              </div>
            )
          : null}
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
