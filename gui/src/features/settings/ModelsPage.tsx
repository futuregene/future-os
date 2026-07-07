import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { TextInput } from "../../components/ui/TextInput";
import { modelKey } from "../../integrations/agent/agentClient";
import { useProviderNames } from "../../integrations/agent/useProviderNames";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function ModelsPage({
  hiddenModels,
  modelOptions,
  onChangeHidden,
}: {
  hiddenModels: string[];
  modelOptions: AgentModelOption[];
  onChangeHidden: (next: string[]) => void;
}) {
  const { t } = useTranslation("settings");
  const [query, setQuery] = useState("");
  const providerNames = useProviderNames();
  const hidden = useMemo(() => new Set(hiddenModels), [hiddenModels]);

  // Built-in catalog providers (deepseek, openai, …) have no name → fall back to id.
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

  // Subtitle = optional model id + input modality ("Text" / "Text Image").
  function modelSubtitle(model: AgentModelOption) {
    const modality = [
      t("models.modality.text"),
      model.supportsImages ? t("models.modality.image") : null,
    ].filter(Boolean).join(" ");
    const id = model.label === model.id ? null : model.id;
    return id ? `${id} · ${modality}` : modality;
  }

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
        placeholder={t("models.searchPlaceholder")}
        value={query}
      />

      {modelOptions.length === 0
        ? <p className="text-sm text-ink-muted">{t("models.noModels")}</p>
        : null}

      {groups.map(([provider, models]) => (
        <SettingsSection key={provider} title={providerLabel(provider)}>
          <SettingsList>
            {models.map(model => (
              <SettingsRow
                key={modelKey(model)}
                title={model.label}
                // Subtitle: raw model id (dropped when it equals the label so it
                // isn't shown twice) followed by the input modality, e.g.
                // "Text" or "Text Image".
                description={modelSubtitle(model)}
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
        ? <p className="text-sm text-ink-muted">{t("models.noMatch")}</p>
        : null}
    </div>
  );
}
