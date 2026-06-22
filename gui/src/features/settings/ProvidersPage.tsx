import type { CustomProvider, ProvidersView } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import {
  deleteCustomProvider,
  listAgentProviders,
  upsertCustomProvider,
} from "../../integrations/agent/providers";
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
              <Badge tone={provider.hasApiKey ? "success" : "neutral"}>
                {provider.hasApiKey ? "已配置密钥" : "未配置密钥"}
              </Badge>
            </SettingsRow>
          ))}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="自定义"
        action={(
          <Button
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
            size="sm"
            variant="secondary"
          >
            + 添加自定义提供商
          </Button>
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
                            <Button onClick={() => void handleDelete(provider.id)} size="sm" variant="danger">
                              移除
                            </Button>
                            <Button onClick={() => setConfirmingDelete(null)} size="sm" variant="secondary">
                              取消
                            </Button>
                          </div>
                        )
                      : (
                          <div className="flex items-center gap-2">
                            <Button
                              onClick={() => {
                                setEditing(provider);
                                setDialogOpen(true);
                              }}
                              size="sm"
                              variant="secondary"
                            >
                              编辑
                            </Button>
                            <Button
                              className="text-ink-soft hover:text-danger"
                              onClick={() => setConfirmingDelete(provider.id)}
                              size="sm"
                              variant="secondary"
                            >
                              移除
                            </Button>
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
