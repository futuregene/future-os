import type { Language } from "../../i18n";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useFutureLoginFlow } from "../../features/settings/useFutureLoginFlow";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { loadAgentModelOptions, localizedModelDescription, modelKey, rememberLastUsedModel, syncFutureModels } from "../../integrations/agent/agentClient";
import { getFutureEnvironment } from "../../integrations/agent/providers";
import { bootstrapBuiltinSkills } from "../../integrations/skills/skillsClient";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { cn } from "../../lib/cn";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";

type EnvironmentId = "production" | "test";

const ENVIRONMENTS: { id: EnvironmentId; labelKey: string }[] = [
  { id: "production", labelKey: "gate.envProduction" },
  { id: "test", labelKey: "gate.envTest" },
];

const INIT_STEPS = ["initAgent", "initModels", "initSkills"] as const;

const MIN_INIT_DURATION_MS = 500;

export interface OnboardingGateProps {
  onEnableBYOK: () => void;
  onInitComplete: () => void;
  onCancelLogin: () => void;
  hasAnyProvider: boolean;
  /** Whether the app's live model catalog is non-empty (from useAgentConnection). */
  modelsReady: boolean;
  /**
   * Whether `useHasProviders` detected a fresh FutureOS key (the gate's parent
   * already knows login just succeeded). Used as a backup trigger for init so
   * the gate never misses the login signal.
   */
  initPending: boolean;
  /** When true, auto-start the login flow on mount (reconnect from Settings). */
  autoLogin?: boolean;
}

/**
 * Onboarding gate shown on first launch when no provider is configured yet. It
 * guides the user toward two paths:
 *
 * 1. Sign in / sign up to FutureOS (primary, prominent button).
 * 2. Bring their own API key (secondary, subdued button) — opens Settings →
 *    Providers to add a custom provider.
 *
 * After a FutureOS login succeeds, the gate stays up and runs initialization:
 * load models, bootstrap built-in skills, and wait for the agent. A progress bar
 * tracks the steps. The gate dismisses only after init completes (min 500ms).
 *
 * While a login flow is in progress (browser open, polling), the BYOK button is
 * replaced by a Cancel button. Cancel stops the login and either closes the gate
 * (if a provider key already exists) or resets to the initial state.
 *
 * The language switcher lives in the top-right corner and is always visible.
 * Dev/test builds additionally show an environment switcher next to it.
 */
