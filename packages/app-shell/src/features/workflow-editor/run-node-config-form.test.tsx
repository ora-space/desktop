import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  createMockWorkflowCapabilities,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import { appI18n } from "../../i18n/i18n-instance";
import conditionSample from "./fixtures/run-condition-preview.json";
import defaults from "./fixtures/run-debug-defaults.json";
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
    target: { runId: "run-7", nodeRunId: "attempt-3" },
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

it("drops the pending member's draft when only its foreach index changes", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "agent",
    config: {
      prompt: "Original",
      executor: { agentCli: "cli", modelId: "one" },
    },
  });
  const item = (index: number) => ({
    runId: "run-7",
    pendingTarget: {
      rootScopeId: "root",
      scopeId: "body",
      containerNodeRunId: "iteration-attempt",
      nodeId: "agent",
      index,
    },
  });
  const view = render(<RunNodeConfigForm {...input} target={item(3)} />);
  const prompt = () => screen.getByLabelText("Custom prompt");
  await user.clear(prompt());
  await user.type(prompt(), "Only for item 3");
  expect(submit()).toBeEnabled();

  view.rerender(<RunNodeConfigForm {...input} target={item(4)} />);
  expect(prompt()).toHaveTextContent("Original");
  expect(submit()).toBeDisabled();
  expect(input.onSubmit).not.toHaveBeenCalled();
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
  expect(
    screen.getByLabelText("Custom prompt").closest('[aria-invalid="true"]'),
  ).not.toBeNull();
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
  const input = props({
    kind: "condition",
    config: { cases: conditionSample.cases },
  });
  render(<RunNodeConfigForm {...input} />);
  expect(
    screen.getByRole("button", { name: "Remove condition" }),
  ).toBeDisabled();
  await user.click(
    screen.getByLabelText(
      appI18n.t("settings.workflow.field.operator", { index: 1 }),
    ),
  );
  const empty = await screen.findByRole("option", { name: "Is not empty" });
  expect(
    screen.queryByRole("option", { name: "Equals" }),
  ).not.toBeInTheDocument();
  await user.click(empty);
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([[conditionSample.preview]]);
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
  const available = await screen.findByRole("option", { name: "start.other" });
  expect(
    screen.queryByRole("option", { name: "member.output" }),
  ).not.toBeInTheDocument();
  await user.click(available);
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
  const available = await screen.findByRole("option", {
    name: "member.output",
  });
  expect(
    screen.queryByRole("option", { name: "start.items" }),
  ).not.toBeInTheDocument();
  await user.click(available);
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

it.each(["nodeRunId", "baseRevision"] as const)(
  "drops a dirty draft when only %s changes",
  async (field) => {
    const input = props({
      kind: "loop",
      config: { maxIterations: 5, until: { logic: "and", conditions: [] } },
    });
    const view = render(<RunNodeConfigForm {...input} />);
    fireEvent.change(screen.getByRole("spinbutton"), {
      target: { value: "9" },
    });
    expect(submit()).toBeEnabled();
    const next =
      field === "nodeRunId"
        ? { target: { ...input.target, nodeRunId: "attempt-4" } }
        : { baseRevision: 3 };
    view.rerender(<RunNodeConfigForm {...input} {...next} />);
    expect(screen.getByRole("spinbutton")).toHaveValue(5);
    expect(submit()).toBeDisabled();
  },
);

it("discards the mounted prompt editor on cancel", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "agent",
    config: {
      prompt: "original",
      executor: { agentCli: "cli", modelId: "one" },
    },
  });
  render(<RunNodeConfigForm {...input} />);
  await user.type(screen.getByLabelText("Custom prompt"), " discarded");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.getByLabelText("Custom prompt")).toHaveTextContent(
    /^original$/,
  );
  expect(submit()).toBeDisabled();
});

it.each(["loop", "condition"] as const)(
  "blocks invisible variables in %s rules",
  async (kind) => {
    const user = userEvent.setup();
    const conditions = [
      { variableSelector: ["other", "item"], operator: "empty" },
      { variableSelector: ["start", "items"], operator: "empty" },
    ];
    const input = props(
      kind === "loop"
        ? {
            kind,
            config: { maxIterations: 5, until: { logic: "and", conditions } },
          }
        : { kind, config: { cases: [{ id: "A", logic: "and", conditions }] } },
    );
    render(<RunNodeConfigForm {...input} />);
    if (kind === "loop")
      fireEvent.change(screen.getByRole("spinbutton"), {
        target: { value: "8" },
      });
    else {
      await user.click(screen.getByLabelText(/Branch 1 logic/));
      await user.click(await screen.findByRole("option", { name: /OR/ }));
    }
    await user.click(submit());
    expect(screen.getByRole("alert")).toHaveTextContent(
      appI18n.t("settings.workflow.field.variable", { index: 1 }),
    );
    expect(input.onSubmit).not.toHaveBeenCalled();
  },
);

