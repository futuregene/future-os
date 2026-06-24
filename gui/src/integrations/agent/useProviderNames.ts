import { useEffect, useState } from "react";
import { listAgentProviders } from "./providers";

/**
 * Map of provider id → display name (built-in FutureGene + custom providers).
 * Built-in catalog providers (deepseek, openai, …) have no entry; callers fall
 * back to the id. Best-effort: errors leave the map empty.
 */
export function useProviderNames(): Record<string, string> {
  const [names, setNames] = useState<Record<string, string>>({});

  useEffect(() => {
    let cancelled = false;
    listAgentProviders()
      .then((view) => {
        if (cancelled)
          return;
        const map: Record<string, string> = {};
        for (const provider of [...view.builtin, ...view.custom]) {
          map[provider.id] = provider.name;
        }
        setNames(map);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  return names;
}
