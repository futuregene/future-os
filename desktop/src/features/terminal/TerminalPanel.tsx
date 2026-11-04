import type { ErrorInfo, ReactNode } from "react";
/**
 * The terminal panel: tab strip, mounted view, and the notices a terminal needs
 * (exited shell, missing session, failed working directory, reconnecting).
 *
 * Only the active tab is mounted, exactly like opencode: switching tabs or
 * collapsing the panel serializes the screen into the tab store and closes the
 * socket, while the shell keeps running on the Rust side.
 */
import type { TerminalApi } from "./api";
import type { TerminalTransport } from "./transport";
import type { TerminalPanelController } from "./useTerminalPanel";
import type { TerminalTabsController } from "./useTerminalTabs";
import { Loader2, Plus, RotateCcw, X } from "lucide-react";
import { Component, lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { cn } from "../../lib/cn";
import { remoteTerminalApi, remoteTerminalScope } from "./api";
import { localTransport } from "./localTransport";
import { terminalPanelLabel } from "./panelLabel";
import { remoteTerminalTransport } from "./remoteTerminal";
import { defaultTitle, useTerminalTabs } from "./useTerminalTabs";

/** xterm is heavy; the panel is the only consumer, so it loads on first open. */
const TerminalView = lazy(() => import("./TerminalView").then(module => ({ default: module.TerminalView })));

export interface TerminalPanelProps {
  threadId: string;
  panel: TerminalPanelController;
  /** Which machine owns the shells. Defaults to this one. */
  transport?: TerminalTransport;
  /** The host's name, shown while the shell is not on this machine. */
  hostLabel?: string;
}

/** This machine's terminal panel: the app's own shells over loopback. */
export function TerminalPanel({ threadId, panel, transport, hostLabel }: TerminalPanelProps) {
  const tabs = useTerminalTabs(threadId);
  return (
    <TerminalPanelShell
      hostLabel={hostLabel}
      panel={panel}
      tabs={tabs}
      transport={transport ?? localTransport}
    />
  );
}

export interface RemoteTerminalPanelProps {
  desktopId: string;
  sessionId: string;
  panel: TerminalPanelController;
  /** The host's name, so a tab strip never reads as this machine's. */
  hostLabel: string;
}

/**
 * A paired desktop's terminal panel.
 *
 * Same panel, same tab controller, different owner: the shells run on that
 * machine, in that conversation's directory, and closing the conversation here
 * only detaches this view. The tab state is namespaced per host and session so
 * switching machines cannot adopt another conversation's tabs.
 */
export function RemoteTerminalPanel({ desktopId, sessionId, panel, hostLabel }: RemoteTerminalPanelProps) {
  const api = useMemo<TerminalApi>(() => remoteTerminalApi(desktopId, sessionId), [desktopId, sessionId]);
  const transport = useMemo(
    () => remoteTerminalTransport(desktopId),
    [desktopId],
  );
  const tabs = useTerminalTabs(remoteTerminalScope(desktopId, sessionId), api);
  return <TerminalPanelShell hostLabel={hostLabel} panel={panel} tabs={tabs} transport={transport} />;
}

interface TerminalPanelShellProps {
  panel: TerminalPanelController;
  tabs: TerminalTabsController;
  transport: TerminalTransport;
  hostLabel?: string;
}

function TerminalPanelShell({ panel, tabs, transport, hostLabel }: TerminalPanelShellProps) {
  const { t } = useTranslation("terminal");
  // Transient transport notice per tab, cleared when it reconnects.
  const [notices, setNotices] = useState<Record<string, string | undefined>>({});
  const autoCreatedRef = useRef(false);

  const activeId = tabs.activeId;
  const activeTab = tabs.activeTab;
  const exitCode = activeTab?.exitCode;
  const showExitBar = Boolean(activeTab) && !activeTab?.missing && exitCode !== undefined && exitCode !== null;

  // Opening the panel with no tabs starts one — the only case where the UI
  // creates a shell without the user pressing "+".
  useEffect(() => {
    if (!panel.open) {
      autoCreatedRef.current = false;
      return;
    }
    if (!tabs.ready || tabs.creating || tabs.tabs.length > 0 || tabs.createError)
      return;
    if (autoCreatedRef.current)
      return;
    autoCreatedRef.current = true;
    void tabs.create();
  }, [panel, tabs]);

  const startResize = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const startY = event.clientY;
    const startHeight = panel.height;
    const onMove = (move: PointerEvent) => {
      panel.setHeight(startHeight + (startY - move.clientY));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  // Collapsing hides the panel; the controller also hands the caret back to the
  // composer (see `useTerminalPanel`).
  const collapse = () => panel.setOpen(false);

  return (
    <section
      aria-label={t("title")}
      className="relative flex shrink-0 flex-col border-t border-line-soft bg-surface-panel"
      data-component="terminal-panel"
      style={{ height: panel.height }}
    >
      <div
        aria-label={t("resize")}
        aria-orientation="horizontal"
        className="absolute -top-1 left-0 z-20 h-2 w-full cursor-row-resize"
        onPointerDown={startResize}
        role="separator"
      />
      <header className="flex h-9 shrink-0 items-center gap-1 border-b border-line-soft px-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto" role="tablist">
          {tabs.tabs.map(tab => (
            <div
              className={cn(
                "group flex h-7 shrink-0 items-center gap-1 rounded-md border border-transparent pl-2 pr-1 text-xs",
                tab.id === activeId
                  ? "border-line bg-surface text-ink"
                  : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
              )}
              key={tab.id}
            >
              <button
                aria-selected={tab.id === activeId}
                className="max-w-40 truncate"
                onClick={() => tabs.setActive(tab.id)}
                role="tab"
                title={tab.title}
                type="button"
              >
                {tab.title || defaultTitle(tab.titleNumber || 1)}
                {tab.missing ? ` · ${t("endedShort")}` : ""}
              </button>
              <button
                aria-label={t("closeTab", { title: tab.title || defaultTitle(tab.titleNumber || 1) })}
                className="rounded p-0.5 text-ink-muted opacity-0 transition-opacity hover:bg-surface-subtle hover:text-ink group-hover:opacity-100 focus-visible:opacity-100"
                onClick={() => void tabs.close(tab.id)}
                title={t("closeTab", { title: tab.title || defaultTitle(tab.titleNumber || 1) })}
                type="button"
              >
                <X className="size-3" />
              </button>
            </div>
          ))}
          <IconButton
            className="size-7 shrink-0"
            disabled={tabs.creating || Boolean(tabs.error)}
            icon={tabs.creating ? <Loader2 className="size-3.5 animate-spin" /> : <Plus className="size-3.5" />}
            label={t("newTab")}
            onClick={() => void tabs.create()}
          />
        </div>
        {hostLabel
          ? (
              <span
                className="shrink-0 rounded border border-line px-1.5 py-0.5 text-2xs text-ink-muted"
                title={t("onHost", { host: hostLabel })}
              >
                {t("onHost", { host: hostLabel })}
              </span>
            )
          : null}
        <IconButton
          className="size-7 shrink-0"
          icon={<X className="size-3.5" />}
          label={terminalPanelLabel(t, true, panel.shortcut)}
          onClick={collapse}
        />
      </header>

      <div className="flex min-h-0 flex-1 flex-col">
        {tabs.createError
          ? <ErrorBar tabs={tabs} />
          : null}
        {showExitBar
          ? (
              <div className="flex shrink-0 items-center justify-between gap-2 border-b border-line-soft bg-surface px-3 py-1 text-xs text-ink-soft">
                <span>{t("processExited", { code: exitCode })}</span>
                <button
                  className="inline-flex items-center gap-1 rounded-md border border-line px-2 py-0.5 text-xs text-ink-soft hover:bg-surface-subtle"
                  onClick={() => activeTab && void tabs.restart(activeTab.id)}
                  type="button"
                >
                  <RotateCcw className="size-3" />
                  {t("restart")}
                </button>
              </div>
            )
          : null}

        <div className="relative min-h-0 flex-1">
          {tabs.error
            ? <Notice tone="danger" title={t("serverUnavailable")} description={tabs.error} />
            : null}
          {!tabs.error && activeTab?.missing
            ? (
                <Notice
                  tone="warning"
                  title={t("sessionEnded")}
                  description={t("sessionEndedHint")}
                  actions={(
                    <button
                      className="rounded-md border border-warning-line bg-warning-soft px-2 py-1 text-xs font-medium text-warning"
                      onClick={() => void tabs.restart(activeTab.id)}
                      type="button"
                    >
                      {t("restart")}
                    </button>
                  )}
                />
              )
            : null}
          {!tabs.error && !activeTab
            ? (
                <Notice
                  tone={tabs.createError ? "warning" : "neutral"}
                  title={tabs.createError ? t("createFailed") : t("emptyTitle")}
                  description={tabs.createError ? undefined : t("emptyHint")}
                  actions={tabs.createError
                    ? undefined
                    : (
                        <button
                          className="rounded-md border border-line px-2 py-1 text-xs text-ink-soft hover:bg-surface-subtle"
                          onClick={() => void tabs.create()}
                          type="button"
                        >
                          {t("newTab")}
                        </button>
                      )}
                />
              )
            : null}

          {notices[activeId ?? ""]
            ? (
                <div className="absolute bottom-1 left-2 z-10 rounded bg-surface/90 px-2 py-0.5 text-2xs text-ink-muted">
                  {t("reconnecting")}
                </div>
              )
            : null}

          {activeTab
            ? (
                <div className="absolute inset-0">
                  <Suspense fallback={<div className="p-3 text-xs text-ink-muted">{t("loading")}</div>}>
                    <TerminalBoundary>
                      <TerminalView
                        autoFocus
                        key={activeTab.id}
                        transport={transport}
                        onConnect={() => setNotice(setNotices, activeTab.id, undefined)}
                        onDisconnected={() => setNotice(setNotices, activeTab.id, "disconnected")}
                        onExit={code => tabs.markExit(activeTab.id, code)}
                        onMissing={() => tabs.markMissing(activeTab.id)}
                        onPersist={patch => tabs.save(activeTab.id, patch)}
                        tab={activeTab}
                      />
                    </TerminalBoundary>
                  </Suspense>
                </div>
              )
            : null}
        </div>
      </div>
    </section>
  );
}

/** Create failures render as a bar so an existing tab stays visible. */
function ErrorBar({ tabs }: { tabs: TerminalTabsController }) {
  const { t } = useTranslation("terminal");
  const error = tabs.createError;
  if (!error)
    return null;
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-warning-line bg-warning-soft px-3 py-1 text-xs text-warning">
      <span className="font-medium">{t("createFailed")}</span>
      <span className="min-w-0 flex-1 truncate text-ink-soft" title={error.message}>{error.message}</span>
      <button
        className="rounded-md border border-line bg-surface px-2 py-0.5 text-xs font-medium text-ink-soft hover:bg-surface-subtle"
        onClick={() => void tabs.retry()}
        type="button"
      >
        {t("retry")}
      </button>
      <button
        className="rounded-md px-2 py-0.5 text-xs text-ink-muted hover:text-ink"
        onClick={tabs.dismissCreateError}
        type="button"
      >
        {t("dismiss")}
      </button>
    </div>
  );
}

/** A chunk-load or render failure inside the terminal must not blank the app. */
class TerminalBoundary extends Component<{ children: ReactNode }, { message: string | null }> {
  override state: { message: string | null } = { message: null };

  static getDerivedStateFromError(error: unknown): { message: string | null } {
    return { message: error instanceof Error ? error.message : String(error) };
  }

  override componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("terminal view failed", error, info.componentStack);
  }

  override render(): ReactNode {
    if (this.state.message !== null)
      return <div className="p-3 text-xs text-danger">{this.state.message}</div>;
    return this.props.children;
  }
}

function Notice(props: {
  tone: "neutral" | "warning" | "danger" | "info";
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  const tones = {
    neutral: "text-ink-soft",
    info: "text-info",
    warning: "text-warning",
    danger: "text-danger",
  } as const;
  return (
    <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 px-6 text-center">
      <div className={cn("text-sm font-medium", tones[props.tone])}>{props.title}</div>
      {props.description ? <div className="max-w-xl text-xs text-ink-muted">{props.description}</div> : null}
      {props.actions ? <div className="flex items-center gap-2">{props.actions}</div> : null}
    </div>
  );
}

type NoticeSetter = React.Dispatch<React.SetStateAction<Record<string, string | undefined>>>;

function setNotice(setter: NoticeSetter, id: string, value: string | undefined): void {
  setter(previous => (previous[id] === value ? previous : { ...previous, [id]: value }));
}
