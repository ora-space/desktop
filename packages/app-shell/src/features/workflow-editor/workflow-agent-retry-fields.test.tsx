import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  createMockWorkflowCapabilities,
  validateWorkflowAgentRetry,
  type WorkflowAgentConfig,
  type WorkflowNodeData,
} from "@ora/workflow-mock";
import { appI18n, translationResources } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowInspector } from "./workflow-inspector";

/** An Agent node built from the catalog default, which carries no `retry` field. */
function agentNode(
  overrides: Record<string, unknown> = {},
  parentId?: string,
): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "agent-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    ...(parentId === undefined ? {} : { parentId }),
    data: {
      kind: "agent",
      title: "Review",
      description: "",
      agentConfig: {
        ...createMockWorkflowCapabilities("en-US").defaultAgentConfig,
        ...overrides,
      } as WorkflowAgentConfig,
    },
  };
}

/**
 * Applies inspector edits to local state like the editor does, exposes the persisted Agent
 * contract, and offers one button per external edit (undo/redo) of the retry policy; an
 * `undefined` policy removes the field.
 */
function Harness({
  node,
  externalChanges = [],
}: {
  node: Node<WorkflowNodeData, "workflow">;
  externalChanges?: readonly (readonly [
    label: string,
    retry: WorkflowAgentConfig["retry"],
  ])[];
}) {
  const [current, setCurrent] = useState(node);
  return (
    <AppI18nProvider>
      <WorkflowInspector
        variableCatalog={[]}
        node={current}
        capabilities={createMockWorkflowCapabilities("en-US")}
        onUpdate={setCurrent}
        onDelete={() => undefined}
        onCloseNode={() => undefined}
      />
      <pre data-testid="agent-config">
        {JSON.stringify(current.data.agentConfig)}
      </pre>
      {externalChanges.map(([label, retry]) => (
        <button
          key={label}
          type="button"
          onClick={() =>
            setCurrent((previous) => {
              const agentConfig = { ...previous.data.agentConfig! };
              if (retry === undefined) {
                delete agentConfig.retry;
              } else {
                agentConfig.retry = retry;
              }
              return { ...previous, data: { ...previous.data, agentConfig } };
            })
          }
        >
          {label}
        </button>
      ))}
    </AppI18nProvider>
  );
}

/** Reads the Agent contract exactly as it would be serialized into the graph. */
function persistedConfig(): Record<string, unknown> {
  return JSON.parse(screen.getByTestId("agent-config").textContent ?? "");
}

function retrySwitch(): HTMLElement {
  return screen.getByRole("switch", { name: "Retry on failure" });
}

function maxRetriesInput(): HTMLInputElement {
  return screen.getByLabelText("Max retries");
}

function initialDelayInput(): HTMLInputElement {
  return screen.getByLabelText("First wait (seconds)");
}

/** Per-field validation messages, which are polite `status` regions. */
function fieldMessages(): string[] {
  return screen
    .queryAllByRole("status")
    .map((element) => element.textContent ?? "");
}

/** Asserts that neither a field message nor the malformed-policy alert is shown. */
function expectNoRetryMessages() {
  expect(fieldMessages()).toEqual([]);
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
}

