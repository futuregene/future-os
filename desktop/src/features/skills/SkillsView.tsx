import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import type { CategoryOption, SkillFilters } from "./skillsFilter";
import { ArrowUpCircle, Blocks, Download, Loader2, RotateCcw, Search, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { EmptyState } from "../../components/ui/EmptyState";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import {
  installSkill,
  listAvailableSkills,
  listInstalledSkills,
  refreshSkills,
  syncSkills,
  uninstallSkill,
} from "../../integrations/skills/skillsClient";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { startWindowDrag } from "../../lib/windowDrag";
import { computeSkillUpgrades } from "./autoUpgrade";
import { SkillGuideStrip } from "./SkillGuideStrip";
import {
  allCategoriesValue,
  categoryOptions,
  matchesAvailableSkill,
  matchesInstalledSkill,
} from "./skillsFilter";

type SkillsTab = "installed" | "all";
type SkillOperation
  = | { kind: "install" | "upgrade" }
    | { kind: "uninstall" | "exiting"; skill: InstalledSkill; index: number };

const emptyFilters: SkillFilters = { category: allCategoriesValue, query: "" };
const minimumBusyMs = 1_000;
const skillRowExitMs = 300;

function wait(milliseconds: number): Promise<void> {
  return new Promise(resolve => window.setTimeout(resolve, milliseconds));
}

function removeOperation(operations: Record<string, SkillOperation>, id: string): Record<string, SkillOperation> {
  const remaining = { ...operations };
  delete remaining[id];
  return remaining;
}

export function SkillsView({
  leftPanelExpanded,
  onToggleLeftPanel,
  onStartCoachConversation,
  onTrySkill,
}: {
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
  onStartCoachConversation: (content: string) => Promise<void>;
  onTrySkill: (skillName: string) => void;
}) {
  const { i18n, t } = useTranslation("skills");
  const useChinese = i18n.language !== "en";
  const [tab, setTab] = useState<SkillsTab>("installed");
  const [installed, setInstalled] = useState<InstalledSkill[]>([]);
  const [available, setAvailable] = useState<AvailableSkill[]>([]);
  const [installedFilters, setInstalledFilters] = useState<SkillFilters>(emptyFilters);
  const [allFilters, setAllFilters] = useState<SkillFilters>(emptyFilters);
  const [loading, setLoading] = useState(true);
  const [installedError, setInstalledError] = useState<string | null>(null);
  const [availableError, setAvailableError] = useState<string | null>(null);
  // Operations also own the temporary display state while the Agent and the
  // row animation finish. A background refresh must not change their buttons.
  const [operations, setOperations] = useState<Record<string, SkillOperation>>({});
  const hasResolvedInitialTabRef = useRef(false);
  // Bumped per refresh so an earlier in-flight load (rapid install/uninstall
  // clicks, or unmount) can't overwrite a newer refresh's results.
  const refreshEpochRef = useRef(0);

  const displayedInstalled = useMemo(() => {
    const visible = installed.filter(skill => operations[skill.id]?.kind !== "install");
    for (const operation of Object.values(operations)) {
      if ((operation.kind === "uninstall" || operation.kind === "exiting")
        && !visible.some(skill => skill.id === operation.skill.id)) {
        visible.splice(Math.min(operation.index, visible.length), 0, operation.skill);
      }
    }
    return visible;
  }, [installed, operations]);

  const installedIds = useMemo(
    () => new Set(displayedInstalled.map(skill => skill.id)),
    [displayedInstalled],
  );

  // Installed skill by id, so the "All" tab can compare the installed version
  // against the catalogue's latest to decide whether to offer an upgrade.
  const installedById = useMemo(
    () => new Map(displayedInstalled.map(skill => [skill.id, skill] as const)),
    [displayedInstalled],
  );

  const allCategories = useMemo(
    () => categoryOptions(available, useChinese),
    [available, useChinese],
  );

  // Catalogue lookup for installed skills. Used for filtering (category + the
  // localized name/description shown on the row) and the category dropdown —
  // uncategorized skills are excluded when a specific category is selected.
  const availableById = useMemo(
    () => new Map(available.map(skill => [skill.id, skill] as const)),
    [available],
  );

  // Categories that have at least one installed skill (matched via catalogue).
  const installedCategories = useMemo(() => {
    const catalogueEntries = displayedInstalled
      .map(skill => availableById.get(skill.id))
      .filter((skill): skill is AvailableSkill => Boolean(skill));
    return categoryOptions(catalogueEntries, useChinese);
  }, [displayedInstalled, availableById, useChinese]);

  const filteredInstalled = useMemo(
    () => displayedInstalled.filter(skill => matchesInstalledSkill(skill, installedFilters, availableById.get(skill.id))),
    [displayedInstalled, installedFilters, availableById],
  );

  const filteredAvailable = useMemo(
    () => available.filter(skill => matchesAvailableSkill(skill, allFilters)),
    [allFilters, available],
  );

  // All installed skills with a newer catalogue version — powers the Installed
  // tab's "Upgrade all" button (disabled when empty). Computed over the full
  // installed set, not the filtered view.
  const skillUpgrades = useMemo(
    () => computeSkillUpgrades(displayedInstalled, available),
    [displayedInstalled, available],
  );

  const refresh = useCallback(async () => {
    const epoch = ++refreshEpochRef.current;
    setLoading(true);
    // Installed comes from the agent; the catalogue needs the platform reachable
    // and may fail independently — keep the installed tab usable either way.
    const [installedResult, availableResult] = await Promise.allSettled([
      listInstalledSkills(),
      listAvailableSkills(),
    ]);
    // A newer refresh started while this one was in flight — drop these results.
    if (epoch !== refreshEpochRef.current)
      return;
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
    // First tell the agent to re-scan skills so the installed list is fresh,
    // then load both lists. refreshSkills is best-effort (agent may be down).
    void refreshSkills().finally(() => void refresh());
  }, [refresh]);

  // The silent auto-upgrade installs newer versions out-of-band; reload so an
  // open view reflects them.
  useEffect(() => onFutureEvent("skills-changed", () => void refresh()), [refresh]);

  const runInstall = useCallback(async (id: string, version: string, kind: "install" | "upgrade") => {
    setOperations(current => ({ ...current, [id]: { kind } }));
    try {
      const [outcome] = await Promise.allSettled([installSkill(id, version), wait(minimumBusyMs)]);
      if (outcome.status === "rejected")
        throw outcome.reason;
      const snapshot = await listInstalledSkills();
      if (!snapshot.some(skill => skill.id === id))
        throw new Error(`Installed skill ${id} was not returned by the Agent`);

      // Replace the list and its temporary action in one render. The button
      // changes directly from "Installing" to "Uninstall".
      refreshEpochRef.current++;
      setInstalled(snapshot);
      setOperations(current => removeOperation(current, id));
      setLoading(false);
      emitFutureEvent("skills-changed", undefined);
    }
    catch (error) {
      setOperations(current => removeOperation(current, id));
      emitFutureEvent("toast", { message: t("actionFailed", { message: errorMessage(error) }), tone: "error" });
    }
  }, [t]);

  const runUninstall = useCallback(async (id: string) => {
    const skill = displayedInstalled.find(item => item.id === id);
    if (!skill)
      return;
    const index = displayedInstalled.indexOf(skill);
    setOperations(current => ({ ...current, [id]: { kind: "uninstall", skill, index } }));
    try {
      const [outcome] = await Promise.allSettled([
        uninstallSkill(id),
        wait(minimumBusyMs),
      ]);
      if (outcome.status === "rejected")
        throw outcome.reason;
      const snapshot = await listInstalledSkills();
      if (snapshot.some(item => item.id === id))
        throw new Error(`Uninstalled skill ${id} is still returned by the Agent`);

      setOperations(current => ({ ...current, [id]: { kind: "exiting", skill, index } }));
      await wait(skillRowExitMs);
      refreshEpochRef.current++;
      setInstalled(snapshot);
      setOperations(current => removeOperation(current, id));
      setLoading(false);
      emitFutureEvent("skills-changed", undefined);
    }
    catch (error) {
      setOperations(current => removeOperation(current, id));
      emitFutureEvent("toast", { message: t("actionFailed", { message: errorMessage(error) }), tone: "error" });
    }
  }, [displayedInstalled, t]);

  // Agent owns the version decision and the install transaction. The local
  // comparison only controls the button's count and busy presentation.
  const upgradeAll = useCallback(async () => {
    if (skillUpgrades.length === 0)
      return;
    const ids = skillUpgrades.map(u => u.id);
    setOperations(current => ({ ...current, ...Object.fromEntries(ids.map(id => [id, { kind: "upgrade" as const }])) }));
    try {
      const [outcome] = await Promise.allSettled([syncSkills(), wait(minimumBusyMs)]);
      if (outcome.status === "rejected")
        throw outcome.reason;
      const result = outcome.value;
      if (result.failed.length > 0)
        throw new Error(result.failed.join("; "));
      const [installedSnapshot, availableSnapshot] = await Promise.all([
        listInstalledSkills(),
        listAvailableSkills(),
      ]);
      refreshEpochRef.current++;
      setInstalled(installedSnapshot);
      setAvailable(availableSnapshot);
      setLoading(false);
      emitFutureEvent("skills-changed", undefined);
    }
    catch (error) {
      emitFutureEvent("toast", { message: t("actionFailed", { message: errorMessage(error) }), tone: "error" });
    }
    finally {
      setOperations(current => ids.reduce(removeOperation, current));
    }
  }, [skillUpgrades, t]);

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft/40 px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle expanded={leftPanelExpanded} onToggle={onToggleLeftPanel} />
          <span className="truncate text-sm font-semibold text-ink">{t("title")}</span>
        </div>
      </header>

      <header className="flex flex-wrap items-end justify-between gap-3 border-b border-line-soft px-8 pb-3 pt-4">
        <div>
          <p className="text-sm text-ink-muted">{t("subtitle")}</p>
          <div className="mt-3 grid w-64 grid-cols-2 gap-1 rounded-md bg-surface-subtle p-1">
            <TabButton active={tab === "installed"} label={t("tab.installed")} onClick={() => setTab("installed")} />
            <TabButton active={tab === "all"} label={t("tab.all")} onClick={() => setTab("all")} />
          </div>
        </div>
        <SkillGuideStrip onStartCoachConversation={onStartCoachConversation} />
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
                  totalCount={displayedInstalled.length}
                  error={installedError}
                  operations={operations}
                  catalogue={available}
                  upgradeCount={skillUpgrades.length}
                  onTrySkill={onTrySkill}
                  onUninstall={id => void runUninstall(id)}
                  onUpgrade={(id, version) => void runInstall(id, version, "upgrade")}
                  onUpgradeAll={() => void upgradeAll()}
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
                  installedById={installedById}
                  error={availableError}
                  operations={operations}
                  onInstall={(id, version) => void runInstall(id, version, "install")}
                  onUninstall={id => void runUninstall(id)}
                  onUpgrade={(id, version) => void runInstall(id, version, "upgrade")}
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
  catalogue,
  categories,
  error,
  filters,
  loading,
  operations,
  onFiltersChange,
  onRetry,
  onTrySkill,
  onUninstall,
  onUpgrade,
  onUpgradeAll,
  resultCount,
  skills,
  totalCount,
  upgradeCount,
}: {
  catalogue: AvailableSkill[];
  categories: CategoryOption[];
  error: string | null;
  filters: SkillFilters;
  loading: boolean;
  operations: Record<string, SkillOperation>;
  onFiltersChange: (filters: SkillFilters) => void;
  onRetry: () => void;
  onTrySkill: (skillName: string) => void;
  onUninstall: (id: string) => void;
  onUpgrade: (id: string, version: string) => void;
  onUpgradeAll: () => void;
  resultCount: number;
  skills: InstalledSkill[];
  totalCount: number;
  upgradeCount: number;
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

  const anyBusy = skills.some(skill => Boolean(operations[skill.id]));
  return (
    <>
      <SkillFiltersBar
        categories={categories}
        filters={filters}
        onChange={onFiltersChange}
        resultCount={resultCount}
        totalCount={totalCount}
        trailing={(
          <Button
            className="shrink-0"
            disabled={upgradeCount === 0 || anyBusy}
            leftIcon={<ArrowUpCircle className="size-3.5" />}
            onClick={onUpgradeAll}
            size="sm"
            variant="secondary"
          >
            {upgradeCount > 0 ? t("upgrade.upgradeAllCount", { count: upgradeCount }) : t("upgrade.upgradeAll")}
          </Button>
        )}
      />
      {skills.length === 0
        ? <EmptyState title={t("filter.emptyTitle")} detail={t("filter.emptyDetail")} />
        : null}
      <div>
        {skills.map((skill) => {
          const cat = catalogueByName.get(skill.id);
          const zh = useChinese && (cat?.nameZh || skill.nameZh);
          const name = zh ? `${cat?.name || skill.name}（${zh}）` : (cat?.name || skill.name);
          const description = useChinese
            ? cat?.descriptionZh || skill.descriptionZh || skill.description
            : cat?.description || skill.description;
          const category = (useChinese && cat?.categoryZh ? cat.categoryZh : cat?.category) || undefined;
          const latest = cat?.latestVersion ?? null;
          const canUpgrade = Boolean(cat?.upgradeAvailable && latest);
          return (
            <SkillRowTransition key={skill.id} exiting={operations[skill.id]?.kind === "exiting"} skillId={skill.id}>
              <SkillRow
                name={name || skill.id}
                description={description}
                version={skill.version}
                meta={category}
                action={(
                  <div className="flex items-center gap-2">
                    <Button onClick={() => onTrySkill(skill.name)} size="sm" variant="secondary">
                      {t("tryIt")}
                    </Button>
                    <UninstallButton operation={operations[skill.id]} onClick={() => onUninstall(skill.id)} />
                    {canUpgrade && latest
                      ? (
                          <UpgradeButton
                            busy={Boolean(operations[skill.id])}
                            version={latest}
                            onClick={() => onUpgrade(skill.id, latest)}
                          />
                        )
                      : null}
                  </div>
                )}
              />
            </SkillRowTransition>
          );
        })}
      </div>
    </>
  );
}

function AllTab({
  categories,
  error,
  filters,
  installedById,
  installedIds,
  loading,
  operations,
  onFiltersChange,
  onInstall,
  onRetry,
  onUninstall,
  onUpgrade,
  resultCount,
  skills,
  totalCount,
}: {
  categories: CategoryOption[];
  error: string | null;
  filters: SkillFilters;
  installedById: Map<string, InstalledSkill>;
  installedIds: Set<string>;
  loading: boolean;
  operations: Record<string, SkillOperation>;
  onFiltersChange: (filters: SkillFilters) => void;
  onInstall: (id: string, version: string) => void;
  onRetry: () => void;
  onUninstall: (id: string) => void;
  onUpgrade: (id: string, version: string) => void;
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
        const zh = useChineseCatalogueText && skill.nameZh;
        const name = zh ? `${skill.name}（${zh}）` : skill.name;
        const description = useChineseCatalogueText ? skill.descriptionZh || skill.description : skill.description;
        const canUpgrade = Boolean(installedById.has(skill.id) && skill.upgradeAvailable);
        return (
          <SkillRow
            key={skill.id}
            name={name || skill.id}
            description={description}
            version={skill.latestVersion}
            meta={(useChineseCatalogueText && skill.categoryZh ? skill.categoryZh : skill.category) || undefined}
            action={(
              <div
                className={cn(
                  "transition-[opacity,transform] duration-300 ease-out motion-reduce:transition-none",
                  operations[skill.id]?.kind === "exiting" ? "translate-y-1 opacity-0" : "translate-y-0 opacity-100",
                )}
                data-skill-action-id={skill.id}
              >
                {isInstalled
                  ? (
                      <div className="flex items-center gap-2">
                        <UninstallButton operation={operations[skill.id]} onClick={() => onUninstall(skill.id)} />
                        {canUpgrade && skill.latestVersion
                          ? (
                              <UpgradeButton
                                busy={Boolean(operations[skill.id])}
                                version={skill.latestVersion}
                                onClick={() => skill.latestVersion && onUpgrade(skill.id, skill.latestVersion)}
                              />
                            )
                          : null}
                      </div>
                    )
                  : (
                      <Button
                        disabled={Boolean(operations[skill.id]) || !canInstall}
                        leftIcon={operations[skill.id]
                          ? <Loader2 aria-hidden="true" className="size-3.5 animate-spin motion-reduce:animate-none" />
                          : <Download className="size-3.5" />}
                        onClick={() => skill.latestVersion && onInstall(skill.id, skill.latestVersion)}
                        size="sm"
                        variant="primary"
                      >
                        {operations[skill.id] ? t("install.installing") : canInstall ? t("install.install") : t("install.noVersion")}
                      </Button>
                    )}
              </div>
            )}
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
  totalCount,
  trailing,
}: {
  categories: CategoryOption[];
  filters: SkillFilters;
  onChange: (filters: SkillFilters) => void;
  resultCount: number;
  totalCount: number;
  /** Optional action rendered at the end of the row (e.g. "Upgrade all"). */
  trailing?: React.ReactNode;
}) {
  const { t } = useTranslation("skills");
  const hasActiveFilters = filters.category !== allCategoriesValue || filters.query.trim().length > 0;

  return (
    <div className="flex flex-col gap-2 rounded-md border border-line-soft bg-surface p-3 sm:flex-row sm:items-center">
      <Select
        aria-label={t("filter.categoryLabel")}
        onChange={event => onChange({ ...filters, category: event.target.value })}
        size="sm"
        value={filters.category}
        wrapperClassName="w-full sm:w-48"
      >
        <option value={allCategoriesValue}>{t("filter.allCategories")}</option>
        {categories.map(option => (
          <option key={option.value} value={option.value}>{option.label}</option>
        ))}
      </Select>
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
      {trailing}
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

function SkillRowTransition({
  children,
  exiting,
  skillId,
}: {
  children: React.ReactNode;
  exiting: boolean;
  skillId: string;
}) {
  return (
    <div
      className={cn(
        "grid transition-[grid-template-rows,opacity,transform,margin] duration-300 ease-out motion-reduce:transition-none",
        exiting ? "mb-0 -translate-y-1 grid-rows-[0fr] opacity-0" : "mb-3 grid-rows-[1fr] opacity-100",
      )}
      data-exiting={exiting ? "true" : "false"}
      data-skill-id={skillId}
    >
      <div className="min-h-0 overflow-hidden">{children}</div>
    </div>
  );
}

function UpgradeButton({ busy, onClick, version }: { busy?: boolean; onClick: () => void; version: string }) {
  const { t } = useTranslation("skills");
  return (
    <Button
      disabled={busy}
      leftIcon={busy
        ? <Loader2 aria-hidden="true" className="size-3.5 animate-spin motion-reduce:animate-none" />
        : <ArrowUpCircle className="size-3.5" />}
      onClick={onClick}
      size="sm"
      title={t("upgrade.available", { version })}
      variant="primary"
    >
      {busy ? t("upgrade.upgrading") : t("upgrade.upgrade")}
    </Button>
  );
}

function UninstallButton({ operation, onClick }: { operation?: SkillOperation; onClick: () => void }) {
  const { t } = useTranslation("skills");
  const [confirming, setConfirming] = useState(false);
  const busy = Boolean(operation);
  const uninstalling = operation?.kind === "uninstall" || operation?.kind === "exiting";
  if (!confirming) {
    return (
      <Button
        disabled={busy}
        leftIcon={uninstalling
          ? <Loader2 aria-hidden="true" className="size-3.5 animate-spin motion-reduce:animate-none" />
          : <Trash2 className="size-3.5" />}
        onClick={() => setConfirming(true)}
        size="sm"
        variant="danger-soft"
      >
        {uninstalling ? t("uninstall.uninstalling") : t("uninstall.uninstall")}
      </Button>
    );
  }
  return (
    <div className="flex items-center gap-2">
      <Button disabled={busy} onClick={() => setConfirming(false)} size="sm" variant="ghost">
        {t("uninstall.cancel")}
      </Button>
      <Button
        disabled={busy}
        leftIcon={uninstalling
          ? <Loader2 aria-hidden="true" className="size-3.5 animate-spin motion-reduce:animate-none" />
          : undefined}
        onClick={onClick}
        size="sm"
        variant="danger"
      >
        {uninstalling ? t("uninstall.uninstalling") : t("uninstall.confirm")}
      </Button>
    </div>
  );
}

function LoadingRow() {
  const { t } = useTranslation("skills");
  return <div className="rounded-md border border-line-soft bg-surface p-3 text-sm text-ink-muted">{t("loading")}</div>;
}
