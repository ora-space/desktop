import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ComponentProps } from "react";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowDraftSaveStatusLabel } from "./workflow-draft-save-status";

/** Renders the save-status label under the same i18n provider the editor uses. */
function renderLabel(
  props: ComponentProps<typeof WorkflowDraftSaveStatusLabel>,
) {
  return render(
    <AppI18nProvider>
      <WorkflowDraftSaveStatusLabel {...props} />
    </AppI18nProvider>,
  );
}

describe("WorkflowDraftSaveStatusLabel", () => {
  it("shows saving and live-saved copy for the matching autosave states", () => {
    const { rerender } = renderLabel({ status: "saving" });
    expect(screen.getByText("保存中…")).toBeInTheDocument();

    rerender(
      <AppI18nProvider>
        <WorkflowDraftSaveStatusLabel
          status="clean"
          draftUpdatedAt="8月7日 15:42:15"
        />
      </AppI18nProvider>,
    );
    expect(
      screen.getByText("已实时保存 最近修改时间：8月7日 15:42:15"),
    ).toBeInTheDocument();
  });

  it("keeps the last live-saved timestamp while edits are still dirty", () => {
    renderLabel({ status: "dirty", draftUpdatedAt: "8月7日 15:42:15" });
    expect(
      screen.getByText("已实时保存 最近修改时间：8月7日 15:42:15"),
    ).toBeInTheDocument();
  });

  it("surfaces save failures distinctly from the live-saved label", () => {
    renderLabel({ status: "error", draftUpdatedAt: "8月7日 15:42:15" });
    expect(screen.getByText("保存工作流失败。")).toBeInTheDocument();
    expect(screen.getByText("保存工作流失败。")).toHaveClass(
      "text-destructive",
    );
  });
});
