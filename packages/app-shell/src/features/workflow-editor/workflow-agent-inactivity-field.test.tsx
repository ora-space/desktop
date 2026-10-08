import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  createMockWorkflowCapabilities,
  type WorkflowAgentConfig,
  type WorkflowNodeData,
} from "@ora/workflow-mock";
import { appI18n, translationResources } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowInspector } from "./workflow-inspector";

/** Keeps inspector edits and external undo/redo changes in the same persisted node contract. */
function Harness({
  config = {},
  parentId,
}: {
  config?: Partial<WorkflowAgentConfig>;
  parentId?: string;
}) {
  const [node, setNode] = useState<Node<WorkflowNodeData, "workflow">>({
    id: "agent",
    type: "workflow",
    position: { x: 0, y: 0 },
    ...(parentId === undefined ? {} : { parentId }),
    data: {
      kind: "agent",
      title: "Research",
      description: "",
      agentConfig: {
        ...createMockWorkflowCapabilities("en-US").defaultAgentConfig,
        ...config,
      },
    },
  });

  function restore(promptInactivity: WorkflowAgentConfig["promptInactivity"]) {
    setNode((previous) => {
      const agentConfig = { ...previous.data.agentConfig! };
      if (promptInactivity === undefined) {
        delete agentConfig.promptInactivity;
      } else {
        agentConfig.promptInactivity = promptInactivity;
      }
      return { ...previous, data: { ...previous.data, agentConfig } };
    });
  }

  return (
    <AppI18nProvider>
      <WorkflowInspector
        node={node}
        capabilities={createMockWorkflowCapabilities("en-US")}
        variableCatalog={[]}
        onUpdate={setNode}
        onDelete={() => undefined}
        onCloseNode={() => undefined}
      />
      <pre data-testid="agent-config">
        {JSON.stringify(node.data.agentConfig)}
      </pre>
      <button onClick={() => restore(undefined)}>Undo</button>
      <button onClick={() => restore("wait")}>Redo</button>
    </AppI18nProvider>
  );
}

/** Reads the graph value rather than inferring persistence from the selected label. */
function savedConfig(): WorkflowAgentConfig {
  return JSON.parse(screen.getByTestId("agent-config").textContent ?? "{}");
}

describe("Agent prompt inactivity settings", () => {
  it.each([undefined, null])(
    "shows the default for %s without filling the saved field",
    async (promptInactivity) => {
      await appI18n.changeLanguage("en-US");
      render(
        <Harness
          config={promptInactivity === undefined ? {} : { promptInactivity }}
        />,
      );
      expect(
        screen.getByRole("combobox", {
          name: "When no progress is reported",
        }),
      ).toHaveTextContent("Use default timeout handling");
      const before = savedConfig();

      await userEvent.setup().click(
        screen.getByRole("switch", {
          name: "Interactive mode",
        }),
      );

      expect(savedConfig()).toEqual({ ...before, interactive: true });
      if (promptInactivity === undefined) {
        expect(savedConfig()).not.toHaveProperty("promptInactivity");
      }
    },
  );

  it("saves the selected policy and follows undo and redo without changing failure retry", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    const retry = { enabled: false, maxRetries: 4, initialDelaySeconds: 30 };
    render(<Harness config={{ retry }} />);
    const before = savedConfig();
    const select = screen.getByRole("combobox", {
      name: "When no progress is reported",
    });

    await user.click(select);
    await user.click(
      await screen.findByRole("option", { name: "Keep waiting" }),
    );
    await waitFor(() =>
      expect(savedConfig()).toEqual({ ...before, promptInactivity: "wait" }),
    );
    expect(
      screen.getByRole("switch", { name: "Retry on failure" }),
    ).not.toBeChecked();

    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(select).toHaveTextContent("Use default timeout handling");
    expect(savedConfig()).toEqual(before);
    await user.click(screen.getByRole("button", { name: "Redo" }));
    expect(select).toHaveTextContent("Keep waiting");
    expect(savedConfig()).toEqual({ ...before, promptInactivity: "wait" });

    await user.click(select);
    await user.click(
      await screen.findByRole("option", {
        name: "Use default timeout handling",
      }),
    );
    await waitFor(() =>
      expect(savedConfig()).toEqual({ ...before, promptInactivity: "timeout" }),
    );
  });

  it("keeps the wait setting available for interactive Agent nodes", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness config={{ interactive: true, promptInactivity: "wait" }} />,
    );
    const select = screen.getByRole("combobox", {
      name: "When no progress is reported",
    });
    expect(select).toBeEnabled();
    expect(select).toHaveTextContent("Keep waiting");
    expect(
      screen.queryByRole("switch", { name: "Retry on failure" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Interactive mode" }));
    expect(savedConfig().promptInactivity).toBe("wait");
    expect(
      screen.getByRole("switch", { name: "Retry on failure" }),
    ).toBeChecked();
  });

  it("leaves silence handling editable inside an iteration region", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<Harness parentId="iteration" />);
    expect(
      screen.getByRole("switch", { name: "Interactive mode" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(
      screen.getByRole("combobox", { name: "When no progress is reported" }),
    ).toBeEnabled();
    await user.click(
      screen.getByRole("combobox", { name: "When no progress is reported" }),
    );
    await user.click(
      await screen.findByRole("option", { name: "Keep waiting" }),
    );
    await waitFor(() => expect(savedConfig().promptInactivity).toBe("wait"));
  });

  it("renders the Chinese choices and explains manual stopping separately from failure retries", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(<Harness />);
    const select = screen.getByRole("combobox", { name: "长时间未收到进展时" });
    expect(select).toHaveTextContent("按默认超时规则处理");
    expect(select).toHaveAttribute(
      "aria-describedby",
      "workflow-agent-prompt-inactivity-hint",
    );
    expect(
      document.getElementById("workflow-agent-prompt-inactivity-hint"),
    ).toHaveTextContent("可手动停止。实际执行错误仍按失败自动重试设置处理。");
    await user.click(select);
    await user.click(await screen.findByRole("option", { name: "持续等待" }));
    await waitFor(() => expect(savedConfig().promptInactivity).toBe("wait"));
  });
});

describe("prompt inactivity translations", () => {
  it("ships editor and run-inspector strings in both languages", () => {
    const keys = [
      "settings.workflow.field.promptInactivity",
      "settings.workflow.promptInactivity.timeout",
      "settings.workflow.promptInactivity.wait",
      "settings.workflow.promptInactivity.hint",
      "workflowRun.inspector.promptInactivityTimeout",
      "workflowRun.inspector.promptInactivityWait",
    ] as const;
    for (const locale of ["zh-CN", "en-US"] as const) {
      for (const key of keys) {
        expect(translationResources[locale][key], key).toBeTruthy();
      }
    }
  });
});
