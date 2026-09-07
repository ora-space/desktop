import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { WorkflowRunStartDialog } from "./workflow-run-start-dialog";

describe("WorkflowRunStartDialog", () => {
  it("collects and parses Start variables before starting the run", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    const onStart = vi.fn(async () => undefined);

    render(
      <AppI18nProvider>
        <WorkflowRunStartDialog
          open
          runName="审查流程 1"
          initialPrompt="请先审查这次的改动"
          variables={[
            {
              name: "topic",
              displayName: "主题",
              valueType: "string",
              maxLength: 4,
              value: "现有主题",
            },
            { name: "limit", valueType: "integer" },
          ]}
          busy={false}
          onOpenChange={vi.fn()}
          onStart={onStart}
        />
      </AppI18nProvider>,
    );

    expect(
      screen.getByRole("heading", { name: "审查流程 1" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("主题")).toHaveValue("现有主题");
    expect(screen.getByLabelText("主题")).toHaveAttribute("maxlength", "4");
    await user.type(screen.getByLabelText("limit"), "1.5");
    await user.click(screen.getByRole("button", { name: "启动" }));
    expect(onStart).not.toHaveBeenCalled();
    expect(screen.getByText("值与 integer 类型不匹配。")).toBeInTheDocument();

    await user.clear(screen.getByLabelText("limit"));
    await user.type(screen.getByLabelText("limit"), "2");
    await user.click(screen.getByRole("button", { name: "启动" }));
    expect(onStart).toHaveBeenCalledWith("请先审查这次的改动", {
      topic: "现有主题",
      limit: 2,
    });
  });

  it("allows an unset deployment value to remain empty", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    const onStart = vi.fn(async () => undefined);

    render(
      <AppI18nProvider>
        <WorkflowRunStartDialog
          open
          runName="空输入流程"
          initialPrompt=""
          variables={[{ name: "optional_note", valueType: "string" }]}
          busy={false}
          onOpenChange={vi.fn()}
          onStart={onStart}
        />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: "启动" }));
    expect(onStart).toHaveBeenCalledWith("", { optional_note: null });
  });

  it("requires configured fields before a deployed run starts", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    const onStart = vi.fn(async () => undefined);

    render(
      <AppI18nProvider>
        <WorkflowRunStartDialog
          open
          runName="必填输入流程"
          initialPrompt="开始执行"
          variables={[
            {
              name: "brief",
              displayName: "需求说明",
              fieldType: "paragraph",
              valueType: "string",
              required: true,
            },
          ]}
          busy={false}
          onOpenChange={vi.fn()}
          onStart={onStart}
        />
      </AppI18nProvider>,
    );

    await user.click(screen.getByRole("button", { name: "启动" }));
    expect(onStart).not.toHaveBeenCalled();
    expect(screen.getByText("请填写此项。")).toBeInTheDocument();

    await user.type(screen.getByLabelText("需求说明"), "实现新的输入表单");
    await user.click(screen.getByRole("button", { name: "启动" }));
    expect(onStart).toHaveBeenCalledWith("开始执行", {
      brief: "实现新的输入表单",
    });
  });
});
