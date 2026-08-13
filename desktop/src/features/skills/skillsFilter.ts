import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";

/** Sentinel category value meaning "no category filter". */
export const allCategoriesValue = "__all__";

export interface SkillFilters {
  category: string;
  query: string;
}

export function matchesInstalledSkill(skill: InstalledSkill, filters: SkillFilters, catalogue?: AvailableSkill) {
  if (!matchesCategory(catalogue?.category, filters.category))
    return false;

  // The Installed row shows the catalogue's localized name/description when the
  // agent-reported skill lacks them (its `*Zh` fields are usually null), so the
  // filter must search the catalogue text too — otherwise a query matching the
  // visible Chinese name would filter the row out.
  return matchesQuery(filters.query, [
    skill.id,
    skill.name,
    skill.nameZh,
    skill.description,
    skill.descriptionZh,
    skill.version,
    catalogue?.name,
    catalogue?.nameZh,
    catalogue?.description,
    catalogue?.descriptionZh,
    catalogue?.category,
    catalogue?.categoryZh,
  ]);
}

export function matchesAvailableSkill(skill: AvailableSkill, filters: SkillFilters) {
  if (!matchesCategory(skill.category, filters.category))
    return false;

  return matchesQuery(filters.query, [
    skill.id,
    skill.name,
    skill.nameZh,
    skill.description,
    skill.descriptionZh,
    skill.category,
    skill.categoryZh,
    skill.latestVersion,
  ]);
}

function matchesCategory(category: string | undefined, selectedCategory: string) {
  return selectedCategory === allCategoriesValue || category === selectedCategory;
}

export function matchesQuery(query: string, values: Array<string | null | undefined>) {
  const normalizedQuery = normalizeSearchText(query);
  if (!normalizedQuery)
    return true;

  return values.some(value => normalizeSearchText(value).includes(normalizedQuery));
}

export function normalizeSearchText(value: string | null | undefined) {
  return (value ?? "").trim().toLowerCase();
}

export function uniqueSorted(values: Array<string | null | undefined>) {
  return Array.from(new Set(values.filter((value): value is string => Boolean(value)))).sort((a, b) => a.localeCompare(b));
}

/** A category filter option: canonical `category` value + locale-aware label. */
export interface CategoryOption {
  value: string;
  label: string;
}

/**
 * Distinct category options sorted by value, empties excluded. With
 * `useChinese` the label is the catalogue's Chinese category when present,
 * falling back to the canonical value.
 */
export function categoryOptions(skills: AvailableSkill[], useChinese: boolean): CategoryOption[] {
  const labels = new Map<string, string>();
  for (const skill of skills) {
    if (!skill.category || labels.has(skill.category))
      continue;
    labels.set(skill.category, useChinese && skill.categoryZh ? skill.categoryZh : skill.category);
  }
  return Array.from(labels, ([value, label]) => ({ value, label }))
    .sort((a, b) => a.value.localeCompare(b.value));
}