it("keeps the rule structure fixed and emits only the existing case logic leaf", async () => {
  const user = userEvent.setup();
  const conditions = [0, 1].map(() => ({
    variableSelector: ["start", "items"],
    operator: "empty",
  }));
  const input = props({
    kind: "condition",
    config: { cases: [{ id: "A", logic: "and", conditions }] },
  });
  const view = render(<RunNodeConfigForm {...input} />);
  expect(screen.getByRole("button", { name: "Add condition" })).toBeDisabled();
  await user.click(screen.getByLabelText(/Branch 1 logic/));
  await user.click(await screen.findByRole("option", { name: /OR/ }));
  await user.click(submit());
  expect(input.onSubmit.mock.calls[0]?.[0].patch).toEqual([
    { op: "replace", path: "/cases/0/logic", value: "or" },
  ]);
  view.rerender(
    <RunNodeConfigForm
      {...input}
      effective={{
        kind: "condition",
        config: { cases: defaults.condition.data.cases },
      }}
    />,
  );
  expect(submit()).toBeDisabled();
  expect(screen.getByLabelText(/Branch 1 logic/)).toHaveTextContent(
    defaults.condition.default.toUpperCase(),
  );
  await user.click(screen.getByLabelText(/Branch 1 logic/));
  await user.click(await screen.findByRole("option", { name: /OR/ }));
  await user.click(submit());
  expect(input.onSubmit.mock.calls[1]?.[0]).toEqual(defaults.condition.preview);
});

it("edits an existing carried variable's type-compatible feedback", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "loop",
    config: {
      until: { logic: "and", conditions: [] },
      maxIterations: 7,
      variables: [
        {
          name: "count",
          valueType: "number",
          initial: { kind: "constant", value: 0 },
          feedback: ["loop", "count"],
        },
      ],
    },
  });
  input.variables.push(
    variable("loop", "count", "number"),
    variable("body", "next", "integer"),
  );
  render(<RunNodeConfigForm {...input} />);
  await user.click(screen.getByLabelText("Feedback for count"));
  const available = await screen.findByRole("option", { name: "body.next" });
  expect(
    screen.queryByRole("option", { name: "start.items" }),
  ).not.toBeInTheDocument();
  await user.click(available);
  await user.click(submit());
  expect(input.onSubmit.mock.calls[0]?.[0].patch).toEqual([
    {
      op: "replace",
      path: "/loopConfig/variables/0/feedback",
      value: ["body", "next"],
    },
  ]);
});

it("filters nested arrays and edits the iteration failure strategy", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "iteration",
    config: defaults.iteration.data.iterationConfig,
  });
  input.variables.push({
    ...variable("body", "items", "array"),
    selector: ["body", "structured_output", "items"],
  });
  render(<RunNodeConfigForm {...input} />);
  await user.click(screen.getByLabelText("Iterator source"));
  await screen.findByRole("option", { name: "start.items" });
  expect(
    screen
      .getAllByRole("option")
      .map((option) => option.getAttribute("aria-label"))
      .sort(),
  ).toEqual(["start.items", "start.other"]);
  await user.keyboard("{Escape}");
  const strategy = screen.getByLabelText(
    appI18n.t("settings.workflow.field.iterationErrorStrategy"),
  );
  expect(strategy).toHaveTextContent(
    appI18n.t(
      `settings.workflow.iteration.${defaults.iteration.default}Strategy`,
    ),
  );
  await user.click(strategy);
  await user.click(
    await screen.findByRole("option", {
      name: appI18n.t("settings.workflow.iteration.continueStrategy"),
    }),
  );
  await user.click(submit());
  expect(input.onSubmit.mock.calls[0]?.[0]).toEqual(defaults.iteration.preview);
  await user.click(
    screen.getByLabelText(
      appI18n.t("settings.workflow.field.iterationErrorStrategy"),
    ),
  );
  await user.click(
    await screen.findByRole("option", {
      name: appI18n.t("settings.workflow.iteration.failStrategy"),
    }),
  );
  expect(submit()).toBeDisabled();
  await user.click(submit());
  expect(input.onSubmit).toHaveBeenCalledTimes(1);
});

