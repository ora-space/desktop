import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockWorkflowCapabilities } from "@ora/workflow-mock";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowNodeCatalog } from "./workflow-node-catalog";

const capabilities = createMockWorkflowCapabilities("en-US");

describe("WorkflowNodeCatalog", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("maps wheel movement to horizontal scrolling without moving the canvas", () => {
    render(
      <AppI18nProvider>
        <WorkflowNodeCatalog
          capabilities={capabilities}
          hasStartNode={false}
          onAdd={vi.fn()}
          onDrop={vi.fn()}
        />
      </AppI18nProvider>,
    );
    const catalog = screen.getByRole("toolbar");
    const viewport = catalog.querySelector<HTMLElement>(
      "[data-workflow-node-scroll]",
    );
    expect(viewport).not.toBeNull();
    configureScrollViewport(viewport!, {
      clientWidth: 240,
      scrollWidth: 600,
      scrollLeft: 40,
    });

    const event = new WheelEvent("wheel", {
      bubbles: true,
      cancelable: true,
      deltaY: 80,
    });
    const propagationSpy = vi.spyOn(event, "stopPropagation");
    fireEvent(catalog, event);

    expect(viewport!.scrollLeft).toBe(120);
    expect(propagationSpy).toHaveBeenCalled();
  });

  it("shows edge resistance even when every button already fits and returns after the wheel stops", () => {
    vi.useFakeTimers();
    render(
      <AppI18nProvider>
        <WorkflowNodeCatalog
          capabilities={capabilities}
          hasStartNode={false}
          onAdd={vi.fn()}
          onDrop={vi.fn()}
        />
      </AppI18nProvider>,
    );
    const catalog = screen.getByRole("toolbar");
    const viewport = catalog.querySelector<HTMLElement>(
      "[data-workflow-node-scroll]",
    );
    const track = catalog.querySelector<HTMLElement>(
      "[data-workflow-node-track]",
    );
    expect(viewport).not.toBeNull();
    expect(track).not.toBeNull();
    configureScrollViewport(viewport!, {
      clientWidth: 600,
      scrollWidth: 600,
      scrollLeft: 0,
    });

    fireEvent.wheel(catalog, { deltaY: 200 });
    expect(viewport!.scrollLeft).toBe(0);
    expect(track!.style.transform).toBe("translate3d(-18px, 0, 0)");

    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(track!.style.transform).toBe("translate3d(0px, 0, 0)");

    viewport!.scrollLeft = 0;
    fireEvent.wheel(catalog, { deltaY: -200 });
    expect(viewport!.scrollLeft).toBe(0);
    expect(track!.style.transform).toBe("translate3d(18px, 0, 0)");
  });

  it("keeps the start entry visible but disabled when the canvas already has one", () => {
    render(
      <AppI18nProvider>
        <WorkflowNodeCatalog
          capabilities={capabilities}
          hasStartNode
          onAdd={vi.fn()}
          onDrop={vi.fn()}
        />
      </AppI18nProvider>,
    );

    expect(screen.getByRole("button", { name: "Start" })).toBeDisabled();
  });
});

/** Defines read-only layout metrics that JSDOM does not calculate. */
function configureScrollViewport(
  viewport: HTMLElement,
  dimensions: {
    clientWidth: number;
    scrollWidth: number;
    scrollLeft: number;
  },
): void {
  Object.defineProperties(viewport, {
    clientWidth: { configurable: true, value: dimensions.clientWidth },
    scrollWidth: { configurable: true, value: dimensions.scrollWidth },
    scrollLeft: {
      configurable: true,
      writable: true,
      value: dimensions.scrollLeft,
    },
  });
}
