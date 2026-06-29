import { useAsyncResource } from "../../lib/useAsyncResource";
import { listAgentProviders } from "./providers";

/**
 * Map of provider id → display name (built-in FutureGene + custom providers).
 * Built-in catalog providers (deepseek, openai, …) have no entry; callers fall
 * back to the id. Best-effort: errors leave the map empty.
 */
export function useProviderNames(): Record<string, string> {
  const { data } = useAsyncResource<Record<string, string>>(
    async () => {
      const view = await listAgentProviders();
      const map: Record<string, string> = {};
      for (const provider of [...view.builtin, ...view.custom]) {
        map[provider.id] = provider.name;
      }
      return map;
    },
    [],
    {},
  );

  return data;
}
