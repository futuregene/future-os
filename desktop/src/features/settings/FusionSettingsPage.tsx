import type { FusionConfiguration, FusionModelConfig, FusionSettings, FusionSettingsPatch } from "@future-os/thread-projection";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import { fusionSettingsIssue, newFusionParticipant } from "@future-os/thread-projection";
import { Plus, Trash2 } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { modelKey, modelThinkingLevels } from "../../integrations/agent/agentClient";
import { getFusionSettings, updateFusionSettings } from "../../integrations/fusion/settings";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { useHostCatalogueInvalidation } from "../remote-peer/remoteHostCatalog";
import { requestRemotePeer } from "../remote-peer/remotePeerClient";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

const localModels = () => invokeCommand<AgentModelOption[]>("list_agent_models");
interface Source {
  revision?: number;
  load: () => Promise<FusionSettings>;
  save: (patch: FusionSettingsPatch) => Promise<FusionSettings>;
  models: () => Promise<AgentModelOption[]>;
}
const LOCAL: Source = { load: getFusionSettings, save: updateFusionSettings, models: localModels };

function ModelRow({ title, value, models, disabled, onChange, onRemove }: {
  title: string;
  value: FusionModelConfig;
  models: AgentModelOption[];
  disabled: boolean;
  onChange: (value: FusionModelConfig) => void;
  onRemove?: () => void;
}) {
  const { t } = useTranslation("settings");
  const { t: agentT } = useTranslation("agent");
  const selected = models.find(model => modelKey(model) === value.model);
  const levels = modelThinkingLevels(value.model, models);
  return (
    <fieldset className="space-y-2 rounded-lg border border-line-soft p-3">
      <legend className="px-1 text-sm font-medium text-ink">{title}</legend>
      <div className="flex items-end gap-2">
        <label className="min-w-0 flex-1 space-y-1 text-xs text-ink-muted">
          <span>{t("fusion.model")}</span>
          <Select
            aria-label={`${title} · ${t("fusion.model")}`}
            disabled={disabled}
            value={value.model}
            onChange={(event) => {
              const nextLevels = modelThinkingLevels(event.target.value, models);
              onChange({ ...newFusionParticipant(), model: event.target.value, thinking_level: nextLevels.includes(value.thinking_level) ? value.thinking_level : nextLevels[0] ?? "default" });
            }}
          >
            <option value="">{t("fusion.chooseModel")}</option>
            {value.model && !selected ? <option value={value.model}>{t("fusion.unavailable", { model: value.model })}</option> : null}
            {models.map(model => <option key={modelKey(model)} value={modelKey(model)}>{`${model.label || model.id} · ${model.provider}`}</option>)}
          </Select>
        </label>
        <label className="w-32 space-y-1 text-xs text-ink-muted">
          <span>{t("fusion.thinking")}</span>
          <Select aria-label={`${title} · ${t("fusion.thinking")}`} disabled={disabled || !selected} value={value.thinking_level} onChange={event => onChange({ ...value, thinking_level: event.target.value })}>
            {!levels.includes(value.thinking_level) ? <option value={value.thinking_level}>{value.thinking_level}</option> : null}
            {levels.map(level => <option key={level} value={level}>{agentT(`composer.thinkingLevelLabels.${level}`, { defaultValue: level })}</option>)}
          </Select>
        </label>
        {onRemove ? <Button variant="ghost" disabled={disabled} onClick={onRemove} aria-label={t("fusion.removeParticipant", { name: title })}><Trash2 className="size-4" /></Button> : null}
      </div>
    </fieldset>
  );
}

function Form({ snapshot, models, save, reload, disabled }: { snapshot: FusionSettings; models: AgentModelOption[]; save: Source["save"]; reload: () => void; disabled: boolean }) {
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
    const [settings, models] = await Promise.all([source.load(), source.models()]);
    return { settings, models };
  }, [source]);
  const resource = useAsyncResource(load, [load], null);
  useTauriEvent("app_settings_changed", resource.reload);
  useTauriEvent("provider-config-changed", resource.reload);
  return resource.data
    ? <Form snapshot={resource.data.settings} models={resource.data.models} save={source.save} reload={resource.reload} disabled={disabled} />
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
  const source = useMemo(() => ({ load, save, models, revision }), [load, save, models, revision]);
  return <FusionSettingsPage key={`${peer.desktopId}:${peer.bridgeInstanceId}`} source={source} disabled={!peer.connected || peer.control === "standby"} />;
}
