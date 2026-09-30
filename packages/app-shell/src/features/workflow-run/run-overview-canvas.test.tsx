import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { GraphWorkflowRun } from "@ora/workflow-runtime";
import { parseWorkflowGraph } from "@ora/workflow-runtime";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { RunOverviewCanvas } from "./run-overview-canvas";
import { WorkflowMembershipProvider } from "../workflow-node-chrome";

const run: GraphWorkflowRun = {
  id: "run",
  projectId: "project",
  definitionId: "definition",
  definitionSnapshot: {
    id: "snapshot",
    name: "Iteration run",
    description: "",
    updatedAt: "2026-09-17T12:00:00+08:00",
    viewport: { x: 0, y: 0, zoom: 1 },
    nodes: [
      {
        id: "iter",
        type: "workflow",
        position: { x: 100, y: 80 },
        initialWidth: 720,
        initialHeight: 420,
        data: { kind: "iteration", title: "Iteration", description: "" },
      },
      {
        id: "body-a",
        type: "workflow",
        parentId: "iter",
        position: { x: 120, y: 100 },
        data: { kind: "agent", title: "Agent A", description: "" },
      },
      {
        id: "body-b",
        type: "workflow",
        parentId: "iter",
        position: { x: 120, y: 250 },
        data: { kind: "agent", title: "Agent B", description: "" },
      },
    ],
    edges: [
      {
        id: "entry-a",
        source: "iter",
        sourceHandle: "iteration-entry",
        target: "body-a",
      },
      {
        id: "entry-b",
        source: "iter",
        sourceHandle: "iteration-entry",
        target: "body-b",
      },
    ],
  },
  name: "Iteration run",
  status: "succeeded",
  nodeStates: {
    iter: { status: "succeeded" },
    "body-a": { status: "succeeded", iteration: 1 },
    "body-b": { status: "succeeded", iteration: 1 },
  },
  openHitls: [],
  createdAt: "2026-09-17T12:00:00+08:00",
  updatedAt: "2026-09-17T12:00:10+08:00",
};

