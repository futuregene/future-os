/**
 * `future models` command — list models available through the running agent.
 */

import { RunClient } from "../rpc/grpc-client.js";

function humanContextWindow(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(0)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(0)}K`;
  return String(tokens);
}

export async function models(args: string[]): Promise<void> {
  const jsonFlag = args.includes("--json");
  const nonFlags = args.filter(a => !a.startsWith("--"));
  const grpcAddr = nonFlags[0] ?? process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051";

  const client = new RunClient(grpcAddr);

  let data: Awaited<ReturnType<typeof client.listModels>>;
  try {
    data = await client.listModels();
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (jsonFlag) {
      console.log(JSON.stringify({ error: msg }));
    } else {
      console.error(`Error: failed to list models — ${msg}`);
    }
    process.exit(1);
  }

  if (jsonFlag) {
    const providers = [...new Set(data.models.map(m => m.provider))].sort();
    const byProvider: Record<string, Array<{
      id: string;
      label: string;
      contextWindow: number;
      supportsImages: boolean;
      thinkingLevel: string;
      isDefault: boolean;
    }>> = {};
    for (const provider of providers) {
      byProvider[provider] = data.models
        .filter(m => m.provider === provider)
        .sort((a, b) => a.id.localeCompare(b.id))
        .map(m => ({
          id: m.id,
          label: m.label,
          contextWindow: m.contextWindow,
          supportsImages: m.supportsImages,
          thinkingLevel: m.thinkingLevel,
          isDefault: m.id === data.defaultModel,
        }));
    }
    console.log(JSON.stringify({
      providers,
      defaultModel: data.defaultModel,
      models: byProvider,
      totalModels: data.models.length,
    }, null, 2));
    return;
  }

  // Group by provider
  const byProvider = new Map<string, typeof data.models>();
  for (const m of data.models) {
    const list = byProvider.get(m.provider) ?? [];
    list.push(m);
    byProvider.set(m.provider, list);
  }

  // Print provider → model hierarchy
  const sorted = [...byProvider.entries()].sort();
  for (const [provider, providerModels] of sorted) {
    console.log(`Provider: ${provider}  (${providerModels.length} models)`);
    for (const m of providerModels) {
      const isDefault = m.id === data.defaultModel;
      const ctxWin = humanContextWindow(m.contextWindow);
      const img = m.supportsImages ? "  image" : "";
      const thinking = m.thinkingLevel !== "off" ? `  thinking:${m.thinkingLevel}` : "";
      const def = isDefault ? "  [default]" : "";
      console.log(`  Model: ${m.id.padEnd(28)} ctx:${ctxWin.padStart(5)}${img}${thinking}${def}`);
    }
    console.log("");
  }

  console.log(`${data.models.length} models, ${sorted.length} providers.  Default model: ${data.defaultModel}`);
}
