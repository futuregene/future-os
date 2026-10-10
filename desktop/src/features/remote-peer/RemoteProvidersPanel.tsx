import type { BuiltinProvider, CustomProvider, ProvidersView } from "../../integrations/agent/providers";
import type { CustomProviderSubmit } from "../settings/CustomProviderDialog";
import type { RemotePeer } from "./remotePeerClient";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { BuiltinProviderKeyDialog } from "../settings/BuiltinProviderKeyDialog";
import { CustomProviderDialog } from "../settings/CustomProviderDialog";
import { SettingsList, SettingsRow, SettingsSection } from "../settings/SettingsPrimitives";
import {
  deleteRemoteCustomProvider,
  listRemoteProviders,
  updateRemoteBuiltinProvider,
  upsertRemoteCustomProvider,
} from "./remotePeerClient";

/**
 * Another computer's providers and API keys.
 *
 * The two dialogs are this app's own — the shape of a write to a host is the
 * same shape this app writes locally, and the validation is the same backend
 * code either way. What differs is where the write lands, which is the only
 * thing this panel supplies.
 *
 * A key is never read back: the view carries `hasApiKey`, so a key can be set or
 * cleared from here but never displayed. That is the host's contract, not a
 * choice made here.
 */
export function RemoteProvidersPanel({
  available,
  peer,
}: {
  available: boolean;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const { t: tSettings } = useTranslation("settings");
  const [view, setView] = useState<ProvidersView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hint, setHint] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<CustomProvider | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingBuiltin, setEditingBuiltin] = useState<BuiltinProvider | null>(null);
  const [removing, setRemoving] = useState<CustomProvider | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setView(await listRemoteProviders(peer.desktopId));
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [peer.desktopId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function write(run: () => Promise<ProvidersView>): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      setView(await run());
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setBusy(false);
    }
  }

  async function submitCustom(input: CustomProviderSubmit) {
    await write(() => upsertRemoteCustomProvider(peer.desktopId, input));
    setDialogOpen(false);
  }

  /**
   * One built-in write, key and base URL together.
   *
   * `updateApiKey` follows whether the dialog touched the key at all: the host
   * reads an absent key as "leave it", so a save that only changed the base URL
   * must not clear the key by sending it empty.
   */
  async function submitBuiltin(
    provider: BuiltinProvider,
    payload: { apiKey?: string | null; baseUrl?: string },
  ) {
    await write(() => updateRemoteBuiltinProvider(peer.desktopId, {
      apiKey: payload.apiKey,
      baseUrl: payload.baseUrl,
      id: provider.id,
      updateApiKey: payload.apiKey !== undefined,
    }));
    setEditingBuiltin(null);
    setHint(payload.apiKey === null
      ? tSettings("providers.keyCleared", { provider: provider.name })
      : tSettings("providers.keySaved", { provider: provider.name }));
  }

  async function removeCustom(provider: CustomProvider) {
    await write(() => deleteRemoteCustomProvider(peer.desktopId, provider.id));
    setRemoving(null);
  }

  const custom = view?.custom ?? [];
  const builtin = view?.builtin ?? [];

  return (
    <SettingsSection description={t("providersRemoteHint")} title={t("providersRemoteSection")}>
      {error ? <p className="text-xs text-danger">{error}</p> : null}
      {hint ? <p className="text-xs text-ink-muted">{hint}</p> : null}

      <SettingsList>
        {custom.map(provider => (
          <SettingsRow
            description={tSettings("providers.modelsCount", { count: provider.models.length })}
            key={provider.id}
            title={provider.name}
          >
            <div className="flex items-center gap-2">
              <Button
                disabled={!available || busy}
                onClick={() => {
                  setEditing(provider);
                  setDialogOpen(true);
                }}
                size="sm"
                variant="ghost"
              >
                {tSettings("providers.edit")}
              </Button>
              <Button disabled={!available || busy} onClick={() => setRemoving(provider)} size="sm" variant="danger-soft">
                {tSettings("providers.remove")}
              </Button>
            </div>
          </SettingsRow>
        ))}
        <SettingsRow title={custom.length === 0 ? tSettings("providers.noCustom") : tSettings("providers.customTitle")}>
          <Button
            disabled={!available || busy}
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
            size="sm"
          >
            {tSettings("providers.addCustom")}
          </Button>
        </SettingsRow>

        {builtin.map(provider => (
          <SettingsRow
            description={provider.requiresBaseUrl
              ? t("providersBaseUrlNeeded", { count: provider.modelCount })
              : `${tSettings("providers.modelsCount", { count: provider.modelCount })} · ${
                provider.hasApiKey ? tSettings("providers.hasApiKey") : tSettings("providers.noApiKey")}`}
            key={provider.id}
            title={provider.name}
          >
            {/* The account provider is signed in from that computer, and the host
                refuses to edit it — so it is shown, not offered. */}
            {provider.id === "future"
              ? <span className="text-xs text-ink-muted">{t("providersManaged")}</span>
              : (
                  <Button disabled={!available || busy} onClick={() => setEditingBuiltin(provider)} size="sm" variant="ghost">
                    {provider.hasApiKey ? t("providersChangeKey") : tSettings("providers.set")}
                  </Button>
                )}
          </SettingsRow>
        ))}
      </SettingsList>

      <CustomProviderDialog
        existing={[...builtin, ...custom].map(provider => ({ id: provider.id, name: provider.name }))}
        initial={editing}
        onClose={() => setDialogOpen(false)}
        onSubmit={submitCustom}
        open={dialogOpen}
      />

      {/* Rendered only with a provider in hand, so the write is bound to the
          provider the dialog was opened for rather than to whatever is in state
          when it is submitted. */}
      {editingBuiltin
        ? (
            <BuiltinProviderKeyDialog
              description={t("providersKeyOnHost", { name: peer.name || peer.desktopId })}
              onClose={() => setEditingBuiltin(null)}
              onSubmit={payload => submitBuiltin(editingBuiltin, payload)}
              open
              provider={editingBuiltin}
            />
          )
        : null}

      {removing
        ? (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-line-soft bg-surface px-4 py-3">
              <span className="text-sm text-ink">
                {t("providersRemoveConfirm", { name: removing.name })}
              </span>
              <div className="flex items-center gap-2">
                <Button onClick={() => setRemoving(null)} size="sm" variant="ghost">
                  {tSettings("providers.cancel")}
                </Button>
                <Button disabled={busy} onClick={() => void removeCustom(removing)} size="sm" variant="danger-soft">
                  {tSettings("providers.confirmRemove")}
                </Button>
              </div>
            </div>
          )
        : null}
    </SettingsSection>
  );
}
