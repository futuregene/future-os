import { useState } from "react";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { SettingsSection } from "./SettingsPrimitives";

type EnvironmentId = "production" | "test";

interface FutureEnvironment {
  /** `production` | `test` | `custom`. */
  environment: string;
  /** Resolved platform root currently in effect (no `/api`). */
  platformUrl: string;
}

// The platform URL for each environment is owned by the Tauri backend
// (`set_future_environment`); the frontend only sends the id.
const ENVIRONMENTS: { id: EnvironmentId; label: string }[] = [
  { id: "production", label: "正式环境" },
  { id: "test", label: "测试环境" },
];

function EnvironmentSection() {
  const current = useAsyncResource<FutureEnvironment | null>(
    () => invokeCommand<FutureEnvironment>("get_future_environment"),
    [],
    null,
  );
  const [selected, setSelected] = useState<EnvironmentId | "">("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const activeId = current.data?.environment;
  // Default the picker to the active environment once it loads.
  const fallback: EnvironmentId | "" = activeId === "test" || activeId === "production" ? activeId : "";
  const value: EnvironmentId | "" = selected || fallback;
  const changed = value !== "" && value !== activeId;

  async function handleSwitch() {
    // `changed` already guarantees a concrete environment id (not "").
    if (!changed) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // Backend pins the new base_url and restarts the app, so this normally
      // never resolves; an error here means the switch failed before restart.
      await invokeCommand("set_future_environment", { environment: value });
    }
    catch (switchError) {
      setError(switchError instanceof Error ? switchError.message : String(switchError));
      setBusy(false);
    }
  }

  return (
    <SettingsSection title="环境">
      <div className="space-y-3 rounded-lg border border-line-soft p-4">
        <div>
          <div className="text-sm font-medium text-ink">切换环境</div>
          <p className="mt-1 text-xs leading-5 text-ink-muted">
            在正式环境与测试环境之间切换登录与接口地址。切换后会清除当前登录并自动重启应用，
            需在新环境重新登录。
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Select
            disabled={busy || current.loading}
            onChange={event => setSelected(event.target.value as EnvironmentId)}
            size="sm"
            value={value}
            wrapperClassName="w-44"
          >
            {value === "" ? <option value="">{current.loading ? "加载中…" : "自定义环境"}</option> : null}
            {ENVIRONMENTS.map(env => (
              <option key={env.id} value={env.id}>{env.label}</option>
            ))}
          </Select>
          <Button disabled={!changed || busy} onClick={() => void handleSwitch()} size="sm" variant="primary">
            {busy ? "切换中…" : "切换并重启"}
          </Button>
        </div>

        <p className="text-xs text-ink-muted">
          当前：
          {current.loading ? "加载中…" : (current.data?.platformUrl ?? "未知")}
        </p>

        {current.error ? <p className="text-xs text-danger">{current.error}</p> : null}
        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </div>
    </SettingsSection>
  );
}

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
      <EnvironmentSection />

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
