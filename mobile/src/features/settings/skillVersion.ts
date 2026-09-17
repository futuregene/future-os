/** Same dotted-version comparison as desktop; never offer a downgrade or an
 * automatic upgrade of a side-loaded skill with no recorded version. */
export function isSkillUpgrade(installed: string | null | undefined, latest: string | null | undefined): boolean {
  if (!installed?.trim() || !latest?.trim()) return false;
  const a = latest.trim().split(".");
  const b = installed.trim().split(".");
  for (let index = 0; index < Math.max(a.length, b.length); index++) {
    const left = (a[index] ?? "0").trim();
    const right = (b[index] ?? "0").trim();
    if (left === right) continue;
    const ln = Number(left);
    const rn = Number(right);
    if (Number.isFinite(ln) && Number.isFinite(rn)) {
      if (ln !== rn) return ln > rn;
    } else return left > right;
  }
  return false;
}
