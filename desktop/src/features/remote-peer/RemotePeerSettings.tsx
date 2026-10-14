import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { RemotePeer } from "./remotePeerClient";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { SettingsLinkRow, SettingsList, SettingsPage, SettingsRow, SettingsSection, Switch } from "../settings/SettingsPrimitives";
import { PeerIconPicker } from "./PeerIconPicker";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { RemoteModelsPanel } from "./RemoteModelsPanel";
import { getRemoteApprovalSettings, requestRemotePeer, setRemoteApprovalTier } from "./remotePeerClient";
import { RemoteProvidersPanel } from "./RemoteProvidersPanel";
import { RemoteSkillsPanel } from "./RemoteSkillsPanel";
import { RemoteWorkspacesPanel } from "./RemoteWorkspacesPanel";

/**
 * The sections too long to sit in the settings column.
 *
 * Each is a list a machine's own settings page shows in full — its skills, its
 * models, its providers, its workspaces — and stacked here they pushed every
 * later setting below a scroll. They are pages of their own instead, reached
 * from a row that says what is behind it.
 */
type SettingsSectionPage = "root" | "skills" | "models" | "providers" | "workspaces";

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
  onOpenTasks,
  onUpdateLabel,
  peer,
}: {
  onBack: () => void;
  onChanged: () => void;
  /** Open that host's tasks: the editor needs the whole column. */
  onOpenTasks: () => void;
  onUpdateLabel: (patch: { name?: string; icon?: string }) => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const [name, setName] = useState(peer.name ?? "");
  const [settings, setSettings] = useState<Record<string, unknown> | null>(null);
  const [approval, setApproval] = useState<{ approvalTier: string; sandboxAvailable: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Which page of these settings is open. See `SettingsSectionPage`. */
  const [page, setPage] = useState<SettingsSectionPage>("root");

  // The host advertises which commands it serves; the approval rows are only
  // offered by a host that says it has them, so an older build shows fewer rows
  // rather than a control that silently does nothing.
  const supportsApprovalTier = peer.features.includes("approval_tier_v1");
  const supportsAutoApproval = peer.features.includes("auto_approval_v1");
  const providersSupported = peer.features.includes("provider_management_v1");

  const load = useCallback(async () => {
    setError(null);
    try {
      // The models are *not* read here any more: their page does that, from
      // `list_models` (which carries each model's provider, so the page can group
      // and name them) rather than the bare id list this used to count.
      const values = await Promise.all([
        requestRemotePeer<Record<string, unknown>>(peer.desktopId, { type: "get_desktop_settings" }, "list"),
        supportsApprovalTier
          ? getRemoteApprovalSettings(peer.desktopId)
          : Promise.resolve(null),
      ]);
      setSettings(values[0]);
      setApproval(values[1]);
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
  // The machine, as the user named it: part of each row's accessible name,
  // because "Models" here means *that* machine's models and the word alone does
  // not say so.
  const machine = peerBadgeText(peer, peer.desktopId);
  const hiddenModels = Array.isArray(settings?.hiddenModels)
    ? (settings.hiddenModels as unknown[]).filter((item): item is string => typeof item === "string")
    : [];

  // The sections that became pages of their own. Rendered from one description
  // rather than four copies of the same header, so a page cannot end up with a
  // different back control or heading from its neighbours.
  const SUB_PAGES: Record<Exclude<SettingsSectionPage, "root">, {
    content: React.ReactNode;
    title: string;
  }> = {
    models: {
      title: t("models"),
      content: (
        <RemoteModelsPanel
          available={available}
          hidden={hiddenModels}
          onChangeHidden={next => void update({ hiddenModels: next })}
          peer={peer}
        />
      ),
    },
    skills: { title: t("skills"), content: <RemoteSkillsPanel available={available} peer={peer} /> },
    providers: {
      title: t("providersRemoteSection"),
      content: <RemoteProvidersPanel available={available} peer={peer} />,
    },
    workspaces: {
      title: t("workspacesTitle"),
      content: <RemoteWorkspacesPanel available={available} onChanged={onChanged} peer={peer} />,
    },
  };

  if (page !== "root") {
    return (
      <SettingsPage
        backLabel={t("backToSettings")}
        onBack={() => setPage("root")}
        title={SUB_PAGES[page].title}
      >
        {error ? <p className="text-xs text-danger">{error}</p> : null}
        {!available ? <p className="text-xs text-ink-muted">{t("settingsOfflineHint")}</p> : null}
        {SUB_PAGES[page].content}
      </SettingsPage>
    );
  }

  return (
    <SettingsPage
      backLabel={t("backToDesktops")}
      icon={<span aria-hidden className="text-base">{iconGlyph(peer.icon)}</span>}
      onBack={onBack}
      title={peerBadgeText(peer, peer.desktopId)}
    >
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
          {/* The glyphs are a grid, not a switch: two dozen of them want more
              width than this column has, and beside the label they took it all
              (the label drawn one character per line, the grid out of the card).
              So this one row puts its control below the label. */}
          <SettingsRow description={t("iconSectionHint")} stacked title={t("icons.label")}>
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

      {/* The long sections, as links. Each is a list a machine's own settings
          page shows in full; rendered inline they pushed everything below them
          off the screen. The rows say what is behind them, and stay visible
          but disabled when the host cannot serve them: a user who paired an
          older build should see that the capability exists. */}
      <SettingsSection description={t("settingsHostContentHint")} title={t("settingsHostContent")}>
        <SettingsList>
          <SettingsLinkRow
            ariaLabel={`${t("models")} · ${machine}`}
            description={t("modelsHint")}
            disabled={!available}
            onClick={() => setPage("models")}
            title={t("models")}
          />
          <SettingsLinkRow
            ariaLabel={`${t("skills")} · ${machine}`}
            description={t("skillsHint")}
            disabled={!available}
            onClick={() => setPage("skills")}
            title={t("skills")}
          />
          {/* Only offered by a host that advertises it, like the panel itself:
              the command would be refused otherwise. */}
          {providersSupported
            ? (
                <SettingsLinkRow
                  ariaLabel={`${t("providersRemoteSection")} · ${machine}`}
                  description={t("providersRemoteHint")}
                  disabled={!available}
                  onClick={() => setPage("providers")}
                  title={t("providersRemoteSection")}
                />
              )
            : null}
          {peer.features.includes("workspace_create_v1")
            ? (
                <SettingsLinkRow
                  ariaLabel={`${t("workspacesTitle")} · ${machine}`}
                  description={t("workspacesHint")}
                  disabled={!available}
                  onClick={() => setPage("workspaces")}
                  title={t("workspacesTitle")}
                />
              )
            : null}
          {/* Tasks keep their own editor, which needs the whole column — so this
              row hands off to the caller, which shows it as a page like this
              one rather than inside it. */}
          <SettingsLinkRow
            ariaLabel={`${t("tasks")} · ${machine}`}
            description={t("tasksHint")}
            disabled={!available}
            onClick={onOpenTasks}
            title={t("tasks")}
          />
        </SettingsList>
      </SettingsSection>
    </SettingsPage>
  );
}