export function OnboardingGate({ onEnableBYOK, onInitComplete, onCancelLogin, hasAnyProvider, modelsReady, initPending, autoLogin }: OnboardingGateProps) {
  const { t, i18n } = useTranslation("layout");
  const { phase, message, begin, cancel } = useFutureLoginFlow(() => {});
  const busy = phase === "starting" || phase === "waiting" || phase === "authorized";
  const failed = phase === "denied" || phase === "expired" || phase === "error";
  const build = useBuildInfo();
  const isDev = build.data != null && !build.data.isRelease;

  const env = useAsyncResource(getFutureEnvironment, [], null);
  const [switching, setSwitching] = useState(false);

  // Post-login initialization
  const [initializing, setInitializing] = useState(false);
  const [initStep, setInitStep] = useState(0);
  const [initDone, setInitDone] = useState(false);
  // Whether runInit actually observed a non-empty model list. When true we wait
  // for the app's live catalog (`modelsReady`) to catch up before closing the
  // gate, so the composer never flashes the "no models configured" banner.
  const [modelsConfirmed, setModelsConfirmed] = useState(false);
  // The catalog snapshot captured during init's confirmation loop. Drives the
  // post-init "choose default model" step without a second fetch.
  const [loadedModels, setLoadedModels] = useState<AgentModelOption[]>([]);
  // True while the post-init model picker is shown (>=2 recommended models).
  const [selecting, setSelecting] = useState(false);
  // The card the user has highlighted in the picker (defaults to the first).
  const [selectedModelId, setSelectedModelId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const startRef = useRef(0);
  // Reset in `runInit` so the finalize gate is re-entrant across init cycles.
  const finalizeHandledRef = useRef(false);

  // Recommended models, in catalog order, capped at 3 — the picker's candidates.
  const recommendedModels = useMemo(
    () => loadedModels.filter(model => model.recommended).slice(0, 3),
    [loadedModels],
  );

  const runInit = useCallback(async () => {
    setInitializing(true);
    // Reset any post-init state from a prior cycle so the finalize effect is
    // re-entrant across mount-less re-inits (edge case).
    setSelecting(false);
    setSelectedModelId(null);
    setStarting(false);
    finalizeHandledRef.current = false;
    startRef.current = Date.now();
    const markStep = (index: number) => setInitStep(index);
    const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

    // Step 0 — wait for the agent to be reachable. list_agent_models throws
    // while the sidecar is still starting; an empty (but successful) response
    // means the agent is up but its registry is not yet populated.
    markStep(0);
    {
      const deadline = Date.now() + 15_000;
      while (Date.now() < deadline) {
        try {
          await loadAgentModelOptions();
          break;
        }
        catch {
          await sleep(1200);
        }
      }
    }

    // Step 1 — synchronously pull the Future catalog into the agent (warming
    // its cache + rebuilding its registry), then confirm the model list is
    // non-empty. This is the "models read successfully" guarantee: the gate
    // will not close (when models load) until the composer would have models.
    markStep(1);
    try {
      await syncFutureModels();
    }
    catch {
      // Agent may still be settling; the confirmation loop below retries.
    }
    {
      let confirmed = false;
      const deadline = Date.now() + 12_000;
      while (Date.now() < deadline) {
        try {
          const models = await loadAgentModelOptions();
          if (models.length > 0) {
            confirmed = true;
            setLoadedModels(models);
            break;
          }
        }
        catch {
          // Agent unreachable again — keep waiting.
        }
        await sleep(1000);
      }
      setModelsConfirmed(confirmed);
    }
    // Nudge the app's live model hook to refresh now, so its state matches the
    // warm registry before the gate closes.
    emitFutureEvent("future-models-synced", undefined);

    // Step 2 — install built-in skills (FutureOS login only). Independent of the
    // model catalog; runs last so a slow CLI sidecar never delays model readiness.
    markStep(2);
    try {
      await bootstrapBuiltinSkills();
    }
    catch {
      // Non-fatal.
    }

    // Enforce minimum duration to prevent flash
    const elapsed = Date.now() - startRef.current;
    if (elapsed < MIN_INIT_DURATION_MS) {
      await sleep(MIN_INIT_DURATION_MS - elapsed);
    }

    setInitDone(true);
  }, []);

  // Persist a chosen default model (global settings.json) + seed the composer's
  // last-used pick + nudge the live catalog to refresh, then close the gate.
  // Any failure is swallowed: onboarding must never get stuck on a write error.
  const applyDefaultAndFinish = useCallback(async (model: AgentModelOption | null) => {
    try {
      if (model) {
        try {
          await invokeCommand("set_default_model", { modelId: modelKey(model) });
        }
        catch {
          // Degrade: keep going so the user still enters the app.
        }
        rememberLastUsedModel(modelKey(model));
      }
      // Refresh the live catalog so its `isDefault` reflects the new default and
      // the composer reconciliation picks the chosen model on first render.
      emitFutureEvent("future-models-synced", undefined);
    }
    finally {
      onInitComplete();
    }
  }, [onInitComplete]);

  // User confirmed a pick on the model picker.
  async function handleStart() {
    if (starting)
      return;
    const model = recommendedModels.find(m => modelKey(m) === selectedModelId) ?? recommendedModels[0] ?? null;
    setStarting(true);
    await applyDefaultAndFinish(model);
  }

  // Login succeeded: the state machine moved to the dedicated "authorized"
  // phase.  Also fires when `initPending` is raised — a backup signal from the
  // parent hook when it detects a fresh FutureOS key (covers any edge case where
  // the phase transition is missed, e.g. a rapid unmount/remount).
  const initTriggeredRef = useRef(false);
  useEffect(() => {
    if ((phase === "authorized" || initPending) && !initializing && !initTriggeredRef.current) {
      initTriggeredRef.current = true;
      void runInit();
    }
  }, [phase, initPending, initializing, runInit]);

  // Once init is done (and the live catalog agrees when we confirmed models),
  // either show the model picker (>=2 recommended) or auto-apply a single
  // recommended default and close. With 0/1 recommended there is no choice to
  // make, so the gate closes exactly as before (1 → its model becomes default).
  useEffect(() => {
    if (!(initDone && (modelsConfirmed ? modelsReady : true)))
      return;
    if (finalizeHandledRef.current)
      return;
    finalizeHandledRef.current = true;
    if (recommendedModels.length >= 2) {
      setSelectedModelId(modelKey(recommendedModels[0]!));
      setSelecting(true);
      return;
    }
    void applyDefaultAndFinish(recommendedModels[0] ?? null);
  }, [initDone, modelsConfirmed, modelsReady, recommendedModels, applyDefaultAndFinish]);

  // Auto-start the login flow when the gate is opened from Settings (reconnect).
  const autoLoginFiredRef = useRef(false);
  useEffect(() => {
    if (autoLogin && !autoLoginFiredRef.current) {
      autoLoginFiredRef.current = true;
      void begin();
    }
  }, [autoLogin, begin]);

  function handleCancel() {
    cancel();
    if (hasAnyProvider) {
      // Providers exist — close the gate, go back to the app.
      onEnableBYOK();
    }
    else {
      // No providers — clear the reconnect flag and BYOK bypass so the gate
      // returns to its initial state (login + BYOK buttons).
      onCancelLogin();
    }
  }

  function handleBYOK() {
    cancel();
    onEnableBYOK();
  }

  const activeId = env.data?.environment;
  const envValue: EnvironmentId | "" = activeId === "test" || activeId === "production" ? activeId : "";

  async function handleEnvChange(value: EnvironmentId) {
    if (value === envValue || switching)
      return;
    setSwitching(true);
    try {
      await invokeCommand("set_future_environment", { environment: value });
    }
    catch {
      setSwitching(false);
    }
  }

  const progress = initDone ? INIT_STEPS.length : initStep;
  const progressPct = Math.round((progress / INIT_STEPS.length) * 100);

  return (
    <div className="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-6 bg-canvas px-6 text-center">
      {/* Top-right corner: language (always) + environment (dev only) */}
      <div className="absolute right-4 top-4 flex items-center gap-2">
        <Select
          onChange={e => setLanguage(e.target.value as Language)}
          size="sm"
          value={getLanguage()}
          wrapperClassName="w-28"
        >
          {SUPPORTED_LANGUAGES.map(lang => (
            <option key={lang} value={lang}>{LANGUAGE_LABELS[lang]}</option>
          ))}
        </Select>
        {isDev
          ? (
              <>
                <span className="text-xs text-ink-muted">{t("gate.envLabel")}</span>
                <Select
                  disabled={switching || env.loading}
                  onChange={e => void handleEnvChange(e.target.value as EnvironmentId)}
                  size="sm"
                  value={envValue}
                  wrapperClassName="w-32"
                >
                  {envValue === "" ? <option value="">{env.loading ? "..." : "custom"}</option> : null}
                  {ENVIRONMENTS.map(item => (
                    <option key={item.id} value={item.id}>{t(item.labelKey)}</option>
                  ))}
                </Select>
                {switching ? <Loader2 className="size-3.5 animate-spin text-ink-muted" /> : null}
              </>
            )
          : null}
      </div>

      <div className="space-y-3">
        <h1 className="text-3xl font-semibold tracking-normal text-ink">
          {selecting ? t("gate.chooseModelTitle") : t("gate.title")}
        </h1>
        <p className="mx-auto max-w-md text-sm text-ink-muted">
          {selecting ? t("gate.chooseModelHint") : t("gate.subtitle")}
        </p>
      </div>

      {failed
        ? <p className="max-w-md text-sm text-danger">{message ?? t("settings:futureLogin.failed")}</p>
        : null}

      {selecting
        ? (
            <div className="flex flex-col items-center gap-6">
              <div className="flex flex-wrap items-stretch justify-center gap-4">
                {recommendedModels.map((model) => {
                  const key = modelKey(model);
                  const active = selectedModelId === key;
                  const description = localizedModelDescription(model, i18n.language);
                  return (
                    <button
                      className={cn(
                        "flex h-32 w-56 flex-col items-start justify-between rounded-lg border p-4 text-left transition-colors",
                        active
                          ? "border-accent bg-accent-soft"
                          : "border-line bg-surface hover:border-ink-muted",
                      )}
                      key={key}
                      onClick={() => setSelectedModelId(key)}
                      type="button"
                    >
                      <span className="text-lg font-semibold text-ink">{model.label}</span>
                      {description
                        ? <span className="line-clamp-2 text-xs text-ink-muted">{description}</span>
                        : null}
                    </button>
                  );
                })}
              </div>
              <Button
                className="min-w-40"
                disabled={starting || selectedModelId == null}
                leftIcon={starting ? <Loader2 className="size-4 animate-spin" /> : undefined}
                onClick={() => void handleStart()}
                variant="primary"
              >
                {t("gate.start")}
              </Button>
            </div>
          )
        : initializing
          ? (
              <div className="flex h-[150px] w-64 flex-col items-center justify-center gap-3">
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-subtle">
                  <div
                    className="h-full rounded-full bg-accent transition-all duration-500 ease-out"
                    style={{ width: `${progressPct}%` }}
                  />
                </div>
                <p className="text-xs text-ink-muted">
                  {initDone
                    ? t("gate.initDone")
                    : `${t("gate.xofy", { current: initStep + 1, total: INIT_STEPS.length })} ${t(`gate.${INIT_STEPS[initStep]}`)}`}
                </p>
              </div>
            )
          : (
              <div className="flex h-[150px] flex-col items-center justify-center gap-3">
                <Button
                  className="min-w-40"
                  disabled={busy}
                  leftIcon={busy ? <Loader2 className="size-4 animate-spin" /> : undefined}
                  onClick={() => void begin()}
                  variant="primary"
                >
                  {busy ? t("gate.loggingIn") : failed ? t("gate.retry") : t("gate.login")}
                </Button>
                {/* Hint + divider are always rendered so the layout height stays
                  identical between the idle and busy states (no vertical jitter
                  when the BYOK button swaps to Cancel). `invisible` reserves the
                  space without painting it while a login is in flight. */}
                <p className={busy ? "invisible text-xs text-ink-muted" : "text-xs text-ink-muted"}>{t("gate.freeTrialHint")}</p>
                <div className={busy ? "invisible my-3 h-px w-48 bg-line" : "my-3 h-px w-48 bg-line"} />
                {busy
                  ? (
                      <Button
                        onClick={handleCancel}
                        size="sm"
                        variant="secondary"
                      >
                        {t("gate.cancel")}
                      </Button>
                    )
                  : (
                      <Button
                        onClick={handleBYOK}
                        size="sm"
                        variant="secondary"
                      >
                        {t("gate.byok")}
                      </Button>
                    )}
              </div>
            )}
    </div>
  );
}
