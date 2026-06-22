import type { AgentPlanStep } from "./agentThreadTypes";
import { Check, Circle, Loader2 } from "lucide-react";
import { cn } from "../../lib/cn";

interface PlanBlockProps {
  steps: AgentPlanStep[];
}

export function PlanBlock({ steps }: PlanBlockProps) {
  return (
    <div className="mt-4 rounded-lg border border-line-soft bg-surface-subtle p-3">
      <div className="mb-3 text-xs font-semibold uppercase text-ink-muted">Plan</div>
      <div className="space-y-2">
        {steps.map(step => (
          <div key={step.id} className="grid grid-cols-[22px_1fr] gap-2">
            <span
              className={cn(
                "mt-0.5 inline-flex size-5 items-center justify-center rounded-full border",
                step.status === "completed" && "border-green-200 bg-green-50 text-green-700",
                step.status === "active" && "border-blue-200 bg-accent-soft text-accent",
                step.status === "pending" && "border-line-soft bg-surface text-ink-muted",
              )}
            >
              {step.status === "completed"
                ? (
                    <Check className="size-3" />
                  )
                : step.status === "active"
                  ? (
                      <Loader2 className="size-3" />
                    )
                  : (
                      <Circle className="size-2.5" />
                    )}
            </span>
            <div>
              <div className="text-sm font-medium text-ink">{step.title}</div>
              <div className="text-xs leading-5 text-ink-soft">{step.detail}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
