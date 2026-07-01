import type { CustomProvider, CustomProviderModel } from "../../integrations/agent/providers";
import { Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { Field } from "../../components/ui/Field";
import { IconButton } from "../../components/ui/IconButton";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";

const API_OPTIONS = [
  { label: "OpenAI Completions", value: "openai-completions" },
  { label: "OpenAI Responses", value: "openai-responses" },
  { label: "Anthropic", value: "anthropic" },
];

// Field-validation rules — mirror the backend (agent_providers.rs); backend is
// authoritative, these give instant feedback.
const PROVIDER_ID_RE = /^[a-z0-9_-]+$/;
const PROVIDER_NAME_RE = /^[\w .()-]+$/; // ASCII letters/digits/_ + space.()-
const MODEL_ID_RE = /^[\w.:/-]+$/;
const PROVIDER_ID_MIN_LEN = 2;
const PROVIDER_ID_MAX_LEN = 40;
const PROVIDER_NAME_MAX_LEN = 40;
const MODEL_ID_MAX_LEN = 100;
const MODEL_NAME_MAX_LEN = 60;
const MAX_MODELS = 100;

export interface CustomProviderSubmit {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  apiKey: string | null;
  models: CustomProviderModel[];
  create: boolean;
}

/**
 * An editable model row. `key` is a stable identity for React reconciliation —
 * the model `id` is user-editable and starts empty, so it can't be the key.
 */
interface ModelRow extends CustomProviderModel {
  key: string;
}

export function CustomProviderDialog({
  existing,
  initial,
  onClose,
  onSubmit,
  open,
}: {
  /** All current providers (built-in + custom), for id/name collision checks. */
  existing: Array<{ id: string; name: string }>;
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
  const [models, setModels] = useState<ModelRow[]>([]);
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
    setModels(initial?.models.map(model => ({ ...model, key: crypto.randomUUID() })) ?? []);
    setError(null);
    setSaving(false);
  }, [initial, open]);

  function updateModel(index: number, patch: Partial<CustomProviderModel>) {
    setModels(current => current.map((model, modelIndex) => (modelIndex === index ? { ...model, ...patch } : model)));
  }

  async function handleSubmit() {
    const trimmedId = id.trim().toLowerCase();
    const trimmedName = name.trim();
    const trimmedBaseUrl = baseUrl.trim();

    // Provider id (only validated when creating; disabled while editing).
    if (!editing) {
      if (!trimmedId) {
        setError("请填写提供商 ID。");
        return;
      }
      if (trimmedId.length < PROVIDER_ID_MIN_LEN || trimmedId.length > PROVIDER_ID_MAX_LEN) {
        setError(`提供商 ID 长度需在 ${PROVIDER_ID_MIN_LEN}–${PROVIDER_ID_MAX_LEN} 个字符之间。`);
        return;
      }
      if (!PROVIDER_ID_RE.test(trimmedId)) {
        setError("提供商 ID 只能包含小写字母、数字、'-' 和 '_'。");
        return;
      }
      if (existing.some(provider => provider.id === trimmedId)) {
        setError("提供商 ID 已存在，请换一个。");
        return;
      }
    }

    // Base URL.
    if (!trimmedBaseUrl) {
      setError("请填写 Base URL。");
      return;
    }
    const parsedUrl = (() => {
      try {
        return new URL(trimmedBaseUrl);
      }
      catch {
        return null;
      }
    })();
    if (!parsedUrl || (parsedUrl.protocol !== "http:" && parsedUrl.protocol !== "https:")) {
      setError("Base URL 必须是合法的 http/https 地址。");
      return;
    }

    // Name (optional; falls back to id on the backend).
    if (trimmedName) {
      if (trimmedName.length > PROVIDER_NAME_MAX_LEN) {
        setError(`提供商名称不能超过 ${PROVIDER_NAME_MAX_LEN} 个字符。`);
        return;
      }
      if (!PROVIDER_NAME_RE.test(trimmedName)) {
        setError("提供商名称只能包含字母、数字、空格和 _.()-，不支持中文 / emoji / 全角字符。");
        return;
      }
      const nameTaken = existing.some(
        provider => provider.id !== initial?.id
          && provider.name.trim().toLowerCase() === trimmedName.toLowerCase(),
      );
      if (nameTaken) {
        setError("提供商名称已存在，请换一个。");
        return;
      }
    }

    // Models.
    const cleanedModels = models
      .map(model => ({ id: model.id.trim(), name: model.name.trim() }))
      .filter(model => model.id.length > 0);
    const seenModelIds = new Set<string>();
    for (const model of cleanedModels) {
      if (model.id.length > MODEL_ID_MAX_LEN) {
        setError(`模型 ID「${model.id}」过长。`);
        return;
      }
      if (!MODEL_ID_RE.test(model.id)) {
        setError(`模型 ID「${model.id}」含非法字符。`);
        return;
      }
      if (seenModelIds.has(model.id)) {
        setError(`模型 ID「${model.id}」重复。`);
        return;
      }
      seenModelIds.add(model.id);
      if (model.name.length > MODEL_NAME_MAX_LEN) {
        setError(`模型名称「${model.name}」过长。`);
        return;
      }
    }
    if (cleanedModels.length > MAX_MODELS) {
      setError(`模型数量不能超过 ${MAX_MODELS} 个。`);
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await onSubmit({
        api,
        apiKey: apiKey.trim() ? apiKey.trim() : null,
        baseUrl: trimmedBaseUrl,
        create: !editing,
        id: trimmedId,
        models: cleanedModels,
        name: trimmedName,
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
          <Button onClick={onClose} variant="secondary">取消</Button>
          <Button disabled={saving} onClick={() => void handleSubmit()} variant="primary">
            {saving ? "保存中…" : "保存"}
          </Button>
        </>
      )}
    >
      <div className="space-y-3">
        <Field label="名称">
          <TextInput onChange={event => setName(event.target.value)} placeholder="例如 DashScope" value={name} />
        </Field>
        <Field label="提供商 ID">
          <TextInput
            disabled={editing}
            onChange={event => setId(event.target.value.toLowerCase())}
            placeholder="例如 dashscope-coding"
            value={id}
          />
        </Field>
        <Field label="API 类型">
          <Select onChange={event => setApi(event.target.value)} value={api}>
            {API_OPTIONS.map(option => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </Select>
        </Field>
        <Field label="Base URL">
          <TextInput onChange={event => setBaseUrl(event.target.value)} placeholder="https://api.example.com/v1" value={baseUrl} />
        </Field>
        <Field label={editing ? "API Key（留空保持不变）" : "API Key"}>
          <TextInput
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
              onClick={() => setModels(current => [...current, { id: "", key: crypto.randomUUID(), name: "" }])}
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
                    <div className="flex items-center gap-2" key={model.key}>
                      <TextInput
                        onChange={event => updateModel(index, { id: event.target.value })}
                        placeholder="模型 ID"
                        value={model.id}
                      />
                      <TextInput
                        onChange={event => updateModel(index, { name: event.target.value })}
                        placeholder="显示名称"
                        value={model.name}
                      />
                      <IconButton
                        className="shrink-0 hover:text-danger"
                        icon={<Trash2 className="size-4" />}
                        label="移除模型"
                        onClick={() => setModels(current => current.filter((_, modelIndex) => modelIndex !== index))}
                      />
                    </div>
                  ))}
                </div>
              )}
        </div>

        {error ? <p className="text-sm text-danger">{error}</p> : null}
      </div>
    </Dialog>
  );
}
