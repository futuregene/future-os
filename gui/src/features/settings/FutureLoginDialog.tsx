import type { FutureLoginStart } from "../../integrations/agent/providers";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { pollFutureLogin, startFutureLogin } from "../../integrations/agent/providers";
import { copyText } from "../../lib/clipboard";
import { usePolling } from "../../lib/usePolling";

type Phase = "starting" | "waiting" | "denied" | "expired" | "error";

const SLOW_DOWN_STEP_MS = 5000;

export function FutureLoginDialog({
  open,
  onClose,
  onAuthorized,
}: {
  open: boolean;
  onClose: () => void;
  /** Called once login succeeds; parent refreshes providers and closes. */
  onAuthorized: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("starting");
  const [start, setStart] = useState<FutureLoginStart | null>(null);
  const [pollIntervalMs, setPollIntervalMs] = useState(5000);
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // Latest login attempt id: a poll response is discarded if a newer attempt
  // (retry) started while it was in flight (usePolling does not cancel in-flight
  // async). Also gates the per-attempt expiry deadline.
  const attemptRef = useRef(0);
  const deadlineRef = useRef(0);

  const begin = useCallback(async () => {
    const attempt = attemptRef.current + 1;
    attemptRef.current = attempt;
    setPhase("starting");
    setMessage(null);
    setStart(null);
    setCopied(false);
    try {
      const next = await startFutureLogin();
      if (attempt !== attemptRef.current)
        return;
      setStart(next);
      setPollIntervalMs(Math.max(1, next.interval) * 1000);
      deadlineRef.current = Date.now() + next.expiresIn * 1000;
      setPhase("waiting");
    }
    catch (error) {
      if (attempt !== attemptRef.current)
        return;
      setMessage(errorText(error));
      setPhase("error");
    }
  }, []);

  useEffect(() => {
    if (open)
      void begin();
    // Bump the attempt id on close so any in-flight poll is ignored.
    else
      attemptRef.current += 1;
  }, [open, begin]);

  usePolling(
    async () => {
      const current = start;
      if (!current)
        return;
      const attempt = attemptRef.current;
      if (Date.now() > deadlineRef.current) {
        setPhase("expired");
        setMessage("授权码已过期，请重试。");
        return;
      }

      let result;
      try {
        result = await pollFutureLogin(current.deviceCode);
      }
      catch (error) {
        if (attempt !== attemptRef.current)
          return;
        setMessage(errorText(error));
        setPhase("error");
        return;
      }
      if (attempt !== attemptRef.current)
        return;

      switch (result.status) {
        case "authorized":
          // Invalidate further polls before handing off to the parent.
          attemptRef.current += 1;
          onAuthorized();
          break;
        case "pending":
          break;
        case "slow_down":
          setPollIntervalMs(ms => ms + SLOW_DOWN_STEP_MS);
          break;
        case "denied":
          setMessage(result.message ?? "授权被拒绝。");
          setPhase("denied");
          break;
        case "expired":
          setMessage(result.message ?? "授权码已过期，请重试。");
          setPhase("expired");
          break;
        default:
          setMessage(result.message ?? "授权失败。");
          setPhase("error");
          break;
      }
    },
    pollIntervalMs,
    { enabled: open && phase === "waiting" && start !== null, deps: [phase, start, pollIntervalMs] },
  );

  async function handleCopyLink() {
    if (!start)
      return;
    await copyText(start.verificationUriComplete);
    setCopied(true);
  }

  return (
    <Dialog
      className="max-w-md"
      onClose={onClose}
      open={open}
      title="连接 FutureGene"
      description="已为你打开浏览器中的授权页面；如果没有自动打开，可复制下面的链接手动访问。"
      footer={(
        <>
          <Button onClick={onClose} variant="secondary">取消</Button>
          {phase === "denied" || phase === "expired" || phase === "error"
            ? <Button onClick={() => void begin()} variant="primary">重试</Button>
            : null}
        </>
      )}
    >
      <div className="space-y-4">
        {phase === "starting" ? <p className="text-sm text-ink-muted">正在获取设备码…</p> : null}

        {phase === "waiting" && start
          ? (
              <>
                <div className="space-y-1">
                  <span className="text-xs font-medium text-ink-muted">验证码</span>
                  <div className="flex items-center gap-3">
                    <code className="select-all rounded-md bg-surface-subtle px-3 py-2 font-mono text-2xl font-semibold tracking-[0.2em] text-ink">
                      {start.userCode}
                    </code>
                    <Button onClick={() => void copyText(start.userCode)} size="sm" variant="secondary">
                      复制验证码
                    </Button>
                  </div>
                </div>
                <div className="space-y-1">
                  <span className="text-xs font-medium text-ink-muted">授权链接</span>
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate rounded-md bg-surface-subtle px-2 py-1.5 text-sm text-ink-soft" title={start.verificationUriComplete}>
                      {start.verificationUriComplete}
                    </span>
                    <Button onClick={() => void handleCopyLink()} size="sm" variant="secondary">
                      {copied ? "已复制" : "复制链接"}
                    </Button>
                  </div>
                </div>
                <p className="text-sm text-ink-muted">在浏览器中确认验证码并授权后，这里会自动完成连接…</p>
              </>
            )
          : null}

        {phase === "denied" || phase === "expired" || phase === "error"
          ? <p className="text-sm text-danger">{message ?? "连接失败。"}</p>
          : null}
      </div>
    </Dialog>
  );
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
