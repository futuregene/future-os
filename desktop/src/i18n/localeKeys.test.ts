import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Every literal i18n key a component asks for must exist in every locale.
 *
 * A missing key is invisible in every other check: the component type-checks,
 * the feature test asserts the element it needs, and the UI quietly renders the
 * key itself (`menu.add` shipped as a panel-menu row reading "menu.addmenu.
 * addHint" because the copy lived under `add.*` instead). Only the built screen
 * shows it, and only if somebody looks at that screen.
 *
 * Dynamic keys (`t(\`…${x}\`)`) are out of reach here; this covers the literal
 * form, which is how nearly every key is written.
 */

const SRC = resolve(__dirname, "..");
const LOCALES = resolve(__dirname, "locales");
const LANGUAGES = readdirSync(LOCALES).filter(name => statSync(join(LOCALES, name)).isDirectory());

type Catalog = Map<string, Set<string>>;

function flatten(node: Record<string, unknown>, prefix = "", out = new Set<string>()): Set<string> {
  for (const [key, value] of Object.entries(node)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object")
      flatten(value as Record<string, unknown>, path, out);
    else
      out.add(path);
  }
  return out;
}

/** namespace → its keys, per language. */
function catalogs(language: string): Catalog {
  const out: Catalog = new Map();
  for (const file of readdirSync(join(LOCALES, language))) {
    if (!file.endsWith(".json"))
      continue;
    const data = JSON.parse(readFileSync(join(LOCALES, language, file), "utf8")) as Record<string, unknown>;
    out.set(file.replace(/\.json$/, ""), flatten(data));
  }
  return out;
}

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      if (entry !== "locales" && entry !== "__snapshots__")
        sourceFiles(path, out);
      continue;
    }
    if (!/\.(?:ts|tsx)$/.test(entry) || /\.test\.(?:ts|tsx)$/.test(entry))
      continue;
    out.push(path);
  }
  return out;
}

/**
 * The keys `t()` is called with, as `[namespace | null, key]`.
 *
 * `namespace:key` is the explicit form; the single-argument form relies on the
 * caller's `useTranslation("…")`, so it is matched against every namespace.
 */
function requestedKeys(): Array<{ key: string; namespace: string | null; file: string }> {
  const found: Array<{ key: string; namespace: string | null; file: string }> = [];
  for (const file of sourceFiles(SRC)) {
    const text = readFileSync(file, "utf8");
    const where = relative(SRC, file);
    for (const match of text.matchAll(/\bt\(\s*"([^"$]+)"/g)) {
      const literal = match[1] ?? "";
      const colon = literal.indexOf(":");
      if (colon > 0)
        found.push({ file: where, key: literal.slice(colon + 1), namespace: literal.slice(0, colon) });
      else if (literal.includes("."))
        found.push({ file: where, key: literal, namespace: null });
    }
  }
  return found;
}

/** i18next stores plural forms under suffixes; a key may exist only as one. */
const PLURAL_SUFFIXES = ["_zero", "_one", "_two", "_few", "_many", "_other", "_plural"];

function has(catalog: Set<string>, key: string): boolean {
  return catalog.has(key) || PLURAL_SUFFIXES.some(suffix => catalog.has(`${key}${suffix}`));
}

describe("i18n keys used by components", () => {
  it("exist in every locale", () => {
    const perLanguage = new Map(LANGUAGES.map(language => [language, catalogs(language)]));
    const missing: string[] = [];

    for (const { key, namespace, file } of requestedKeys()) {
      for (const language of LANGUAGES) {
        const catalog = perLanguage.get(language)!;
        if (namespace !== null) {
          if (!has(catalog.get(namespace) ?? new Set(), key))
            missing.push(`${language}: ${namespace}:${key} (used by ${file})`);
          continue;
        }
        if (![...catalog.values()].some(keys => has(keys, key)))
          missing.push(`${language}: ${key} (used by ${file})`);
      }
    }

    // Deduplicated: the same key is often requested from several screens.
    expect([...new Set(missing)]).toEqual([]);
  });

  it("reads the keys its scanner expects to see", () => {
    // Falsification guard for the test above: a scanner that silently matched
    // nothing would report no missing keys for any code base.
    const keys = requestedKeys();
    expect(keys.length).toBeGreaterThan(500);
    expect(keys.some(entry => entry.namespace === "layout")).toBe(true);
  });
});