describe("Agent retry settings", () => {
  it("shows the default policy for a node without retry and does not write it", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode()} />);

    expect(retrySwitch()).toBeChecked();
    expect(maxRetriesInput()).toHaveValue(2);
    expect(initialDelayInput()).toHaveValue(10);
    expect(maxRetriesInput()).toBeEnabled();
    expect(initialDelayInput()).toBeEnabled();
    expect(maxRetriesInput()).toHaveAttribute("min", "0");
    expect(maxRetriesInput()).toHaveAttribute("max", "5");
    expect(initialDelayInput()).toHaveAttribute("min", "0");
    expect(initialDelayInput()).toHaveAttribute("max", "300");
    expectNoRetryMessages();
    expect(persistedConfig()).not.toHaveProperty("retry");
  });

  it("keeps retry absent when another Agent setting is edited", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<Harness node={agentNode({ interactive: false })} />);
    const interactive = screen.getByRole("switch", {
      name: "Interactive mode",
    });

    await user.click(interactive);
    expect(persistedConfig()).toMatchObject({ interactive: true });
    await user.click(interactive);

    expect(persistedConfig()).toMatchObject({ interactive: false });
    expect(persistedConfig()).not.toHaveProperty("retry");
    expect(retrySwitch()).toBeChecked();
  });

  it("explains which failures retry, output feedback, backoff, files, and interactive nodes", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode()} />);

    const hint = document.getElementById("workflow-agent-retry-hint");
    expect(hint).not.toBeNull();
    expect(retrySwitch()).toHaveAttribute(
      "aria-describedby",
      "workflow-agent-retry-hint",
    );
    const items = within(hint!)
      .getAllByRole("listitem")
      .map((item) => item.textContent);
    expect(items).toEqual([
      "Reruns this node when the agent session fails, or when the agent's reply doesn't match the structured output schema, is a refusal, or stops for an unknown reason.",
      "When retrying after a reply problem, the agent is told why the previous attempt failed.",
      "After the first wait, each wait doubles, up to 600 seconds. File changes are not rolled back before a retry.",
      "Nodes in interactive mode are never retried.",
    ]);
  });

  it("writes the full policy when switched off and disables both numbers", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<Harness node={agentNode()} />);

    await user.click(retrySwitch());

    expect(retrySwitch()).not.toBeChecked();
    expect(persistedConfig().retry).toEqual({
      enabled: false,
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
    expect(maxRetriesInput()).toBeDisabled();
    expect(initialDelayInput()).toBeDisabled();
    expect(maxRetriesInput()).toHaveValue(2);
    expect(initialDelayInput()).toHaveValue(10);

    await user.click(retrySwitch());

    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
    expect(maxRetriesInput()).toBeEnabled();
    expect(initialDelayInput()).toBeEnabled();
  });

  it("writes typed numbers into the full policy", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<Harness node={agentNode()} />);

    await user.clear(maxRetriesInput());
    await user.type(maxRetriesInput(), "4");
    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 4,
      initialDelaySeconds: 10,
    });

    await user.clear(initialDelayInput());
    await user.type(initialDelayInput(), "45");
    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 4,
      initialDelaySeconds: 45,
    });
    expect(maxRetriesInput()).toHaveValue(4);
    expect(initialDelayInput()).toHaveValue(45);
    expectNoRetryMessages();
  });

  it.each([
    ["Max retries", "0", { maxRetries: 0 }],
    ["Max retries", "5", { maxRetries: 5 }],
    ["First wait (seconds)", "0", { initialDelaySeconds: 0 }],
    ["First wait (seconds)", "300", { initialDelaySeconds: 300 }],
  ])("accepts the boundary %s = %s", async (label, value, patch) => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode()} />);

    fireEvent.change(screen.getByLabelText(label), { target: { value } });

    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 10,
      ...patch,
    });
    expectNoRetryMessages();
  });

  it.each([
    ["Max retries", "6", "Max retries must be between 0 and 5."],
    ["Max retries", "-1", "Max retries must be between 0 and 5."],
    ["Max retries", "1.5", "Max retries must be a whole number."],
    ["Max retries", "", "Max retries is required."],
    [
      "First wait (seconds)",
      "301",
      "First wait (seconds) must be between 0 and 300.",
    ],
    [
      "First wait (seconds)",
      "-1",
      "First wait (seconds) must be between 0 and 300.",
    ],
    [
      "First wait (seconds)",
      "2.5",
      "First wait (seconds) must be a whole number.",
    ],
    ["First wait (seconds)", "", "First wait (seconds) is required."],
  ])(
    "keeps %s = %o out of the graph and explains why",
    async (label, value, message) => {
      await appI18n.changeLanguage("en-US");
      render(<Harness node={agentNode()} />);
      const input = screen.getByLabelText(label);

      fireEvent.change(input, { target: { value } });

      expect(screen.getByRole("status")).toHaveTextContent(message);
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      expect(input).toHaveAttribute("aria-invalid", "true");
      expect(input).toHaveAttribute(
        "aria-describedby",
        screen.getByRole("status").id,
      );
      expect(persistedConfig()).not.toHaveProperty("retry");
    },
  );

  it("clears the message and commits once an out-of-range value is corrected", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode()} />);

    fireEvent.change(maxRetriesInput(), { target: { value: "6" } });
    expect(maxRetriesInput()).toHaveValue(6);
    expect(fieldMessages()).toEqual(["Max retries must be between 0 and 5."]);

    fireEvent.change(maxRetriesInput(), { target: { value: "3" } });

    expectNoRetryMessages();
    expect(maxRetriesInput()).toHaveAttribute("aria-invalid", "false");
    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 3,
      initialDelaySeconds: 10,
    });
  });

  it("keeps the last valid policy and the typed text when the switch changes while a number is invalid", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<Harness node={agentNode()} />);

    fireEvent.change(initialDelayInput(), { target: { value: "999" } });
    await user.click(retrySwitch());

    expect(persistedConfig().retry).toEqual({
      enabled: false,
      maxRetries: 2,
      initialDelaySeconds: 10,
    });
    expect(initialDelayInput()).toHaveValue(999);
    expect(initialDelayInput()).toBeDisabled();
    expect(fieldMessages()).toEqual([
      "First wait (seconds) must be between 0 and 300.",
    ]);
  });

  it("drops typed text that an external change (undo) replaced", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        node={agentNode()}
        externalChanges={[
          [
            "Apply external change",
            { enabled: true, maxRetries: 1, initialDelaySeconds: 5 },
          ],
        ]}
      />,
    );

    fireEvent.change(maxRetriesInput(), { target: { value: "9" } });
    expect(fieldMessages()).toEqual(["Max retries must be between 0 and 5."]);

    await user.click(
      screen.getByRole("button", { name: "Apply external change" }),
    );

    expect(maxRetriesInput()).toHaveValue(1);
    expect(initialDelayInput()).toHaveValue(5);
    expectNoRetryMessages();
  });

  it("does not bring back discarded text when a redo restores the value it was typed over", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        node={agentNode()}
        externalChanges={[
          ["Undo", undefined],
          ["Redo", { enabled: true, maxRetries: 3, initialDelaySeconds: 10 }],
        ]}
      />,
    );

    fireEvent.change(maxRetriesInput(), { target: { value: "3" } });
    fireEvent.change(maxRetriesInput(), { target: { value: "39" } });
    expect(maxRetriesInput()).toHaveValue(39);

    await user.click(screen.getByRole("button", { name: "Undo" }));

    expect(persistedConfig()).not.toHaveProperty("retry");
    expect(maxRetriesInput()).toHaveValue(2);
    expectNoRetryMessages();

    await user.click(screen.getByRole("button", { name: "Redo" }));

    expect(maxRetriesInput()).toHaveValue(3);
    expectNoRetryMessages();
  });

  it("shows stored custom values, including a turned-off policy", async () => {
    await appI18n.changeLanguage("en-US");
    render(
      <Harness
        node={agentNode({
          retry: { enabled: false, maxRetries: 3, initialDelaySeconds: 60 },
        })}
      />,
    );

    expect(retrySwitch()).not.toBeChecked();
    expect(maxRetriesInput()).toHaveValue(3);
    expect(initialDelayInput()).toHaveValue(60);
    expect(maxRetriesInput()).toBeDisabled();
    expect(initialDelayInput()).toBeDisabled();
  });

  it("flags an imported out-of-range value until the author fixes it", async () => {
    await appI18n.changeLanguage("en-US");
    render(
      <Harness
        node={agentNode({
          retry: { enabled: true, maxRetries: 9, initialDelaySeconds: 10 },
        })}
      />,
    );

    expect(maxRetriesInput()).toHaveValue(9);
    expect(fieldMessages()).toEqual(["Max retries must be between 0 and 5."]);

    fireEvent.change(maxRetriesInput(), { target: { value: "4" } });

    expectNoRetryMessages();
    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 4,
      initialDelaySeconds: 10,
    });
  });

  it("flags an imported policy with a missing field", async () => {
    await appI18n.changeLanguage("en-US");
    render(
      <Harness node={agentNode({ retry: { enabled: true, maxRetries: 2 } })} />,
    );

    expect(initialDelayInput()).toHaveValue(null);
    expect(fieldMessages()).toEqual(["First wait (seconds) is required."]);

    fireEvent.change(initialDelayInput(), { target: { value: "20" } });

    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 20,
    });
  });

  it.each([
    ["a missing number", { enabled: true, maxRetries: 2 }],
    [
      "an out-of-range number",
      { enabled: true, maxRetries: 9, initialDelaySeconds: 10 },
    ],
    ["a fractional number", { enabled: true, maxRetries: 1.5 }],
    ["an empty object", {}],
  ])(
    "never writes %s back when only the switch changes",
    async (_case, retry) => {
      await appI18n.changeLanguage("en-US");
      const user = userEvent.setup();
      render(<Harness node={agentNode({ retry })} />);

      await user.click(retrySwitch());

      const written = persistedConfig().retry;
      expect(written).toEqual({
        enabled: false,
        maxRetries: 2,
        initialDelaySeconds: 10,
      });
      expect(validateWorkflowAgentRetry(written)).toEqual([]);
      expectNoRetryMessages();
    },
  );

  it("fills a missing number with its default when the other number is edited", async () => {
    await appI18n.changeLanguage("en-US");
    render(
      <Harness node={agentNode({ retry: { enabled: true, maxRetries: 2 } })} />,
    );

    fireEvent.change(maxRetriesInput(), { target: { value: "3" } });

    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 3,
      initialDelaySeconds: 10,
    });
    expect(initialDelayInput()).toHaveValue(10);
    expectNoRetryMessages();
  });

  it("replaces a malformed stored value with a complete policy on the next edit", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode({ retry: "yes" })} />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "The saved retry settings are malformed.",
    );
    expect(retrySwitch()).toBeChecked();
    expect(maxRetriesInput()).toHaveValue(2);
    expect(initialDelayInput()).toHaveValue(10);

    fireEvent.change(maxRetriesInput(), { target: { value: "3" } });

    expectNoRetryMessages();
    expect(persistedConfig().retry).toEqual({
      enabled: true,
      maxRetries: 3,
      initialDelaySeconds: 10,
    });
  });

  it("repairs a non-boolean switch by toggling it", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        node={agentNode({
          retry: { enabled: "yes", maxRetries: 1, initialDelaySeconds: 5 },
        })}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "The saved retry settings are malformed.",
    );

    await user.click(retrySwitch());

    expectNoRetryMessages();
    expect(persistedConfig().retry).toEqual({
      enabled: false,
      maxRetries: 1,
      initialDelaySeconds: 5,
    });
  });

  it("treats a null retry like an absent one", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode({ retry: null })} />);

    expect(retrySwitch()).toBeChecked();
    expect(maxRetriesInput()).toHaveValue(2);
    expect(initialDelayInput()).toHaveValue(10);
    expectNoRetryMessages();
  });

  it("hides the section for interactive nodes", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode({ interactive: true })} />);

    expect(
      screen.getByRole("switch", { name: "Interactive mode" }),
    ).toBeChecked();
    expect(
      screen.queryByRole("switch", { name: "Retry on failure" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Max retries")).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("First wait (seconds)"),
    ).not.toBeInTheDocument();
  });

  it("hides the section when interactive mode is turned on and keeps the stored policy", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(
      <Harness
        node={agentNode({
          retry: { enabled: true, maxRetries: 4, initialDelaySeconds: 30 },
        })}
      />,
    );
    const interactive = screen.getByRole("switch", {
      name: "Interactive mode",
    });

    await user.click(interactive);

    expect(
      screen.queryByRole("switch", { name: "Retry on failure" }),
    ).not.toBeInTheDocument();
    expect(persistedConfig()).toMatchObject({
      interactive: true,
      retry: { enabled: true, maxRetries: 4, initialDelaySeconds: 30 },
    });

    await user.click(interactive);

    expect(retrySwitch()).toBeChecked();
    expect(maxRetriesInput()).toHaveValue(4);
    expect(initialDelayInput()).toHaveValue(30);
  });

  it("stays available to Agents inside an iteration region, which cannot be interactive", async () => {
    await appI18n.changeLanguage("en-US");
    render(<Harness node={agentNode({}, "iteration-1")} />);

    expect(
      screen.getByRole("switch", { name: "Interactive mode" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(retrySwitch()).toBeChecked();
    expect(maxRetriesInput()).toBeEnabled();
  });

  it("renders the Chinese copy and messages", async () => {
    await appI18n.changeLanguage("zh-CN");
    render(<Harness node={agentNode()} />);

    expect(screen.getByRole("switch", { name: "失败自动重试" })).toBeChecked();
    expect(screen.getByLabelText("最多重试次数")).toHaveValue(2);
    expect(screen.getByLabelText("首次等待（秒）")).toHaveValue(10);
    expect(
      screen.getByText(
        "首次等待之后，每次等待时间翻倍，最长 600 秒；重试前不会回滚文件改动。",
      ),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("首次等待（秒）"), {
      target: { value: "301" },
    });

    expect(fieldMessages()).toEqual(["首次等待（秒）必须在 0 到 300 之间。"]);
  });
});

describe("retry translations", () => {
  // Plural keys are listed per form because each language owns its own plural categories.
  const keys = [
    "settings.workflow.field.retry",
    "settings.workflow.field.retryMaxRetries",
    "settings.workflow.field.retryInitialDelay",
    "settings.workflow.retry.hintFailures",
    "settings.workflow.retry.hintFeedback",
    "settings.workflow.retry.hintBackoff",
    "settings.workflow.retry.hintInteractive",
    "settings.workflow.retry.issue.malformed",
    "settings.workflow.retry.issue.missing",
    "settings.workflow.retry.issue.notInteger",
    "settings.workflow.retry.issue.outOfRange",
    "workflowRun.inspector.retryDefault_one",
    "workflowRun.inspector.retryDefault_other",
    "workflowRun.inspector.retryConfigured_one",
    "workflowRun.inspector.retryConfigured_other",
    "workflowRun.inspector.retryDisabled",
    "workflowRun.inspector.retryNone",
    "workflowRun.inspector.retryInteractive",
  ] as const;

  it("ships every retry string in both languages", () => {
    for (const key of keys) {
      const chinese = translationResources["zh-CN"][key];
      const english = translationResources["en-US"][key];
      expect(chinese, key).toBeTruthy();
      expect(english, key).toBeTruthy();
      expect(chinese, key).not.toBe(english);
    }
  });

  it("uses the same interpolation variables in both languages", () => {
    const variables = (text: string) =>
      [...text.matchAll(/\{\{(\w+)\}\}/g)].map((match) => match[1]).sort();
    for (const key of keys) {
      expect(variables(translationResources["zh-CN"][key]), key).toEqual(
        variables(translationResources["en-US"][key]),
      );
    }
  });
});
