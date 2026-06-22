import type { CustomProvider, ProvidersView } from "../../integrations/storage/threadStore";
import { useEffect, useState } from "react";
import {
  deleteCustomProvider,
  listAgentProviders,
  upsertCustomProvider,
} from "../../integrations/storage/threadStore";
import { CustomProviderDialog } from "./CustomProviderDialog";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

export function ProvidersPage() {
  const [providers, setProviders] = useState<ProvidersView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<CustomProvider | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    listAgentProviders()
      .then((view) => {
        if (!cancelled) {
          setProviders(view);
          setError(null);
        }
      })
      .catch((loadError) => {
        if (!cancelled) {
          setError(loadError instanceof Error ? loadError.message : String(loadError));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleDelete(id: string) {
    const view = await deleteCustomProvider(id);
    setProviders(view);
    setConfirmingDelete(null);
  }

  if (loading) {
    return <p className="text-sm text-ink-muted">加载提供商…</p>;
  }

  return (
    <div className="space-y-6">
      {error ? <p className="text-sm text-red-600">{error}</p> : null}

      <SettingsSection title="内置">
        <SettingsList>
          {(providers?.builtin ?? []).map(provider => (
            <SettingsRow
              key={provider.id}
              title={provider.name}
              description={provider.baseUrl}
            >
              <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${provider.hasApiKey ? "bg-emerald-50 text-emerald-700" : "bg-surface-subtle text-ink-muted"}`}>
                {provider.hasApiKey ? "已配置密钥" : "未配置密钥"}
              </span>
            </SettingsRow>
          ))}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="自定义"
        action={(
          <button
            className="h-8 rounded-md border border-line-soft bg-white px-3 text-xs font-medium text-ink transition-colors hover:bg-surface-subtle"
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
            type="button"
          >
            + 添加自定义提供商
          </button>
        )}
      >
        {providers && providers.custom.length > 0
          ? (
              <SettingsList>
                {providers.custom.map(provider => (
                  <SettingsRow
                    key={provider.id}
                    title={provider.name}
                    description={`${provider.baseUrl} · ${provider.models.length} 个模型`}
                  >
                    {confirmingDelete === provider.id
                      ? (
                          <div className="flex items-center gap-2">
                            <span className="text-xs text-ink-muted">确认移除？</span>
                            <button
                              className="h-8 rounded-md bg-red-600 px-3 text-xs font-medium text-white transition-colors hover:bg-red-700"
                              onClick={() => void handleDelete(provider.id)}
                              type="button"
                            >
                              移除
                            </button>
                            <button
                              className="h-8 rounded-md border border-line-soft bg-white px-3 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle"
                              onClick={() => setConfirmingDelete(null)}
                              type="button"
                            >
                              取消
                            </button>
                          </div>
                        )
                      : (
                          <div className="flex items-center gap-2">
                            <button
                              className="h-8 rounded-md border border-line-soft bg-white px-3 text-xs font-medium text-ink transition-colors hover:bg-surface-subtle"
                              onClick={() => {
                                setEditing(provider);
                                setDialogOpen(true);
                              }}
                              type="button"
                            >
                              编辑
                            </button>
                            <button
                              className="h-8 rounded-md border border-line-soft bg-white px-3 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-red-600"
                              onClick={() => setConfirmingDelete(provider.id)}
                              type="button"
                            >
                              移除
                            </button>
                          </div>
                        )}
                  </SettingsRow>
                ))}
              </SettingsList>
            )
          : <p className="text-sm text-ink-muted">还没有自定义提供商。</p>}
      </SettingsSection>

      <CustomProviderDialog
        initial={editing}
        onClose={() => setDialogOpen(false)}
        onSubmit={async (input) => {
          const view = await upsertCustomProvider(input);
          setProviders(view);
        }}
        open={dialogOpen}
      />
    </div>
  );
}