it.each([
  {
    selector: ["start", "items"],
    operator: "greater_than",
    value: 5,
    field: "operator",
  },
  {
    selector: ["start", "count"],
    operator: "greater_than",
    value: "{{#start.items#}}",
    field: "value",
  },
] as const)("rejects an incompatible $field before preview", async (row) => {
  const input = props({
    kind: "loop",
    config: {
      until: {
        logic: "and",
        conditions: [
          {
            variableSelector: [...row.selector],
            operator: row.operator,
            value: row.value,
          },
        ],
      },
      maxIterations: 5,
    },
  });
  input.variables.push(variable("start", "count", "number"));
  render(<RunNodeConfigForm {...input} />);
  fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "7" } });
  await userEvent.setup().click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent(
    appI18n.t(`settings.workflow.field.${row.field}`, { index: 1 }),
  );
  expect(input.onSubmit).not.toHaveBeenCalled();
});

it("rejects a changed model removed from the live catalog", async () => {
  const user = userEvent.setup();
  const input = props({
    kind: "agent",
    config: { prompt: "Test", executor: { agentCli: "cli", modelId: "one" } },
  });
  const view = render(<RunNodeConfigForm {...input} />);
  await user.click(
    screen.getByLabelText(appI18n.t("settings.workflow.field.agentModel")),
  );
  await user.click(await screen.findByRole("option", { name: "Two" }));
  view.rerender(
    <RunNodeConfigForm
      {...input}
      capabilities={{ ...input.capabilities, agentModels: [] }}
    />,
  );
  await user.click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent(
    appI18n.t("settings.workflow.field.agentModel"),
  );
  expect(input.onSubmit).not.toHaveBeenCalled();
});

it("discards dirty fields when the same execution receives changed effective config", async () => {
  const input = props({
    kind: "loop",
    config: { maxIterations: 5, until: { logic: "and", conditions: [] } },
  });
  const view = render(<RunNodeConfigForm {...input} />);
  fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "9" } });
  expect(submit()).toBeEnabled();
  view.rerender(
    <RunNodeConfigForm
      {...input}
      effective={{
        kind: "loop",
        config: { maxIterations: 7, until: { logic: "and", conditions: [] } },
      }}
    />,
  );
  expect(screen.getByRole("spinbutton")).toHaveValue(7);
  expect(submit()).toBeDisabled();
});

it("serializes an edited variable and a cleared operand as distinct replace operations", async () => {
  const input = props({
    kind: "condition",
    config: {
      cases: [
        {
          id: "case-1",
          conditions: [
            {
              variableSelector: ["start", "items"],
              operator: "contains",
              value: "first",
            },
          ],
        },
      ],
    },
  });
  const user = userEvent.setup();
  render(<RunNodeConfigForm {...input} />);
  await user.click(screen.getByLabelText("Variable 1"));
  await user.click(await screen.findByRole("option", { name: "start.other" }));
  await user.click(screen.getByLabelText("Condition 1"));
  await user.click(await screen.findByRole("option", { name: "Is empty" }));
  await user.click(submit());
  expect(
    JSON.parse(JSON.stringify(input.onSubmit.mock.calls[0]?.[0].patch)),
  ).toEqual([
    {
      op: "replace",
      path: "/cases/0/conditions/0/variableSelector",
      value: ["start", "other"],
    },
    { op: "replace", path: "/cases/0/conditions/0/operator", value: "empty" },
    { op: "replace", path: "/cases/0/conditions/0/value", value: null },
  ]);
});

it("blocks a preview with incompatible existing feedback", async () => {
  const input = props({
    kind: "loop",
    config: {
      maxIterations: 5,
      until: { logic: "and", conditions: [] },
      variables: [
        {
          name: "count",
          valueType: "number",
          initial: { kind: "constant", value: 0 },
          feedback: ["start", "items"],
        },
      ],
    },
  });
  render(<RunNodeConfigForm {...input} />);
  fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "7" } });
  await userEvent.setup().click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent("Feedback for count");
  expect(input.onSubmit).not.toHaveBeenCalled();
});

