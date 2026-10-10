import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { RemotePeer } from "./remotePeerClient";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "../settings/SettingsPrimitives";
import { PeerIconPicker } from "./PeerIconPicker";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { getRemoteApprovalSettings, requestRemotePeer, setRemoteApprovalTier } from "./remotePeerClient";
import { RemoteSkillsPanel } from "./RemoteSkillsPanel";

/**
 * Settings for *another* machine, mirroring the phone's settings rows.
 *
 * Every value here lives on the host: this screen reads it over the wire and
 * writes it back as a command. That is why it is its own page rather than rows
 * in the app's Settings dialog — mixing the two would make "auto-upgrade
 * skills" ambiguous between this machine and the remote one, and the two have
 * genuinely different answers.
 *
 * Rows the host cannot serve are disabled rather than hidden: a user who paired
 * a machine running an older build should see that the capability exists and is
 * unavailable, not wonder why their settings look different from someone
 * else's.
 */
export function RemotePeerSettings({
  onBack,
  onChanged,
  onUnpair,
  onUpdateLabel,
  peer,
}: {
  onBack: () => void;
  onChanged: () => void;
  onUnpair: () => void;
  onUpdateLabel: (patch: { name?: string; icon?: string }) => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const [name, setName] = useState(peer.name ?? "");
  const [settings, setSettings] = useState<Record<string, unknown> | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [providers, setProviders] = useState<string[]>([]);
  const [tasks, setTasks] = useState<string[]>([]);
  const [approval, setApproval] = useState<{ approvalTier: string; sandboxAvailable: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The host advertises which commands it serves; the approval rows are only
  // offered by a host that says it has them, so an older build shows fewer rows
  // rather than a control that silently does nothing.
  const supportsApprovalTier = peer.features.includes("approval_tier_v1");
  const supportsAutoApproval = peer.features.includes("auto_approval_v1");

  const load = useCallback(async () => {
    setError(null);
    try {
      const values = await Promise.all([
        requestRemotePeer<Record<string, unknown>>(peer.desktopId, { type: "get_desktop_settings" }, "list"),
        readList(peer.desktopId, { type: "list_settings_models" }, "models"),
        readList(peer.desktopId, { type: "list_providers" }, "providers"),
        readList(peer.desktopId, { type: "list_tasks" }, "tasks"),
        supportsApprovalTier
          ? getRemoteApprovalSettings(peer.desktopId)
          : Promise.resolve(null),
      ]);
      setSettings(values[0]);
      setModels(values[1]);
      setProviders(values[2]);
      setTasks(values[3]);
      setApproval(values[4]);
    }
    catch (err) {
      // One machine's settings page can fail as a whole (the host went away);
      // that is a page-level error, not a per-row one.
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [peer.desktopId, supportsApprovalTier]);

  useEffect(() => {
    void load();
  }, [load]);

  async function update(patch: Record<string, unknown>) {
    setBusy(true);
    setError(null);
    try {
      const next = await requestRemotePeer<Record<string, unknown>>(
        peer.desktopId,
        { type: "update_desktop_settings", settings: patch },
        "list",
      );
      setSettings(next);
      onChanged();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  async function setApprovalTier(tier: ApprovalTier) {
    setBusy(true);
    setError(null);
    try {
      // The host answers with the tier it settled on, not the one asked for:
      // `sandbox` on a host whose sandbox is missing comes back as `manual`.
      const settled = await setRemoteApprovalTier(peer.desktopId, tier);
      setApproval(current => current
        ? { ...current, approvalTier: settled }
        : { approvalTier: settled, sandboxAvailable: false });
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  const available = peer.connected;
  const setting = (key: string, fallback: boolean): boolean =>
    typeof settings?.[key] === "boolean" ? settings[key] as boolean : fallback;

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Button onClick={onBack} size="sm" variant="ghost">{t("backToDesktops")}</Button>
        <span aria-hidden className="text-lg">{iconGlyph(peer.icon)}</span>
        <span className="min-w-0 truncate text-sm font-semibold text-ink">
          {peerBadgeText(peer, peer.desktopId)}
        </span>
      </div>

      {error ? <p className="text-xs text-danger">{error}</p> : null}
      {!available ? <p className="text-xs text-ink-muted">{t("settingsOfflineHint")}</p> : null}

      {/* Naming and the icon are *local*: they never reach the host or the
          platform, which is why they sit beside the host's own (remote)
          settings rather than among them — the labels say which is which. */}
      <SettingsSection description={t("nameHint")} title={t("localSection")}>
        <SettingsList>
          <SettingsRow title={t("nameLabel")}>
            <div className="flex items-center gap-2">
              <TextInput
                aria-label={t("nameLabel")}
                onChange={event => setName(event.target.value)}
                placeholder={t("namePlaceholder")}
                value={name}
              />
              <Button
                disabled={name === (peer.name ?? "")}
                onClick={() => onUpdateLabel({ name: name.trim() })}
                size="sm"
              >
                {t("save")}
              </Button>
            </div>
          </SettingsRow>
          <SettingsRow description={t("iconSectionHint")} title={t("icons.label")}>
            <PeerIconPicker onChange={icon => onUpdateLabel({ icon })} value={peer.icon} />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        description={t("settingsAutomationHint")}
        title={t("settingsAutomation")}
      >
        <SettingsList>
          <SettingsRow description={t("settingAutoUpgradeSkillsHint")} title={t("settingAutoUpgradeSkills")}>
            <Switch
              checked={setting("autoUpgradeSkills", false)}
              disabled={!available || busy}
              label={t("settingAutoUpgradeSkills")}
              onChange={value => void update({ autoUpgradeSkills: value })}
            />
          </SettingsRow>
          <SettingsRow description={t("settingAutoTitleHint")} title={t("settingAutoTitle")}>
            <Switch
              checked={setting("autoTitleFirstTurn", false)}
              disabled={!available || busy}
              label={t("settingAutoTitle")}
              onChange={value => void update({ autoTitleFirstTurn: value })}
            />
          </SettingsRow>
          <SettingsRow description={t("settingSkillRecommendHint")} title={t("settingSkillRecommend")}>
            <Switch
              checked={setting("skillRecommend", true)}
              disabled={!available || busy}
              label={t("settingSkillRecommend")}
              onChange={value => void update({ skillRecommend: value })}
            />
          </SettingsRow>
          <SettingsRow description={t("settingAutoConnectHint")} title={t("settingAutoConnect")}>
            <Switch
              checked={setting("autoConnectRemote", false)}
              disabled={!available || busy}
              label={t("settingAutoConnect")}
              onChange={value => void update({ autoConnectRemote: value })}
            />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      {/* The host's approval mode. It sits here rather than in the app's own
          Settings dialog because it is the *other* machine's mode: this machine
          has its own, and the two are genuinely different answers. */}
      {supportsApprovalTier && approval
        ? (
            <SettingsSection description={t("approvalHint")} title={t("approvalSection")}>
              <SettingsList>
                <SettingsRow
                  description={t(`approvalTierDescription.${approval.approvalTier}`, {
                    defaultValue: t("approvalTierDescription.default"),
                  })}
                  title={t("approvalTier")}
                >
                  <Select
                    aria-label={t("approvalTier")}
                    disabled={!available || busy}
                    onChange={event => void setApprovalTier(event.target.value as ApprovalTier)}
                    size="sm"
                    value={approval.approvalTier}
                    wrapperClassName="w-40"
                  >
                    <option value="manual">{t("approvalTierLabel.manual")}</option>
                    <option disabled={!approval.sandboxAvailable} value="sandbox">
                      {t(approval.sandboxAvailable ? "approvalTierLabel.sandbox" : "approvalTierLabel.sandboxUnavailable")}
                    </option>
                    <option disabled={!approval.sandboxAvailable || !supportsAutoApproval} value="auto">
                      {t(supportsAutoApproval ? "approvalTierLabel.auto" : "approvalTierLabel.autoUnsupported")}
                    </option>
                    <option value="off">{t("approvalTierLabel.off")}</option>
                  </Select>
                </SettingsRow>
              </SettingsList>
            </SettingsSection>
          )
        : null}

      {/* Skills are manageable from here rather than summarised: installing one
          is the errand that makes "go to that machine" unreasonable. */}
      <RemoteSkillsPanel available={available} peer={peer} />

      <SettingsSection description={t("settingsHostContentHint")} title={t("settingsHostContent")}>
        <SettingsList>
          <SettingsRow description={countOf(models, t("countModels"))} title={t("models")}>
            <span className="text-xs text-ink-muted">{models.length}</span>
          </SettingsRow>
          <SettingsRow description={t("providersHint")} title={t("providers")}>
            <span className="text-xs text-ink-muted">{providers.length}</span>
          </SettingsRow>
          <SettingsRow description={t("tasksHint")} title={t("tasks")}>
            <span className="text-xs text-ink-muted">{tasks.length}</span>
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <SettingsSection description={t("unpairHint")} title={t("dangerSection")}>
        <SettingsList>
          <SettingsRow title={t("unpair")}>
            <Button onClick={onUnpair} size="sm" variant="danger-soft">{t("unpair")}</Button>
          </SettingsRow>
        </SettingsList>
      </SettingsSection>
    </div>
  );
}

async function readList(
  desktopId: string,
  command: Record<string, unknown>,
  key: string,
): Promise<string[]> {
  const data = await requestRemotePeer<Record<string, unknown>>(desktopId, command, "list");
  const items = data?.[key];
  if (!Array.isArray(items))
    return [];
  return items.flatMap((item) => {
    if (typeof item === "string")
      return [item];
    if (typeof item === "object" && item !== null) {
      const record = item as Record<string, unknown>;
      const name = record.name ?? record.id ?? record.title;
      return typeof name === "string" ? [name] : [];
    }
    return [];
  });
}

function countOf(values: string[], unit: string): string {
  return `${values.length} ${unit}`;
}
