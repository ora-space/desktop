import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WorkflowSurfaceBoundary } from "./workflow-surface-boundary";
import { appI18n } from "../../i18n/i18n-instance";

/** Models an unexpected workflow rendering failure beyond the guarded JSON fields. */
function BrokenWorkflow(): never {
  throw new Error("Malformed composite configuration");
}

describe("workflow surface recovery", () => {
  it("retries a failed editor only when its owner requests recovery", async () => {
    await appI18n.changeLanguage("en-US");
    const onRenderFailure = vi.fn();
    const view = render(
      <WorkflowSurfaceBoundary
        scopeId="editor"
        recoveryRevision={0}
        onRenderFailure={onRenderFailure}
        onClose={vi.fn()}
      >
        <BrokenWorkflow />
      </WorkflowSurfaceBoundary>,
      { onCaughtError: vi.fn() },
    );
    expect(onRenderFailure).toHaveBeenCalledOnce();
    view.rerender(
      <WorkflowSurfaceBoundary
        scopeId="editor"
        recoveryRevision={0}
        onRenderFailure={onRenderFailure}
        onClose={vi.fn()}
      >
        <p>Healthy workflow</p>
      </WorkflowSurfaceBoundary>,
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();
    expect(screen.queryByText("Healthy workflow")).not.toBeInTheDocument();
    view.rerender(
      <WorkflowSurfaceBoundary
        scopeId="editor"
        recoveryRevision={1}
        onRenderFailure={onRenderFailure}
        onClose={vi.fn()}
      >
        <p>Healthy workflow</p>
      </WorkflowSurfaceBoundary>,
    );
    expect(screen.getByText("Healthy workflow")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(onRenderFailure).toHaveBeenCalledOnce();
  });

  it.each(["en-US", "zh-CN"])(
    "keeps navigation and recovery usable after a workflow throws (%s)",
    async (locale) => {
      await appI18n.changeLanguage(locale);
      const onNavigate = vi.fn();
      const onClose = vi.fn();
      const onCaughtError = vi.fn();
      const user = userEvent.setup();
      const view = render(
        <>
          <button onClick={onNavigate}>Other session</button>
          <WorkflowSurfaceBoundary scopeId="broken" onClose={onClose}>
            <BrokenWorkflow />
          </WorkflowSurfaceBoundary>
        </>,
        { onCaughtError },
      );
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(onCaughtError).toHaveBeenCalled();
      await user.click(screen.getByRole("button", { name: "Other session" }));
      await user.click(
        screen.getByRole("button", { name: /Return to workspace|返回工作区/ }),
      );
      expect(onNavigate).toHaveBeenCalledOnce();
      expect(onClose).toHaveBeenCalledOnce();
      view.rerender(
        <WorkflowSurfaceBoundary scopeId="healthy" onClose={onClose}>
          <p>Healthy workflow</p>
        </WorkflowSurfaceBoundary>,
      );
      expect(screen.getByText("Healthy workflow")).toBeInTheDocument();
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    },
  );
});