it("describes the invalid field in the second branch by its displayed position", async () => {
  const rule = { variableSelector: ["missing", "item"], operator: "empty" };
  const input = props({
    kind: "condition",
    config: {
      cases: [
        { id: "internal-if", conditions: [] },
        { id: "internal-other", conditions: [rule] },
      ],
    },
  });
  render(<RunNodeConfigForm {...input} />);
  const field = screen.getByLabelText("Variable 1");
  expect(field).toHaveAttribute("aria-invalid", "true");
  expect(field).toHaveAccessibleDescription(/ELIF 1 · Variable 1/);
  const user = userEvent.setup();
  await user.click(screen.getByLabelText("Condition 1"));
  await user.click(await screen.findByRole("option", { name: "Is not empty" }));
  await user.click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent("ELIF 1 · Variable 1");
  expect(input.onSubmit).not.toHaveBeenCalled();
});

it("blocks a strategy edit when the iteration source is unavailable", async () => {
  const input = props({
    kind: "iteration",
    config: {
      iteratorSelector: ["outside", "items"],
      collectSelector: ["member", "output"],
    },
  });
  render(<RunNodeConfigForm {...input} />);
  expect(screen.getByLabelText("Iterator source")).toHaveAttribute(
    "aria-invalid",
    "true",
  );
  const user = userEvent.setup();
  await user.click(
    screen.getByLabelText(
      appI18n.t("settings.workflow.field.iterationErrorStrategy"),
    ),
  );
  await user.click(
    await screen.findByRole("option", {
      name: appI18n.t("settings.workflow.iteration.continueStrategy"),
    }),
  );
  await user.click(submit());
  expect(screen.getByRole("alert")).toHaveTextContent("Iterator source");
  expect(input.onSubmit).not.toHaveBeenCalled();
});

it("accepts file-array feedback for object arrays and only previews the changed limit", async () => {
  const input = props({
    kind: "loop",
    config: {
      maxIterations: 5,
      until: { logic: "and", conditions: [] },
      variables: [
        {
          name: "refs",
          valueType: "array[object]",
          initial: { kind: "constant", value: [] },
          feedback: ["start", "files"],
        },
      ],
    },
  });
  input.variables.push(variable("start", "files", "array[file]"));
  render(<RunNodeConfigForm {...input} />);
  fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "7" } });
  await userEvent.setup().click(submit());
  expect(input.onSubmit.mock.calls[0]?.[0].patch).toEqual([
    { op: "replace", path: "/loopConfig/maxIterations", value: 7 },
  ]);
});

it.each([0, 101, 1.5])(
  "rejects a changed exit rule when the effective limit is %s",
  async (maxIterations) => {
    const input = props({
      kind: "loop",
      config: {
        maxIterations,
        until: {
          logic: "and",
          conditions: [
            { variableSelector: ["start", "items"], operator: "empty" },
          ],
        },
      },
    });
    render(<RunNodeConfigForm {...input} />);
    const user = userEvent.setup();
    await user.click(screen.getByLabelText("Condition 1"));
    await user.click(
      await screen.findByRole("option", { name: "Is not empty" }),
    );
    expect(screen.getByRole("spinbutton")).toBeInvalid();
    // Exercise the form's own submit guard independently of native number constraints.
    fireEvent.submit(submit().closest("form")!);
    expect(screen.getByRole("alert")).toHaveTextContent(
      appI18n.t("settings.workflow.field.maxIterations"),
    );
    expect(input.onSubmit).not.toHaveBeenCalled();
  },
);

it("submits the selected undispatched logical position from the preview contract", async () => {
  const input = props({
    kind: "agent",
    config: { prompt: "old", executor: { agentCli: "cli", modelId: "one" } },
  });
  const pendingTarget = { rootScopeId: "rootP", scopeId: "rootP", nodeId: "X" };
  render(
    <RunNodeConfigForm
      {...input}
      target={{ runId: "P", pendingTarget }}
      baseRevision={0}
      variables={[variable("start", "text", "string")]}
    />,
  );
  const user = userEvent.setup();
  const prompt = screen.getByLabelText("Custom prompt");
  await user.clear(prompt);
  await user.click(prompt);
  await user.paste("复述 {{#start.text#}}");
  await user.click(submit());
  expect(input.onSubmit.mock.calls).toEqual([
    [
      {
        runId: "P",
        pendingTarget,
        baseRevision: 0,
        patch: [
          {
            op: "replace",
            path: "/agentConfig/prompt",
            value: "复述 {{#start.text#}}",
          },
        ],
      },
    ],
  ]);
});
