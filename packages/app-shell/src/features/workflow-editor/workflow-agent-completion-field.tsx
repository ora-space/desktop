import { useTranslation } from "react-i18next";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@ora/ui";
import type { WorkflowAgentConfig } from "@ora/workflow-mock";
import { InspectorField } from "./workflow-node-details";

/** Chooses how an interactive node decides its settled turn is complete. */
export function WorkflowAgentCompletionField({
  config,
  onChange,
}: {
  config: WorkflowAgentConfig;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const value = config.completionSignal === "agent" ? "agent" : "end_turn";

  return (
    <>
      <InspectorField
        label={t("settings.workflow.field.completionSignal")}
        htmlFor="workflow-agent-completion-signal"
      >
        <Select
          value={value}
          onValueChange={(completionSignal) => {
            if (completionSignal === "agent") {
              onChange({
                ...config,
                // Writing the policy requires the newer schema version.
                schemaVersion: 4,
                completionSignal: "agent",
              });
            } else if (completionSignal === "end_turn") {
              onChange({
                ...config,
                schemaVersion: 4,
                completionSignal: "end_turn",
                autoAcceptAgentCompletion: false,
              });
            }
          }}
        >
          <SelectTrigger
            id="workflow-agent-completion-signal"
            className="w-full"
            aria-describedby="workflow-agent-completion-signal-hint"
          >
            <SelectValue>
              {t(`settings.workflow.completionSignal.${value}`)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="end_turn">
              {t("settings.workflow.completionSignal.end_turn")}
            </SelectItem>
            <SelectItem value="agent">
              {t("settings.workflow.completionSignal.agent")}
            </SelectItem>
          </SelectContent>
        </Select>
        <p
          id="workflow-agent-completion-signal-hint"
          className="text-[10px] leading-relaxed text-muted-foreground"
        >
          {t("settings.workflow.completionSignal.hint")}
        </p>
      </InspectorField>
      {value === "agent" && (
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 space-y-0.5">
            <p className="text-[11px] font-medium">
              {t("settings.workflow.field.autoAcceptAgentCompletion")}
            </p>
            <p className="text-[10px] leading-relaxed text-muted-foreground">
              {t("settings.workflow.autoAcceptAgentCompletion.hint")}
            </p>
          </div>
          <Switch
            id="workflow-agent-auto-accept-completion"
            className="shrink-0"
            checked={config.autoAcceptAgentCompletion ?? false}
            onCheckedChange={(autoAcceptAgentCompletion) =>
              onChange({ ...config, autoAcceptAgentCompletion })
            }
          />
        </div>
      )}
    </>
  );
}
