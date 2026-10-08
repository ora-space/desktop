import { useTranslation } from "react-i18next";
import {
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
  const value = config.promptInactivity ?? "timeout";

  return (
    <InspectorField
      label={t("settings.workflow.field.promptInactivity")}
      htmlFor="workflow-agent-prompt-inactivity"
    >
      <Select
        value={value}
        onValueChange={(promptInactivity) => {
          if (promptInactivity === "timeout" || promptInactivity === "wait") {
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
        </SelectContent>
      </Select>
      <p
        id="workflow-agent-prompt-inactivity-hint"
        className="text-[10px] leading-relaxed text-muted-foreground"
      >
        {t("settings.workflow.promptInactivity.hint")}
      </p>
    </InspectorField>
  );
}
