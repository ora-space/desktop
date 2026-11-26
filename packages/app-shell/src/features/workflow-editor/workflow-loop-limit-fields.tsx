import { useTranslation } from "react-i18next";
import type { WorkflowLoopConfig } from "@ora/workflow-mock";
import { Input, Label, Switch } from "@ora/ui";

/** Keeps the safety ceiling and its exhaustion policy together in the Loop inspector. */
export function WorkflowLoopLimitFields<
  T extends Pick<WorkflowLoopConfig, "maxIterations" | "maxIterationsBehavior">,
>({
  config,
  onChange,
  showExhaustionPolicy = true,
}: {
  showExhaustionPolicy?: boolean;
  config: T | undefined;
  onChange: (config: T) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="space-y-3">
      <div className="min-w-0 space-y-1.5">
        <Label htmlFor="workflow-node-max-iterations" className="text-[11px]">
          {t("settings.workflow.field.maxIterations")}
        </Label>
        <Input
          id="workflow-node-max-iterations"
          type="number"
          min={1}
          max={100}
          value={config?.maxIterations ?? 3}
          disabled={config === undefined}
          onChange={(event) => {
            const parsed = Number(event.target.value);
            if (config === undefined || !Number.isFinite(parsed)) return;
            onChange({
              ...config,
              maxIterations: Math.min(100, Math.max(1, Math.trunc(parsed))),
            });
          }}
        />
      </div>
      {showExhaustionPolicy && (
        <>
          <div className="flex items-center justify-between gap-3">
            <Label
              htmlFor="workflow-loop-interrupt-at-limit"
              className="text-[11px]"
            >
              {t("settings.workflow.loop.interruptAtLimit")}
            </Label>
            <Switch
              id="workflow-loop-interrupt-at-limit"
              checked={(config?.maxIterationsBehavior ?? "fail") === "fail"}
              disabled={config === undefined}
              aria-describedby="workflow-loop-limit-hint"
              onCheckedChange={(checked) => {
                if (config === undefined) return;
                onChange({
                  ...config,
                  maxIterationsBehavior: checked ? "fail" : "succeed",
                });
              }}
            />
          </div>
          <p
            id="workflow-loop-limit-hint"
            className="text-[11px] leading-5 text-muted-foreground"
          >
            {t("settings.workflow.loop.limitHint")}
          </p>
        </>
      )}
    </div>
  );
}
