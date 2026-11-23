import { useTranslation } from "react-i18next";
import {
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@ora/ui";
import type { WorkflowAgentConfig } from "@ora/workflow-mock";
import { InspectorField } from "./workflow-node-details";

/** Edits silence handling independently of node failure retries, including interactive nodes. */
export function WorkflowAgentInactivityField({
  config,
  onChange,
}: {
  config: WorkflowAgentConfig;
  onChange: (config: WorkflowAgentConfig) => void;
}) {
  const { t } = useTranslation();
  const policy = config.promptInactivity ?? "timeout";
  const value = typeof policy === "object" ? "wait" : policy;
  const minutes = typeof policy === "object" ? policy.waitMinutes : 30;

  return (
    <InspectorField
      label={t("settings.workflow.field.promptInactivity")}
      htmlFor="workflow-agent-prompt-inactivity"
    >
      <Select
        value={value}
        onValueChange={(promptInactivity) => {
          if (
            promptInactivity === "timeout" ||
            promptInactivity === "wait" ||
            promptInactivity === "unbounded"
          ) {
            onChange({ ...config, promptInactivity });
          }
        }}
      >
        <SelectTrigger
          id="workflow-agent-prompt-inactivity"
          className="w-full"
          aria-describedby="workflow-agent-prompt-inactivity-hint"
        >
          <SelectValue>
            {t(`settings.workflow.promptInactivity.${value}`)}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="timeout">
            {t("settings.workflow.promptInactivity.timeout")}
          </SelectItem>
          <SelectItem value="wait">
            {t("settings.workflow.promptInactivity.wait")}
          </SelectItem>
          <SelectItem value="unbounded">
            {t("settings.workflow.promptInactivity.unbounded")}
          </SelectItem>
        </SelectContent>
      </Select>
      {value === "wait" && (
        <label className="grid gap-1 text-xs">
          {t("settings.workflow.promptInactivity.minutes")}
          <Input
            key={minutes}
            type="number"
            min={1}
            max={65535}
            step={1}
            defaultValue={minutes}
            onBlur={(event) => {
              const input = event.currentTarget;
              if (!input.value || !input.validity.valid) {
                input.value = String(minutes);
              } else if (input.valueAsNumber !== minutes) {
                onChange({
                  ...config,
                  promptInactivity: { waitMinutes: input.valueAsNumber },
                });
              }
            }}
          />
        </label>
      )}
      <p
        id="workflow-agent-prompt-inactivity-hint"
        className="text-[10px] leading-relaxed text-muted-foreground"
      >
        {t("settings.workflow.promptInactivity.hint")}
      </p>
    </InspectorField>
  );
}
