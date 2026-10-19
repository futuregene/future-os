import type { RemoteStatus } from "./remoteClient";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ConfirmDeleteDialog } from "../../components/layout/EntityDialogs";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { SettingsPage, SettingsRow, SettingsSection } from "../settings/SettingsPrimitives";
import {
  disconnectRemoteDevice,
  takeoverRemote,
  unpairRemoteDevice,
} from "./remoteClient";
import { deviceLabel, deviceState, formatPairId } from "./remoteHostFormat";

/**
 * One paired client's own page.
 *
 * The list on the hub says who is paired and what state they are in; the
 * actions that change that — disconnect, delete, hand control — live here, so
 * the list stays a list. Same shape as the other direction's machine settings
 * page: its own header with a back control, in this window.
 */
export function RemoteHostClientPage({
  onBack,
  onRefreshRemote,
  pairId,
  remoteStatus,
}: {
  onBack: () => void;
  onRefreshRemote: () => Promise<void>;
  pairId: string;
  remoteStatus: RemoteStatus | null;
}) {
  const { t } = useTranslation("remote");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);

  const device = useMemo(
    () => (remoteStatus?.devices ?? []).find(item => item.pairId === pairId) ?? null,
    [remoteStatus?.devices, pairId],
  );

  if (!device) {
    // The client disappeared while its page was open (it was removed from
    // another surface, or the bridge stopped). Say so rather than rendering an
    // empty shell.
    return (
      <SettingsPage
        backLabel={t("back")}
        onBack={onBack}
        title={t("clientDetailTitle")}
      >
        <p className="text-sm text-ink-muted">{t("clientGone")}</p>
      </SettingsPage>
    );
  }

  const state = deviceState(device, t);

  async function run(action: () => Promise<unknown>, tag: string) {
    setBusy(true);
    setError(null);
    try {
      await action();
    }
    catch (err) {
      console.error(`remote ${tag} failed:`, err);
      setError(t("error.generic"));
    }
    finally {
      setBusy(false);
      await onRefreshRemote();
    }
  }

  return (
    <SettingsPage
      backLabel={t("back")}
      onBack={onBack}
      title={deviceLabel(device, t)}
    >
      <SettingsSection title={t("clientDetailTitle")}>
        <div className="rounded-lg border border-line-soft bg-surface px-4">
          <SettingsRow title={t("clientName")}>
            <span className="text-sm text-ink-soft">{deviceLabel(device, t)}</span>
          </SettingsRow>
          <SettingsRow title={t("clientPairId")}>
            <span className="font-mono text-sm text-ink-soft">{formatPairId(device.pairId)}</span>
          </SettingsRow>
          <SettingsRow title={t("clientState")}>
            <Badge tone={state.tone}>{state.label}</Badge>
          </SettingsRow>
        </div>
      </SettingsSection>

      {error
        ? (
            <div className="flex items-center gap-3 rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">
              <span className="min-w-0 flex-1">{error}</span>
            </div>
          )
        : null}

      <SettingsSection title={t("clientActions")}>
        <div className="flex flex-wrap items-center gap-2">
          {/* Disconnect is offered only while this client actually has a live
              bridge; on a stopped device it would have nothing to stop. */}
          {device.connected
            ? (
                <Button
                  disabled={busy}
                  onClick={() => void run(() => disconnectRemoteDevice(device.pairId), "disconnect device")}
                  size="sm"
                  variant="secondary"
                >
                  {t("disconnect")}
                </Button>
              )
            : null}
          {device.control === "standby" && device.connected && device.pairingCode == null
            ? (
                <Button
                  disabled={busy}
                  onClick={() => void run(() => takeoverRemote(device.pairId), "takeover")}
                  size="sm"
                  variant="secondary"
                >
                  {t("takeover")}
                </Button>
              )
            : null}
          <Button
            disabled={busy}
            onClick={() => setConfirmOpen(true)}
            size="sm"
            variant="secondary"
          >
            {t("removeClient")}
          </Button>
        </div>
        <p className="mt-2 text-xs leading-5 text-ink-muted">{t("clientActionsHint")}</p>
      </SettingsSection>

      <ConfirmDeleteDialog
        description={t("removeClientConfirmDesc", { name: deviceLabel(device, t) })}
        error={null}
        onClose={() => setConfirmOpen(false)}
        onConfirm={() => {
          setConfirmOpen(false);
          void run(async () => {
            await unpairRemoteDevice(device.pairId);
            onBack();
          }, "unpair device");
        }}
        open={confirmOpen}
        submitting={false}
        title={t("removeClientConfirmTitle")}
      >
        <p className="text-sm text-ink-soft">{formatPairId(device.pairId)}</p>
      </ConfirmDeleteDialog>
    </SettingsPage>
  );
}
