import { readFile } from "node:fs/promises";

import { AUTH_FILE, DEFAULT_PLATFORM_URL, FUTURE_AUTH_PROVIDER } from "../constants.js";
import { isRecord } from "./object.js";
import { trimTrailingSlash } from "./string.js";

/**
 * Resolve the Future Platform base URL with this priority:
 *   1. Explicit override (e.g. --url CLI argument)
 *   2. FUTURE_BASE_URL environment variable
 *   3. auth.json → future.base_url (strip /api)
 *   4. DEFAULT_PLATFORM_URL
 */
export async function getPlatformUrl(override?: string): Promise<string> {
  // Priority 1: explicit override
  if (override) return trimTrailingSlash(override);

  // Priority 2: environment variable
  const envUrl = process.env["FUTURE_BASE_URL"];
  if (envUrl) return trimTrailingSlash(envUrl);

  // Priority 3: auth.json
  try {
    const raw = await readFile(AUTH_FILE, "utf8");
    const auth = JSON.parse(raw) as unknown;
    if (isRecord(auth)) {
      const future = auth[FUTURE_AUTH_PROVIDER];
      if (isRecord(future)) {
        const rec = future as Record<string, unknown>;

        const baseUrl = rec.base_url;
        if (typeof baseUrl === "string" && baseUrl.length > 0) {
          return trimTrailingSlash(baseUrl.replace(/\/api\/?$/, ""));
        }
      }
    }
  } catch {
    // auth.json not found or unreadable — fall through
  }

  // Priority 4: default
  return DEFAULT_PLATFORM_URL;
}
