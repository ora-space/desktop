import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  createMockWorkflowCapabilities,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import { appI18n } from "../../i18n/i18n-instance";
import { RunNodeConfigForm, type RunNodeConfig } from "./run-node-config-form";

beforeEach(async () => {
  await appI18n.changeLanguage("en-US");
});

function variable(
  node: string,
  name: string,
  valueType: WorkflowVariableCatalogEntry["valueType"],
): WorkflowVariableCatalogEntry {
  return {
    sourceNodeId: node,
    variableName: name,
    valueType,
    selector: [node, name],
  };
}
function props(effective: RunNodeConfig) {
  return {
    runId: "run-7",
    nodeRunId: "attempt-3",
    baseRevision: 2,
    effective,
    capabilities: createMockWorkflowCapabilities("en-US", [
      { agentCli: "cli", modelId: "one", label: "One" },
      { agentCli: "cli", modelId: "two", label: "Two" },
    ]),
    variables: [
      variable("start", "items", "array"),
      variable("start", "other", "array"),
    ],
    memberOutputs: [variable("member", "output", "string")],
    onCancel: vi.fn(),
    onSubmit: vi.fn(),
  };
}
const submit = () => screen.getByRole("button", { name: "Preview changes" });

it("isolates a draft, discards cancel and reopens the selected execution at its latest revision", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "loop",
    config: { until: { logic: "and", conditions: [] }, maxIterations: 7 },
  });
  const view = render(<RunNodeConfigForm {...input} />);
  const limit = () => screen.getByRole("spinbutton");
  expect(limit()).toHaveValue(7);
  fireEvent.change(limit(), { target: { value: "9" } });
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(input.onCancel).toHaveBeenCalledOnce();
  expect(input.onSubmit).not.toHaveBeenCalled();
  expect(limit()).toHaveValue(7);
  fireEvent.change(limit(), { target: { value: "8" } });
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [{ op: "replace", path: "/loopConfig/maxIterations", value: 8 }],
      },
    ],
  ]);
  view.rerender(
    <RunNodeConfigForm
      {...input}
      baseRevision={3}
      effective={{
        kind: "loop",
        config: { until: { logic: "and", conditions: [] }, maxIterations: 8 },
      }}
    />,
  );
  expect(limit()).toHaveValue(8);
  expect(submit()).toBeDisabled();
  expect(input.effective).toEqual({
    kind: "loop",
    config: { until: { logic: "and", conditions: [] }, maxIterations: 7 },
  });
});

it("blocks a model edit with an out-of-scope prompt token, then emits the corrected edit", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "agent",
    config: {
      prompt: "Prior {{#other.secret#}}",
      executor: { agentCli: "cli", modelId: "one" },
    },
  });
  render(<RunNodeConfigForm {...input} />);
  await user.click(
    screen.getByLabelText(appI18n.t("settings.workflow.field.agentModel")),
  );
  await user.click(await screen.findByRole("option", { name: "Two" }));
  await user.click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent("Custom prompt");
  expect(input.onSubmit).not.toHaveBeenCalled();
  const prompt = screen.getByLabelText("Custom prompt");
  await user.clear(prompt);
  await user.type(prompt, "Use this item");
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [
          {
            op: "replace",
            path: "/agentConfig/prompt",
            value: "Use this item",
          },
          {
            op: "replace",
            path: "/agentConfig/executor/modelId",
            value: "two",
          },
        ],
      },
    ],
  ]);
});

it("edits an existing condition without changing branch IDs or order", async () => {
  const user = userEvent.setup();
  const comparison = {
    variableSelector: ["start", "items"],
    operator: "empty",
  };
  const cases = [
    { id: "retained-if", conditions: [comparison] },
    { id: "retained-elif", conditions: [] },
  ];
  const input = props({ kind: "condition", config: { cases } });
  render(<RunNodeConfigForm {...input} />);
  await user.click(
    screen.getByLabelText(
      appI18n.t("settings.workflow.field.operator", { index: 1 }),
    ),
  );
  await user.click(await screen.findByRole("option", { name: "Is not empty" }));
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [
          {
            op: "replace",
            path: "/cases/0/conditions",
            value: [{ ...comparison, operator: "not_empty" }],
          },
        ],
      },
    ],
  ]);
});

it("uses separate iteration entry and member catalogs and returns just the changed selector", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "iteration",
    config: {
      iteratorSelector: ["start", "items"],
      collectSelector: ["member", "output"],
    },
  });
  render(<RunNodeConfigForm {...input} />);
  await user.click(screen.getByLabelText("Iterator source"));
  expect(
    screen.queryByRole("option", { name: "member.output" }),
  ).not.toBeInTheDocument();
  await user.click(await screen.findByRole("option", { name: "start.other" }));
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [
          {
            op: "replace",
            path: "/iterationConfig/iteratorSelector",
            value: ["start", "other"],
          },
        ],
      },
    ],
  ]);
});

it("edits a Loop termination rule with the selected round's variables", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "loop",
    config: {
      maxIterations: 6,
      until: {
        logic: "and",
        conditions: [
          { variableSelector: ["start", "items"], operator: "empty" },
        ],
      },
    },
  });
  render(<RunNodeConfigForm {...input} />);
  await user.click(
    screen.getByLabelText(
      appI18n.t("settings.workflow.field.variable", { index: 1 }),
    ),
  );
  await user.click(await screen.findByRole("option", { name: "start.other" }));
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [
          {
            op: "replace",
            path: "/loopConfig/until",
            value: {
              logic: "and",
              conditions: [
                { variableSelector: ["start", "other"], operator: "empty" },
              ],
            },
          },
        ],
      },
    ],
  ]);
});

it("repairs a stale collect selector using only this iteration's member outputs", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "iteration",
    config: {
      iteratorSelector: ["start", "items"],
      collectSelector: ["former", "output"],
    },
  });
  render(<RunNodeConfigForm {...input} />);
  await user.click(screen.getByLabelText("Iterator source"));
  await user.click(await screen.findByRole("option", { name: "start.other" }));
  await user.click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent("Collect target");
  expect(input.onSubmit).not.toHaveBeenCalled();
  await user.click(screen.getByLabelText("Collect target"));
  expect(
    screen.queryByRole("option", { name: "start.items" }),
  ).not.toBeInTheDocument();
  await user.click(
    await screen.findByRole("option", { name: "member.output" }),
  );
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [
          {
            op: "replace",
            path: "/iterationConfig/iteratorSelector",
            value: ["start", "other"],
          },
          {
            op: "replace",
            path: "/iterationConfig/collectSelector",
            value: ["member", "output"],
          },
        ],
      },
    ],
  ]);
});

it("returns a leaf replacement for an existing boolean branch operand", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "condition",
    config: {
      cases: [
        {
          id: "A",
          conditions: [
            {
              variableSelector: ["start", "flag"],
              operator: "is",
              value: true,
            },
          ],
        },
      ],
    },
  });
  input.variables.push(variable("start", "flag", "boolean"));
  render(<RunNodeConfigForm {...input} />);
  await user.click(
    screen.getByLabelText(
      appI18n.t("settings.workflow.field.value", { index: 1 }),
    ),
  );
  await user.click(await screen.findByRole("option", { name: "false" }));
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "run-7",
        nodeRunId: "attempt-3",
        baseRevision: 2,
        patch: [
          { op: "replace", path: "/cases/0/conditions/0/value", value: false },
        ],
      },
    ],
  ]);
});
