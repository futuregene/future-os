import type {
  CustomProvider,
  CustomProviderModel,
  ThinkingLevelProbeInput,
  ThinkingLevelProbeOutcome,
} from "../../integrations/agent/providers";
import { Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { Field } from "../../components/ui/Field";
import { IconButton } from "../../components/ui/IconButton";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { knownThinkingLevels, thinkingLevelName } from "../../integrations/agent/agentClient";
import { errorMessage } from "../../lib/errors";

const API_OPTIONS = [
  { label: "OpenAI Completions", value: "openai-completions" },
  { label: "OpenAI Responses", value: "openai-responses" },
  { label: "Anthropic", value: "anthropic" },
];

/**
 * The thinking levels a new model row offers by default. `default` is implicit
 * (always offered, never stored), so it is absent here; the four wire values are
 * the common middle of the vocabulary.
 */
const DEFAULT_THINKING_LEVELS: string[] = ["off", "low", "medium", "high"];

/**
 * Normalize one row's selected levels for the write: trimmed, empties and the
 * reserved `default` dropped, first occurrence of a duplicate kept. The host
 * applies the same rules (and the caps), so this only keeps the payload clean.
 */
function normalizeThinkingLevels(levels: string[] | null | undefined): string[] {
  const out: string[] = [];
  for (const raw of levels ?? []) {
    const level = raw.trim();
    if (!level || level === "default" || out.includes(level))
      continue;
    out.push(level);
  }
  return out;
}

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

function RequiredLabel({ children }: { children: string }) {
  return (
    <>
      <span aria-hidden="true" className="mr-0.5 text-danger">*</span>
      {children}
    </>
  );
}

/**
 * The custom-provider editor. Two things here are not plain form fields: the
 * thinking-level chips are the model's declared wire vocabulary, and (where the
 * caller can reach the provider) a detect run replaces them with the levels the
 * endpoint actually accepts.
 */
export function CustomProviderDialog({
  existing,
  initial,
  onClose,
  onProbe,
  onSubmit,
  open,
}: {
  /** All current providers (built-in + custom), for id/name collision checks. */
  existing: Array<{ id: string; name: string }>;
  initial: CustomProvider | null;
  onClose: () => void;
  /**
   * Probe one thinking level against the provider being edited (nothing is
   * saved first). Absent where the provider is not served by this machine
   * (a remote peer), which hides the detect control.
   */
  onProbe?: (input: ThinkingLevelProbeInput) => Promise<ThinkingLevelProbeOutcome>;
  onSubmit: (input: CustomProviderSubmit) => Promise<void>;
  open: boolean;
}) {
  const { t } = useTranslation("settings");
  const editing = Boolean(initial);
  const [name, setName] = useState("");
  const [id, setId] = useState("");
  const [api, setApi] = useState("openai-completions");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [models, setModels] = useState<ModelRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  /**
   * The row whose thinking levels are being detected, and the level currently
   * on the wire (`""` = the calibration call). One detection at a time keeps
   * the progress text unambiguous and the provider from taking parallel
   * requests.
   */
  const [probing, setProbing] = useState<{ key: string; level: string } | null>(null);
  /** Per-row result of the last detection: selected count, or why it stopped. */
  const [probeNotice, setProbeNotice] = useState<{
    key: string;
    kind: "ok" | "error";
    text: string;
  } | null>(null);
  /**
   * Bumped whenever the dialog is (re)opened or closed. A detection run checks
   * it after every round trip and stops when it has been superseded — closing
   * the dialog must not keep spending model calls behind it.
   */
  const probeRunRef = useRef(0);

  useEffect(() => {
    // Opening, closing or switching provider supersedes a detection run that is
    // still in flight: the run checks this after every round trip and stops, so
    // a closed dialog cannot keep spending model calls.
    probeRunRef.current += 1;
    setProbing(null);
    setProbeNotice(null);
    if (!open) {
      return;
    }
    setName(initial?.name ?? "");
    setId(initial?.id ?? "");
    setApi(initial?.api || "openai-completions");
    setBaseUrl(initial?.baseUrl ?? "");
    setApiKey("");
    setModels(initial?.models.map(model => ({
      ...model,
      // A host that predates the field sends none: the editor then starts from
      // "no declared levels" and writes an explicit list on save. Stored values
      // are copied so unknown wire tokens survive the round trip untouched.
      thinkingLevels: [...(model.thinkingLevels ?? [])],
      key: crypto.randomUUID(),
    })) ?? []);
    setError(null);
    setSaving(false);
  }, [initial, open]);

  function updateModel(index: number, patch: Partial<CustomProviderModel>) {
    const editedKey = models[index]?.key;
    if (editedKey)
      clearProbeNotice(editedKey);
    setModels(current => current.map((model, modelIndex) => (modelIndex === index ? { ...model, ...patch } : model)));
  }

  /**
   * A chip label: the level's **wire** value, untranslated.
   *
   * These chips configure the model's wire vocabulary — the exact strings the
   * agent sends — so they read as `off`, `xhigh`, `max` (see
   * {@link thinkingLevelName}); the point of this row is to match the
   * endpoint's documentation and the saved config, and a translated name would
   * hide the string being configured.
   */
  function levelLabel(level: string) {
    return thinkingLevelName(level);
  }

  /** A result describes one probe's inputs; editing them makes it stale. */
  function clearProbeNotice(key?: string) {
    setProbeNotice(current => (current && (key === undefined || current.key === key) ? null : current));
  }

  /** Add or remove one thinking level on a model row. */
  function toggleThinkingLevel(index: number, level: string) {
    clearProbeNotice(models[index]?.key);
    setModels(current => current.map((model, modelIndex) => {
      if (modelIndex !== index)
        return model;
      const levels = model.thinkingLevels ?? [];
      return {
        ...model,
        thinkingLevels: levels.includes(level)
          ? levels.filter(existing => existing !== level)
          : [...levels, level],
      };
    }));
  }

  /** Replace one row's levels by key, so a probe survives row reordering. */
  function setModelThinkingLevels(key: string, levels: string[]) {
    setModels(current => current.map(model => (model.key === key ? { ...model, thinkingLevels: levels } : model)));
  }

  /**
   * Detect which thinking levels this endpoint accepts.
   *
   * The implicit `default` level goes first and sends no thinking parameter at
   * all: if even that fails, the Base URL / API type / key / model ID is wrong
   * and no level verdict would mean anything, so the levels are left alone and
   * the failure is reported instead. Only then is every known level tried, and
   * the accepted ones replace the row's selection — the point of the feature is
   * to offer exactly the levels that work, not to keep a guess.
   *
   * A level the provider refuses (`ok: false`) is an ordinary answer and simply
   * is not selected. A rejected *call* (agent unreachable, timeout) stops the
   * run and keeps what was already proven.
   */
  async function detectThinkingLevels(model: ModelRow) {
    if (!onProbe)
      return;
    const trimmedBaseUrl = baseUrl.trim();
    const modelId = model.id.trim();
    if (!trimmedBaseUrl || !modelId) {
      setProbeNotice({ key: model.key, kind: "error", text: t("customProvider.probe.needBaseUrl") });
      return;
    }
    const run = (level: string) => onProbe({
      api,
      apiKey: apiKey.trim() || null,
      baseUrl: trimmedBaseUrl,
      level,
      modelId,
      providerId: id.trim() || undefined,
    });
    const runId = ++probeRunRef.current;
    setError(null);
    setProbeNotice(null);
    setProbing({ key: model.key, level: "" });
    let calibration: ThinkingLevelProbeOutcome;
    try {
      calibration = await run("default");
    }
    catch (probeError) {
      if (probeRunRef.current !== runId)
        return;
      setProbing(null);
      setProbeNotice({
        key: model.key,
        kind: "error",
        text: t("customProvider.probe.failed", { error: errorMessage(probeError) }),
      });
      return;
    }
    if (probeRunRef.current !== runId)
      return;
    if (!calibration.ok) {
      setProbing(null);
      setProbeNotice({
        key: model.key,
        kind: "error",
        text: t("customProvider.probe.calibrationFailed", { error: calibration.error ?? "" }),
      });
      return;
    }

    const accepted: string[] = [];
    for (const level of knownThinkingLevels) {
      setProbing({ key: model.key, level });
      try {
        const outcome = await run(level);
        if (probeRunRef.current !== runId)
          return;
        if (outcome.ok)
          accepted.push(level);
      }
      catch (probeError) {
        if (probeRunRef.current !== runId)
          return;
        setProbing(null);
        setModelThinkingLevels(model.key, accepted);
        setProbeNotice({
          key: model.key,
          kind: "error",
          text: t("customProvider.probe.failed", { error: errorMessage(probeError) }),
        });
        return;
      }
    }
    if (probeRunRef.current !== runId)
      return;
    setProbing(null);
    setModelThinkingLevels(model.key, accepted);
    setProbeNotice({
      key: model.key,
      kind: "ok",
      text: accepted.length === 0
        ? t("customProvider.probe.none")
        : t("customProvider.probe.accepted", { count: accepted.length }),
    });
  }

  async function handleSubmit() {
    const trimmedId = id.trim().toLowerCase();
    const trimmedName = name.trim();
    const trimmedBaseUrl = baseUrl.trim();

    // Provider id (only validated when creating; disabled while editing).
    if (!editing) {
      if (!trimmedId) {
        setError(t("customProvider.errors.idRequired"));
        return;
      }
      if (trimmedId.length < PROVIDER_ID_MIN_LEN || trimmedId.length > PROVIDER_ID_MAX_LEN) {
        setError(t("customProvider.errors.idLength", { min: PROVIDER_ID_MIN_LEN, max: PROVIDER_ID_MAX_LEN }));
        return;
      }
      if (!PROVIDER_ID_RE.test(trimmedId)) {
        setError(t("customProvider.errors.idPattern"));
        return;
      }
      if (existing.some(provider => provider.id === trimmedId)) {
        setError(t("customProvider.errors.idExists"));
        return;
      }
    }

    // Base URL.
    if (!trimmedBaseUrl) {
      setError(t("customProvider.errors.baseUrlRequired"));
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
      setError(t("customProvider.errors.baseUrlInvalid"));
      return;
    }

    // Name (optional; falls back to id on the backend).
    if (trimmedName) {
      if (trimmedName.length > PROVIDER_NAME_MAX_LEN) {
        setError(t("customProvider.errors.nameLength", { max: PROVIDER_NAME_MAX_LEN }));
        return;
      }
      if (!PROVIDER_NAME_RE.test(trimmedName)) {
        setError(t("customProvider.errors.namePattern"));
        return;
      }
      const nameTaken = existing.some(
        provider => provider.id !== initial?.id
          && provider.name.trim().toLowerCase() === trimmedName.toLowerCase(),
      );
      if (nameTaken) {
        setError(t("customProvider.errors.nameExists"));
        return;
      }
    }

    // Models.
    const cleanedModels = models
      .map(model => ({
        id: model.id.trim(),
        name: model.name.trim(),
        supportsImages: model.supportsImages,
        // No UI anymore: the flag is round-tripped exactly as it was stored (a
        // new row starts at `true`), the way an old client would have left it.
        reasoning: model.reasoning,
        // Always written explicitly: an empty list is a real answer ("only the
        // implicit default"), and `default` is never stored. Unknown wire values
        // kept from the stored list ride along untouched.
        thinkingLevels: normalizeThinkingLevels(model.thinkingLevels),
        contextWindow: model.contextWindow,
        maxTokens: model.maxTokens,
        // Prices stay as typed; validation below rejects negative / non-finite
        // values before anything is sent (an empty input already reads as 0).
        inputCost: model.inputCost,
        outputCost: model.outputCost,
        cacheReadCost: model.cacheReadCost,
        cacheWriteCost: model.cacheWriteCost,
      }))
      .filter(model => model.id.length > 0);
    const seenModelIds = new Set<string>();
    for (const model of cleanedModels) {
      if (model.id.length > MODEL_ID_MAX_LEN) {
        setError(t("customProvider.errors.modelIdLength", { id: model.id }));
        return;
      }
      if (!MODEL_ID_RE.test(model.id)) {
        setError(t("customProvider.errors.modelIdPattern", { id: model.id }));
        return;
      }
      if (seenModelIds.has(model.id)) {
        setError(t("customProvider.errors.modelIdDuplicate", { id: model.id }));
        return;
      }
      seenModelIds.add(model.id);
      if (model.name.length > MODEL_NAME_MAX_LEN) {
        setError(t("customProvider.errors.modelNameLength", { name: model.name }));
        return;
      }
      if (!Number.isSafeInteger(model.contextWindow) || model.contextWindow <= 0
        || !Number.isSafeInteger(model.maxTokens) || model.maxTokens <= 0) {
        setError(t("customProvider.errors.tokenLimitsPositive", { id: model.id }));
        return;
      }
      if (model.maxTokens > model.contextWindow) {
        setError(t("customProvider.errors.maxTokensExceedContext", { id: model.id }));
        return;
      }
      if ([model.inputCost, model.outputCost, model.cacheReadCost, model.cacheWriteCost]
        .some(price => !Number.isFinite(price) || price < 0)) {
        setError(t("customProvider.errors.priceInvalid", { id: model.id }));
        return;
      }
    }
    if (cleanedModels.length > MAX_MODELS) {
      setError(t("customProvider.errors.modelsMax", { max: MAX_MODELS }));
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
      bodyClassName="-ml-0.5 -mr-5 min-h-0 flex-1 overflow-y-auto pl-0.5 pr-5"
      className="flex max-h-[min(760px,calc(100vh-3rem))] max-w-lg flex-col"
      onClose={onClose}
      open={open}
      title={editing ? t("customProvider.editTitle") : t("customProvider.addTitle")}
      footer={(
        <>
          <Button onClick={onClose} variant="secondary">{t("customProvider.cancel")}</Button>
          <Button disabled={saving || probing !== null} onClick={() => void handleSubmit()} variant="primary">
            {saving ? t("customProvider.saving") : t("customProvider.save")}
          </Button>
        </>
      )}
    >
      <div className="space-y-3">
        <Field label={<RequiredLabel>{t("customProvider.idLabel")}</RequiredLabel>}>
          <TextInput
            disabled={editing}
            onChange={(event) => {
              setProbeNotice(null);
              setId(event.target.value.toLowerCase());
            }}
            placeholder={t("customProvider.idPlaceholder")}
            value={id}
          />
        </Field>
        <Field label={t("customProvider.nameLabel")}>
          <TextInput onChange={event => setName(event.target.value)} placeholder={t("customProvider.namePlaceholder")} value={name} />
        </Field>
        <Field label={<RequiredLabel>{t("customProvider.apiTypeLabel")}</RequiredLabel>}>
          <Select
            onChange={(event) => {
              setProbeNotice(null);
              setApi(event.target.value);
            }}
            value={api}
          >
            {API_OPTIONS.map(option => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </Select>
        </Field>
        <Field label={<RequiredLabel>{t("customProvider.baseUrlLabel")}</RequiredLabel>}>
          <TextInput
            onChange={(event) => {
              setProbeNotice(null);
              setBaseUrl(event.target.value);
            }}
            placeholder={t("customProvider.baseUrlPlaceholder")}
            value={baseUrl}
          />
        </Field>
        <Field label={editing ? t("customProvider.apiKeyLabelEditing") : t("customProvider.apiKeyLabel")}>
          <TextInput
            onChange={(event) => {
              setProbeNotice(null);
              setApiKey(event.target.value);
            }}
            placeholder={t("customProvider.apiKeyPlaceholder")}
            type="password"
            value={apiKey}
          />
        </Field>

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="text-sm font-medium text-ink">{t("customProvider.modelsHeading")}</span>
            <button
              className="text-xs font-medium text-accent transition-colors hover:underline"
              onClick={() => setModels(current => [...current, {
                id: "",
                key: crypto.randomUUID(),
                name: "",
                supportsImages: false,
                reasoning: true,
                thinkingLevels: [...DEFAULT_THINKING_LEVELS],
                contextWindow: 128000,
                maxTokens: 16384,
                inputCost: 0,
                outputCost: 0,
                cacheReadCost: 0,
                cacheWriteCost: 0,
              }])}
              type="button"
            >
              {t("customProvider.addModel")}
            </button>
          </div>
          {models.length === 0
            ? <p className="text-xs text-ink-muted">{t("customProvider.noModels")}</p>
            : (
                <div className="space-y-2">
                  {models.map((model, index) => (
                    <div className="flex items-start gap-2" key={model.key}>
                      <div className="flex-1 space-y-4 rounded-md border border-line-soft p-3">
                        <div className="grid grid-cols-2 gap-3">
                          <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                            <span><RequiredLabel>{t("customProvider.modelIdLabel")}</RequiredLabel></span>
                            <TextInput
                              onChange={event => updateModel(index, { id: event.target.value })}
                              placeholder={t("customProvider.modelIdPlaceholder")}
                              value={model.id}
                            />
                          </label>
                          <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                            <span>{t("customProvider.modelNameLabel")}</span>
                            <TextInput
                              onChange={event => updateModel(index, { name: event.target.value })}
                              placeholder={t("customProvider.modelNamePlaceholder")}
                              value={model.name}
                            />
                          </label>
                          <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                            <span><RequiredLabel>{t("customProvider.contextWindowLabel")}</RequiredLabel></span>
                            <TextInput
                              min="1"
                              step="1"
                              onChange={event => updateModel(index, { contextWindow: Number(event.target.value) })}
                              type="number"
                              value={model.contextWindow}
                            />
                          </label>
                          <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                            <span><RequiredLabel>{t("customProvider.maxTokensLabel")}</RequiredLabel></span>
                            <TextInput
                              min="1"
                              step="1"
                              onChange={event => updateModel(index, { maxTokens: Number(event.target.value) })}
                              type="number"
                              value={model.maxTokens}
                            />
                          </label>
                        </div>
                        {/* Multi-select of the known wire values; `default` is
                            implicit (always offered, never stored), so it is
                            not a chip. Unknown values already stored survive
                            the round trip without a chip of their own. */}
                        <fieldset className="space-y-1.5">
                          <legend className="text-xs text-ink-muted">{t("customProvider.thinkingLevelsLabel")}</legend>
                          {/* Detection: the failing `default` request is what
                              tells a wrong Base URL/key/model id from a level
                              the endpoint merely refuses, so it is reported
                              separately from the level selection. */}
                          {onProbe
                            ? (
                                <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                                  <button
                                    className="text-xs font-medium text-accent transition-colors hover:underline disabled:cursor-not-allowed disabled:text-ink-soft disabled:no-underline"
                                    disabled={probing !== null}
                                    onClick={() => void detectThinkingLevels(model)}
                                    type="button"
                                  >
                                    {probing?.key === model.key
                                      ? t("customProvider.probe.detecting", { level: levelLabel(probing.level) })
                                      : t("customProvider.probe.detect")}
                                  </button>
                                  {probeNotice && probeNotice.key === model.key
                                    ? (
                                        <span className={probeNotice.kind === "error" ? "text-xs text-danger" : "text-xs text-ink-muted"}>
                                          {probeNotice.text}
                                        </span>
                                      )
                                    : null}
                                </div>
                              )
                            : null}
                          <div className="flex flex-wrap items-center gap-1.5">
                            {knownThinkingLevels.map((level) => {
                              const selected = (model.thinkingLevels ?? []).includes(level);
                              return (
                                <button
                                  aria-pressed={selected}
                                  className={selected
                                    ? "rounded-md border border-accent bg-accent/10 px-2 py-1 text-xs font-medium text-accent"
                                    : "rounded-md border border-line-soft px-2 py-1 text-xs font-medium text-ink-muted transition-colors hover:text-ink"}
                                  key={level}
                                  onClick={() => toggleThinkingLevel(index, level)}
                                  type="button"
                                >
                                  {levelLabel(level)}
                                </button>
                              );
                            })}
                          </div>
                        </fieldset>
                        <div className="flex items-center gap-4">
                          <span className="text-xs text-ink-muted">{t("customProvider.modalityLabel")}</span>
                          <label className="flex cursor-not-allowed items-center gap-1.5 text-xs text-ink-soft">
                            <input checked className="size-3.5 accent-accent" disabled type="checkbox" />
                            {t("customProvider.modalityText")}
                          </label>
                          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-ink">
                            <input
                              checked={model.supportsImages}
                              className="size-3.5 accent-accent"
                              onChange={event => updateModel(index, { supportsImages: event.target.checked })}
                              type="checkbox"
                            />
                            {t("customProvider.modalityImage")}
                          </label>
                        </div>
                        {/* Prices are uncontrolled on purpose: re-parsing every
                            keystroke into a number would drop the trailing
                            decimal point ("1." → 1) and make 1.5 untruthable. */}
                        <div className="space-y-2 border-t border-line-soft pt-3">
                          <span className="text-xs text-ink-muted">{t("customProvider.priceHeading")}</span>
                          <div className="grid grid-cols-2 gap-3">
                            <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                              <span>{t("customProvider.priceInput")}</span>
                              <TextInput
                                min="0"
                                defaultValue={model.inputCost === 0 ? "" : model.inputCost}
                                onChange={event => updateModel(index, { inputCost: Number(event.target.value) })}
                                placeholder="0"
                                step="any"
                                type="number"
                              />
                            </label>
                            <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                              <span>{t("customProvider.priceOutput")}</span>
                              <TextInput
                                min="0"
                                defaultValue={model.outputCost === 0 ? "" : model.outputCost}
                                onChange={event => updateModel(index, { outputCost: Number(event.target.value) })}
                                placeholder="0"
                                step="any"
                                type="number"
                              />
                            </label>
                            <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                              <span>{t("customProvider.priceCacheRead")}</span>
                              <TextInput
                                min="0"
                                defaultValue={model.cacheReadCost === 0 ? "" : model.cacheReadCost}
                                onChange={event => updateModel(index, { cacheReadCost: Number(event.target.value) })}
                                placeholder="0"
                                step="any"
                                type="number"
                              />
                            </label>
                            <label className="flex flex-col gap-1.5 text-xs text-ink-muted">
                              <span>{t("customProvider.priceCacheWrite")}</span>
                              <TextInput
                                min="0"
                                defaultValue={model.cacheWriteCost === 0 ? "" : model.cacheWriteCost}
                                onChange={event => updateModel(index, { cacheWriteCost: Number(event.target.value) })}
                                placeholder="0"
                                step="any"
                                type="number"
                              />
                            </label>
                          </div>
                          <p className="text-xs text-ink-soft">{t("customProvider.priceHint")}</p>
                        </div>
                      </div>
                      <IconButton
                        className="shrink-0 hover:text-danger"
                        icon={<Trash2 className="size-4" />}
                        label={t("customProvider.removeModel")}
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
