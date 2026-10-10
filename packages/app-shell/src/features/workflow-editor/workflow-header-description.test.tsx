import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowHeaderDescription } from "./workflow-header-description";

void appI18n;

describe("WorkflowHeaderDescription", () => {
  it("shows the truncated line and reveals the full copy on hover", async () => {
    const user = userEvent.setup();
    const text =
      "8×6 网格 + 三层嵌套迭代 + 远处节点。缩放到约 30% 后拖中间卡片，测视口外是否仍参与渲染。";
    render(
      <AppI18nProvider>
        <WorkflowHeaderDescription text={text} />
      </AppI18nProvider>,
    );

    expect(screen.getByLabelText("工作流描述")).toHaveTextContent(text);
    expect(screen.queryByRole("tooltip", { hidden: true })).toBeNull();

    await user.hover(screen.getByLabelText("工作流描述"));
    expect(
      await screen.findByRole("tooltip", { hidden: true }),
    ).toHaveTextContent(text);
  });
});
