import { useState } from "react";
import { Button } from "../../components/ui/Button";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { SettingsSection } from "./SettingsPrimitives";

export function ResetPage() {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleClear() {
    setBusy(true);
    setError(null);
    try {
      // Backend clears local data and restarts the app, so this normally never
      // resolves; an error here means the wipe failed before the restart.
      await invokeCommand("clear_app_data");
    }
    catch (clearError) {
      setError(clearError instanceof Error ? clearError.message : String(clearError));
      setBusy(false);
      setConfirming(false);
    }
  }

  return (
    <div className="space-y-6">
      <SettingsSection title="重置">
        <div className="space-y-3 rounded-lg border border-line-soft p-4">
          <div>
            <div className="text-sm font-medium text-ink">清空数据</div>
            <p className="mt-1 text-xs leading-5 text-ink-muted">
              清空 GUI 本地数据库与临时 workspace（对话、后台程序、审查记录等），完成后自动重启应用。
              登录与提供商配置不受影响。此操作不可撤销。
            </p>
          </div>

          {error ? <p className="text-xs text-danger">{error}</p> : null}

          {confirming
            ? (
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs text-ink-muted">确认清空全部本地数据并重启？</span>
                  <Button disabled={busy} onClick={() => void handleClear()} size="sm" variant="danger">
                    {busy ? "清空中…" : "确认清空"}
                  </Button>
                  <Button disabled={busy} onClick={() => setConfirming(false)} size="sm" variant="secondary">
                    取消
                  </Button>
                </div>
              )
            : (
                <Button onClick={() => setConfirming(true)} size="sm" variant="danger">
                  清空数据
                </Button>
              )}
        </div>
      </SettingsSection>
    </div>
  );
}
