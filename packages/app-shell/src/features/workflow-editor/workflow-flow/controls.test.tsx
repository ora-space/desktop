import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../../i18n/i18n";
import { appI18n } from "../../../i18n/i18n-instance";
import { WorkflowCanvasControls, WorkflowScrollSelectedNode } from "./controls";
import {
  collapsedIterationAncestorIds,
  unionScreenBox,
  viewportDeltaToCenter,
} from "./scroll-geometry";

const { fitView, setViewport, zoomTo, flow } = vi.hoisted(() => ({
  fitView: vi.fn(),
  setViewport: vi.fn(),
  zoomTo: vi.fn(),
  flow: {
    nodes: [] as { id: string; selected?: boolean }[],
    getViewport: vi.fn(() => ({ x: 0, y: 0, zoom: 1 })),
    addSelectedNodes: vi.fn(),
  },
}));

vi.mock("@xyflow/react", () => ({
  useReactFlow: () => ({
    fitView,
    setViewport,
    zoomTo,
    getViewport: flow.getViewport,
  }),
  useStore: (selector: (state: { nodes: typeof flow.nodes }) => unknown) =>
    selector({ nodes: flow.nodes }),
  useStoreApi: () => ({
    getState: () => ({ addSelectedNodes: flow.addSelectedNodes }),
  }),
  useViewport: () => ({ zoom: 1 }),
}));

describe("workflow canvas viewport controls", () => {
  beforeEach(async () => {
    await appI18n.changeLanguage("en-US");
    fitView.mockReset();
    setViewport.mockReset();
    zoomTo.mockReset();
    flow.nodes = [];
    flow.getViewport.mockReset();
    flow.getViewport.mockReturnValue({ x: 10, y: 20, zoom: 0.8 });
    flow.addSelectedNodes.mockReset();
    document.body.replaceChildren();
  });

  it("uses the compact Dify-style bottom-right control group", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <WorkflowCanvasControls defaultViewport={{ x: 12, y: 20, zoom: 1 }} />
      </AppI18nProvider>,
    );

    const toolbar = screen.getByRole("toolbar", {
      name: "Canvas view controls",
    });
    expect(toolbar).toHaveAttribute("data-workflow-viewport-controls");
    expect(toolbar).toHaveClass(
      "absolute",
      "bottom-3",
      "right-3",
      "z-40",
      "rounded-xl",
      "p-1",
      "shadow-lg",
    );
    await user.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(zoomTo).toHaveBeenCalledWith(0.9);

    await user.click(
      screen.getByRole("button", { name: "Fit workflow to view" }),
    );
    expect(fitView).toHaveBeenCalledWith({
      duration: 220,
      maxZoom: 1,
      minZoom: 0.4,
      padding: 0.16,
    });

    await user.click(screen.getByRole("button", { name: "Reset canvas view" }));
    expect(setViewport).toHaveBeenCalledWith(
      { x: 12, y: 20, zoom: 1 },
      { duration: 180 },
    );
  });

  it("pans by the on-screen distance from the card to the canvas center", async () => {
    const user = userEvent.setup();
    flow.nodes = [{ id: "agent-1", selected: true }];
    const pane = document.createElement("div");
    pane.className = "workflow-flow";
    const card = document.createElement("div");
    card.dataset.workflowNodeId = "agent-1";
    pane.appendChild(card);
    document.body.appendChild(pane);
    vi.spyOn(pane, "getBoundingClientRect").mockReturnValue(
      domRect(0, 0, 800, 600),
    );
    vi.spyOn(card, "getBoundingClientRect").mockReturnValue(
      domRect(1000, -200, 200, 100),
    );
    render(
      <AppI18nProvider>
        <WorkflowScrollSelectedNode />
      </AppI18nProvider>,
    );

    await user.click(
      screen.getByRole("button", { name: "Scroll to selected node" }),
    );
    expect(setViewport).toHaveBeenCalledWith(
      { x: 10 - 700, y: 20 + 450, zoom: 0.8 },
      { duration: 220 },
    );
  });

  it("asks for a hidden card to be revealed, then pans once it is measurable", async () => {
    const user = userEvent.setup();
    flow.nodes = [{ id: "agent-1", selected: true }];
    const pane = document.createElement("div");
    pane.className = "workflow-flow";
    document.body.appendChild(pane);
    vi.spyOn(pane, "getBoundingClientRect").mockReturnValue(
      domRect(0, 0, 800, 600),
    );
    const reveal = vi.fn(() => {
      const card = document.createElement("div");
      card.dataset.workflowNodeId = "agent-1";
      pane.appendChild(card);
      vi.spyOn(card, "getBoundingClientRect").mockReturnValue(
        domRect(1000, -200, 200, 100),
      );
    });
    const realFrame = window.requestAnimationFrame;
    let frames = 0;
    window.requestAnimationFrame = ((callback: FrameRequestCallback) => {
      frames += 1;
      if (frames <= 4) {
        callback(0);
      }
      return frames;
    }) as typeof window.requestAnimationFrame;
    render(
      <AppI18nProvider>
        <WorkflowScrollSelectedNode onRevealHiddenSelection={reveal} />
      </AppI18nProvider>,
    );

    await user.click(
      screen.getByRole("button", { name: "Scroll to selected node" }),
    );
    window.requestAnimationFrame = realFrame;
    expect(reveal).toHaveBeenCalledWith(["agent-1"]);
    expect(setViewport).toHaveBeenCalledWith(
      { x: 10 - 700, y: 20 + 450, zoom: 0.8 },
      { duration: 220 },
    );
  });

  it("keeps the last node after the selection is cleared", async () => {
    const user = userEvent.setup();
    flow.nodes = [{ id: "agent-1", selected: true }];
    const pane = document.createElement("div");
    pane.className = "workflow-flow";
    const card = document.createElement("div");
    card.dataset.workflowNodeId = "agent-1";
    pane.appendChild(card);
    document.body.appendChild(pane);
    vi.spyOn(pane, "getBoundingClientRect").mockReturnValue(
      domRect(0, 0, 800, 600),
    );
    vi.spyOn(card, "getBoundingClientRect").mockReturnValue(
      domRect(1000, -200, 200, 100),
    );
    const view = render(
      <AppI18nProvider>
        <WorkflowScrollSelectedNode />
      </AppI18nProvider>,
    );
    flow.nodes = [];
    view.rerender(
      <AppI18nProvider>
        <WorkflowScrollSelectedNode />
      </AppI18nProvider>,
    );

    const button = screen.getByRole("button", {
      name: "Scroll to selected node",
    });
    expect(button).toBeEnabled();
    await user.click(button);
    expect(flow.addSelectedNodes).toHaveBeenCalledWith(["agent-1"]);
    expect(setViewport).toHaveBeenCalledWith(
      { x: 10 - 700, y: 20 + 450, zoom: 0.8 },
      { duration: 220 },
    );
  });

  it("stays disabled until a node is selected", async () => {
    const user = userEvent.setup();
    render(
      <AppI18nProvider>
        <WorkflowScrollSelectedNode />
      </AppI18nProvider>,
    );

    const button = screen.getByRole("button", {
      name: "Scroll to selected node",
    });
    expect(button).toBeDisabled();
    await user.click(button);
    expect(setViewport).not.toHaveBeenCalled();
  });
});

