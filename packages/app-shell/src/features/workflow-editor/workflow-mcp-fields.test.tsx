import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import {
  createMockWorkflowCapabilities,
  type WorkflowAgentConfig,
  type WorkflowMcpChoice,
} from "@ora/workflow-mock";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  workflowMcpChoices,
  type WorkflowMcpCatalogStatus,
} from "./mcp-catalog";
import { WorkflowMcpFields } from "./workflow-mcp-fields";

/** Creates installed metadata without starting a plugin process. */
function plugin(
  id: string,
  overrides: Partial<
    Pick<InstalledPlugin, "configuration" | "installationValidity">
  > = {},
): InstalledPlugin {
  return {
    id,
    namespace: "official",
    name: "tools",
    displayName: "Tools",
    version: "1.0.0",
    description: "Project tools",
    homepage: null,
    license: null,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
    kind: "mcp",
    ...overrides,
  };
}

/** Keeps state updates on React's actual interaction boundary and exposes the persisted contract. */
function Harness({
  choices,
  initial = [],
  catalog,
}: {
  choices: WorkflowMcpChoice[];
  initial?: WorkflowAgentConfig["mcps"];
  catalog?: WorkflowMcpCatalogStatus;
}) {
  const [config, setConfig] = useState({
    ...createMockWorkflowCapabilities("en-US").defaultAgentConfig,
    mcps: initial,
  });
  return (
    <AppI18nProvider>
      <WorkflowMcpFields
        config={config}
        choices={choices}
        catalog={catalog}
        onChange={setConfig}
      />
      <output data-testid="bindings">{JSON.stringify(config.mcps)}</output>
    </AppI18nProvider>
  );
}

describe("workflow MCP catalog", () => {
  it("uses installed MCP identities and readiness, regardless of process runtime", () => {
    expect(
      workflowMcpChoices([
        plugin("official/tools"),
        plugin("local/tools", {
          configuration: { state: "available", completeness: "incomplete" },
        }),
        plugin("other/invalid", {
          installationValidity: {
            validity: "invalid_declaration",
            errorCode: "invalid",
          },
        }),
        plugin("other/unreadable", {
          configuration: { state: "unavailable", errorCode: "read" },
        }),
        { ...plugin("official/skill"), kind: "skill" },
      ]),
    ).toEqual([
      { value: "official/tools", label: "Tools" },
      {
        value: "local/tools",
        label: "Tools",
        unavailableReason: "configurationIncomplete",
      },
      {
        value: "other/invalid",
        label: "Tools",
        unavailableReason: "invalidDeclaration",
      },
      {
        value: "other/unreadable",
        label: "Tools",
        unavailableReason: "configurationUnavailable",
      },
    ]);
  });
});

describe("workflow MCP bindings", () => {
  it("adds same-name plugins by ID, prevents duplicates, toggles, and removes", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        choices={workflowMcpChoices([
          plugin("official/tools"),
          plugin("local/tools"),
        ])}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Add MCP" }));
    await user.click(
      await screen.findByRole("option", { name: /Tools official\/tools/ }),
    );
    expect(screen.getByTestId("bindings")).toHaveTextContent(
      '[{"mcpId":"official/tools","enabled":true}]',
    );
    await user.click(screen.getByRole("button", { name: "Add MCP" }));
    expect(
      screen.queryByRole("option", { name: /Tools official\/tools/ }),
    ).not.toBeInTheDocument();
    await user.click(
      await screen.findByRole("option", { name: /Tools local\/tools/ }),
    );
    await user.click(
      screen.getAllByRole("switch", { name: "Enable or disable Tools" })[0],
    );
    expect(
      JSON.parse(screen.getByTestId("bindings").textContent ?? ""),
    ).toEqual([
      { mcpId: "official/tools", enabled: false },
      { mcpId: "local/tools", enabled: true },
    ]);
    await user.click(
      screen.getAllByRole("button", { name: "Remove Tools" })[0],
    );
    expect(
      JSON.parse(screen.getByTestId("bindings").textContent ?? ""),
    ).toEqual([{ mcpId: "local/tools", enabled: true }]);
  });

  it("retains missing bindings and lets authors disable them", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        choices={[]}
        initial={[{ mcpId: "missing/tools", enabled: true }]}
      />,
    );
    expect(
      screen.getByText("Plugin is not installed or unavailable"),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("switch", { name: "Enable or disable missing/tools" }),
    );
    expect(
      JSON.parse(screen.getByTestId("bindings").textContent ?? ""),
    ).toEqual([{ mcpId: "missing/tools", enabled: false }]);
  });

  it("shows retry instead of claiming a plugin was uninstalled when discovery fails", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    const onRetry = vi.fn();
    render(
      <Harness
        choices={[]}
        initial={[{ mcpId: "official/tools", enabled: true }]}
        catalog={{ isLoading: false, isError: true, onRetry }}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Existing bindings are preserved",
    );
    expect(
      screen.queryByText("Plugin is not installed or unavailable"),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reload MCPs" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("allows incomplete plugins to be selected and shows their status", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        choices={workflowMcpChoices([
          plugin("local/tools", {
            configuration: { state: "available", completeness: "incomplete" },
          }),
        ])}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Add MCP" }));
    await user.click(
      await screen.findByRole("option", { name: /Tools local\/tools/ }),
    );
    expect(
      screen.getByText("Plugin configuration is incomplete"),
    ).toBeInTheDocument();
    expect(
      JSON.parse(screen.getByTestId("bindings").textContent ?? ""),
    ).toEqual([{ mcpId: "local/tools", enabled: true }]);
  });
});