describe("run overview canvas", () => {
  it.each(["loop", "iteration"])(
    "keeps recovered %s children distinct and detached nodes selectable",
    async (kind) => {
      const graph = JSON.stringify({
        nodes: [
          { id: "container", data: { kind, title: "Container" } },
          ...["First", "Second", "Third"].map((title) => ({
            id: title,
            ...(kind === "iteration" ? { parentId: "container" } : {}),
            data: {
              kind: "agent",
              title,
              ...(kind === "loop" ? { containerId: "container" } : {}),
            },
          })),
          {
            id: "orphan",
            parentId: "absent",
            extent: "parent",
            data: { kind: "agent", title: "Orphan", containerId: "absent" },
          },
        ],
      });
      const importedRun = {
        ...run,
        definitionSnapshot: {
          ...run.definitionSnapshot,
          ...parseWorkflowGraph(graph),
        },
        nodeStates: {},
      };
      const onFocusNode = vi.fn();
      render(
        <AppI18nProvider>
          <div style={{ width: 1200, height: 800 }}>
            <RunOverviewCanvas
              run={importedRun}
              focusedNodeId={null}
              onFocusNode={onFocusNode}
            />
          </div>
        </AppI18nProvider>,
      );
      const cards = await Promise.all(
        ["First", "Second", "Third"].map((title) =>
          screen.findByLabelText(new RegExp(`^${title}:`)),
        ),
      );
      expect(
        new Set(
          cards.map(
            (card) =>
              card.closest<HTMLElement>(".react-flow__node")?.style.transform,
          ),
        ).size,
      ).toBe(3);
      for (const card of [
        ...cards,
        await screen.findByLabelText("Orphan: Idle"),
      ])
        fireEvent.click(card);
      expect(onFocusNode.mock.calls).toEqual([
        ["First"],
        ["Second"],
        ["Third"],
        ["orphan"],
      ]);
    },
  );

  it("opens a generated snapshot without layout fields and keeps zoom controls usable", async () => {
    await appI18n.changeLanguage("en-US");
    const graph = JSON.stringify({
      nodes: [
        {
          id: "start",
          data: { kind: "start", title: "Imported start", description: "" },
        },
        {
          id: "agent",
          data: { kind: "agent", title: "Imported agent", description: "" },
        },
      ],
      edges: [{ source: "start", target: "agent" }],
    });
    const importedRun: GraphWorkflowRun = {
      ...run,
      definitionSnapshot: {
        ...run.definitionSnapshot,
        ...parseWorkflowGraph(graph),
      },
      nodeStates: {},
    };
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <div style={{ width: 1200, height: 800 }}>
          <RunOverviewCanvas
            run={importedRun}
            focusedNodeId={null}
            onFocusNode={vi.fn()}
          />
        </div>
      </AppI18nProvider>,
    );
    expect(
      await screen.findByLabelText("Imported start: Idle"),
    ).toBeInTheDocument();
    expect(
      await screen.findByLabelText("Imported agent: Idle"),
    ).toBeInTheDocument();
    const controls = screen.getByRole("toolbar", {
      name: "Run canvas view controls",
    });
    const percentage = within(controls).getByText(/%$/);
    const before = percentage.textContent;
    await user.click(
      screen.getByRole("button", { name: "Zoom in run canvas" }),
    );
    await waitFor(() => expect(percentage.textContent).not.toBe(before));
  });

  it("shows excluded nodes independently of pending runtime status", async () => {
    await appI18n.changeLanguage("en-US");
    const unusedRun = { ...run, nodeStates: {} };
    render(
      <AppI18nProvider>
        <WorkflowMembershipProvider
          unusedNodeIds={["iter", "body-a", "body-b"]}
        >
          <div style={{ width: 1200, height: 800 }}>
            <RunOverviewCanvas
              run={unusedRun}
              focusedNodeId={null}
              onFocusNode={vi.fn()}
            />
          </div>
        </WorkflowMembershipProvider>
      </AppI18nProvider>,
    );
    await waitFor(() =>
      expect(screen.getAllByText("Excluded from execution")).toHaveLength(3),
    );
    expect(
      screen.getByLabelText("Iteration: Excluded from execution"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("Agent A: Excluded from execution"),
    ).toBeInTheDocument();
  });
  beforeEach(async () => {
    await appI18n.changeLanguage("en-US");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 1200,
    });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 800,
    });
  });

  it("keeps the iteration frame and internal entry handle after the run finishes", async () => {
    render(
      <AppI18nProvider>
        <div style={{ width: 1200, height: 800 }}>
          <RunOverviewCanvas
            run={run}
            focusedNodeId={null}
            onFocusNode={vi.fn()}
          />
        </div>
      </AppI18nProvider>,
    );

    expect(
      await screen.findByLabelText("Iteration: Succeeded"),
    ).toHaveAttribute("data-workflow-run-iteration-frame");
    expect(
      document.querySelector("[data-workflow-run-iteration-start]"),
    ).not.toBeNull();
  });

  it("offers explicit zoom controls in addition to wheel and pinch gestures", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <div style={{ width: 1200, height: 800 }}>
          <RunOverviewCanvas
            run={run}
            focusedNodeId={null}
            onFocusNode={vi.fn()}
          />
        </div>
      </AppI18nProvider>,
    );

    const zoomIn = await screen.findByRole("button", {
      name: "Zoom in run canvas",
    });
    const controls = screen.getByRole("toolbar", {
      name: "Run canvas view controls",
    });
    const percentage = within(controls).getByText(/%$/);
    const before = percentage.textContent;
    expect(zoomIn).toBeEnabled();
    await user.click(zoomIn);
    await waitFor(() => {
      expect(percentage.textContent).not.toBe(before);
    });
    expect(
      screen.getByRole("button", { name: "Zoom out run canvas" }),
    ).toBeEnabled();
  });
});
