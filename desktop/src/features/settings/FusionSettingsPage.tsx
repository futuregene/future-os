import type { FusionConfiguration, FusionModelConfig, FusionSettings, FusionSettingsPatch } from "@future-os/thread-projection";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ProvidersView } from "../../integrations/agent/providers";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import { fusionSettingsIssue, newFusionParticipant } from "@future-os/thread-projection";
import { ChevronDown, Plus, Trash2 } from "lucide-react";
import { useCallback, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { SelectMenu, SelectMenuItem } from "../../components/ui/SelectMenu";
import { localizedModelDescription, modelKey, modelThinkingLevels } from "../../integrations/agent/agentClient";
import { listAgentProviders } from "../../integrations/agent/providers";
import { getFusionSettings, updateFusionSettings } from "../../integrations/fusion/settings";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { useHostCatalogueInvalidation } from "../remote-peer/remoteHostCatalog";
import { listRemoteProviders, requestRemotePeer } from "../remote-peer/remotePeerClient";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

const localModels = () => invokeCommand<AgentModelOption[]>("list_agent_models");
interface Source {
  revision?: number;
  load: () => Promise<FusionSettings>;
  save: (patch: FusionSettingsPatch) => Promise<FusionSettings>;
  models: () => Promise<AgentModelOption[]>;
  providers: () => Promise<ProvidersView>;
}
const LOCAL: Source = { load: getFusionSettings, save: updateFusionSettings, models: localModels, providers: listAgentProviders };

function ModelRow({ title, value, models, providerNames, disabled, onChange, onRemove }: {
  title: string;
  value: FusionModelConfig;
  models: AgentModelOption[];
  providerNames: Record<string, string>;
  disabled: boolean;
  onChange: (value: FusionModelConfig) => void;
  onRemove?: () => void;
}) {
  const { t } = useTranslation("settings");
  const { t: agentT, i18n } = useTranslation("agent");
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const modelId = useId();
  const thinkingId = useId();
  const selected = models.find(model => modelKey(model) === value.model);
  const levels = modelThinkingLevels(value.model, models);
  const displayName = (model: AgentModelOption) => `${providerNames[model.provider] ?? model.provider}/${model.label || model.id}`;
  return (
    <fieldset className="rounded-lg border border-line-soft px-3 pb-3 pt-2">
      <legend className="px-1 text-sm font-medium text-ink">{title}</legend>
      <div className="flex items-end gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          <label htmlFor={modelId} className="text-xs leading-4 text-ink-muted">{t("fusion.model")}</label>
          <SelectMenu
            open={modelMenuOpen}
            onDismiss={() => setModelMenuOpen(false)}
            portal
            align="left"
            panelClassName="max-h-[40vh] overflow-y-auto"
            trigger={(
              <button
                id={modelId}
                aria-label={`${title} · ${t("fusion.model")}`}
                aria-expanded={modelMenuOpen}
                aria-haspopup="true"
                disabled={disabled}
                className="flex h-9 w-full min-w-0 items-center gap-2 rounded-md border border-line-soft bg-surface pl-3 pr-2.5 text-left text-sm text-ink outline-none transition-colors focus:border-focus focus:ring-2 focus:ring-focus disabled:cursor-not-allowed disabled:opacity-50"
                onClick={() => setModelMenuOpen(open => !open)}
                type="button"
              >
                <span className="min-w-0 flex-1 truncate">{selected ? displayName(selected) : value.model ? t("fusion.unavailable", { model: value.model }) : t("fusion.chooseModel")}</span>
                <ChevronDown className="size-4 shrink-0 text-ink-muted" />
              </button>
            )}
          >
            {models.map(model => (
              <SelectMenuItem
                className="py-1"
                key={modelKey(model)}
                selected={modelKey(model) === value.model}
                disabled={disabled}
                title={localizedModelDescription(model, i18n.language) ?? undefined}
                onSelect={() => {
                  const nextLevels = modelThinkingLevels(modelKey(model), models);
                  onChange({ ...newFusionParticipant(), model: modelKey(model), thinking_level: nextLevels.includes(value.thinking_level) ? value.thinking_level : nextLevels[0] ?? "default" });
                  setModelMenuOpen(false);
                }}
              >
                <span className="min-w-0 flex-1 truncate font-medium leading-tight text-ink">{displayName(model)}</span>
              </SelectMenuItem>
            ))}
            {models.length === 0 ? <div className="px-3 py-2 text-sm text-ink-muted">{agentT("composer.startAgentForModels")}</div> : null}
          </SelectMenu>
        </div>
        <div className="flex w-32 flex-col gap-1.5">
          <label htmlFor={thinkingId} className="text-xs leading-4 text-ink-muted">{t("fusion.thinking")}</label>
          <Select id={thinkingId} aria-label={`${title} · ${t("fusion.thinking")}`} disabled={disabled || !selected} value={value.thinking_level} onChange={event => onChange({ ...value, thinking_level: event.target.value })}>
            {!levels.includes(value.thinking_level) ? <option value={value.thinking_level}>{value.thinking_level}</option> : null}
            {levels.map(level => <option key={level} value={level}>{agentT(`composer.thinkingLevelLabels.${level}`, { defaultValue: level })}</option>)}
          </Select>
        </div>
        {onRemove ? <Button variant="ghost" disabled={disabled} onClick={onRemove} aria-label={t("fusion.removeParticipant", { name: title })}><Trash2 className="size-4" /></Button> : null}
      </div>
    </fieldset>
  );
}

function Form({ snapshot, models, providerNames, save, reload, disabled }: { snapshot: FusionSettings; models: AgentModelOption[]; providerNames: Record<string, string>; save: Source["save"]; reload: () => void; disabled: boolean }) {
  const { t } = useTranslation("settings");
  const [saved, setSaved] = useState(snapshot);
  const [draft, setDraft] = useState<FusionConfiguration>(snapshot.configuration);
  const [rowIds, setRowIds] = useState(() => snapshot.configuration.panel.map(() => crypto.randomUUID()));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const conflict = snapshot.revision > saved.revision;
  const issue = fusionSettingsIssue(draft);
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved.configuration);
  const locked = disabled || busy || conflict;
  async function persist(configuration?: FusionConfiguration, enabled?: boolean) {
    if (locked)
      return;
    setBusy(true);
    setError(null);
    setDone(false);
    try {
      const next = await save({ expected_revision: saved.revision, configuration, enabled });
      setSaved(next);
      if (configuration) {
        setDraft(next.configuration);
        setDone(true);
      }
      reload();
    }
    catch (error) {
      setError(error instanceof Error ? error.message : String(error));
      reload();
    }
    finally { setBusy(false); }
  }
  return (
    <div className="space-y-5">
      <SettingsList>
        <SettingsRow title={t("fusion.enabled")} description={t("fusion.enabledHint")}>
          <Switch label={t("fusion.enabled")} checked={saved.enabled} disabled={locked} onChange={enabled => void persist(undefined, enabled)} />
        </SettingsRow>
      </SettingsList>
      <p className="text-xs leading-5 text-ink-muted">{t("fusion.rolesHint")}</p>
      {conflict
        ? (
            <div role="alert" className="space-y-2 text-xs text-danger">
              <p>{t("fusion.conflict")}</p>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => {
                  setSaved(snapshot);
                  setDraft(snapshot.configuration);
                  setRowIds(snapshot.configuration.panel.map(() => crypto.randomUUID()));
                  setError(null);
                }}
              >
                {t("fusion.reload")}
              </Button>
            </div>
          )
        : null}
      <ModelRow
        title={t("fusion.moderator")}
        value={draft.moderator}
        models={models}
        providerNames={providerNames}
        disabled={locked}
        onChange={(moderator) => {
          setDone(false);
          setDraft({ ...draft, moderator });
        }}
      />
      <SettingsSection title={t("fusion.participants")} description={t("fusion.participantsHint")}>
        <div className="space-y-3">
          {draft.panel.map((model, index) => (
            <ModelRow
              key={rowIds[index]}
              title={t("fusion.participant", { number: index + 1 })}
              value={model}
              models={models}
              providerNames={providerNames}
              disabled={locked}
              onChange={(value) => {
                setDone(false);
                setDraft({ ...draft, panel: draft.panel.map((entry, i) => i === index ? value : entry) });
              }}
              onRemove={draft.panel.length > 2
                ? () => {
                    setDone(false);
                    setDraft({ ...draft, panel: draft.panel.filter((_, i) => i !== index) });
                    setRowIds(rowIds.filter((_, i) => i !== index));
                  }
                : undefined}
            />
          ))}
        </div>
      </SettingsSection>
      <Button
        variant="secondary"
        size="sm"
        leftIcon={<Plus className="size-3.5" />}
        disabled={locked || draft.panel.length >= 5}
        onClick={() => {
          setDone(false);
          setDraft({ ...draft, panel: [...draft.panel, newFusionParticipant()] });
          setRowIds([...rowIds, crypto.randomUUID()]);
        }}
      >
        {t("fusion.addParticipant")}
      </Button>
      {issue ? <p className="text-xs text-danger" role="alert">{t(`fusion.${issue}`)}</p> : null}
      {error ? <p className="text-xs text-danger" role="alert">{error}</p> : null}
      <div className="flex items-center gap-3">
        <Button disabled={locked || !!issue || !dirty} onClick={() => void persist(draft)}>{busy ? t("fusion.saving") : t("fusion.save")}</Button>
        {done ? <span role="status" className="text-xs text-ink-muted">{t("fusion.saved")}</span> : null}
      </div>
    </div>
  );
}

