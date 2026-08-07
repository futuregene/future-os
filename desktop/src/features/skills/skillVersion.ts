/**
 * Compare two dotted version strings (semver-ish, e.g. "1.2.0").
 * Each dot-separated segment is compared numerically when both sides are
 * numeric, otherwise lexically. Missing trailing segments count as 0, so
 * "1.2" and "1.2.0" compare equal. Returns >0 when `a` is newer than `b`,
 * <0 when older, 0 when equal.
 */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(".");
  const pb = b.split(".");
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i++) {
    const sa = (pa[i] ?? "0").trim();
    const sb = (pb[i] ?? "0").trim();
    if (sa === sb)
      continue;
    const na = Number(sa);
    const nb = Number(sb);
    if (Number.isFinite(na) && Number.isFinite(nb)) {
      if (na !== nb)
        return na < nb ? -1 : 1;
      continue;
    }
    return sa < sb ? -1 : 1;
  }
  return 0;
}

/**
 * Whether `latest` is a strictly newer version than `installed`. Returns false
 * when either version is missing/blank or when they can't be compared — the
 * upgrade affordance should only appear when we're confident there's a newer
 * version available.
 */
export function isUpgradeAvailable(
  installed: string | null | undefined,
  latest: string | null | undefined,
): boolean {
  if (!installed || !latest)
    return false;
  const a = installed.trim();
  const b = latest.trim();
  if (!a || !b)
    return false;
  return compareVersions(b, a) > 0;
}
