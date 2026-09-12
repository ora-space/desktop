import type { InstalledPlugin } from "@ora/contracts";
import type { WorkflowMcpChoice } from "@ora/workflow-mock";

/** Maps installed identities and secret-free readiness to the authoring catalog. */
export function workflowMcpChoices(
  plugins: readonly InstalledPlugin[],
): WorkflowMcpChoice[] {
  return plugins
    .filter((plugin) => plugin.kind === "mcp")
    .map((plugin) => {
      const choice = { value: plugin.id, label: plugin.displayName };
      if (plugin.installationValidity.validity !== "valid") {
        return { ...choice, unavailableReason: "invalidDeclaration" };
      }
      if (plugin.configuration.state === "unavailable") {
        return { ...choice, unavailableReason: "configurationUnavailable" };
      }
      if (
        plugin.configuration.state === "available" &&
        plugin.configuration.completeness !== "complete"
      ) {
        return { ...choice, unavailableReason: "configurationIncomplete" };
      }
      return choice;
    });
}

/** Loading failures must not turn persisted bindings into allegedly uninstalled plugins. */
export interface WorkflowMcpCatalogStatus {
  isLoading: boolean;
  isError: boolean;
  onRetry: () => void;
}