describe("scroll-to-selected screen geometry", () => {
  it("centers the union of the selected cards in the pane", () => {
    expect(
      viewportDeltaToCenter(
        { left: 0, top: 0, width: 800, height: 600 },
        { left: 1000, top: -200, width: 200, height: 100 },
      ),
    ).toEqual({ x: -700, y: 450 });
    expect(
      unionScreenBox([
        { left: 0, top: 10, width: 40, height: 20 },
        { left: 80, top: 0, width: 20, height: 10 },
      ]),
    ).toEqual({ left: 0, top: 0, width: 100, height: 30 });
    expect(unionScreenBox([])).toBeNull();
  });

  it("lists collapsed iteration frames that hide the selected cards", () => {
    const nodes = [
      {
        id: "iteration-1",
        data: { kind: "iteration", collapsed: true },
      },
      {
        id: "iteration-2",
        parentId: "iteration-1",
        data: { kind: "iteration", collapsed: false },
      },
      {
        id: "agent-1",
        parentId: "iteration-2",
        data: { kind: "agent" },
      },
    ];
    expect(collapsedIterationAncestorIds(["agent-1"], nodes)).toEqual([
      "iteration-1",
    ]);
    expect(collapsedIterationAncestorIds(["iteration-1"], nodes)).toEqual([]);
  });
});

function domRect(
  left: number,
  top: number,
  width: number,
  height: number,
): DOMRect {
  return {
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    x: left,
    y: top,
    toJSON: () => ({}),
  };
}
