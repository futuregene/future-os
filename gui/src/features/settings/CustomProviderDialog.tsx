import type { ReactNode } from "react";
import type { CustomProvider, CustomProviderModel } from "../../integrations/storage/threadStore";
import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Dialog } from "../../components/ui/Dialog";

const API_OPTIONS = [
  { label: "OpenAI Completions", value: "openai-completions" },
  { label: "OpenAI Responses", value: "openai-responses" },
  { label: "Anthropic", value: "anthropic" },
];

const inputClass = "h-9 w-full rounded-md border border-line-soft bg-white px-3 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted focus:border-blue-300 focus:ring-2 focus:ring-blue-100 disabled:bg-surface-subtle disabled:text-ink-muted";

export interface CustomProviderSubmit {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  apiKey: string | null;
  models: CustomProviderModel[];
}

export function CustomProviderDialog({
  initial,
  onClose,
  onSubmit,
  open,
}: {
  initial: CustomProvider | null;
  onClose: () => void;
  onSubmit: (input: CustomProviderSubmit) => Promise<void>;
  open: boolean;
}) {
  const editing = Boolean(initial);
  const [name, setName] = useState("");
  const [id, setId] = useState("");
  const [api, setApi] = useState("openai-completions");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [models, setModels] = useState<CustomProviderModel[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    setName(initial?.name ?? "");
    setId(initial?.id ?? "");
    setApi(initial?.api || "openai-completions");
    setBaseUrl(initial?.baseUrl ?? "");
    setApiKey("");
    setModels(initial?.models.map(model => ({ ...model })) ?? []);
    setError(null);
    setSaving(false);
  }, [initial, open]);

  function updateModel(index: number, patch: Partial<CustomProviderModel>) {
    setModels(current => current.map((model, modelIndex) => (modelIndex === index ? { ...model, ...patch } : model)));
  }

  async function handleSubmit() {
    if (!editing && !id.trim()) {
      setError("请填写提供商 ID。");
      return;
    }
    if (!baseUrl.trim()) {
      setError("请填写 Base URL。");
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await onSubmit({
        api,
        apiKey: apiKey.trim() ? apiKey.trim() : null,
        baseUrl: baseUrl.trim(),
        id: id.trim(),
        models: models
          .map(model => ({ id: model.id.trim(), name: model.name.trim() }))
          .filter(model => model.id.length > 0),
        name: name.trim(),
      });
      onClose();
    }
    catch (submitError) {
      setError(submitError instanceof Error ? submitError.message : String(submitError));
      setSaving(false);
    }
  }

  return (
    <Dialog
      className="max-w-lg"
      onClose={onClose}
      open={open}
      title={editing ? "编辑自定义提供商" : "添加自定义提供商"}
      description="提供商写入 ~/.future/agent/models.json，Agent 可能需要重启后才能加载新模型。"
      footer={(
        <>
          <button
            className="h-9 rounded-md border border-line-soft bg-white px-4 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle"
            onClick={onClose}
            type="button"
          >
            取消
          </button>
          <button
            className="h-9 rounded-md bg-slate-900 px-4 text-sm font-medium text-white transition-colors hover:bg-slate-800 disabled:opacity-50"
            disabled={saving}
            onClick={() => void handleSubmit()}
            type="button"
          >
            {saving ? "保存中…" : "保存"}
          </button>
        </>
      )}
    >
      <div className="space-y-3">
        <Field label="名称">
          <input className={inputClass} onChange={event => setName(event.target.value)} placeholder="例如 DashScope" value={name} />
        </Field>
        <Field label="提供商 ID">
          <input
            className={inputClass}
            disabled={editing}
            onChange={event => setId(event.target.value)}
            placeholder="例如 dashscope-coding"
            value={id}
          />
        </Field>
        <Field label="API 类型">
          <div className="relative">
            <select
              className={`${inputClass} appearance-none pr-8`}
              onChange={event => setApi(event.target.value)}
              value={api}
            >
              {API_OPTIONS.map(option => (
                <option key={option.value} value={option.value}>{option.label}</option>
              ))}
            </select>
          </div>
        </Field>
        <Field label="Base URL">
          <input className={inputClass} onChange={event => setBaseUrl(event.target.value)} placeholder="https://api.example.com/v1" value={baseUrl} />
        </Field>
        <Field label={editing ? "API Key（留空保持不变）" : "API Key"}>
          <input
            className={inputClass}
            onChange={event => setApiKey(event.target.value)}
            placeholder="sk-…"
            type="password"
            value={apiKey}
          />
        </Field>

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-ink">模型</span>
            <button
              className="text-xs font-medium text-accent transition-colors hover:underline"
              onClick={() => setModels(current => [...current, { id: "", name: "" }])}
              type="button"
            >
              + 添加模型
            </button>
          </div>
          {models.length === 0
            ? <p className="text-xs text-ink-muted">尚未添加模型。</p>
            : (
                <div className="space-y-2">
                  {models.map((model, index) => (
                    // eslint-disable-next-line react/no-array-index-key
                    <div className="flex items-center gap-2" key={index}>
                      <input
                        className={inputClass}
                        onChange={event => updateModel(index, { id: event.target.value })}
                        placeholder="模型 ID"
                        value={model.id}
                      />
                      <input
                        className={inputClass}
                        onChange={event => updateModel(index, { name: event.target.value })}
                        placeholder="显示名称"
                        value={model.name}
                      />
                      <button
                        aria-label="移除模型"
                        className="inline-flex size-9 shrink-0 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-red-600"
                        onClick={() => setModels(current => current.filter((_, modelIndex) => modelIndex !== index))}
                        type="button"
                      >
                        <Trash2 className="size-4" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
        </div>

        {error ? <p className="text-sm text-red-600">{error}</p> : null}
      </div>
    </Dialog>
  );
}

function Field({ children, label }: { children: ReactNode; label: string }) {
  return (
    <label className="block space-y-1">
      <span className="text-sm font-medium text-ink">{label}</span>
      {children}
    </label>
  );
}
