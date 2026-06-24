import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { useEffect, useMemo, useState } from "react";
import { TextInput } from "../../components/ui/TextInput";
import { listAgentProviders } from "../../integrations/agent/providers";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

function modelKey(model: { id: string; provider: string }) {
  return `${model.provider}/${model.id}`;
}

export function ModelsPage({
  hiddenModels,
  modelOptions,
  onChangeHidden,
}: {
  hiddenModels: string[];
  modelOptions: AgentModelOption[];
  onChangeHidden: (next: string[]) => void;
}) {
  const [query, setQuery] = useState("");
  const [providerNames, setProviderNames] = useState<Record<string, string>>({});
  const hidden = useMemo(() => new Set(hiddenModels), [hiddenModels]);

  // Map provider id → display name (built-in FutureGene + custom providers).
  // Built-in catalog providers (deepseek, openai, …) have no entry → fall back
  // to the id in providerLabel().
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
        setProviderNames(map);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const providerLabel = (providerId: string) => providerNames[providerId] ?? providerId;

  const groups = useMemo(() => {
    const label = (providerId: string) => providerNames[providerId] ?? providerId;
    const needle = query.trim().toLowerCase();
    const byProvider = new Map<string, AgentModelOption[]>();
    for (const model of modelOptions) {
      if (needle
        && !model.label.toLowerCase().includes(needle)
        && !model.id.toLowerCase().includes(needle)
        && !model.provider.toLowerCase().includes(needle)
        && !label(model.provider).toLowerCase().includes(needle)) {
        continue;
      }
      const list = byProvider.get(model.provider) ?? [];
      list.push(model);
      byProvider.set(model.provider, list);
    }
    return [...byProvider.entries()].sort(([a], [b]) => label(a).localeCompare(label(b)));
  }, [modelOptions, query, providerNames]);

  function setVisibility(model: AgentModelOption, visible: boolean) {
    const key = modelKey(model);
    if (visible) {
      onChangeHidden(hiddenModels.filter(item => item !== key));
    }
    else if (!hidden.has(key)) {
      onChangeHidden([...hiddenModels, key]);
    }
  }

  return (
    <div className="space-y-6">
      <TextInput
        onChange={event => setQuery(event.target.value)}
        placeholder="搜索模型…"
        value={query}
      />

      {modelOptions.length === 0
        ? <p className="text-sm text-ink-muted">未从 Agent 读取到模型。请确认 Agent 已启动并完成登录。</p>
        : null}

      {groups.map(([provider, models]) => (
        <SettingsSection key={provider} title={providerLabel(provider)}>
          <SettingsList>
            {models.map(model => (
              <SettingsRow
                key={modelKey(model)}
                title={model.label}
                description={model.isDefault ? "默认模型" : model.id}
              >
                <Switch
                  checked={!hidden.has(modelKey(model))}
                  label={model.label}
                  onChange={visible => setVisibility(model, visible)}
                />
              </SettingsRow>
            ))}
          </SettingsList>
        </SettingsSection>
      ))}

      {modelOptions.length > 0 && groups.length === 0
        ? <p className="text-sm text-ink-muted">没有匹配的模型。</p>
        : null}
    </div>
  );
}
