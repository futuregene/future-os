import type { TFunction } from "i18next";

/**
 * A declared thinking level's label, falling back to the wire value itself.
 *
 * The host declares the values its models accept, and that vocabulary is open:
 * `"default"` and the legacy six have labels, but `"max"` and a provider's own
 * token (`"turbo"`, …) reach the UI untranslated. Rendering the translation key
 * there would leak `thinking.turbo` into a picker, so the raw value is the label.
 */
export function thinkingLevelLabel(
  t: TFunction,
  level: string,
  keyPrefix = "thinking",
): string {
  const key = `${keyPrefix}.${level}`;
  const label = t(key);
  return label === key ? level : label;
}