export function FusionSettingsPage({ source = LOCAL, disabled = false }: { source?: Source; disabled?: boolean }) {
  const { t } = useTranslation("settings");
  const load = useCallback(async () => {
    const [settings, models, providers] = await Promise.all([source.load(), source.models(), source.providers().catch(() => null)]);
    const providerNames = Object.fromEntries([...(providers?.builtin ?? []), ...(providers?.custom ?? [])].map(provider => [provider.id, provider.name]));
    return { settings, models, providerNames };
  }, [source]);
  const resource = useAsyncResource(load, [load], null);
  useTauriEvent("app_settings_changed", resource.reload);
  useTauriEvent("provider-config-changed", resource.reload);
  return resource.data
    ? <Form snapshot={resource.data.settings} models={resource.data.models} providerNames={resource.data.providerNames} save={source.save} reload={resource.reload} disabled={disabled} />
    : (
        <div className="space-y-2 text-sm text-ink-muted">
          <p>{resource.error ? t("fusion.loadFailed") : t("fusion.loading")}</p>
          {resource.error ? <Button variant="secondary" onClick={resource.reload}>{t("fusion.reload")}</Button> : null}
        </div>
      );
}

export function RemoteFusionSettingsPage({ peer }: { peer: RemotePeer }) {
  const [revision, setRevision] = useState(0);
  useHostCatalogueInvalidation(peer.desktopId, (domain) => {
    if (domain === "settings" || domain === "models")
      setRevision(value => value + 1);
  });
  const load = useCallback(() => requestRemotePeer<FusionSettings>(peer.desktopId, { type: "get_fusion_settings" }, "list"), [peer.desktopId]);
  const save = useCallback((settings: FusionSettingsPatch) => requestRemotePeer<FusionSettings>(peer.desktopId, { type: "update_fusion_settings", fusionSettings: settings }, "mutation"), [peer.desktopId]);
  const models = useCallback(() => requestRemotePeer<{ models: AgentModelOption[] }>(peer.desktopId, { type: "list_settings_models" }, "list").then(value => value.models), [peer.desktopId]);
  const providers = useCallback(() => listRemoteProviders(peer.desktopId), [peer.desktopId]);
  const source = useMemo(() => ({ load, save, models, providers, revision }), [load, save, models, providers, revision]);
  return <FusionSettingsPage key={`${peer.desktopId}:${peer.bridgeInstanceId}`} source={source} disabled={!peer.connected || peer.control === "standby"} />;
}
